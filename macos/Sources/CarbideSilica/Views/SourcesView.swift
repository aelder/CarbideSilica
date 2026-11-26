import CarbideCore
import SwiftUI

/// Every node and public API that the app knows, with its state: the terminal's Sources (F2).
///
/// States apply to this session at once. Save writes them to Carbide's settings file, which
/// the terminal app also reads.
struct SourcesView: View {
    @Environment(AppModel.self) private var model
    @State private var overview: SourcesOverview?
    @State private var selectedKey: String?
    @State private var message: String?
    @State private var confirmingAPI: (row: SourceRow, state: SourceState)?

    var body: some View {
        Page {
            if let overview {
                Card("Nodes and APIs", trailing: overview.unsaved ? "Not saved" : nil) {
                    Text(
                        "A disabled source receives no request. An enabled source answers what you open. A monitored node is also followed in the background. Changes apply at once. Save writes them to the settings file that the terminal app also reads."
                    )
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    notes(overview)
                    SourceTable(
                        rows: overview.rows, selectedKey: $selectedKey,
                        change: change)
                    HStack {
                        if let message {
                            Note(text: message, warning: true)
                        }
                        Spacer()
                        Button("Save") { save() }
                            .disabled(!overview.unsaved || overview.settingsProblem != nil)
                            .help("Write the states to Carbide's settings file")
                    }
                }
                if let row = selected(in: overview) {
                    SourceDetail(row: row)
                    SourceActivity(row: row)
                }
            } else {
                Card("Nodes and APIs") {
                    ProgressView()
                }
            }
        }
        .navigationTitle("Sources")
        .publishPlace(PlaceActions(refresh: { reload() }))
        .task(id: model.generation) {
            // The rows hold the session's state, read in memory. Refresh them while visible.
            while !Task.isCancelled {
                reload()
                try? await Task.sleep(for: .seconds(3))
            }
        }
        .confirmationDialog(
            "Enable this public API?",
            isPresented: Binding(
                get: { confirmingAPI != nil }, set: { if !$0 { confirmingAPI = nil } })
        ) {
            Button("Enable") {
                if let pending = confirmingAPI { apply(pending.row, pending.state) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(
                "When the local node cannot answer, the app sends the request to \(confirmingAPI?.row.endpoint ?? "the API"). The operator of the API can see what you look up."
            )
        }
    }

    @ViewBuilder
    private func notes(_ overview: SourcesOverview) -> some View {
        if let problem = overview.settingsProblem {
            Note(text: problem, warning: true)
        }
        if let problem = overview.cacheProblem {
            Note(text: problem, warning: true)
        }
        if let file = overview.settingsFile {
            FieldRow(label: "Settings file", value: file, monospaced: true)
        }
    }

    private func selected(in overview: SourcesOverview) -> SourceRow? {
        overview.rows.first { $0.key == selectedKey }
            ?? overview.rows.first { $0.current }
    }

    private func reload() {
        guard let explorer = model.explorer else { return }
        let fresh = explorer.sourcesOverview()
        overview = fresh
        // A PBaaS chain's folder is its hex ID. Learn its name from its identity.
        let unnamed = fresh.rows.filter { $0.name == nil }.map(\.systemId)
        if !unnamed.isEmpty {
            Task { await model.resolveNames(unnamed) }
        }
    }

    private func change(_ row: SourceRow, _ state: SourceState) {
        if row.kind == .publicApi, row.state == .disabled, state != .disabled {
            confirmingAPI = (row, state)
        } else {
            apply(row, state)
        }
    }

    private func apply(_ row: SourceRow, _ state: SourceState) {
        do {
            try model.requireExplorer().setSourceState(key: row.key, state: state)
            message = nil
        } catch {
            message = describe(error)
        }
        reload()
    }

    private func save() {
        do {
            try model.requireExplorer().saveSources()
            message = nil
        } catch {
            message = describe(error)
        }
        reload()
    }
}

/// The sources as rows: chain, kind, endpoint, state, traffic and age of the last answer.
private struct SourceTable: View {
    @Environment(AppModel.self) private var model
    let rows: [SourceRow]
    @Binding var selectedKey: String?
    let change: (SourceRow, SourceState) -> Void

    var body: some View {
        Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 6) {
            GridRow {
                NameText("Chain")
                NameText("Source")
                NameText("Endpoint")
                NameText("State")
                NameText("Traffic")
                NameText("Last answer").gridColumnAlignment(.trailing)
            }
            Divider().gridCellUnsizedAxes(.horizontal)
            ForEach(rows, id: \.key) { row in
                GridRow {
                    HStack(spacing: 6) {
                        Text(sourceName(row, names: model.names))
                            .fontWeight(row.current ? .semibold : .regular)
                        if row.current {
                            Plate("Current")
                        }
                    }
                    Text(row.kind == .localNode ? "Node" : "API")
                    Text(row.endpoint)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: 260, alignment: .leading)
                    StatePicker(row: row, change: change)
                    TrafficMark(traffic: row.traffic)
                    Text(row.lastSuccessAge.map { "\(shortAge($0)) ago" } ?? "None")
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                        .gridColumnAlignment(.trailing)
                }
                .padding(.vertical, 2)
                .background(
                    selectedKey == row.key
                        ? Rectangle().fill(Tone.present.opacity(0.1))
                        : nil
                )
                .contentShape(Rectangle())
                .onTapGesture { selectedKey = row.key }
                .cursorStop(token: row.key) { selectedKey = row.key }
            }
        }
    }
}

