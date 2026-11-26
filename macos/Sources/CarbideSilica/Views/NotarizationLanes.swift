import CarbideCore
import SwiftUI

/// Carbide's notarization browser.
///
/// The first sentence gives the state of the link: the time since the last confirmed
/// notarization against the usual interval. The rhythm strip shows the earlier intervals
/// and the wait now on one scale. The graph shows the entries that the node has: the best
/// chain is the straight line, and a side branch is a second line. Below the graph, each
/// row gives the committed heights by system, the observer's block that includes it, and
/// the check of each foreign proof root on the owning chain's own source.
struct NotarizationLanesCard: View {
    let currency: CurrencyDetail
    /// The name or ID the currency page was opened with.
    let route: String

    @Environment(AppModel.self) private var model
    @State private var subject: String?
    @State private var perspectives: [NotarizationPerspective] = []
    @State private var active = 0
    @State private var views: [Int: NotarizationBrowser] = [:]
    @State private var loading = false
    @State private var failure: String?
    @State private var saved: String?
    @State private var showJSON = false
    /// The earlier notarizations of the active view, newest first.
    @State private var records: [Int: [RecordPoint]] = [:]
    @State private var selected: UInt32?
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    var body: some View {
        let _ = (showsNames, showsDates)
        // A stack, not a Group: an empty Group has no view for `.task` to attach to.
        VStack(alignment: .leading, spacing: 12) {
            if subject != nil {
                Card("Notarizations", trailing: trailing) {
                    header
                    if let view = views[active] {
                        content(view)
                    } else if loading {
                        ProgressView("Checking notarizations…").frame(maxWidth: .infinity, minHeight: 80)
                    }
                    if let failure {
                        Note(text: failure, warning: true)
                    }
                }
            }
        }
        .task(id: "\(currency.currencyId ?? currency.name)|\(model.generation)|\(model.status?.chainId ?? "")") {
            await start()
        }
    }

    private var trailing: String? {
        guard let view = views[active], view.load == .ready else { return nil }
        return "\(view.rows.count) in the current view"
    }

    // MARK: Header

