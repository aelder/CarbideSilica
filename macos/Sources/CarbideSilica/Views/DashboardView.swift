import CarbideCore
import SwiftUI

/// Chain status, recent blocks and recent confirmed transactions, refreshed while the
/// window is open.
struct DashboardView: View {
    @Environment(AppModel.self) private var model
    @State private var feed = DashboardFeed.shared
    /// Keep the newest block selected and in view, as the terminal's "Stream Blocks" row.
    @AppStorage("dashboardStreamsBlocks") private var streaming = true
    @FocusState private var focus: DashboardList?
    /// True when the status has been a stored answer for some time.
    @State private var storedLong = false

    var body: some View {
        Group {
            if let status = model.status {
                PlacePage(key: "dashboard") { proxy in
                    StatusLine(status: status)
                    // A stored answer for a short time is usual: at the start, and while the app
                    // reconciles new blocks. The source line says it. The large note comes only
                    // when the state stays, so that the page does not move down and up again.
                    if status.provenance.retained, storedLong {
                        ProvenanceNote(provenance: status.provenance)
                    }
                    if let fraction = model.syncFraction {
                        SyncBanner(fraction: fraction, tipTime: status.tipTime)
                    }
                    if let error = model.statusError {
                        Note(text: error, warning: true)
                    }
                    if !model.blocks.isEmpty {
                        Card("Chain pulse") {
                            ChainPulse(
                                blocks: model.blocks,
                                live: model.syncFraction == nil && !status.provenance.retained,
                                waiting: status.provenance.retained ? nil : model.node?.mempoolCount,
                                selected: feed.selectedBlock)
                        }
                    }
                    ViewThatFits(in: .horizontal) {
                        HStack(alignment: .top, spacing: 24) {
                            blockList(proxy).frame(minWidth: 430)
                            transactionList(proxy).frame(minWidth: 430)
                        }
                        VStack(alignment: .leading, spacing: 20) {
                            blockList(proxy)
                            transactionList(proxy)
                        }
                    }
                }
            } else if let error = model.statusError {
                NotConnectedView(message: error)
            } else {
                ProgressView("Connecting…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task(id: model.status?.provenance.retained == true) {
            storedLong = false
            guard model.status?.provenance.retained == true else { return }
            try? await Task.sleep(for: .seconds(45))
            if !Task.isCancelled { storedLong = true }
        }
        .navigationTitle("Carbide Silica")
        .publishPlace(PlaceActions(
                refresh: { Task { await model.refresh() } },
                streamBlocks: $streaming))
        .task(id: model.generation) {
            while !Task.isCancelled {
                await model.refresh()
                await feed.update(model)
                try? await Task.sleep(for: Launch.refreshInterval)
            }
        }
        .onAppear {
            if streaming { feed.selectedBlock = model.blocks.first?.hash }
        }
        .onChange(of: model.blocks.first?.hash) {
            if streaming { feed.selectedBlock = model.blocks.first?.hash }
        }
        .onChange(of: streaming) {
            if streaming { feed.selectedBlock = model.blocks.first?.hash }
        }
    }

    private var blocks: [BlockRow] { model.blocks + feed.older }

    private func blockList(_ proxy: ScrollViewProxy) -> some View {
        @Bindable var feed = feed
        return RecentBlocks(
            blocks: blocks, selection: $feed.selectedBlock, streaming: $streaming,
            loadingOlder: feed.loadingOlder, olderError: feed.olderError,
            loadOlder: { Task { await feed.loadOlder(model) } }
        )
    }

    private func transactionList(_ proxy: ScrollViewProxy) -> some View {
        @Bindable var feed = feed
        return RecentTransactions(
            rows: feed.transactions, loading: feed.loading, failure: feed.failure,
            selection: $feed.selectedTransaction, shown: $feed.transactionsShown
        )
    }

    /// Tab and Shift-Tab switch between the two lists, as in the terminal.
    private func switchList() -> KeyPress.Result {
        focus = focus == .blocks ? .transactions : .blocks
        return .handled
    }
}

/// The dashboard's two lists, for keyboard focus.
enum DashboardList: Hashable {
    case blocks
    case transactions
}

/// Recent confirmed transactions and older pages of blocks for the dashboard. It keeps what
/// it loaded, so a refresh requests only the blocks that are new.
@MainActor
@Observable
final class DashboardFeed {
    static let shared = DashboardFeed()

    /// Newest block first; within a block, last position first, as in the terminal.
    private(set) var transactions: [RecentTransaction] = []
    private(set) var loading = false
    private(set) var failure: String?
    /// Blocks older than the ones the app model keeps, loaded on request.
    private(set) var older: [BlockRow] = []
    private(set) var loadingOlder = false
    private(set) var olderError: String?
    /// The selections and the length of the transaction list, kept for the session.
    var selectedBlock: String?
    var selectedTransaction: String?
    var transactionsShown = 30

    private var loadedBlocks: Set<String> = []
    private var generation = -1

    private let blockWindow = 30
    private let transactionLimit = 100

    func update(_ model: AppModel) async {
        guard let explorer = model.explorer else { return }
        if generation != model.generation {
            generation = model.generation
            transactions = []
            loadedBlocks = []
            older = []
            olderError = nil
        }
        let window = Array(model.blocks.prefix(blockWindow))
        let current = Set(window.map(\.hash))
        let new = window.filter { !loadedBlocks.contains($0.hash) }.map(\.hash)
        guard !new.isEmpty, !loading else { return }
        loading = true
        defer { loading = false }
        let generation = model.generation
        do {
            let fetched = try await explorer.recentTransactions(
                blockHashes: new, limit: UInt32(transactionLimit))
            guard generation == model.generation else { return }
            var seen = Set<String>()
            // Rows of blocks that left the active chain are dropped with them.
            transactions = (fetched + transactions)
                .filter { current.contains($0.blockHash) }
                .sorted {
                    ($0.blockHeight, $0.position) > ($1.blockHeight, $1.position)
                }
                .filter { seen.insert($0.txid).inserted }
                .prefix(transactionLimit)
                .map { $0 }
            loadedBlocks.formUnion(new)
            loadedBlocks.formIntersection(current)
            failure = nil
        } catch {
            failure = describe(error)
        }
    }

    /// The next 30 blocks below the oldest one shown, as the terminal loads older pages.
    func loadOlder(_ model: AppModel) async {
        guard let explorer = model.explorer, !loadingOlder,
            let oldest = (older.last ?? model.blocks.last)?.height, oldest > 0
        else { return }
        loadingOlder = true
        defer { loadingOlder = false }
        do {
            let page = try await explorer.recentBlocks(before: oldest - 1, count: 30)
            let known = Set(model.blocks.map(\.hash) + older.map(\.hash))
            older += page.filter { !known.contains($0.hash) }
            olderError = nil
        } catch {
            olderError = describe(error)
        }
    }
}

/// The terminal's status line: tip against the longest chain, the age of the last block,
/// the node and API state, and the peer count.
private struct StatusLine: View {
    let status: ChainStatus
    @Environment(AppModel.self) private var model

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            VStack(alignment: .leading, spacing: 6) {
            SourceLine(provenance: status.provenance, short: model.currentChain)
            HStack(spacing: 18) {
                item(
                    "Height", value: Format.integer(status.blocks), detail: behind,
                    // When the node is not at the tip, the line has more items: no fixed width.
                    reserve: live ? "0,000,000  at the longest chain" : nil)
                // The chain pulse shows the time since the last block. When the node is
                // not at the tip, the pulse does not, thus the line shows it.
                if !live {
                    item(
                        "Last block",
                        value: status.tipTime.map { Format.age($0, now: context.date) } ?? "unknown",
                        detail: nil)
                }
                // The usual state has no label. The line says when the node is off.
                if !nodeOn { state("Node", on: false) }
                state("Public API", on: model.connection?.apiEndpoint != nil, inUse: status.source == .publicApi)
                item("Peers", value: status.connections.map { "\($0)" } ?? "unknown", detail: nil, reserve: "00")
                if let difficulty = status.difficulty {
                    item("Difficulty", value: Format.difficulty(difficulty), detail: nil, reserve: "0.00 T")
                }
                if let version = status.version {
                    item("Version", value: version, detail: nil)
                }
                Spacer(minLength: 0)
            }
            }
            .font(.callout)
            .lineLimit(1)
        }
        .accessibilityElement(children: .combine)
    }