/// Disabled, Enabled and, for a node, Monitored.
private struct StatePicker: View {
    let row: SourceRow
    let change: (SourceRow, SourceState) -> Void

    var body: some View {
        Picker(
            "State",
            selection: Binding(get: { row.state }, set: { change(row, $0) })
        ) {
            Text("Disabled").tag(SourceState.disabled)
            Text("Enabled").tag(SourceState.enabled)
            if row.kind == .localNode {
                Text("Monitored").tag(SourceState.monitored)
            }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
        .fixedSize()
        .disabled(row.kind == .localNode && !row.hasCredentials)
    }
}

/// Observed traffic, as shape, colour and word.
struct TrafficMark: View {
    let traffic: SourceTraffic

    var body: some View {
        Label(word, systemImage: symbol)
            .foregroundStyle(color)
            .lineLimit(1)
    }

    private var word: String {
        switch traffic {
        case .off: "Off"
        case .notUsed: "Not used"
        case .working: "Working"
        case .responding: "Responding"
        case .idle: "Idle"
        case .unavailable: "Unavailable"
        }
    }

    private var symbol: String {
        switch traffic {
        case .off: "minus.circle"
        case .notUsed: "circle"
        case .working: "arrow.triangle.2.circlepath"
        case .responding: "checkmark.circle.fill"
        case .idle: "circle.fill"
        case .unavailable: "xmark.octagon.fill"
        }
    }

    private var color: Color {
        switch traffic {
        case .responding: Tone.good
        case .working: Tone.record
        case .unavailable: Tone.alert
        default: .secondary
        }
    }
}

/// What one source is, what its state permits, and its last chain check.
private struct SourceDetail: View {
    let row: SourceRow
    @Environment(AppModel.self) private var model

    var body: some View {
        Card("\(sourceName(row, names: model.names)) \(row.kind == .localNode ? "node" : "API")") {
            FieldRow(label: "System ID", value: row.systemId, monospaced: true)
            FieldRow(label: "Endpoint", value: row.endpoint, monospaced: true)
            if let file = row.configFile {
                FieldRow(label: "Configuration file", value: file, monospaced: true)
            }
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                NameText("State").frame(width: 170, alignment: .leading)
                Text(stateMeaning)
            }
            if let check = row.chainCheck {
                HStack(alignment: .firstTextBaseline, spacing: 12) {
                    NameText("Chain check")
                        .frame(width: 170, alignment: .leading)
                    Note(text: check, warning: row.chainCheckProblem)
                }
            }
            if row.kind == .localNode && !row.hasCredentials {
                Note(
                    text: "The configuration file has no RPC user and password. The app cannot use this node.",
                    warning: true)
            }
            if row.kind == .localNode, !row.current, row.configFile != nil {
                HStack {
                    Text("The current chain sets the dashboard, mempool, peers and search.")
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Make Current Chain") { model.switchChain(to: row) }
                }
            }
        }
    }

