import CarbideCore
import SwiftUI

/// Unconfirmed transactions in the local node's mempool, as a waiting list or as a table.
///
/// The mempool is empty most of the time, and each block takes all that waits. Thus the
/// first view is a waiting list on a time record: what waits, how long against the usual
/// wait for a block, and what left with the last blocks. The table shows the same data as
/// the terminal's mempool view.
struct MempoolView: View {
    @Environment(AppModel.self) private var model
    @State private var feed = MempoolFeed.shared
    @AppStorage("mempoolShowsList") private var showsList = false

    var body: some View {
        PlacePage(key: "mempool") { proxy in
            Totals(feed: feed)
            LampSegments(options: [(false, "Waiting"), (true, "List")], selection: $showsList)

            Card(showsList ? "Unconfirmed transactions" : "Waiting for a block", showsTitle: showsList) {
                if let failure = feed.failure, feed.entries.isEmpty {
                    Text(
                        model.status?.source == .publicApi
                            ? "Only a local node supplies the mempool. The public API does not."
                            : failure
                    )
                    .foregroundStyle(.secondary)
                } else if !feed.loaded {
                    ProgressView("Loading the mempool…").frame(maxWidth: .infinity, minHeight: 200)
                } else if showsList {
                    if feed.entries.isEmpty {
                        Text("The mempool is empty.").foregroundStyle(.secondary)
                    } else {
                        MempoolList(entries: feed.entries, feed: feed, proxy: proxy)
                    }
                } else {
                    WaitingRoom(feed: feed)
                }
            }
            if !showsList, feed.loaded, feed.failure == nil || !feed.entries.isEmpty {
                Card("Left with recent blocks") {
                    LeftList(feed: feed)
                }
            }
        }
        .navigationTitle("Mempool")
        .publishPlace(PlaceActions(refresh: { Task { await feed.refresh(model) } }, mempoolAsList: $showsList))
        // The page sets the mempool against the newest blocks, thus it keeps the chain
        // status fresh while it is open, as the dashboard does.
        .task(id: model.generation) {
            while !Task.isCancelled {
                await model.refresh()
                try? await Task.sleep(for: Launch.refreshInterval)
            }
        }
    }
}

/// The mempool with the terminal's loading pattern: decoded transactions in bulk on the first
/// load when the node supports it, then fast refreshes of membership and metadata, with one
/// request for each new entry.
@MainActor
@Observable
final class MempoolFeed {
    static let shared = MempoolFeed()
    /// The terminal refreshes its mempool every 2 seconds.
    static let interval: Duration = .seconds(2)

    private(set) var entries: [MempoolEntry] = []
    private(set) var totalBytes: UInt64 = 0
    private(set) var loaded = false
    private(set) var refreshing = false
    private(set) var failure: String?
    private(set) var lastRefresh: Date?
    /// The source of the last answer. The mempool is always live: it is never stored.
    private(set) var provenance: Provenance?
    /// The list's selection and order, kept for the session.
    var selection: String?
    var sort: MempoolSort = .arrival
    var descending = true

    /// The transactions that the app saw in the mempool in this session, by transaction ID.
    private(set) var sightings: [String: MempoolSighting] = [:]
    /// What the Mempool page says about a transaction: tags, statement, expiry and block.
    private(set) var details: [String: WaitingTransaction] = [:]
    /// The transactions of the last blocks that were not block rewards or stakes.
    private(set) var took: [RecentTransaction] = []
    /// The blocks whose transactions `took` holds, also when a block took none.
    private(set) var tookBlocks: Set<String> = []
    /// How many blocks back the page looks for what left.
    static let leftBlocks = 30
    /// A sighting stays for this long after the transaction left.
    static let memory: TimeInterval = 20 * 60

    private var bulkDone = false
    private var generation = -1
    /// Tags already loaded, by transaction ID. An entry keeps them across refreshes.
    private var classified: [String: [String]] = [:]
    /// Entries that a request could not find. They are not requested again.
    private var unavailable: Set<String> = []

    func refresh(_ model: AppModel) async {
        guard let explorer = model.explorer, !refreshing else { return }
        if generation != model.generation {
            generation = model.generation
            entries = []
            provenance = nil
            totalBytes = 0
            loaded = false
            bulkDone = false
            classified = [:]
            unavailable = []
            failure = nil
            sightings = [:]
            details = [:]
            took = []
            tookBlocks = []
        }
        refreshing = true
        defer { refreshing = false }
        let generation = model.generation
        do {
            let snapshot = try await explorer.mempoolSnapshot(full: !bulkDone)
            guard generation == model.generation else { return }
            bulkDone = true
            for entry in snapshot.entries where entry.classified {
                classified[entry.txid] = entry.tags
            }
            let members = Set(snapshot.entries.map(\.txid))
            classified = classified.filter { members.contains($0.key) }
            unavailable.formIntersection(members)
            entries = snapshot.entries.map(merged)
            totalBytes = snapshot.totalBytes
            provenance = snapshot.provenance
            failure = nil
            loaded = true
            lastRefresh = .now

            let missing = entries.filter { !$0.classified && !unavailable.contains($0.txid) }.map(\.txid)
            if !missing.isEmpty {
                let found = await explorer.classifyMempool(txids: missing)
                guard generation == model.generation else { return }
                for item in found {
                    if item.found {
                        classified[item.txid] = item.tags
                    } else {
                        unavailable.insert(item.txid)
                    }
                }
                entries = entries.map(merged)
            }
            note(members)
            await readDetails(explorer, model)
            await readTook(explorer, model)
        } catch {
            guard generation == model.generation else { return }
            failure = describe(error)
            provenance = nil
            loaded = true
            lastRefresh = .now
        }
    }

    /// Records arrivals and departures. The node gives the arrival time. The departure
    /// time is the refresh that first did not have the transaction.
    private func note(_ members: Set<String>) {
        let now = Date.now
        for entry in entries {
            if var known = sightings[entry.txid] {
                known.left = nil
                known.gone = false
                known.size = entry.size
                known.feeSats = entry.feeSats
                known.parents = entry.parents
                sightings[entry.txid] = known
            } else {
                sightings[entry.txid] = MempoolSighting(
                    txid: entry.txid,
                    arrival: entry.time.map { Date(timeIntervalSince1970: TimeInterval($0)) } ?? now,
                    size: entry.size, feeSats: entry.feeSats, parents: entry.parents)
            }
        }
        for (txid, sighting) in sightings where sighting.left == nil && !members.contains(txid) {
            sightings[txid]?.left = now
        }
        sightings = sightings.filter { $0.value.left.map { now.timeIntervalSince($0) < Self.memory } ?? true }
    }