    /// True when the node is at the tip and the data is not a stored observation.
    private var live: Bool { model.syncFraction == nil && !status.provenance.retained }

    private var nodeOn: Bool {
        status.source == .localDaemon && model.statusError == nil && !status.provenance.retained
    }

    /// "0 behind", as the terminal's Δ.
    private var behind: String {
        guard let longest = status.longestChain else { return "longest chain unknown" }
        let delta = Int64(longest) - Int64(status.blocks)
        return delta <= 0 ? "at the longest chain" : "\(Format.integer(UInt64(delta))) behind \(Format.integer(longest))"
    }

    /// `reserve` is the widest text of the value and its detail. The item keeps that width,
    /// so that the items after it do not move when the text gets longer or shorter.
    private func item(_ label: String, value: String, detail: String?, reserve: String? = nil) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            NameText(label)
            ZStack(alignment: .leading) {
                if let reserve {
                    Text(reserve).fontWeight(.semibold).monospacedDigit().hidden().accessibilityHidden(true)
                }
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(value).fontWeight(.semibold).monospacedDigit()
                    if let detail {
                        Text(detail).foregroundStyle(.secondary)
                    }
                }
            }
        }
    }

    /// State is a shape, a colour and a word.
    private func state(_ label: String, on: Bool, inUse: Bool = false) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: on ? "circle.fill" : "circle")
                .font(.system(size: 8))
                .foregroundStyle(on ? Tone.good : Tone.label)
            NameText(label)
            Text(on ? (inUse ? "in use" : "on") : "off").fontWeight(.semibold)
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
                Text("The node is not synchronized. It validated \(fraction.formatted(.percent.precision(.fractionLength(1)))) of the chain.")
                    .fontWeight(.semibold)
                Text(detail).font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Show progress") { model.show(.node) }
        }
        .padding(14)
        .background(Tone.screen)
        .overlay(Rectangle().strokeBorder(Tone.rule, lineWidth: 1))
    }

    private var detail: String {
        guard let tipTime else { return "This data is not current." }
        let date = Date(timeIntervalSince1970: TimeInterval(tipTime))
            .formatted(.dateTime.month(.wide).year())
        return "This data is from \(date). The node cannot find newer blocks, addresses or identities at this time."
    }
}

