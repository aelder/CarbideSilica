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
            Picker("Show as", selection: $showsList) {
                Text("Waiting").tag(false)
                Text("List").tag(true)
            }
            .pickerStyle(.segmented)
            .fixedSize()
            .labelsHidden()

            Card(showsList ? "Unconfirmed transactions" : "Waiting for a block") {
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
        .task(id: model.generation) {
            while !Task.isCancelled {
                await feed.refresh(model)
                try? await Task.sleep(for: MempoolFeed.interval)
            }
        }
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
                SourceLine(provenance: feed.provenance)
                HStack(spacing: 18) {
                    item("Transactions", feed.loaded ? "\(feed.entries.count)" : "–")
                    item("Serialized size", feed.loaded ? Format.bytes(feed.totalBytes) : "–")
                    item(
                        "Last block",
                        model.status?.tipTime.map { Format.age($0, now: context.date) } ?? "unknown")
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
        HStack(alignment: .firstTextBaseline, spacing: 5) {
            Text(label).foregroundStyle(.secondary)
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
                transaction: Text("Transaction"), arrival: header(.arrival), fee: header(.fee),
                size: header(.size), height: header(.height), parents: header(.parents),
                tags: Text("Tags"))
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
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
            HStack(spacing: 2) {
                Text(column.rawValue)
                if feed.sort == column {
                    Image(systemName: feed.descending ? "chevron.down" : "chevron.up")
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
                .buttonStyle(.link)
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
                ForEach(entry.tags, id: \.self) { TagChip(text: $0) }
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
                Text(sentence(now))
                    .font(.title3)
                    .fixedSize(horizontal: false, vertical: true)
                MempoolTimeRecord(feed: feed, now: now)
                if !feed.entries.isEmpty {
                    if feed.entries.count > Self.manyRows { kinds }
                    rows(now)
                    nextBlock
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
                    TagChip(text: "\(name) \(count)", color: kind == name ? .accentColor : .secondary)
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

    // MARK: The next block

    private var nextBlock: some View {
        let bytes = feed.totalBytes
        let share = Double(bytes) / Double(Self.blockLimit)
        let percent = share * 100
        let text = share <= 1
            ? "The next block: \(Format.bytes(bytes)) of \(Format.bytes(Self.blockLimit)), "
                + "\(percent < 1 ? String(format: "%.2f", percent) : String(format: "%.1f", percent)) of each 100. "
                + "All that waits fits in one block."
            : "More waits than one block holds: \(Format.bytes(bytes)) of \(Format.bytes(Self.blockLimit)). "
                + "A block takes the transactions with the higher fee for each byte first."
        return VStack(alignment: .leading, spacing: 4) {
            GeometryReader { geometry in
                ZStack(alignment: .leading) {
                    Rectangle().fill(Color.secondary.opacity(0.18))
                    Rectangle()
                        .fill(share <= 1 ? Color.accentColor : Color.red)
                        .frame(width: max(geometry.size.width * min(share, 1), 2))
                }
            }
            .frame(height: 6)
            .clipShape(RoundedRectangle(cornerRadius: 2))
            .accessibilityHidden(true)
            Text(text).font(.callout).foregroundStyle(.secondary)
        }
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
                        ForEach(tags.prefix(4), id: \.self) { TagChip(text: $0) }
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
/// A line that reaches the right edge is a transaction that waits now.
private struct MempoolTimeRecord: View {
    let feed: MempoolFeed
    let now: Date
    @Environment(AppModel.self) private var model

    private let span: Double = 15 * 60
    private let inset: CGFloat = 8
    private let laneStep: CGFloat = 11
    private let maxLanes = 8

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

    var body: some View {
        let lines = lines
        let lanes = max((lines.map(\.lane).max() ?? -1) + 1, 1)
        VStack(alignment: .leading, spacing: 4) {
            Canvas { context, size in
                draw(context, size, lines: lines, lanes: lanes)
            }
            .frame(height: CGFloat(lanes) * laneStep + 46)
            .accessibilityLabel(spoken(lines))
            Text(
                "Each mark on the line is a block: up for staked, down for mined. A thick line is a "
                    + "transaction, from its arrival to the block that took it. The app sees only the "
                    + "transactions that arrive while this page is open.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
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
        let width = size.width - inset * 2
        let axis = CGFloat(lanes) * laneStep + 14
        func x(_ date: Date) -> CGFloat {
            inset + CGFloat(min(max(date.timeIntervalSince(start) / span, 0), 1)) * width
        }
        context.fill(
            Path(CGRect(x: inset, y: axis - 0.5, width: width, height: 1)), with: .color(.secondary.opacity(0.6)))
        for minutes in stride(from: 15, through: 0, by: -5) {
            let position = x(now.addingTimeInterval(-Double(minutes) * 60))
            context.draw(
                Text(minutes == 0 ? "now" : "\(minutes) min ago").font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: position, y: axis + 20),
                anchor: minutes == 0 ? .trailing : minutes == 15 ? .leading : .center)
        }
        for block in model.blocks {
            let seen = model.arrivals[block.hash] ?? Date(timeIntervalSince1970: TimeInterval(block.time))
            guard seen > start else { continue }
            let position = x(seen)
            context.fill(
                Path(CGRect(x: position - 0.5, y: 4, width: 1, height: axis - 4)),
                with: .color(.secondary.opacity(0.22)))
            let mark = CGRect(x: position - 2, y: block.staked ? axis - 9 : axis, width: 4, height: 9)
            context.fill(Path(roundedRect: mark, cornerRadius: 1), with: .color(block.staked ? .purple : .orange))
        }
        for line in lines {
            let y = 8 + CGFloat(line.lane) * laneStep
            let from = x(line.sighting.arrival)
            let to = max(x(end(line.sighting)), from + 3)
            var path = Path()
            path.move(to: CGPoint(x: from, y: y))
            path.addLine(to: CGPoint(x: to, y: y))
            let waiting = line.sighting.left == nil
            let color: Color = waiting ? .accentColor : line.sighting.gone ? .red : .secondary
            context.stroke(
                path, with: .color(color.opacity(waiting ? 1 : 0.7)),
                style: StrokeStyle(lineWidth: 5, lineCap: .round, dash: line.sighting.gone ? [2, 7] : []))
        }
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
                            .buttonStyle(.link)
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