    /// Reads the statement of each waiting transaction, the block of each one that left,
    /// and the statement of the newest transactions that the last blocks took.
    private func readDetails(_ explorer: Explorer, _ model: AppModel) async {
        let generation = model.generation
        let waiting = entries.map(\.txid).filter { details[$0] == nil }
        let departed = sightings.values
            .filter { $0.left != nil && $0.height == nil && !$0.gone }
            .map(\.txid)
        let recent = took.prefix(LeftList.shown).map(\.txid).filter { details[$0] == nil }
        let wanted = Array((departed + waiting + recent).prefix(12))
        guard !wanted.isEmpty else { return }
        let found = await explorer.waitingTransactions(txids: wanted)
        guard generation == model.generation else { return }
        for item in found {
            if item.found { details[item.txid] = item }
            guard var sighting = sightings[item.txid], sighting.left != nil else { continue }
            if let height = item.height {
                sighting.height = height
                sighting.blockHash = item.blockHash
            } else {
                // The node does not have it in a block. After some refreshes, say so.
                sighting.checks += 1
                if !item.found || sighting.checks >= 5 { sighting.gone = true }
            }
            sightings[item.txid] = sighting
        }
        let kept = Set(entries.map(\.txid)).union(sightings.keys).union(took.map(\.txid))
        details = details.filter { kept.contains($0.key) }
    }

    /// The transactions of the last blocks, without block rewards and stakes.
    private func readTook(_ explorer: Explorer, _ model: AppModel) async {
        let window = model.blocks.prefix(Self.leftBlocks).map(\.hash)
        let current = Set(window)
        let new = window.filter { !tookBlocks.contains($0) }
        guard !new.isEmpty else { return }
        let generation = model.generation
        guard let fetched = try? await explorer.recentTransactions(blockHashes: new, limit: 100),
            generation == model.generation
        else { return }
        let reward: Set<String> = ["Coinbase", "Stake"]
        var seen = Set<String>()
        took = (fetched.filter { reward.isDisjoint(with: $0.tags) } + took)
            .filter { current.contains($0.blockHash) }
            .sorted { ($0.blockHeight, $0.position) > ($1.blockHeight, $1.position) }
            .filter { seen.insert($0.txid).inserted }
        tookBlocks = tookBlocks.union(new).intersection(current)
    }

    private func merged(_ entry: MempoolEntry) -> MempoolEntry {
        guard let tags = classified[entry.txid] else { return entry }
        var entry = entry
        entry.classified = true
        entry.tags = tags
        return entry
    }
}

/// A transaction that the app saw in the mempool.
struct MempoolSighting {
    let txid: String
    /// When the node first had it.
    let arrival: Date
    var size: UInt64
    var feeSats: Int64?
    var parents: [String]
    /// The refresh that first did not have it. Nil while it waits.
    var left: Date?
    /// The block that took it.
    var height: UInt64?
    var blockHash: String?
    /// The node does not have it in a block: it expired, or another transaction replaced it.
    var gone = false
    var checks = 0
}

/// The terminal's mempool summary: count, serialized size, last block and refresh age.
private struct Totals: View {
    let feed: MempoolFeed
    @Environment(AppModel.self) private var model

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            VStack(alignment: .leading, spacing: 6) {
                SourceLine(provenance: feed.provenance, short: model.currentChain, labelled: true)
                // The instrument below has the count, the size and the last block.
                HStack(spacing: 18) {
                    item("Refresh", refresh(context.date))
                    Spacer(minLength: 0)
                }
            }
            .font(.callout)
            .lineLimit(1)
        }
        .accessibilityElement(children: .combine)
    }

    private func refresh(_ now: Date) -> String {
        guard let last = feed.lastRefresh else { return "every 2 s, waiting for the first refresh" }
        let seconds = max(Int(now.timeIntervalSince(last)), 0)
        return feed.refreshing ? "every 2 s, refreshing" : "every 2 s, updated \(seconds) s ago"
    }

    private func item(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            NameText(label)
            Text(value).fontWeight(.semibold).monospacedDigit()
        }
    }
}

/// The list's sort columns.
enum MempoolSort: String, CaseIterable {
    case arrival = "Arrival"
    case fee = "Fee"
    case size = "Size"
    case height = "Entry height"
    case parents = "Unconfirmed parents"
}

/// The same data as the tiles, as rows in the page. A column heading sorts. Up and Down
/// select, Return or a double-click opens the transaction.
private struct MempoolList: View {
    let entries: [MempoolEntry]
    @Bindable var feed: MempoolFeed
    let proxy: ScrollViewProxy
    @Environment(AppModel.self) private var model
    // Redraws the entry heights when the dates setting changes.
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    var body: some View {
        let rows = sorted
        let _ = showsDates
        VStack(alignment: .leading, spacing: 0) {
            Columns(
                transaction: NameText("Transaction"), arrival: header(.arrival), fee: header(.fee),
                size: header(.size), height: header(.height), parents: header(.parents),
                tags: NameText("Tags"))
                .padding(.horizontal, 8)
                .padding(.bottom, 6)
            Divider()
            TimelineView(.periodic(from: .now, by: 1)) { context in
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(rows, id: \.txid) { entry in
                        SelectableRow(id: entry.txid, selection: $feed.selection, open: { open(entry.txid) }) {
                            line(entry, now: context.date)
                        }
                        Divider()
                    }
                }
            }
        }
        Text("\"Unconfirmed parents\" is the number of mempool transactions that this transaction spends from. "
            + "\"Classification pending\" means that the app has not loaded the transaction yet.")
            .font(.caption)
            .foregroundStyle(.secondary)
    }

    private func open(_ txid: String) { model.open(.transaction(txid: txid)) }

    private var sorted: [MempoolEntry] {
        let ascending: (MempoolEntry, MempoolEntry) -> Bool =
            switch feed.sort {
            case .arrival: { ($0.time ?? 0, $0.txid) < ($1.time ?? 0, $1.txid) }
            case .fee: { ($0.feeSats ?? -1, $0.txid) < ($1.feeSats ?? -1, $1.txid) }
            case .size: { ($0.size, $0.txid) < ($1.size, $1.txid) }
            case .height: { ($0.height ?? 0, $0.txid) < ($1.height ?? 0, $1.txid) }
            case .parents: { ($0.parents.count, $0.txid) < ($1.parents.count, $1.txid) }
            }
        let list = entries.sorted(by: ascending)
        return feed.descending ? list.reversed() : list
    }

    private func header(_ column: MempoolSort) -> some View {
        Button {
            if feed.sort == column {
                feed.descending.toggle()
            } else {
                feed.sort = column
                feed.descending = true
            }
        } label: {
            HStack(spacing: 3) {
                NameText(column.rawValue)
                if feed.sort == column {
                    Image(systemName: feed.descending ? "chevron.down" : "chevron.up")
                        .font(.system(size: 8, weight: .semibold))
                        .foregroundStyle(Tone.present)
                        .accessibilityLabel(feed.descending ? "descending" : "ascending")
                }
            }
        }
        .buttonStyle(.plain)
        .help("Sort by \(column.rawValue.lowercased())")
    }

    private func line(_ entry: MempoolEntry, now: Date) -> some View {
        Columns(
            transaction: Button(Format.short(entry.txid, head: 10, tail: 6)) { open(entry.txid) }
                .buttonStyle(.displayLink)
                .font(.callout.monospaced())
                .help(entry.txid),
            arrival: Text(entry.time.map { Format.age($0, now: now) } ?? "unknown"),
            fee: Text(entry.feeSats.map { "\(Format.coins($0)) \(model.coin)" } ?? "unknown"),
            size: Text(Format.bytes(entry.size)),
            height: Text(entry.height.map { HeightDates.shared.text($0, model: model) } ?? "unknown"),
            parents: Text(entry.parents.isEmpty ? "none" : "\(entry.parents.count)"),
            tags: tags(entry))
            .monospacedDigit()
            .padding(.vertical, 5)
    }

    @ViewBuilder
    private func tags(_ entry: MempoolEntry) -> some View {
        if !entry.classified {
            Text("classification pending").foregroundStyle(.secondary)
        } else if entry.tags.isEmpty {
            Text("no tags").foregroundStyle(.secondary)
        } else {
            HStack(spacing: 4) {
                ForEach(entry.tags, id: \.self) { Plate($0, fill: signalTagFill($0)) }
            }
        }
    }
}