/// Recent blocks, in the page's own scroll. Up and Down select, Return or a double-click
/// opens, Tab moves to the transactions.
private struct RecentBlocks: View {
    let blocks: [BlockRow]
    @Binding var selection: String?
    @Binding var streaming: Bool
    let loadingOlder: Bool
    let olderError: String?
    let loadOlder: () -> Void

    @Environment(AppModel.self) private var model

    var body: some View {
        Card("Recent blocks", trailing: blocks.isEmpty ? nil : "\(blocks.count) blocks") {
            Toggle("Stream blocks", isOn: $streaming)
                .toggleStyle(.checkbox)
                .help("When this is on, the app selects each new block.")
            if blocks.isEmpty {
                Text("Waiting for data from the node.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 120)
            } else {
                TimelineView(.periodic(from: .now, by: 5)) { context in
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(blocks, id: \.hash) { block in
                            SelectableRow(id: block.hash, selection: $selection, open: { open(block.hash) }) {
                                BlockLine(block: block, now: context.date)
                            }
                            Divider()
                        }
                    }
                }
                .onChange(of: selection) {
                    // Selecting an older block stops the stream, as moving off the
                    // terminal's "Stream Blocks" row does.
                    if selection != blocks.first?.hash, selection != nil { streaming = false }
                }
                HStack {
                    if loadingOlder {
                        ProgressView().controlSize(.small)
                        Text("Loading older blocks…").foregroundStyle(.secondary)
                    } else if blocks.last?.height == 0 {
                        Text("The first block is reached.").foregroundStyle(.secondary)
                    } else {
                        Button("Load Older Blocks", action: loadOlder)
                    }
                    if let olderError {
                        Text(olderError).foregroundStyle(.secondary).lineLimit(2)
                    }
                }
            }
        }
    }

