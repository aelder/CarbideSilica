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
                Card("Currencies", trailing: browser.rows.isEmpty ? nil : "\(visible.count) of \(browser.rows.count)") {
                    controls
                        .id("currencies-top")
                    if browser.loading && browser.rows.isEmpty {
                        ProgressView("Loading currencies…").frame(maxWidth: .infinity, minHeight: 200)
                    } else {
                        if let failure = browser.failure {
                            Note(
                                text: browser.rows.isEmpty
                                    ? failure : "The list did not refresh. The app shows the list from the earlier request. \(failure)",
                                warning: true)
                        }
                        if let provenance = browser.provenance {
                            ProvenanceNote(provenance: provenance)
                            Text("Each row shows the state that the source reported in the list.")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                        LazyVStack(spacing: 0) {
                            ForEach(visible, id: \.self) { index in
                                let row = browser.rows[index]
                                Divider()
                                CurrencyTile(row: row, selected: browser.selected == row.currencyId) {
                                    browser.selected = row.currencyId
                                    model.open(.currency(name: row.currencyId))
                                }
                                .id(row.currencyId)
                            }
                        }
                        if visible.isEmpty && !browser.rows.isEmpty {
                            Text("No currency matches the filters.").foregroundStyle(.secondary)
                        }
                    }
                }
            }
            .onAppear {
                if let selected = browser.selected {
                    proxy.scrollTo(selected, anchor: .center)
                }
            }
            .onChange(of: browser.filter) {
                // As in Carbide, a filter change returns the list to the top.
                proxy.scrollTo("currencies-top", anchor: .top)
            }
        }
        .navigationTitle("Currencies")
        .task(id: model.generation) { await load(force: false) }
        .task(id: FilterKey(rows: browser.rows.count, filter: browser.filter)) { apply() }
    }

    private struct FilterKey: Equatable {
        let rows: Int
        let filter: CurrencyFilter
    }

    // MARK: Controls

    private var controls: some View {
        @Bindable var browser = browser
        return VStack(alignment: .leading, spacing: 10) {
            HStack {
                TextField("Filter", text: $browser.filter.text, prompt: Text("Filter names, IDs or options"))
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 300)
                Spacer()
                if browser.loading { ProgressView().controlSize(.small) }
                Button("Refresh") { Task { await load(force: true) } }
                    .disabled(browser.loading)
            }
            HStack(spacing: 8) {
                Text("Launch status").foregroundStyle(.secondary).frame(width: 100, alignment: .leading)
                ForEach(LaunchState.all, id: \.self) { state in
                    LaunchToggle(state: state, isOn: launchBinding(state))
                }
                Menu("Only") {
                    ForEach(LaunchState.all, id: \.self) { state in
                        Button(state.word) { browser.filter.launchMask = launchStateBit(state: state) }
                    }
                    Divider()
                    Button("Include All") { browser.filter.launchMask = 15 }
                    Button("Default") { browser.filter.launchMask = defaultCurrencyFilter().launchMask }
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
            }
            HStack(spacing: 8) {
                Text("Option flags").foregroundStyle(.secondary).frame(width: 100, alignment: .leading)
                Text(flagSummary).lineLimit(1).truncationMode(.tail)
                Spacer()
                Button(browser.showFlags ? "Hide Flags" : "Edit Flags") { browser.showFlags.toggle() }
                Button("Any Flags") {
                    browser.filter.required = 0
                    browser.filter.excluded = 0
                }
                Button("Liquidity Pools") {
                    browser.filter.required = 1
                    browser.filter.excluded = 0
                }
            }
            if browser.showFlags {
                flagGrid
            }
        }
    }

    /// Each flag is a button that cycles Any, Show, Hide. Its state is a word, a shape
    /// and a colour.
    private var flagGrid: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 230), alignment: .leading)], alignment: .leading, spacing: 6) {
            ForEach(currencyOptionFlags(), id: \.bit) { flag in
                let state = currencyFlagState(filter: browser.filter, bit: flag.bit)
                Button {
                    browser.filter = cycleCurrencyFlag(filter: browser.filter, bit: flag.bit)
                } label: {
                    HStack(spacing: 6) {
                        Image(systemName: symbol(state)).foregroundStyle(color(state))
                        Text(flag.name)
                        Spacer(minLength: 4)
                        Text(word(state)).font(.caption.weight(.semibold)).foregroundStyle(color(state))
                    }
                    .frame(width: 210, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.borderless)
                .help("Click to change: Any, then Show, then Hide.")
            }
        }
        .padding(.leading, 108)
    }

    private func symbol(_ state: FlagState) -> String {
        switch state {
        case .any: "circle"
        case .show: "plus.circle.fill"
        case .hide: "minus.circle.fill"
        }
    }

    private func color(_ state: FlagState) -> Color {
        switch state {
        case .any: .secondary
        case .show: .green
        case .hide: .red
        }
    }

    private func word(_ state: FlagState) -> String {
        switch state {
        case .any: "Any"
        case .show: "Show"
        case .hide: "Hide"
        }
    }

    private var flagSummary: String {
        let flags = currencyOptionFlags()
        let shown = flags.filter { browser.filter.required & $0.bit != 0 }.map(\.name)
        let hidden = flags.filter { browser.filter.excluded & $0.bit != 0 }.map(\.name)
        if shown.isEmpty && hidden.isEmpty { return "Any" }
        var parts: [String] = []
        if !shown.isEmpty { parts.append("Show " + shown.joined(separator: ", ")) }
        if !hidden.isEmpty { parts.append("Hide " + hidden.joined(separator: ", ")) }
        return parts.joined(separator: " · ")
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
        visible = filterCurrencies(rows: browser.rows, filter: browser.filter).map { Int($0) }
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

/// One currency, with what Carbide's summary tile shows: the name, launch state and
/// option tags, the i-address, the start block, supply and reserves.
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
            VStack(alignment: .leading, spacing: 4) {
                HStack(spacing: 6) {
                    Text(row.name).fontWeight(.semibold).lineLimit(1)
                    CurrencyLaunchBadge(state: row.launch)
                    ForEach(row.optionTags, id: \.self) { TagChip(text: $0, color: .secondary) }
                    Spacer(minLength: 0)
                }
                Text(row.currencyId)
                    .font(.callout.monospaced())
                    .foregroundStyle(Color.accentColor)
                    .lineLimit(1)
                Text(summary)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
            .padding(.vertical, 8)
            .padding(.horizontal, 8)
            .background(
                selected ? AnyShapeStyle(Color.accentColor.opacity(0.12))
                    : hovering ? AnyShapeStyle(.quaternary) : AnyShapeStyle(.clear),
                in: RoundedRectangle(cornerRadius: 6))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .help("Open \(row.name)")
    }

    private var summary: String {
        var parts: [String] = []
        if let start = row.startBlock { parts.append("Start \(heightText(start, model: model))") }
        parts.append("Supply \(row.supply.map { Format.amount($0) } ?? "unavailable")")
        let count = row.reserveIds.count
        parts.append("\(count) \(count == 1 ? "reserve" : "reserves")")
        if count > 0 {
            let names = row.reserveIds.enumerated().map { index, id in
                knownName(id, model: model, fallback: row.reserveNames[safe: index])
            }
            parts.append(names.joined(separator: ", "))
        }
        return parts.joined(separator: " · ")
    }
}

/// A launch-status filter. On is a filled checkmark, the state's colour and a tinted
/// border; off is an empty circle in grey with no fill, so the state never rests on colour.
private struct LaunchToggle: View {
    let state: LaunchState
    @Binding var isOn: Bool

    var body: some View {
        Button {
            isOn.toggle()
        } label: {
            Label(state.word, systemImage: isOn ? "checkmark.circle.fill" : "circle")
                .font(.callout.weight(isOn ? .semibold : .regular))
                .foregroundStyle(isOn ? Color.accentColor : .secondary)
                .padding(.horizontal, 8)
                .padding(.vertical, 3)
                .background(
                    isOn ? Color.accentColor.opacity(0.14) : .clear,
                    in: Capsule())
                .overlay(Capsule().strokeBorder(isOn ? Color.accentColor : Color.secondary.opacity(0.4), lineWidth: 1))
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityValue(isOn ? "Included" : "Not included")
        .help(isOn ? "\(state.word) currencies are included. Click to leave them out." : "\(state.word) currencies are left out. Click to include them.")
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
        case .launched: .green
        case .prelaunch: .accentColor
        case .failed: .red
        case .unknown: .orange
        }
    }
}

fileprivate extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