/// One column layout for the list's heading and rows. The columns fit the page width.
private struct Columns<A: View, B: View, C: View, D: View, E: View, F: View, G: View>: View {
    let transaction: A
    let arrival: B
    let fee: C
    let size: D
    let height: E
    let parents: F
    let tags: G

    var body: some View {
        HStack(spacing: 10) {
            transaction.frame(width: 150, alignment: .leading)
            arrival.frame(width: 70, alignment: .leading)
            fee.frame(width: 120, alignment: .trailing)
            size.frame(width: 70, alignment: .trailing)
            height.frame(minWidth: 80, maxWidth: 170, alignment: .trailing)
            parents.frame(width: 80, alignment: .trailing)
            tags.frame(maxWidth: .infinity, alignment: .leading)
        }
        .lineLimit(1)
    }
}

enum MempoolText {
    /// A length of time in words: seconds to two minutes, then minutes.
    static func span(_ seconds: Double) -> String {
        let seconds = max(Int(seconds.rounded()), 0)
        if seconds < 120 { return "\(seconds) s" }
        if seconds < 3600 { return "\(seconds / 60) min" }
        return "\(seconds / 3600) h \(seconds % 3600 / 60) min"
    }

    static func count(_ number: Int, _ one: String, _ many: String) -> String {
        "\(number) \(number == 1 ? one : many)"
    }
}

/// What waits for a block: one sentence, a time record with the blocks on it, a row for
/// each transaction, and the part of a block that the queue fills.
///
/// The rows are in order of arrival and do not move when a transaction arrives. A
/// transaction that spends from another waiting transaction is below it, with an indent.
private struct WaitingRoom: View {
    let feed: MempoolFeed
    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @State private var record = WaitRecord.shared
    /// The kind that the rows are limited to. Nil shows each kind.
    @State private var kind: String?