    private func open(_ hash: String) { model.open(.block(id: hash)) }
}

/// Recent confirmed transactions with their tags, from the blocks in the other list. The list
/// shows 30 rows at first and grows with the page.
private struct RecentTransactions: View {
    let rows: [RecentTransaction]
    let loading: Bool
    let failure: String?
    @Binding var selection: String?
    @Binding var shown: Int

    @Environment(AppModel.self) private var model

    var body: some View {
        Card("Recent transactions", trailing: rows.isEmpty ? nil : "\(rows.count) confirmed") {
            Text("The newest transactions in the recent blocks, newest block first.")
                .font(.callout)
                .foregroundStyle(.secondary)
            if let failure, rows.isEmpty {
                Note(text: failure, warning: true)
            } else if rows.isEmpty {
                HStack {
                    if loading { ProgressView().controlSize(.small) }
                    Text(loading ? "Loading transactions…" : "No transactions in recent blocks.")
                        .foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, minHeight: 120)
            } else {
                let visible = Array(rows.prefix(shown))
                TimelineView(.periodic(from: .now, by: 5)) { context in
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(visible, id: \.txid) { row in
                            SelectableRow(id: row.txid, selection: $selection, open: { open(row.txid) }) {
                                TransactionLine(row: row, now: context.date)
                            }
                            Divider()
                        }
                    }
                }
                if rows.count > shown {
                    Button("Show \(min(30, rows.count - shown)) More") { shown += 30 }
                }
            }
        }
    }

    private func open(_ txid: String) { model.open(.transaction(txid: txid)) }
}

private struct BlockLine: View {
    let block: BlockRow
    let now: Date
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 10) {
            Text(Format.integer(block.height)).fontWeight(.medium).monospacedDigit()
                .frame(width: 84, alignment: .leading)
            Plate(block.staked ? "Staked" : "Mined", fill: block.staked ? Tone.staked : Tone.mined)
                .frame(width: 64, alignment: .leading)
            Text(Format.age(block.time, now: now)).foregroundStyle(.secondary).monospacedDigit()
                .frame(width: 76, alignment: .leading)
            Text("\(block.transactionCount) tx").monospacedDigit()
                .frame(width: 44, alignment: .trailing)
            Button(Format.short(block.hash, head: 8, tail: 6)) { model.open(.block(id: block.hash)) }
                .buttonStyle(.displayLink)
                .font(.callout.monospaced())
                .help(block.hash)
            Spacer(minLength: 0)
        }
        .padding(.vertical, 5)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            "Block \(block.height), \(block.staked ? "staked" : "mined"), \(block.transactionCount) transactions")
    }
}

private struct TransactionLine: View {
    let row: RecentTransaction
    let now: Date
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 8) {
                Button(Format.short(row.txid, head: 10, tail: 6)) { model.open(.transaction(txid: row.txid)) }
                    .buttonStyle(.displayLink)
                    .font(.callout.monospaced())
                    .help(row.txid)
                Spacer(minLength: 4)
                Button("Block \(Format.integer(row.blockHeight))") { model.open(.block(id: row.blockHash)) }
                    .buttonStyle(.displayLink)
                    .foregroundStyle(.secondary)
                Text(Format.age(row.blockTime, now: now)).foregroundStyle(.secondary).monospacedDigit()
            }
            HStack(spacing: 6) {
                if row.loaded {
                    ForEach(row.tags.prefix(4), id: \.self) { Plate($0, fill: signalTagFill($0)) }
                    Text("\(row.inputCount) in, \(row.outputCount) out").foregroundStyle(.secondary)
                    if let sats = row.outputSats, sats > 0 {
                        CoinAmount(sats: sats).foregroundStyle(.secondary)
                    }
                } else {
                    Text("The details did not load.").foregroundStyle(.secondary)
                }
            }
            .font(.callout)
        }
        .padding(.vertical, 3)
        .accessibilityElement(children: .combine)
    }
}

/// The signal colour of a transaction tag: violet for staked, amber for mined. Other tags
/// have no colour and show as an outline.
func signalTagFill(_ tag: String) -> Color? {
    switch tag.lowercased() {
    case "staked", "stake": Tone.staked
    case "mined": Tone.mined
    default: nil
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
