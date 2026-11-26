import CarbideCore
import SwiftUI

/// Every currency the node lists, filtered as Carbide's currency browser filters them.
///
/// Each option flag cycles through Any, Show and Hide. A currency matches when it has all
/// the Show flags and none of the Hide flags, and its launch state is included. The
/// default view is liquidity pools with known failed launches hidden. The filters and the
/// selection last for the session.
struct CurrenciesView: View {
    @Environment(AppModel.self) private var model
    @State private var browser = CurrencyBrowserState.shared
    @State private var visible: [Int] = []

    var body: some View {
        ScrollViewReader { proxy in
            Page {
                VStack(alignment: .leading, spacing: 6) {
                    SourceLine(provenance: browser.provenance, short: model.currentChain, labelled: true)
                        .font(.callout)
                }
                .id("currencies-top")
                Card("Currencies", showsTitle: false) {
                    console
                    if browser.loading && browser.rows.isEmpty {
                        ProgressView("Loading currencies…").frame(maxWidth: .infinity, minHeight: 200)
                    } else {
                        if let failure = browser.failure {
                            Note(
                                text: browser.rows.isEmpty
                                    ? failure : "The list did not refresh. This is the list from the earlier request. \(failure)",
                                warning: true)
                        }
                        table
                    }
                }
            }
            .onAppear {
                if let selected = browser.selected {
                    proxy.scrollTo(selected, anchor: .center)
                }
            }
        }
        .navigationTitle("Currencies")
        .task(id: model.generation) { await load(force: false) }
        .publishPlace(PlaceActions(refresh: { Task { await load(force: true) } }))
        .task(id: FilterKey(rows: browser.rows.count, filter: browser.filter, sort: browser.sort, descending: browser.descending)) {
            apply()
        }
    }

    private struct FilterKey: Equatable {
        let rows: Int
        let filter: CurrencyFilter
        let sort: CurrencySort?
        let descending: Bool
    }

    // MARK: The console