    /// The serialized size that one block can hold: the consensus limit of Verus.
    static let blockLimit: UInt64 = 2_000_000
    /// Above this many rows, the page shows the number of each kind first.
    static let manyRows = 12
    static let untagged = "No tag"

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            let now = context.date
            VStack(alignment: .leading, spacing: 14) {
                // The time record and the next block are one instrument.
                MempoolTimeRecord(feed: feed, now: now, nextBlock: feed.entries.isEmpty ? nil : feed.totalBytes)
                analysis(now)
                if !feed.entries.isEmpty {
                    if feed.entries.count > Self.manyRows { kinds }
                    rows(now)
                }
            }
        }
        .task(id: model.generation) { await record.keep(model, live: true) }
    }

    // MARK: The sentence

    private func lastBlock(_ now: Date) -> (wait: Double, took: Int?)? {
        guard let block = model.blocks.first else { return nil }
        // The time of the block, as in the line of totals and in the record of waits.
        let seen = Date(timeIntervalSince1970: TimeInterval(block.time))
        let took = feed.tookBlocks.contains(block.hash)
            ? feed.took.filter { $0.blockHash == block.hash }.count : nil
        return (max(now.timeIntervalSince(seen), 0), took)
    }

    private func sentence(_ now: Date) -> String {
        let waiting = feed.entries.count
        var parts = [
            waiting == 0
                ? "Nothing waits."
                : "\(MempoolText.count(waiting, "transaction waits", "transactions wait"))."
        ]
        guard let last = lastBlock(now) else { return parts[0] }
        var block = "The last block came \(MempoolText.span(last.wait)) ago"
        if let share = record.shareShorter(than: last.wait), share >= 0.5 {
            block += ", longer than \(Int((share * 100).rounded(.down))) of 100 recent waits"
        }
        parts.append(block + ".")
        if waiting == 0, let took = last.took {
            parts.append(
                took == 0
                    ? "It took no transaction from the mempool."
                    : "It took \(MempoolText.count(took, "transaction", "transactions")).")
        }
        if model.syncFraction != nil {
            parts.append("A node does not collect unconfirmed transactions during synchronization.")
        }
        return parts.joined(separator: " ")
    }

    // MARK: The analysis

    /// What the mempool is, what waits in it now, and what that means for a person who has
    /// a transaction in it.
    private func analysis(_ now: Date) -> some View {
        let kinds = kindCounts(feed.entries)
        return VStack(alignment: .leading, spacing: 14) {
            Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 22, verticalSpacing: 10) {
                ForEach(meaning(now), id: \.name) { point in
                    GridRow {
                        NameText(point.name).gridColumnAlignment(.leading)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(point.value).font(Tone.figureSmall).monospacedDigit()
                            if let note = point.note {
                                Text(note)
                                    .font(Tone.caption)
                                    .foregroundStyle(Tone.label)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }
                    }
                }
            }
            // With one kind, the first line says all that the table says.
            if kinds.count > 1 {
                PanelRule()
                kindTable(now)
            }
        }
        .padding(.vertical, 4)
    }

    /// One point of the analysis: a name, a short value and, where it helps, what it means.
    private struct Point {
        let name: String
        let value: String
        var note: String?
    }

    /// The usual wait for a block, from the record of the last day: the middle wait and the
    /// wait that 9 of 10 are below.
    private var usualWait: (middle: Double, most: Double)? {
        guard let waits = record.history?.waits, waits.count >= 20 else { return nil }
        return (Double(waits[waits.count / 2]), Double(waits[waits.count * 9 / 10]))
    }

    private func usualPoint() -> Point? {
        guard let usual = usualWait else { return nil }
        return Point(
            name: "Usual wait", value: MempoolText.span(usual.middle),
            note: "In the last day, half of the blocks came within this time of the block before. "
                + "9 of 10 came within \(MempoolText.span(usual.most)).")
    }

    private func meaning(_ now: Date) -> [Point] {
        var points: [Point] = []
        let entries = feed.entries
        if entries.isEmpty {
            points.append(Point(
                name: "Mempool", value: "Empty",
                note: "The mempool holds the transactions that no block has yet. A transaction stays "
                    + "here until a miner or a staker puts it in a block."))
            if let usual = usualPoint() { points.append(usual) }
            let passed = feed.sightings.values.filter { $0.left != nil && !$0.gone }
            if !passed.isEmpty {
                let longest = passed.compactMap { sighting -> Double? in
                    sighting.left.map { $0.timeIntervalSince(sighting.arrival) }
                }.max() ?? 0
                points.append(Point(
                    name: "Went into a block", value: MempoolText.count(passed.count, "transaction", "transactions"),
                    note: "This is the count while this page was open. The longest wait was \(MempoolText.span(longest))."))
            }
            return points
        }

        // What waits, by kind.
        let names = kindCounts(entries).map { "\($0.count) \($0.name.lowercased())" }
        points.append(Point(
            name: "Waiting", value: MempoolText.count(entries.count, "transaction", "transactions"),
            note: names.joined(separator: ", ")))

        // How long, and how long more.
        if let oldest = entries.compactMap(\.time).min() {
            points.append(Point(
                name: "Oldest", value: "\(MempoolText.span(now.timeIntervalSince1970 - Double(oldest))) ago"))
        }
        if let usual = usualPoint() { points.append(usual) }

        // Room, fees, and what can stop a transaction.
        points.append(
            feed.totalBytes <= Self.blockLimit
                ? Point(name: "Next block", value: "All fit")
                : Point(
                    name: "Next block", value: "Not all fit",
                    note: "A block takes the transactions with the higher fee for each byte first."))
        let fees = entries.compactMap(\.feeSats)
        if !fees.isEmpty {
            points.append(Point(name: "Fees", value: "\(Format.coins(fees.reduce(0, +))) \(model.coin)"))
        }
        let waiting = Set(entries.map(\.txid))
        let chained = entries.filter { $0.parents.contains(where: waiting.contains) }.count
        if chained > 0 {
            points.append(Point(
                name: "Spend from waiting", value: MempoolText.count(chained, "transaction", "transactions"),
                note: "Each spends from a transaction that also waits. A block must take that one first or with it."))
        }
        if let tip = model.status?.blocks {
            let soon = entries.filter { entry in
                guard let expiry = feed.details[entry.txid]?.expiryHeight else { return false }
                return expiry <= tip + 10
            }.count
            if soon > 0 {
                points.append(Point(
                    name: "Expire soon", value: MempoolText.count(soon, "transaction", "transactions"),
                    note: "These expire in 10 blocks or less. A transaction that expires leaves the mempool and does not go into a block."))
            }
        }
        return points
    }

    private func kindCounts(_ entries: [MempoolEntry]) -> [(name: String, count: Int, bytes: UInt64, fees: Int64, oldest: UInt64?)] {
        var order: [String] = []
        var groups: [String: [MempoolEntry]] = [:]
        for entry in entries {
            let name = kindOf(entry) ?? "Not read yet"
            if groups[name] == nil { order.append(name) }
            groups[name, default: []].append(entry)
        }
        return order.map { name in
            let members = groups[name] ?? []
            return (
                name == Self.untagged ? "Payment" : name, members.count, members.map(\.size).reduce(0, +),
                members.compactMap(\.feeSats).reduce(0, +), members.compactMap(\.time).min()
            )
        }
        .sorted { $0.count > $1.count }
    }

    /// One row for each kind of transaction that waits.
    private func kindTable(_ now: Date) -> some View {
        Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 26, verticalSpacing: 5) {
            GridRow {
                NameText("Kind")
                NameText("Count").gridColumnAlignment(.trailing)
                NameText("Size").gridColumnAlignment(.trailing)
                NameText("Fees, \(model.coin)").gridColumnAlignment(.trailing)
                NameText("Oldest")
            }
            ForEach(kindCounts(feed.entries), id: \.name) { kind in
                GridRow {
                    Text(kind.name)
                    Text("\(kind.count)").monospacedDigit()
                    Text(Format.bytes(kind.bytes)).monospacedDigit()
                    Text(Format.coins(kind.fees)).monospacedDigit()
                    Text(kind.oldest.map { "\(MempoolText.span(now.timeIntervalSince1970 - Double($0))) ago" } ?? "–")
                        .foregroundStyle(Tone.label)
                }
            }
        }
        .padding(.top, 2)
    }

    // MARK: The rows

    private func kindOf(_ entry: MempoolEntry) -> String? {
        entry.classified ? (entry.tags.first ?? Self.untagged) : nil
    }

    private var kinds: some View {
        let counts = Dictionary(grouping: feed.entries.compactMap(kindOf), by: { $0 }).mapValues(\.count)
        return FlowLayout(spacing: 6) {
            ForEach(counts.sorted { ($0.value, $1.key) > ($1.value, $0.key) }, id: \.key) { name, count in
                Button {
                    kind = kind == name ? nil : name
                } label: {
                    Plate("\(name) \(count)", fill: kind == name ? Tone.present : nil)
                }
                .buttonStyle(.plain)
                .help(kind == name ? "Show each kind." : "Show only this kind.")
            }
        }
    }

    /// Order of arrival. A transaction that spends from a waiting transaction comes
    /// directly below it.
    private var ordered: [(entry: MempoolEntry, child: Bool)] {
        let waiting = Set(feed.entries.map(\.txid))
        let byArrival = feed.entries.sorted { ($0.time ?? 0, $0.txid) < ($1.time ?? 0, $1.txid) }
        var placed = Set<String>()
        var result: [(MempoolEntry, Bool)] = []
        func place(_ entry: MempoolEntry, child: Bool) {
            guard placed.insert(entry.txid).inserted else { return }
            result.append((entry, child))
            for next in byArrival where next.parents.contains(entry.txid) { place(next, child: true) }
        }
        for entry in byArrival where entry.parents.allSatisfy({ !waiting.contains($0) }) {
            place(entry, child: false)
        }
        for entry in byArrival { place(entry, child: false) }
        return result
    }

    private func rows(_ now: Date) -> some View {
        let shown = ordered.filter { kind == nil || kindOf($0.entry) == kind }
        return VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(shown.enumerated()), id: \.element.entry.txid) { position, row in
                if position > 0 { Divider() }
                Button {
                    model.open(.transaction(txid: row.entry.txid))
                } label: {
                    MempoolRow(
                        tags: row.entry.classified ? row.entry.tags : nil,
                        words: words(row.entry.txid),
                        facts: facts(row.entry, now: now),
                        txid: row.entry.txid)
                        .padding(.leading, row.child ? 22 : 0)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Open this transaction.")
                .cursorStop(token: row.entry.txid, copy: row.entry.txid) {
                    model.open(.transaction(txid: row.entry.txid))
                }
            }
        }
    }

    private func words(_ txid: String) -> String? {
        _ = showsNames
        guard let detail = feed.details[txid], let statement = detail.statement else { return nil }
        return StatementWords(
            statement: statement, coin: detail.coin, tags: detail.tags, coinbase: detail.coinbase,
            feePool: detail.feePoolCheckpoint, model: model
        ).sentence
    }

    private func facts(_ entry: MempoolEntry, now: Date) -> String {
        var parts: [String] = []
        if let time = entry.time {
            parts.append("Waited \(MempoolText.span(now.timeIntervalSince1970 - Double(time)))")
        }
        if let fee = entry.feeSats { parts.append("fee \(Format.coins(fee)) \(model.coin)") }
        parts.append(Format.bytes(entry.size))
        if let detail = feed.details[entry.txid], let tip = model.status?.blocks {
            if let expiry = detail.expiryHeight {
                parts.append(
                    expiry > tip
                        ? "expires in \(MempoolText.count(Int(expiry - tip), "block", "blocks"))"
                        : "at its expiry height")
            } else {
                parts.append("does not expire")
            }
        }
        let waiting = Set(feed.entries.map(\.txid))
        let parents = entry.parents.filter(waiting.contains).count
        if parents > 0 {
            parts.append("spends from \(MempoolText.count(parents, "transaction", "transactions")) that also \(parents == 1 ? "waits" : "wait")")
        }
        return parts.joined(separator: " · ")
    }
}