    @ViewBuilder
    private var header: some View {
        HStack {
            if perspectives.count > 1 {
                Picker("Perspective", selection: $active) {
                    ForEach(Array(perspectives.enumerated()), id: \.offset) { index, perspective in
                        Text("\(name(perspective.observerId, perspective.observerName)) view").tag(index)
                    }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .frame(maxWidth: 360)
            }
            if let perspective = perspectives[safe: active] {
                Text("The \(name(perspective.observerId, perspective.observerName)) node asked about \(name(perspective.subjectId, perspective.subjectName)).")
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if loading { ProgressView().controlSize(.small) }
            Button("Refresh") { Task { await load(active, force: true) } }
                .disabled(loading)
            Button("Bookmark This View") { bookmark() }
                .disabled(perspectives[safe: active] == nil)
                .help("Saves a notarization bookmark with this perspective")
        }
        .onChange(of: active) {
            selected = nil
            Task { await load(active, force: false) }
        }
        if let saved {
            Text(saved).font(.callout).foregroundStyle(.secondary)
        }
    }

    // MARK: Content

    @ViewBuilder
    private func content(_ view: NotarizationBrowser) -> some View {
        if let provenance = view.provenance {
            ProvenanceNote(provenance: provenance)
            if provenance.retained {
                Text("The app does not check the blocks of a stored view.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        switch view.load {
        case .unavailable:
            Note(text: view.message ?? "Notarization data unavailable", warning: true)
        case .noData:
            Text(view.message ?? "No notarization data returned").foregroundStyle(.secondary)
        case .ready:
            ready(view)
        }
    }

    @ViewBuilder
    private func ready(_ view: NotarizationBrowser) -> some View {
        let intervals = NotarizationMath.intervals(records[active] ?? [])
        let wait = blocksSinceConfirmed(view)
        let state = LinkState(wait: wait, intervals: intervals)
        let marks = NotarizationPicture.marks(view)
        Text(sentence(view, marks: marks, intervals: intervals, state: state))
            .font(.title3)
            .fixedSize(horizontal: false, vertical: true)
            .textSelection(.enabled)
        if let state {
            Text(state.rule).font(.callout).foregroundStyle(state.color)
        } else if view.perspective.local, view.rows.contains(where: \.confirmed) {
            Text("The app has no record of earlier notarizations to compare the wait with.")
                .font(.callout).foregroundStyle(.secondary)
        }
        if let confirmed = view.rows.first(where: \.confirmed) {
            confirmedLine(confirmed, view)
        }
        if !intervals.isEmpty || !view.rows.isEmpty {
            // The rhythm and the graph are one instrument: the scale of the rhythm starts
            // at the left edge of the lanes of the graph.
            DisplayPanel(padding: 0) {
                VStack(alignment: .leading, spacing: 0) {
                    if !intervals.isEmpty {
                        NotarizationRhythm(intervals: intervals, wait: wait, state: state)
                            .padding(.top, 12)
                        Text("Each dot is the number of \(name(view.perspective.observerId, view.perspective.observerName)) blocks between two earlier notarizations in sequence, for the last \(intervals.count). Each step to the right is ten times more.")
                            .font(Tone.caption).foregroundStyle(Tone.label)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.horizontal, 18).padding(.vertical, 10)
                    }
                    if !intervals.isEmpty && !view.rows.isEmpty {
                        PanelRule()
                    }
                    if !view.rows.isEmpty {
                        NotarizationGraph(
                            marks: marks,
                            observer: name(view.perspective.observerId, view.perspective.observerName),
                            other: otherName(view),
                            note: { note($0, view) },
                            selected: $selected)
                        PanelRule()
                        Text(caption(view)).font(Tone.caption).foregroundStyle(Tone.label)
                            .fixedSize(horizontal: false, vertical: true)
                            .padding(.horizontal, 18).padding(.vertical, 10)
                    }
                }
            }
        }
        if view.rows.isEmpty {
            Text("No notarizations in current view").foregroundStyle(.secondary)
        } else {
            VStack(alignment: .leading, spacing: 0) {
                let groups = NotarizationPicture.ordered(marks)
                ForEach(groups, id: \.lane) { group in
                    if groups.count > 1 {
                        Plate(
                            group.lane == 0 ? "Best chain" : groups.count > 2 ? "Side branch \(group.lane)" : "Side branch",
                            fill: group.lane == 0 ? nil : Tone.mined)
                            .padding(.top, 10).padding(.bottom, 4)
                    }
                    ForEach(group.rows, id: \.index) { row in
                        Rectangle().fill(Tone.rule).frame(height: 1)
                        NotarizationRowView(row: row, perspective: view.perspective, observerTip: observerTip(view))
                            .padding(.horizontal, 6)
                            .background(
                                selectedRows(marks).contains(row.index) ? Tone.present.opacity(0.1) : .clear,
                                in: Rectangle())
                    }
                }
            }
            if view.checksLimited {
                Note(text: "Some proof roots were not checked. The app checks at most 160 blocks for one view.", warning: true)
            }
            DisclosureGroup(isExpanded: $showJSON) {
                ScrollView(.horizontal) {
                    Text(view.json).font(.callout.monospaced()).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            } label: {
                HStack {
                    Text(showJSON ? "Hide" : "Show the notarization data from the node, without the rows")
                        .foregroundStyle(.secondary)
                    Spacer()
                    CopyButton(text: view.json)
                }
            }
        }
    }

    private func selectedRows(_ marks: [NotarizationMark]) -> Set<UInt32> {
        Set(marks.first { $0.id == selected }?.rows.map(\.index) ?? [])
    }

    private func confirmedLine(_ row: BrowserNotarization, _ view: NotarizationBrowser) -> some View {
        HStack(spacing: 10) {
            NameText("Last confirmed").frame(width: 170, alignment: .leading)
            if row.synthetic {
                Text("Synthetic proof root")
            } else if let txid = row.txid {
                if view.perspective.local {
                    RouteLink(text: txid, route: .transaction(txid: txid))
                } else {
                    Text(Format.short(txid)).font(.body.monospaced()).textSelection(.enabled)
                        .help(txid)
                }
                Text("output \(row.output.map(String.init) ?? "–")").foregroundStyle(.secondary)
                Text("included \(inclusionText(row))").foregroundStyle(.secondary)
                if !view.perspective.local {
                    OtherChainNote(systemId: view.perspective.observerId, fallback: view.perspective.observerName)
                }
            } else {
                Text("transaction unavailable").foregroundStyle(.secondary)
            }
        }
    }

    // MARK: Loading

    /// The subject is the currency for a gateway or chain, and its parent for a currency
    /// such as a bridge whose parent carries the running series. Take the fuller record.
    private func start() async {
        guard let explorer = model.explorer, let chain = model.status?.chainId else { return }
        let requested = NotarizationRequests.shared.take([route, currency.currencyId, currency.name, currency.fullyQualifiedName])
        var best: (String, NotarizationBrowser)?
        for candidate in [currency.currencyId, currency.parent].compactMap({ $0 }) where candidate != chain {
            guard let found = try? await explorer.notarizationBrowser(
                observerId: chain, subjectId: candidate, subjectName: nil, check: false),
                found.load == .ready
            else { continue }
            if found.rows.count > (best?.1.rows.count ?? 0) { best = (candidate, found) }
        }
        // A view of this chain from another chain needs no rows here to be offered.
        let chosen = best?.0 ?? (requested != nil ? currency.currencyId : nil)
        guard let chosen else {
            subject = nil
            return
        }
        subject = chosen
        let isCurrency = chosen == currency.currencyId
        perspectives = explorer.notarizationPerspectives(
            subjectId: chosen,
            subjectName: isCurrency ? currency.name : nil,
            options: isCurrency ? currency.options : nil,
            definitionSystem: isCurrency ? currency.systemId : nil)
        await model.resolveNames(perspectives.flatMap { [$0.observerId, $0.subjectId] })
        views = [:]
        active = perspectives.firstIndex { $0.observerId == requested } ?? 0
        await load(active, force: true)
    }

    private func load(_ index: Int, force: Bool) async {
        guard let perspective = perspectives[safe: index], let explorer = model.explorer else { return }
        if !force, views[index] != nil { return }
        loading = true
        failure = nil
        defer { loading = false }
        do {
            let view = try await explorer.notarizationBrowser(
                observerId: perspective.observerId, subjectId: perspective.subjectId,
                subjectName: perspective.subjectName, check: true)
            views[index] = view
            records[index] = []
            if perspective.local, let txid = view.rows.first(where: \.confirmed)?.txid {
                records[index] = await explorer.notarizationRecord(subjectId: perspective.subjectId, txid: txid)
            }
            await model.resolveNames(view.rows.flatMap { $0.roots.map(\.systemId) })
        } catch is CancellationError {
        } catch {
            failure = describe(error)
        }
    }

    private func bookmark() {
        guard let perspective = perspectives[safe: active], let explorer = model.explorer else { return }
        do {
            try explorer.saveBookmark(
                systemId: perspective.observerId, kind: "notarizations", target: perspective.subjectId,
                description: "")
            saved = "The bookmark is saved with the \(name(perspective.observerId, perspective.observerName)) perspective."
        } catch {
            saved = describe(error)
        }
        model.reloadBookmarks()
    }

    // MARK: Text

    private func name(_ id: String, _ fallback: String?) -> String {
        knownName(id, model: model, fallback: fallback)
    }

    private func observerTip(_ view: NotarizationBrowser) -> UInt64? {
        view.perspective.local ? model.status?.blocks : nil
    }

    private func otherName(_ view: NotarizationBrowser) -> String {
        if let root = view.rows.lazy.flatMap(\.roots).first(where: \.foreign) {
            return name(root.systemId, root.systemName)
        }
        return name(view.perspective.subjectId, view.perspective.subjectName)
    }

    private func inclusionText(_ row: BrowserNotarization) -> String {
        guard let height = row.includedHeight else { return row.inclusion }
        var text = "at block \(Format.integer(height))"
        if let time = row.includedTime {
            text += ", \(Format.date(time)) (\(Format.age(time)))"
        }
        return text
    }

    /// The blocks from the block that includes the last confirmed notarization to the tip.
    private func blocksSinceConfirmed(_ view: NotarizationBrowser) -> UInt64? {
        guard let row = view.rows.first(where: \.confirmed),
            let height = row.includedHeight ?? NotarizationPicture.observerHeight(row),
            let tip = observerTip(view), tip > height
        else { return nil }
        return tip - height
    }

    private func sentence(
        _ view: NotarizationBrowser, marks: [NotarizationMark], intervals: [UInt64], state: LinkState?
    ) -> String {
        let observer = name(view.perspective.observerId, view.perspective.observerName)
        let other = otherName(view)
        guard let confirmed = view.rows.first(where: \.confirmed),
            let height = confirmed.includedHeight ?? NotarizationPicture.observerHeight(confirmed)
        else {
            return "The \(observer) node returned no confirmed notarization of \(other). \(waiting(marks))"
        }
        var text = state.map { "\($0.word). " } ?? ""
        text += "\(observer) last confirmed a notarization of \(other) at block \(Format.integer(height))"
        if let wait = blocksSinceConfirmed(view) {
            text += ", \(NotarizationMath.span(blocks: wait)) ago"
        }
        text += "."
        if let usual = NotarizationMath.median(intervals), let longest = intervals.max() {
            let verb = state == .current ? "is" : "was"
            text += " The usual interval \(verb) \(Format.integer(usual)) blocks, and the longest of the last \(intervals.count) \(verb) \(Format.integer(longest))."
        }
        return "\(text) \(waiting(marks))"
    }

    /// "5 later entries wait on the best chain. 4 entries are on a side branch."
    private func waiting(_ marks: [NotarizationMark]) -> String {
        let best = marks.filter { $0.lane == 0 }.flatMap(\.rows).filter { !$0.confirmed && !$0.synthetic }.count
        let side = marks.filter { $0.lane > 0 }.flatMap(\.rows).count
        var parts: [String] = []
        if best > 0 {
            parts.append("\(best) later \(best == 1 ? "entry waits" : "entries wait") on the best chain.")
        }
        if side > 0 {
            parts.append("\(side) \(side == 1 ? "entry is" : "entries are") on a side branch.")
        }
        return parts.isEmpty ? "No later entry waits." : parts.joined(separator: " ")
    }

    /// The text below a mark of the graph.
    private func note(_ mark: NotarizationMark, _ view: NotarizationBrowser) -> String {
        guard let row = mark.newest else { return "" }
        if mark.lane > 0, let confirmed = view.rows.first(where: \.confirmed),
            NotarizationPicture.otherHeight(confirmed) == NotarizationPicture.otherHeight(row)
        {
            return mark.rows.count == 1 ? "the confirmed root again" : "the confirmed root, \(mark.rows.count) times"
        }
        var parts: [String] = []
        if mark.confirmed { parts.append("confirmed") }
        if mark.rows.count > 1 { parts.append("\(mark.rows.count) entries") }
        if let tip = observerTip(view), let height = NotarizationPicture.observerHeight(row), tip >= height {
            parts.append("\(NotarizationMath.span(blocks: tip - height)) ago")
        } else if let time = row.includedTime {
            parts.append(Format.age(time))
        }
        return parts.joined(separator: ", ")
    }

    private func caption(_ view: NotarizationBrowser) -> String {
        let checked = view.rows.flatMap(\.roots).filter { $0.foreign && $0.check != nil }
        let other = otherName(view)
        let lead = "Each mark is one notarization with the two heights that it commits to. A mark with a number is that number of entries for one \(other) block."
        if checked.isEmpty {
            return "\(lead) The app did not check \(other) blocks. The rows say why."
        }
        let active = checked.filter { $0.check == .active }.count
        return "\(lead) \(active) of \(checked.count) \(other) proof roots are active on the \(other) source."
    }

}

/// One notarization: its status, output, committed heights by system, inclusion, and the
/// check of each foreign proof root.
private struct NotarizationRowView: View {
    let row: BrowserNotarization
    let perspective: NotarizationPerspective
    let observerTip: UInt64?
    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    var body: some View {
        let _ = (showsNames, showsDates)
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 12) {
                status.frame(width: 150, alignment: .leading)
                if let txid = row.txid {
                    if perspective.local {
                        RouteLink(text: txid, route: .transaction(txid: txid))
                    } else {
                        Text(Format.short(txid)).font(.body.monospaced()).textSelection(.enabled)
                            .help(txid)
                    }
                    Text("output \(row.output.map(String.init) ?? "–")").foregroundStyle(.secondary)
                    if !perspective.local {
                        OtherChainNote(systemId: perspective.observerId, fallback: perspective.observerName)
                    }
                } else {
                    Text("–").foregroundStyle(.secondary)
                }
                Spacer()
                if !row.onBestChain {
                    Label("On a side branch", systemImage: "arrow.triangle.branch")
                        .font(.callout).foregroundStyle(Tone.mined)
                }
            }
            HStack(spacing: 12) {
                NameText("Proof roots").frame(width: 150, alignment: .leading)
                Text(heights).monospacedDigit()
            }
            HStack(spacing: 12) {
                NameText("Included").frame(width: 150, alignment: .leading)
                if let height = row.includedHeight, let hash = row.blockHash, perspective.local {
                    RouteLink(text: heightText(height, known: row.includedTime, model: model), route: .block(id: hash), shorten: false)
                    if let time = row.includedTime {
                        Text("\(Format.date(time)) (\(Format.age(time)))").foregroundStyle(.secondary)
                    }
                } else if let height = row.includedHeight {
                    Text("\(knownName(perspective.observerId, model: model, fallback: perspective.observerName)) block \(Format.integer(height))")
                    if let time = row.includedTime {
                        Text("\(Format.date(time)) (\(Format.age(time)))").foregroundStyle(.secondary)
                    }
                } else {
                    Text(row.inclusion).foregroundStyle(.secondary)
                }
            }
            ForEach(Array(row.roots.filter(\.foreign).enumerated()), id: \.offset) { _, root in
                rootLine(root)
            }
            if let previous = row.previous {
                HStack(spacing: 12) {
                    NameText("Previous").frame(width: 150, alignment: .leading)
                    Text("on \(knownName(previous.output.systemId, model: model, fallback: previous.systemName))")
                    Text("\(Format.short(previous.output.txid)):\(previous.output.output)")
                        .font(.callout.monospaced()).textSelection(.enabled)
                    Label(previous.status, systemImage: previous.found ? "checkmark.circle" : "questionmark.circle")
                        .foregroundStyle(previous.found ? Tone.good : Tone.mined)
                }
            }
        }
        .padding(.vertical, 8)
    }

    @ViewBuilder
    private var status: some View {
        if row.synthetic {
            Label("Synthetic proof root", systemImage: "circle.dotted").foregroundStyle(Tone.label)
        } else if row.confirmed {
            Label("Confirmed", systemImage: "checkmark.seal.fill").foregroundStyle(Tone.good)
        } else {
            Label("Unconfirmed", systemImage: "circle.dashed").foregroundStyle(Tone.mined)
        }
    }

    /// Committed heights by system: the observer, the subject, then any other roots.
    private var heights: String {
        var systems = [perspective.observerId]
        if perspective.subjectId != perspective.observerId { systems.append(perspective.subjectId) }
        systems += row.roots.map(\.systemId)
        var seen = Set<String>()
        return systems.filter { seen.insert($0).inserted }.map { id in
            let root = row.roots.first { $0.systemId == id }
            // Dates are known for the connected chain's heights only.
            let height = root?.height.map {
                id == model.status?.chainId ? heightText($0, model: model) : "#\(Format.integer($0))"
            } ?? "—"
            return "\(knownName(id, model: model, fallback: root?.systemName ?? (id == perspective.observerId ? perspective.observerName : perspective.subjectName))) \(height)"
        }
        .joined(separator: " · ")
    }

    @ViewBuilder
    private func rootLine(_ root: ProofRootRow) -> some View {
        let system = knownName(root.systemId, model: model, fallback: root.systemName)
        HStack(spacing: 12) {
            NameText("\(system) root").frame(width: 150, alignment: .leading)
            if let check = root.check, let text = root.status {
                Label(text, systemImage: symbol(check)).foregroundStyle(color(check))
            } else {
                Text("Not checked. A Verus node cannot check an Ethereum block.")
                    .foregroundStyle(.secondary)
            }
            if let hash = root.blockHash {
                if root.resolved && root.systemId == model.status?.chainId {
                    RouteLink(text: hash, route: .block(id: hash))
                } else {
                    Text(Format.short(hash)).font(.callout.monospaced()).foregroundStyle(.secondary)
                        .textSelection(.enabled)
                        .help(hash)
                    if root.resolved {
                        OtherChainNote(systemId: root.systemId, fallback: root.systemName)
                    }
                }
            }
            if let source = root.checkedOn {
                Text("checked on \(observationSource(source))").font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
        }
    }

    private func symbol(_ check: RootCheck) -> String {
        switch check {
        case .active: "checkmark.circle.fill"
        case .knownOffChain, .notFound: "exclamationmark.triangle.fill"
        case .heightMismatch: "exclamationmark.circle"
        case .presentMembershipUnknown: "circle.lefthalf.filled"
        case .sourceBehind: "clock"
        case .noSourceConfigured, .noSourceEnabled: "slash.circle"
        case .chainQueryFailed, .blockLookupFailed, .invalidHeader: "xmark.circle"
        case .missingHash, .notChecked: "questionmark.circle"
        }
    }

    private func color(_ check: RootCheck) -> Color {
        switch check {
        case .active: Tone.good
        case .knownOffChain, .notFound, .heightMismatch: Tone.alert
        case .presentMembershipUnknown, .sourceBehind: Tone.mined
        default: Tone.label
        }
    }
}

fileprivate extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