    /// The figures of the list and the keys that filter it, as one instrument.
    private var console: some View {
        @Bindable var browser = browser
        let rows = browser.rows
        return DisplayPanel(padding: 0) {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 26) {
                    Readout("Shown", rows.isEmpty ? "–" : "\(visible.count) of \(rows.count)")
                    Readout("Launched", count(.launched))
                    Readout("Prelaunch", count(.prelaunch))
                    Readout("Failed", count(.failed), tone: count(.failed) == "0" ? Tone.value : Tone.alert)
                    Readout("Fractional", rows.isEmpty ? "–" : "\(rows.filter { ($0.options ?? 0) & 1 != 0 }.count)")
                    Spacer(minLength: 0)
                    if browser.loading { ProgressView().controlSize(.small) }
                    Button("Refresh") { Task { await load(force: true) } }
                        .disabled(browser.loading)
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 12)
                PanelRule()
                Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 16, verticalSpacing: 12) {
                    GridRow {
                        NameText("Find")
                        TextField("Find", text: $browser.filter.text, prompt: Text("Name, ID or option"))
                            .textFieldStyle(.plain)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 5)
                            .frame(maxWidth: 320)
                            .overlay(Rectangle().strokeBorder(Tone.axis, lineWidth: 1))
                    }
                    GridRow {
                        NameText("Launch")
                        HStack(spacing: 4) {
                            ForEach(LaunchState.all, id: \.self) { state in
                                let bit = launchStateBit(state: state)
                                LampToggle(title: state.word, lit: browser.filter.launchMask & bit != 0) {
                                    browser.filter.launchMask ^= bit
                                }
                                .help("Click to include or to leave out the currencies in this state.")
                            }
                        }
                    }
                    GridRow {
                        NameText("Options")
                        VStack(alignment: .leading, spacing: 10) {
                            HStack(spacing: 4) {
                                LampToggle(title: "Any", lit: browser.filter.required == 0 && browser.filter.excluded == 0) {
                                    browser.filter.required = 0
                                    browser.filter.excluded = 0
                                }
                                LampToggle(title: "Liquidity pools", lit: browser.filter.required == 1 && browser.filter.excluded == 0) {
                                    browser.filter.required = 1
                                    browser.filter.excluded = 0
                                }
                                LampToggle(title: "Each option", lit: browser.showFlags) { browser.showFlags.toggle() }
                            }
                            if browser.showFlags { flagKeys }
                        }
                    }
                }
                .padding(.horizontal, 18)
                .padding(.vertical, 14)
            }
        }
    }

    private func count(_ state: LaunchState) -> String {
        browser.rows.isEmpty ? "–" : "\(browser.rows.filter { $0.launch == state }.count)"
    }

    /// One key for each option. A click changes it: any, then show, then hide.
    private var flagKeys: some View {
        VStack(alignment: .leading, spacing: 8) {
            FlowLayout(spacing: 4) {
                ForEach(currencyOptionFlags(), id: \.bit) { flag in
                    let state = currencyFlagState(filter: browser.filter, bit: flag.bit)
                    LampToggle(title: flag.name, lit: state == .show, struck: state == .hide) {
                        browser.filter = cycleCurrencyFlag(filter: browser.filter, bit: flag.bit)
                    }
                    .accessibilityValue(state == .show ? "Show" : state == .hide ? "Hide" : "Any")
                }
            }
            Text("A lit key shows only the currencies with the option. A key with a line through it hides them. Click a key to change it.")
                .font(Tone.caption)
                .foregroundStyle(Tone.label)
        }
    }

    // MARK: The table

    private static let reservesWidth: CGFloat = 250
    private static let supplyWidth: CGFloat = 150
    private static let startWidth: CGFloat = 86

    private var table: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 16) {
                heading(.name).frame(maxWidth: .infinity, alignment: .leading)
                heading(.reserves).frame(width: Self.reservesWidth, alignment: .leading)
                heading(.supply).frame(width: Self.supplyWidth, alignment: .trailing)
                heading(.start).frame(width: Self.startWidth, alignment: .trailing)
            }
            .padding(.horizontal, 8)
            .padding(.bottom, 8)
            LazyVStack(spacing: 0) {
                ForEach(visible, id: \.self) { index in
                    let row = browser.rows[index]
                    PanelRule()
                    CurrencyTile(row: row, selected: browser.selected == row.currencyId) {
                        browser.selected = row.currencyId
                        model.open(.currency(name: row.currencyId))
                    }
                    .modifier(
                        CursorStopModifier(
                            id: row.currencyId, token: row.currencyId, copy: row.currencyId, tags: false
                        ) {
                            browser.selected = row.currencyId
                            model.open(.currency(name: row.currencyId))
                        }
                    )
                    .id(row.currencyId)
                }
            }
            if visible.isEmpty && !browser.rows.isEmpty {
                PanelRule()
                Text("No currency is in these filters.").foregroundStyle(Tone.label).padding(8)
            }
        }
    }

    /// The name of a column. A click puts the list in the order of the column, a second
    /// click turns the order, and a third click gives the order of the source again.
    private func heading(_ column: CurrencySort) -> some View {
        let active = browser.sort == column
        return Button {
            if !active {
                browser.sort = column
                browser.descending = column == .supply || column == .start || column == .reserves
            } else if browser.descending == (column == .name) {
                browser.sort = nil
            } else {
                browser.descending.toggle()
            }
        } label: {
            HStack(spacing: 4) {
                NameText(column.rawValue)
                if active {
                    Image(systemName: browser.descending ? "arrowtriangle.down.fill" : "arrowtriangle.up.fill")
                        .font(.system(size: 7))
                        .foregroundStyle(Tone.present)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Click to put the list in the order of this column.")
    }

    private func launchBinding(_ state: LaunchState) -> Binding<Bool> {
        let bit = launchStateBit(state: state)
        return Binding(
            get: { browser.filter.launchMask & bit != 0 },
            set: { on in
                if on { browser.filter.launchMask |= bit } else { browser.filter.launchMask &= ~bit }
            })
    }

    // MARK: Data

    private func apply() {
        var found = filterCurrencies(rows: browser.rows, filter: browser.filter).map { Int($0) }
        if let sort = browser.sort {
            let rows = browser.rows
            found.sort { left, right in
                let a = rows[left], b = rows[right]
                let ascending: Bool
                switch sort {
                case .name: ascending = a.name.localizedCaseInsensitiveCompare(b.name) == .orderedAscending
                case .reserves: ascending = a.reserveIds.count < b.reserveIds.count
                case .supply: ascending = (a.supply ?? -1) < (b.supply ?? -1)
                case .start: ascending = (a.startBlock ?? 0) < (b.startBlock ?? 0)
                }
                let equal: Bool
                switch sort {
                case .name: equal = false
                case .reserves: equal = a.reserveIds.count == b.reserveIds.count
                case .supply: equal = a.supply == b.supply
                case .start: equal = a.startBlock == b.startBlock
                }
                if equal { return left < right }
                return browser.descending ? !ascending : ascending
            }
        }
        visible = found
    }

    /// Loads once per connection. A failed refresh keeps the earlier list, as Carbide does.
    private func load(force: Bool) async {
        guard let explorer = model.explorer else { return }
        if !force, browser.generation == model.generation, !browser.rows.isEmpty { return }
        if browser.generation != model.generation {
            browser.rows = []
            browser.source = nil
            browser.provenance = nil
        }
        browser.loading = true
        let generation = model.generation
        do {
            let list = try await explorer.currencyList()
            guard generation == model.generation else { return }
            browser.rows = list.rows
            browser.source = list.source
            browser.provenance = list.provenance
            browser.failure = nil
            browser.generation = generation
        } catch is CancellationError {
        } catch {
            browser.failure = describe(error)
        }
        browser.loading = false
        apply()
        await model.resolveNames(browser.rows.flatMap(\.reserveIds))
    }
}

/// One currency as a row of the table: the name with its state and options, the ID, the
/// reserves, the supply and the start block.
private struct CurrencyTile: View {
    let row: CurrencyListing
    let selected: Bool
    let open: () -> Void
    @Environment(AppModel.self) private var model
    @State private var hovering = false
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    var body: some View {
        let _ = (showsNames, showsDates)
        Button(action: open) {
            HStack(alignment: .top, spacing: 16) {
                VStack(alignment: .leading, spacing: 5) {
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text(row.name).font(.body.weight(.semibold)).foregroundStyle(Tone.value).lineLimit(1)
                            .layoutPriority(1)
                        Text(row.currencyId)
                            .font(.caption.monospaced())
                            .foregroundStyle(Tone.label)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                    FlowLayout(spacing: 5) {
                        // Launched is the usual state: only a different state gets a plate.
                        if row.launch != .launched {
                            Plate(row.launch.word, fill: row.launch == .failed ? Tone.alert : Tone.mined)
                        }
                        ForEach(row.optionTags, id: \.self) { Plate($0) }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Text(reserves)
                    .font(.callout)
                    .foregroundStyle(row.reserveIds.isEmpty ? Tone.label : Tone.value.opacity(0.85))
                    .lineLimit(2)
                    .frame(width: 250, alignment: .leading)
                Text(row.supply.map { Format.amount($0) } ?? "–")
                    .font(.callout)
                    .monospacedDigit()
                    .foregroundStyle(Tone.value)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                    .frame(width: 150, alignment: .trailing)
                Text(row.startBlock.map { heightText($0, model: model) } ?? "–")
                    .font(.callout)
                    .monospacedDigit()
                    .foregroundStyle(Tone.label)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                    .frame(width: 86, alignment: .trailing)
            }
            .padding(.vertical, 9)
            .padding(.horizontal, 8)
            .background(
                selected ? AnyShapeStyle(Tone.present.opacity(0.1))
                    : hovering ? AnyShapeStyle(Tone.grid) : AnyShapeStyle(.clear),
                in: Rectangle())
            .overlay(alignment: .leading) {
                if selected {
                    Rectangle().fill(Tone.present).frame(width: 2)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help("Open \(row.name)")
    }

    private var reserves: String {
        guard !row.reserveIds.isEmpty else { return "No reserve" }
        return row.reserveIds.enumerated().map { index, id in
            knownName(id, model: model, fallback: row.reserveNames[safe: index])
        }.joined(separator: ", ")
    }
}

/// Launch state as a word, a shape and a colour.
struct CurrencyLaunchBadge: View {
    let state: LaunchState

    var body: some View {
        Label(state.word, systemImage: symbol)
            .font(.caption.weight(.semibold))
            .foregroundStyle(color)
            .lineLimit(1)
            .fixedSize()
    }

    private var symbol: String {
        switch state {
        case .launched: "checkmark.circle.fill"
        case .prelaunch: "clock"
        case .failed: "arrow.uturn.backward.circle"
        case .unknown: "questionmark.circle"
        }
    }

    private var color: Color {
        switch state {
        case .launched: Tone.good
        case .prelaunch: Tone.record
        case .failed: Tone.alert
        case .unknown: Tone.mined
        }
    }
}

fileprivate extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