/// One transaction: its tags, what it does, and its facts.
private struct MempoolRow: View {
    /// Nil until the app has the transaction.
    let tags: [String]?
    /// The sentence of the statement. Nil until the app has read it.
    let words: String?
    let facts: String
    let txid: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                if let tags, !tags.isEmpty {
                    HStack(spacing: 4) {
                        ForEach(tags.prefix(4), id: \.self) { Plate($0, fill: signalTagFill($0)) }
                    }
                }
                Text(words.flatMap { $0.isEmpty ? nil : $0 } ?? (tags == nil ? "Classification pending." : "The app reads this transaction."))
                    .foregroundStyle(words == nil ? .secondary : .primary)
                    .fixedSize(horizontal: false, vertical: true)
                    .multilineTextAlignment(.leading)
                if !facts.isEmpty {
                    Text(facts).font(.callout).foregroundStyle(.secondary).monospacedDigit()
                }
            }
            Spacer(minLength: 8)
            Text(Format.short(txid, head: 8, tail: 6))
                .font(.callout.monospaced())
                .foregroundStyle(.secondary)
        }
        .padding(.vertical, 7)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The last 15 minutes on one time axis that ends at the present. Each block is a mark:
/// up for a staked block, down for a mined block, as in the chain pulse. Each transaction
/// that the app saw in the mempool is a line from its arrival to the block that took it.
/// A line that reaches the right edge is a transaction that waits now. Below the record,
/// a bar shows the part of the next block that the waiting transactions fill.
private struct MempoolTimeRecord: View {
    let feed: MempoolFeed
    let now: Date
    /// The serialized size of what waits. Nil when nothing waits: the panel then has no bar.
    var nextBlock: UInt64?
    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    /// The transaction or the block under the pointer.
    @State private var pointed: Target?

    private enum Target: Equatable {
        case transaction(String)
        case block(String)
    }

    private let span: Double = 15 * 60
    private let gutter: CGFloat = 92
    private let rightInset: CGFloat = 22
    private let laneStep: CGFloat = 12
    private let maxLanes = 8
    /// The part for the transactions has room for this many lines also when it has none.
    private let minLanes = 4
    /// The middle of the first transaction lane.
    private let lanesTop: CGFloat = 30
    /// The width of a lane's plate.
    private let plateWidth: CGFloat = 60

    /// The end of a sighting's line: the present, the arrival of its block, or the refresh
    /// that first did not have it.
    private func end(_ sighting: MempoolSighting) -> Date {
        guard let left = sighting.left else { return now }
        if let hash = sighting.blockHash, let arrived = model.arrivals[hash] { return min(arrived, left) }
        return left
    }

    private var lines: [(sighting: MempoolSighting, lane: Int)] {
        let start = now.addingTimeInterval(-span)
        let visible = feed.sightings.values
            .filter { end($0) > start }
            .sorted { ($0.arrival, $0.txid) < ($1.arrival, $1.txid) }
        var free: [Date] = []
        var result: [(MempoolSighting, Int)] = []
        for sighting in visible {
            let lane = free.firstIndex { $0 <= sighting.arrival } ?? free.count
            if lane == free.count { free.append(.distantPast) }
            free[lane] = end(sighting).addingTimeInterval(4)
            result.append((sighting, min(lane, maxLanes - 1)))
        }
        return result
    }

    /// The line of the blocks. The staked marks are above it and the mined marks below.
    private func axisY(_ lanes: Int) -> CGFloat { lanesTop + CGFloat(lanes) * laneStep + 34 }

