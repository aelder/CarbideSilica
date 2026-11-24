import CarbideCore
import SwiftUI

/// Chain status and the most recent blocks, refreshed while the window is open.
struct DashboardView: View {
    @Environment(AppModel.self) private var model

    private let refreshInterval: Duration = .seconds(15)

    var body: some View {
        Group {
            if let status = model.status {
                Page {
                    if let fraction = model.syncFraction {
                        SyncBanner(fraction: fraction, tipTime: status.tipTime)
                    }
                    StatusTiles(status: status)
                    if let error = model.statusError {
                        Label(error, systemImage: "exclamationmark.triangle")
                            .foregroundStyle(.orange)
                    }
                    if !model.blocks.isEmpty {
                        Card("Chain pulse", trailing: "blocks of the last 25 minutes") {
                            ChainPulse(
                                blocks: model.blocks, live: model.syncFraction == nil,
                                waiting: model.node?.mempoolCount)
                        }
                    }
                    RecentBlocks(blocks: model.blocks)
                }
            } else if let error = model.statusError {
                NotConnectedView(message: error)
            } else {
                ProgressView("Connecting…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationTitle("Carbide Silica")
        .task(id: model.generation) {
            while !Task.isCancelled {
                await model.refresh()
                try? await Task.sleep(for: refreshInterval)
            }
        }
    }
}

/// Says plainly that the dashboard is showing history while the node catches up.
private struct SyncBanner: View {
    let fraction: Double
    let tipTime: UInt64?
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 12) {
            ProgressView(value: fraction)
                .progressViewStyle(.circular)
                .controlSize(.small)
            VStack(alignment: .leading, spacing: 2) {
                Text("The node is not fully synchronized. \(fraction.formatted(.percent.precision(.fractionLength(1)))) of the chain is validated.")
                    .fontWeight(.semibold)
                Text(detail).font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Show progress") { model.show(.node) }
        }
        .padding(14)
        .background(Color.accentColor.opacity(0.1), in: RoundedRectangle(cornerRadius: 10))
    }

    private var detail: String {
        guard let tipTime else { return "This data is not current." }
        let date = Date(timeIntervalSince1970: TimeInterval(tipTime))
            .formatted(.dateTime.month(.wide).year())
        return "This data is from \(date). The node cannot find newer blocks, addresses or identities at this time."
    }
}

private struct StatusTiles: View {
    let status: ChainStatus

    var body: some View {
        HStack(alignment: .top, spacing: 24) {
            Tile(label: "Height", value: Format.integer(status.blocks), detail: syncDetail)
            if let tip = status.tipTime {
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    Tile(
                        label: "Last block",
                        value: Format.age(tip, now: context.date),
                        detail: Format.date(tip))
                }
            }
            if let difficulty = status.difficulty {
                Tile(label: "Difficulty", value: Format.difficulty(difficulty), detail: nil)
            }
            if let connections = status.connections {
                Tile(label: "Peers", value: "\(connections)", detail: nil)
            }
            Tile(
                label: "Source",
                value: status.source == .publicApi ? "Public API" : "Local node",
                detail: status.version.map { "Verus \($0)" })
        }
    }

    private var syncDetail: String? {
        guard let longest = status.longestChain, longest > status.blocks else { return nil }
        return "\(Format.integer(longest - status.blocks)) behind"
    }
}

private struct Tile: View {
    let label: String
    let value: String
    let detail: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(label)
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
            Text(value)
                .font(.title3.weight(.semibold))
                .monospacedDigit()
                .lineLimit(1)
                .minimumScaleFactor(0.7)
            Text(detail ?? " ")
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct RecentBlocks: View {
    let blocks: [BlockRow]

    var body: some View {
        Card("Recent blocks") {
            if blocks.isEmpty {
                ProgressView()
                    .frame(maxWidth: .infinity)
                    .padding(.vertical, 24)
            } else {
                VStack(spacing: 0) {
                    BlockColumns(
                        height: Text("Height"), age: Text("Age"), kind: Text("Type"),
                        transactions: Text("Txs"), size: Text("Size"), hash: Text("Hash")
                    )
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                    .padding(.bottom, 6)
                    Divider()
                    TimelineView(.periodic(from: .now, by: 5)) { context in
                        VStack(spacing: 0) {
                            ForEach(blocks, id: \.hash) { block in
                                BlockRowView(block: block, now: context.date)
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One fixed column layout shared by the header and the rows.
private struct BlockColumns<A: View, B: View, C: View, D: View, E: View, F: View>: View {
    let height: A
    let age: B
    let kind: C
    let transactions: D
    let size: E
    let hash: F

    var body: some View {
        HStack(spacing: 12) {
            height.frame(width: 90, alignment: .leading)
            age.frame(width: 100, alignment: .leading)
            kind.frame(width: 80, alignment: .leading)
            transactions.frame(width: 50, alignment: .trailing)
            size.frame(width: 80, alignment: .trailing)
            hash.frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 8)
    }
}

private struct BlockRowView: View {
    let block: BlockRow
    let now: Date

    @Environment(AppModel.self) private var model
    @State private var hovering = false

    var body: some View {
        Button {
            model.open(.block(id: block.hash))
        } label: {
            BlockColumns(
                height: Text(Format.integer(block.height)).fontWeight(.medium).monospacedDigit(),
                age: Text(Format.age(block.time, now: now))
                    .foregroundStyle(.secondary).monospacedDigit(),
                kind: TagChip(
                    text: block.staked ? "Staked" : "Mined",
                    color: block.staked ? .purple : .orange),
                transactions: Text("\(block.transactionCount)").monospacedDigit(),
                size: Text(block.size.map(Format.bytes) ?? "–")
                    .foregroundStyle(.secondary).monospacedDigit(),
                hash: Text(block.hash)
                    .font(.callout.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            )
            .padding(.vertical, 7)
            .background(
                hovering ? AnyShapeStyle(.quaternary) : AnyShapeStyle(.clear),
                in: RoundedRectangle(cornerRadius: 6)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .accessibilityLabel("Block \(block.height)")
    }
}

/// Shown when neither the local daemon nor an allowed API answered.
private struct NotConnectedView: View {
    let message: String
    @Environment(AppModel.self) private var model

    var body: some View {
        ContentUnavailableView {
            Label("Not connected to Verus", systemImage: "bolt.horizontal.circle")
        } description: {
            VStack(spacing: 8) {
                Text(message)
                if let connection = model.connection {
                    Text(detail(connection))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
            }
        } actions: {
            Button("Try Again") { Task { await model.refresh() } }
            if model.connection?.apiEndpoint == nil {
                Button("Use Public API") { model.enablePublicAPI() }
                    .help(
                        "Send requests to \(SettingsKey.defaultAPI) when the local node cannot answer."
                    )
            }
            SettingsLink { Text("Settings…") }
        }
    }

    private func detail(_ connection: ConnectionInfo) -> String {
        if connection.configFound {
            return "The app tried the node at \(connection.daemonUrl)."
        }
        return "The app found no node configuration for \(connection.chain). "
            + "It tried \(connection.daemonUrl)."
    }
}
