import CarbideCore
import SwiftUI

/// Carbide's notarization browser, with two systems drawn as parallel lanes.
///
/// Each notarization is a tie between the observer's lane and the notarized system's lane.
/// The confirmed tie is solid; unconfirmed ties are dashed; a tie on a side branch is red.
/// The observer's lane runs to its current tip, so the distance since the last confirmed
/// tie is visible. Below the picture, each row gives the committed heights by system, the
/// observer's block that includes it, and the check of each foreign proof root on the
/// owning chain's own source.
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
        .onChange(of: active) { Task { await load(active, force: false) } }
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
        Note(text: summary(view), warning: stale(view))
        if let confirmed = view.rows.first(where: \.confirmed) {
            confirmedLine(confirmed, view)
        }
        if view.rows.isEmpty {
            Text("No notarizations in current view").foregroundStyle(.secondary)
        } else {
            Canvas { context, size in
                draw(context, size, view)
            }
            .frame(height: 150)
            .accessibilityLabel(summary(view))

            HStack(spacing: 18) {
                legend(color: .accentColor, dashed: false, text: "Confirmed")
                legend(color: .secondary, dashed: true, text: "Unconfirmed")
                if view.rows.contains(where: { !$0.onBestChain }) {
                    legend(color: .red, dashed: false, text: "On a side branch")
                }
                Spacer()
            }
            Text(caption(view)).font(.caption).foregroundStyle(.secondary)

            VStack(spacing: 0) {
                ForEach(view.rows.reversed(), id: \.index) { row in
                    Divider()
                    NotarizationRowView(row: row, perspective: view.perspective, observerTip: observerTip(view))
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

    private func confirmedLine(_ row: BrowserNotarization, _ view: NotarizationBrowser) -> some View {
        HStack(spacing: 10) {
            Text("Last confirmed").foregroundStyle(.secondary).frame(width: 170, alignment: .leading)
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

    private func observerHeight(_ row: BrowserNotarization) -> UInt64? {
        row.roots.first { !$0.foreign }?.height ?? row.includedHeight
    }

    private func otherHeight(_ row: BrowserNotarization) -> UInt64? {
        row.roots.first { $0.foreign }?.height
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

    private func blocksSinceConfirmed(_ view: NotarizationBrowser) -> UInt64? {
        guard let row = view.rows.first(where: \.confirmed), let height = observerHeight(row),
            let tip = observerTip(view), tip > height
        else { return nil }
        return tip - height
    }

    /// More than a day without a confirmed notarization is worth pointing out.
    private func stale(_ view: NotarizationBrowser) -> Bool {
        (blocksSinceConfirmed(view) ?? 0) > 1440
    }

    private func summary(_ view: NotarizationBrowser) -> String {
        let observer = name(view.perspective.observerId, view.perspective.observerName)
        let unconfirmed = "\(view.unconfirmed) unconfirmed in the current view."
        guard let confirmed = view.rows.first(where: \.confirmed), let height = observerHeight(confirmed) else {
            return "The node returned no confirmed notarization. \(unconfirmed)"
        }
        // Dates are known for the connected chain's heights only.
        let at = view.perspective.local ? heightText(height, model: model) : Format.integer(height)
        var text = "The last confirmed notarization commits to \(observer) block \(at)"
        if let behind = blocksSinceConfirmed(view) {
            let days = Double(behind) / 1440
            text += ", \(Format.integer(behind)) blocks ago"
            text += days >= 1 ? " (approximately \(Int(days.rounded())) days)." : "."
        } else {
            text += "."
        }
        return "\(text) \(unconfirmed)"
    }

    private func caption(_ view: NotarizationBrowser) -> String {
        let checked = view.rows.flatMap(\.roots).filter { $0.foreign && $0.check != nil }
        let other = otherName(view)
        if checked.isEmpty {
            return "Each tie joins the heights that one notarization commits to. The app did not check \(other) blocks. The rows say why."
        }
        let active = checked.filter { $0.check == .active }.count
        return "Each tie joins the heights that one notarization commits to. \(active) of \(checked.count) \(other) proof roots are active on the \(other) source."
    }

    private func legend(color: Color, dashed: Bool, text: String) -> some View {
        HStack(spacing: 6) {
            Canvas { context, size in
                context.stroke(
                    Path { path in
                        path.move(to: CGPoint(x: 0, y: size.height / 2))
                        path.addLine(to: CGPoint(x: size.width, y: size.height / 2))
                    }, with: .color(color), style: StrokeStyle(lineWidth: 2.4, dash: dashed ? [4, 3] : []))
            }
            .frame(width: 22, height: 10)
            Text(text).font(.caption).foregroundStyle(.secondary)
        }
    }

    // MARK: Drawing

    private func draw(_ context: GraphicsContext, _ size: CGSize, _ view: NotarizationBrowser) {
        let rows = view.rows.filter { observerHeight($0) != nil }
        let locals = rows.compactMap(observerHeight)
        guard let firstLocal = locals.min(), let lastLocal = locals.max() else { return }
        let tip = max(observerTip(view) ?? 0, lastLocal)
        let others = rows.compactMap(otherHeight)
        let firstOther = others.min() ?? 0
        let lastOther = max(others.max() ?? 1, firstOther + 1)

        let left: CGFloat = 96
        let right = size.width - 16
        let top: CGFloat = 36
        let bottom: CGFloat = 112
        let pad = Double(max(tip - firstLocal, 1)) * 0.04
        let xLocal = { (height: UInt64) -> CGFloat in
            left + CGFloat((Double(height) - Double(firstLocal) + pad) / (Double(tip - firstLocal) + pad * 2))
                * (right - left)
        }
        // The other lane spans the same horizontal stretch as the notarized part of this one.
        let xOther = { (height: UInt64) -> CGFloat in
            let fraction = Double(height - min(height, firstOther)) / Double(lastOther - firstOther)
            return xLocal(firstLocal) + CGFloat(fraction) * (xLocal(lastLocal) - xLocal(firstLocal))
        }
        let observer = name(view.perspective.observerId, view.perspective.observerName)
        let checked = view.rows.flatMap(\.roots).contains { $0.foreign && $0.check == .active }
        for (y, laneName, faint) in [(top, observer, false), (bottom, otherName(view), !checked)] {
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: left - 8, y: y))
                    path.addLine(to: CGPoint(x: right, y: y))
                }, with: .color(.secondary.opacity(faint ? 0.16 : 0.3)),
                style: StrokeStyle(lineWidth: 12, lineCap: .round))
            context.draw(
                Text(laneName).font(.callout.weight(.semibold)).foregroundStyle(faint ? .secondary : .primary),
                at: CGPoint(x: 0, y: y), anchor: .leading)
        }

        for row in rows {
            guard let local = observerHeight(row) else { continue }
            let color: Color = !row.onBestChain ? .red : row.confirmed ? .accentColor : .secondary
            let a = CGPoint(x: xLocal(local), y: top)
            let b = CGPoint(x: otherHeight(row).map(xOther) ?? a.x, y: bottom)
            context.stroke(
                Path { path in
                    path.move(to: a)
                    path.addLine(to: b)
                }, with: .color(color),
                style: StrokeStyle(lineWidth: row.confirmed ? 2.6 : 1.6, dash: row.confirmed ? [] : [5, 4]))
            for point in [a, b] {
                context.fill(
                    Path(ellipseIn: CGRect(x: point.x - 5, y: point.y - 5, width: 10, height: 10)),
                    with: .color(color))
            }
        }

        // The observer's tip, and the stretch since the last confirmed tie.
        let tipX = xLocal(tip)
        if let confirmed = view.rows.first(where: \.confirmed).flatMap(observerHeight), stale(view) {
            let from = xLocal(confirmed)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: from, y: top - 16))
                    path.addLine(to: CGPoint(x: tipX, y: top - 16))
                }, with: .color(.orange), style: StrokeStyle(lineWidth: 1.5, dash: [3, 4]))
            context.draw(
                Text("not confirmed since").font(.caption2.weight(.semibold)).foregroundStyle(.orange),
                at: CGPoint(x: (from + tipX) / 2, y: top - 26), anchor: .center)
        }
        if observerTip(view) != nil {
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: tipX, y: top - 12))
                    path.addLine(to: CGPoint(x: tipX, y: top + 12))
                }, with: .color(.primary), lineWidth: 1.5)
            context.draw(
                Text("now").font(.caption2).foregroundStyle(.primary),
                at: CGPoint(x: tipX, y: top + 22), anchor: .center)
        }
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
                        .font(.callout).foregroundStyle(.red)
                }
            }
            HStack(spacing: 12) {
                Text("Proof roots").foregroundStyle(.secondary).frame(width: 150, alignment: .leading)
                Text(heights).monospacedDigit()
            }
            HStack(spacing: 12) {
                Text("Included").foregroundStyle(.secondary).frame(width: 150, alignment: .leading)
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
                    Text("Previous").foregroundStyle(.secondary).frame(width: 150, alignment: .leading)
                    Text("on \(knownName(previous.output.systemId, model: model, fallback: previous.systemName))")
                    Text("\(Format.short(previous.output.txid)):\(previous.output.output)")
                        .font(.callout.monospaced()).textSelection(.enabled)
                    Label(previous.status, systemImage: previous.found ? "checkmark.circle" : "questionmark.circle")
                        .foregroundStyle(previous.found ? .green : .orange)
                }
            }
        }
        .padding(.vertical, 8)
    }

    @ViewBuilder
    private var status: some View {
        if row.synthetic {
            Label("Synthetic proof root", systemImage: "circle.dotted").foregroundStyle(.secondary)
        } else if row.confirmed {
            Label("Confirmed", systemImage: "checkmark.seal.fill").foregroundStyle(.green)
        } else {
            Label("Unconfirmed", systemImage: "circle.dashed").foregroundStyle(.orange)
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
            Text("\(system) root").foregroundStyle(.secondary).frame(width: 150, alignment: .leading)
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
        case .active: .green
        case .knownOffChain, .notFound, .heightMismatch: .red
        case .presentMembershipUnknown, .sourceBehind: .orange
        default: .secondary
        }
    }
}

fileprivate extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