    var body: some View {
        let lines = lines
        let lanes = max((lines.map(\.lane).max() ?? -1) + 1, minLanes)
        DisplayPanel(padding: 0) {
            VStack(alignment: .leading, spacing: 0) {
                readouts
                    .padding(.horizontal, 18)
                    .padding(.top, 14)
                    .padding(.bottom, 10)
                PanelRule()
                GeometryReader { geometry in
                    let shapes = shapes(width: geometry.size.width, lines: lines, lanes: lanes)
                    Canvas { context, size in
                        draw(context, size, lines: lines, lanes: lanes)
                    }
                    .contentShape(Rectangle())
                    // A line and a mark can be opened: a click on a line opens its
                    // transaction, and a click on a mark opens its block.
                    .onContinuousHover { phase in
                        switch phase {
                        case .active(let point): pointed = target(at: point, in: shapes)
                        case .ended: pointed = nil
                        }
                    }
                    .onTapGesture(coordinateSpace: .local) { point in
                        switch target(at: point, in: shapes) {
                        case .transaction(let txid)?: model.open(.transaction(txid: txid))
                        case .block(let hash)?: model.open(.block(id: hash))
                        case nil: break
                        }
                    }
                    // The pulse of the chain: with each second a front goes from the present
                    // to the left and gives light to the line, the blocks and the transactions.
                    .overlay {
                        LightTrace(
                            parts: [
                                .init(path: shapes.axis, stroke: true, color: LightTrace.warm, glow: LightTrace.warm),
                                // The guides, the marks and the lines get their new places at
                                // the front. The guides get no light.
                                .init(
                                    path: shapes.guides, stroke: true, color: Self.noLight, glow: Self.noLight,
                                    base: NSColor(Tone.grid).cgColor),
                                .init(
                                    path: shapes.staked, stroke: false, color: Self.stakedLight, glow: Self.stakedGlow,
                                    base: NSColor(Tone.staked).cgColor, baseGlow: 0.6),
                                .init(
                                    path: shapes.mined, stroke: false, color: Self.minedLight, glow: Self.minedGlow,
                                    base: NSColor(Tone.mined).cgColor, baseGlow: 0.6),
                                .init(
                                    path: shapes.waiting, stroke: false, color: LightTrace.pale, glow: LightTrace.blue,
                                    base: NSColor(Tone.present).cgColor, baseGlow: 0.7),
                                .init(
                                    path: shapes.taken, stroke: false, color: LightTrace.pale, glow: LightTrace.blue,
                                    base: NSColor(Tone.record).cgColor),
                                .init(
                                    path: shapes.gone, stroke: false, color: LightTrace.pale, glow: LightTrace.blue,
                                    base: NSColor(Tone.alert).cgColor),
                            ],
                            area: CGRect(
                                x: gutter, y: lanesTop - 10, width: max(geometry.size.width - rightInset - gutter, 0),
                                height: axisY(lanes) + 22 - (lanesTop - 10)),
                            beat: Int(now.timeIntervalSince1970)
                        )
                        .allowsHitTesting(false)
                    }
                }
                .frame(height: axisY(lanes) + 46)
                .accessibilityLabel(spoken(lines))
                // What the line or the mark under the pointer is, above the lines.
                .overlay(alignment: .topLeading) {
                    if let pointed {
                        Text(caption(pointed))
                            .font(Tone.caption)
                            .foregroundStyle(Tone.value)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .padding(.leading, gutter)
                            .padding(.trailing, 80)
                            .padding(.top, 6)
                            .allowsHitTesting(false)
                    }
                }
            }
        }
    }

    // MARK: Shapes, the pointer and the pulse

    private static let stakedLight = CGColor(red: 0.88, green: 0.87, blue: 0.9, alpha: 1)
    private static let minedLight = CGColor(red: 0.9, green: 0.86, blue: 0.68, alpha: 1)
    private static let stakedGlow = CGColor(red: 0.56, green: 0.3, blue: 0.86, alpha: 1)
    private static let minedGlow = CGColor(red: 1, green: 0.5, blue: 0.12, alpha: 1)

    private static let noLight = CGColor(gray: 0, alpha: 0)

    private struct Shapes {
        var axis = CGMutablePath()
        /// A guide up from each block through the transactions.
        var guides = CGMutablePath()
        var staked = CGMutablePath()
        var mined = CGMutablePath()
        /// Transactions that wait, that a block took, and that left without a block.
        var waiting = CGMutablePath()
        var taken = CGMutablePath()
        var gone = CGMutablePath()
        /// Where each transaction and each block is, for the pointer.
        var targets: [(target: Target, box: CGRect)] = []
    }

    /// The same places as the drawing, as shapes: for the light of the pulse and for the
    /// pointer.
    private func shapes(width: CGFloat, lines: [(sighting: MempoolSighting, lane: Int)], lanes: Int) -> Shapes {
        var shapes = Shapes()
        let start = now.addingTimeInterval(-span)
        let right = width - rightInset
        let axis = axisY(lanes)
        func x(_ date: Date) -> CGFloat {
            gutter + CGFloat(min(max(date.timeIntervalSince(start) / span, 0), 1)) * (right - gutter)
        }
        shapes.axis.move(to: CGPoint(x: gutter, y: axis))
        shapes.axis.addLine(to: CGPoint(x: right, y: axis))
        for line in lines {
            let y = lanesTop + CGFloat(line.lane) * laneStep
            let from = x(line.sighting.arrival)
            let to = max(x(end(line.sighting)), from + 3)
            let box = CGRect(x: from, y: y - 1.5, width: to - from, height: 3)
            if line.sighting.left == nil {
                shapes.waiting.addRect(box)
            } else if line.sighting.gone {
                // A line of dashes.
                for dash in stride(from: from, to: to, by: 9) {
                    shapes.gone.addRect(CGRect(x: dash, y: y - 1.5, width: min(2, to - dash), height: 3))
                }
            } else {
                shapes.taken.addRect(box)
                // The end of the line: the block that took the transaction.
                shapes.taken.addRect(CGRect(x: to - 1, y: y - 4, width: 2, height: 8))
            }
            shapes.targets.append((.transaction(line.sighting.txid), box.insetBy(dx: -3, dy: -4)))
        }
        for block in model.blocks {
            let seen = model.arrivals[block.hash] ?? Date(timeIntervalSince1970: TimeInterval(block.time))
            guard seen > start else { continue }
            let position = min(x(seen), right - 5)
            shapes.guides.move(to: CGPoint(x: position, y: lanesTop - 10))
            shapes.guides.addLine(to: CGPoint(x: position, y: axis))
            let mark = CGRect(x: position - 4.5, y: block.staked ? axis - 16 : axis + 7, width: 9, height: 9)
            if block.staked { shapes.staked.addEllipse(in: mark) } else { shapes.mined.addRect(mark) }
            shapes.targets.append((.block(block.hash), mark.insetBy(dx: -4, dy: -4)))
        }
        return shapes
    }