    private var stateMeaning: String {
        switch (row.kind, row.state) {
        case (_, .disabled):
            "Disabled. The app sends no request to this source."
        case (.publicApi, _):
            "Enabled. The app uses this API when the local node cannot answer."
        case (_, .enabled):
            row.current
                ? "Enabled. The app sends requests for what you open, and keeps the chain's block history current."
                : "Enabled. The app sends requests for what you open on this chain."
        case (_, .monitored):
            "Monitored. Enabled, and the app also follows the tip and the mempool in the background."
        }
    }
}

/// Significant events that the cache recorded: reorganizations, changed blocks and
/// disagreements between sources.
private struct SourceActivity: View {
    let row: SourceRow
    @Environment(AppModel.self) private var model
    @AppStorage(SettingsKey.allSourceActivity) private var all = false
    @State private var events: [ActivityRow]?
    @State private var failure: String?

    var body: some View {
        Card("Recent activity", trailing: all ? "All sources" : "This source") {
            Toggle("Show the activity of all sources", isOn: $all)
            if let failure {
                Note(text: failure, warning: true)
            } else if let events {
                if events.isEmpty {
                    Text("The cache has no recorded activity for this selection.")
                        .foregroundStyle(.secondary)
                } else {
                    ForEach(Array(events.enumerated()), id: \.offset) { _, event in
                        ActivityLine(event: event)
                    }
                }
                Text("The cache keeps the newest 200 events. A missing event does not prove that nothing happened.")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .task(id: "\(row.key)|\(all)|\(model.generation)") {
            while !Task.isCancelled {
                await load()
                try? await Task.sleep(for: .seconds(5))
            }
        }
    }

    private func load() async {
        do {
            events = try await model.requireExplorer().sourceActivity(key: row.key, all: all)
            failure = nil
        } catch {
            failure = describe(error)
        }
    }
}

private struct ActivityLine: View {
    let event: ActivityRow
    @State private var expanded = false

    var body: some View {
        DisclosureGroup(isExpanded: $expanded) {
            Text(event.message)
                .font(.callout.monospaced())
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        } label: {
            HStack(spacing: 12) {
                Text(Format.age(event.observedAt)).monospacedDigit()
                    .foregroundStyle(.secondary)
                    .frame(width: 90, alignment: .leading)
                Text(event.status)
                    .frame(width: 110, alignment: .leading)
                Text(event.summary).lineLimit(1)
            }
        }
    }
}

/// A source's chain by name: the bridge's name, a resolved name, or the short form.
func sourceName(_ row: SourceRow, names: [String: String]) -> String {
    if let name = row.name { return name }
    if let name = names[row.systemId] {
        return name.hasSuffix("@") ? String(name.dropLast()) : name
    }
    return chainName(row.chain)
}

/// PBaaS chains are stored under their hex ID. Show a short form until a name is known.
func chainName(_ chain: String) -> String {
    chain.count == 40 ? "PBaaS chain \(chain.prefix(8))…" : chain
}

/// Carbide's short ages: <1s, 42s, 5m, 3h, 2d.
func shortAge(_ seconds: UInt64) -> String {
    switch seconds {
    case 0: "<1s"
    case 1..<60: "\(seconds)s"
    case 60..<3600: "\(seconds / 60)m"
    case 3600..<86400: "\(seconds / 3600)h"
    default: "\(seconds / 86400)d"
    }
}