    private func target(at point: CGPoint, in shapes: Shapes) -> Target? {
        shapes.targets.first { $0.box.contains(point) }?.target
    }

    /// What the line or the mark under the pointer is.
    private func caption(_ target: Target) -> String {
        _ = showsNames
        switch target {
        case .transaction(let txid):
            var parts: [String] = []
            if let detail = feed.details[txid], let statement = detail.statement {
                let sentence = StatementWords(
                    statement: statement, coin: detail.coin, tags: detail.tags, coinbase: detail.coinbase,
                    feePool: detail.feePoolCheckpoint, model: model
                ).sentence
                if !sentence.isEmpty { parts.append(sentence) }
            }
            if parts.isEmpty { parts.append("Transaction \(Format.short(txid, head: 10, tail: 6)).") }
            if let sighting = feed.sightings[txid] {
                var facts: [String] = []
                if sighting.left == nil {
                    facts.append("Waits for \(MempoolText.span(now.timeIntervalSince(sighting.arrival)))")
                } else if sighting.gone {
                    facts.append("Left the mempool and is not in a block")
                } else {
                    facts.append("Waited \(MempoolText.span(end(sighting).timeIntervalSince(sighting.arrival)))")
                    if let height = sighting.height { facts.append("block \(Format.integer(height)) took it") }
                }
                if let fee = sighting.feeSats { facts.append("fee \(Format.coins(fee)) \(model.coin)") }
                facts.append(Format.bytes(sighting.size))
                parts.append(facts.joined(separator: " · ") + ". Click to open.")
            }
            return parts.joined(separator: " ")
        case .block(let hash):
            guard let block = model.blocks.first(where: { $0.hash == hash }) else { return "" }
            var text = "Block \(Format.integer(block.height)), \(block.staked ? "staked" : "mined"), with "
                + "\(MempoolText.count(Int(block.transactionCount), "transaction", "transactions"))."
            if feed.tookBlocks.contains(hash) {
                let took = feed.took.filter { $0.blockHash == hash }.count
                text += took == 0
                    ? " It took no transaction from the mempool."
                    : " It took \(MempoolText.count(took, "transaction", "transactions")) from the mempool."
            }
            return text + " Click to open."
        }
    }

    // MARK: Readouts

    /// The figures that answer first: how much waits, how full the next block is, and how
    /// long ago the last block came.
    private var readouts: some View {
        let waiting = feed.entries.count
        let share = Double(feed.totalBytes) / Double(WaitingRoom.blockLimit)
        let percent = share * 100
        let seen = lines.count
        func row(_ spacing: CGFloat) -> some View {
            HStack(alignment: .lastTextBaseline, spacing: spacing) {
                Readout(
                    "Waiting", feed.loaded ? "\(waiting)" : "–", tone: waiting > 0 ? Tone.present : Tone.value,
                    reserve: "00")
                Readout("Size", feed.loaded ? Format.bytes(feed.totalBytes) : "–", reserve: "Zero KB")
                HStack(alignment: .lastTextBaseline, spacing: 8) {
                    NameText("Next block")
                    Canvas { context, size in
                        context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(Tone.grid))
                        let filled = CGRect(x: 0, y: 0, width: size.width * min(share, 1), height: size.height)
                        context.fill(Path(filled), with: .color(share <= 1 ? Tone.record : Tone.alert))
                        context.fill(Path(CGRect(x: size.width - 1, y: -2, width: 1, height: size.height + 4)), with: .color(Tone.axis))
                    }
                    .frame(width: 110, height: 8)
                    .alignmentGuide(.lastTextBaseline) { $0[.bottom] - 3 }
                    ZStack(alignment: .leading) {
                        // The widest value, so that the figures after it keep their places.
                        Text("0.00%").font(Tone.figure).monospacedDigit().hidden().accessibilityHidden(true)
                        Text(waiting == 0 ? "0%" : percent < 1 ? String(format: "%.2f%%", percent) : String(format: "%.0f%%", percent))
                            .font(Tone.figure).monospacedDigit()
                            .foregroundStyle(share <= 1 ? Tone.value : Tone.alert)
                    }
                }
                .help("The part of one block that the waiting transactions fill. A block holds \(Format.bytes(WaitingRoom.blockLimit)).")
                if let block = model.blocks.first {
                    Readout(
                        "Last block", Self.clock(max(now.timeIntervalSince1970 - Double(block.time), 0)), reserve: "00:00")
                }
                Readout("Seen in 15 min", "\(seen)", reserve: "00")
                Spacer(minLength: 0)
            }
        }
        // The row gets less space between its figures when the page is narrow, and it never
        // makes the page wider: a wider row moved the page when a figure got one more digit.
        return ViewThatFits(in: .horizontal) {
            row(26)
            row(16)
            row(9)
        }
        .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
        .clipped()
    }

    /// "6:08" for six minutes and eight seconds.
    private static func clock(_ seconds: Double) -> String {
        let whole = Int(seconds)
        return whole >= 3600
            ? "\(whole / 3600):\(String(format: "%02d", whole / 60 % 60)):\(String(format: "%02d", whole % 60))"
            : "\(whole / 60):\(String(format: "%02d", whole % 60))"
    }

    private func roomText(_ bytes: UInt64) -> String {
        let limit = WaitingRoom.blockLimit
        return Double(bytes) <= Double(limit)
            ? "All that waits fits in one block: \(Format.bytes(bytes)) of \(Format.bytes(limit))."
            : "More waits than one block holds: \(Format.bytes(bytes)) of \(Format.bytes(limit)). "
                + "A block takes the transactions with the higher fee for each byte first."
    }

    private func spoken(_ lines: [(sighting: MempoolSighting, lane: Int)]) -> String {
        let start = now.addingTimeInterval(-span).timeIntervalSince1970
        let blocks = model.blocks.filter { Double($0.time) >= start }.count
        let waiting = lines.filter { $0.sighting.left == nil }.count
        return "The last 15 minutes: \(MempoolText.count(blocks, "block", "blocks")), "
            + "\(MempoolText.count(lines.count, "transaction", "transactions")) seen, \(waiting) waiting now."
    }

    private func draw(
        _ context: GraphicsContext, _ size: CGSize, lines: [(sighting: MempoolSighting, lane: Int)], lanes: Int
    ) {
        let start = now.addingTimeInterval(-span)
        let right = size.width - rightInset
        let axis = axisY(lanes)
        let top = lanesTop - 10
        let waitBottom = lanesTop + CGFloat(lanes) * laneStep
        func x(_ date: Date) -> CGFloat {
            gutter + CGFloat(min(max(date.timeIntervalSince(start) / span, 0), 1)) * (right - gutter)
        }

        // The names of the lanes: the transactions that wait, and the blocks, as on the
        // chain pulse.
        context.plate("Waiting", y: (top + waitBottom) / 2, width: plateWidth, fill: nil)
        context.plate("Staked", y: axis - 13, width: plateWidth, fill: Tone.staked)
        context.plate("Mined", y: axis + 13, width: plateWidth, fill: Tone.mined)

        // The part of the transactions has a line below it.
        context.rule(gutter, waitBottom + 4, right, waitBottom + 4, Tone.grid)

        // A tick for each minute, and a line for each five.
        for minute in 1...15 {
            let gx = x(now.addingTimeInterval(-Double(minute) * 60))
            if minute % 5 == 0 {
                context.rule(gx, top, gx, axis + 22, Tone.grid)
                context.name("\(minute) min", gx, axis + 33)
            } else {
                context.rule(gx, axis - 2.5, gx, axis + 2.5, Tone.rule)
            }
        }

        // The line of the blocks.
        context.rule(gutter, axis, right, axis, Tone.axis)

        // The guides and the marks of the blocks and the lines of the transactions are in
        // the pulse layer, which gives them their places at the front.
        if lines.isEmpty {
            context.name("no transaction in the last 15 minutes", (gutter + right) / 2, (top + waitBottom) / 2)
        }
        // The line or the mark under the pointer.
        switch pointed {
        case .transaction(let txid)?:
            if let line = lines.first(where: { $0.sighting.txid == txid }) {
                let y = lanesTop + CGFloat(line.lane) * laneStep
                let from = x(line.sighting.arrival)
                let to = max(x(end(line.sighting)), from + 3)
                // A frame round the line: the line itself is in the pulse layer, above this.
                context.stroke(
                    Path(CGRect(x: from - 3, y: y - 4.5, width: to - from + 6, height: 9)), with: .color(Tone.present),
                    lineWidth: 1)
            }
        case .block(let hash)?:
            if let block = model.blocks.first(where: { $0.hash == hash }) {
                let seen = model.arrivals[block.hash] ?? Date(timeIntervalSince1970: TimeInterval(block.time))
                let position = min(x(seen), right - 5)
                let mark = CGRect(x: position - 6, y: block.staked ? axis - 17.5 : axis + 5.5, width: 12, height: 12)
                context.stroke(
                    block.staked ? Path(ellipseIn: mark) : Path(mark), with: .color(Tone.present), lineWidth: 1.5)
            }
        case nil: break
        }

        // The right edge: the present.
        context.lit(Path(CGRect(x: right - 0.75, y: top, width: 1.5, height: axis + 22 - top)), Tone.present, strength: 0.7)
        context.name("now", right, top - 8, .trailing, color: Tone.present)
    }
}

/// The transactions that the last blocks took, newest block first. A person who sent a
/// transaction finds it here after it leaves the mempool.
private struct LeftList: View {
    let feed: MempoolFeed
    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    /// The most rows that the list shows.
    static let shown = 12

    var body: some View {
        let rows = Array(feed.took.prefix(Self.shown))
        let gone = feed.sightings.values.filter(\.gone).sorted { $0.arrival > $1.arrival }
        TimelineView(.periodic(from: .now, by: 5)) { context in
            VStack(alignment: .leading, spacing: 0) {
                ForEach(gone, id: \.txid) { sighting in
                    Button {
                        model.open(.transaction(txid: sighting.txid))
                    } label: {
                        MempoolRow(
                            tags: feed.details[sighting.txid]?.tags ?? [],
                            words: words(sighting.txid)
                                ?? "This transaction left the mempool. The node does not have it in a block.",
                            facts: "It can be expired, or another transaction can have replaced it.",
                            txid: sighting.txid)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    Divider()
                }
                if rows.isEmpty {
                    Text(
                        feed.tookBlocks.isEmpty
                            ? "The app reads the last blocks."
                            : "The last \(feed.tookBlocks.count) blocks took no transaction from the mempool.")
                        .foregroundStyle(.secondary)
                }
                ForEach(Array(rows.enumerated()), id: \.element.txid) { position, row in
                    if position == 0 || rows[position - 1].blockHash != row.blockHash {
                        if position > 0 { Divider() }
                        HStack(spacing: 6) {
                            Button("Block \(Format.integer(row.blockHeight))") {
                                model.open(.block(id: row.blockHash))
                            }
                            .buttonStyle(.displayLink)
                            Text(Format.age(row.blockTime, now: context.date)).foregroundStyle(.secondary)
                        }
                        .font(.callout)
                        .padding(.top, 8)
                    }
                    Button {
                        model.open(.transaction(txid: row.txid))
                    } label: {
                        MempoolRow(
                            tags: row.loaded ? row.tags : nil, words: words(row.txid) ?? fallback(row),
                            facts: facts(row), txid: row.txid)
                            .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .help("Open this transaction.")
                }
                if feed.took.count > rows.count {
                    Divider()
                    Text("\(feed.took.count - rows.count) more in the last \(MempoolFeed.leftBlocks) blocks.")
                        .font(.callout).foregroundStyle(.secondary).padding(.top, 6)
                }
            }
        }
    }

    private func words(_ txid: String) -> String? {
        _ = showsNames
        guard let detail = feed.details[txid], let statement = detail.statement else { return nil }
        let sentence = StatementWords(
            statement: statement, coin: detail.coin, tags: detail.tags, coinbase: detail.coinbase,
            feePool: detail.feePoolCheckpoint, model: model
        ).sentence
        return sentence.isEmpty ? nil : sentence
    }

    private func fallback(_ row: RecentTransaction) -> String? {
        row.loaded
            ? "\(MempoolText.count(Int(row.inputCount), "input", "inputs")), "
                + "\(MempoolText.count(Int(row.outputCount), "output", "outputs"))."
            : nil
    }

    /// The wait of a transaction that the app saw: from its arrival to the arrival of its
    /// block, or to the refresh that first did not have it.
    private func facts(_ row: RecentTransaction) -> String {
        guard let sighting = feed.sightings[row.txid], let left = sighting.left else { return "" }
        let end = model.arrivals[row.blockHash].map { min($0, left) } ?? left
        return "Waited \(MempoolText.span(end.timeIntervalSince(sighting.arrival)))"
    }
}
