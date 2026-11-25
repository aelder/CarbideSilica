import CarbideCore
import SwiftUI

/// Unconfirmed transactions in the local node's mempool, as tiles or as a list.
///
/// The tiles are sized by bytes, darker the longer the transaction has been in the mempool,
/// and dashed when it spends from another unconfirmed transaction. The list shows the same
/// data as the terminal's mempool view.
struct MempoolView: View {
    @Environment(AppModel.self) private var model
    @State private var feed = MempoolFeed.shared
    @AppStorage("mempoolShowsList") private var showsList = false

    var body: some View {
        PlacePage(key: "mempool") { proxy in
            Totals(feed: feed)
            Picker("Show as", selection: $showsList) {
                Text("Tiles").tag(false)
                Text("List").tag(true)
            }
            .pickerStyle(.segmented)
            .fixedSize()
            .labelsHidden()

            Card("Unconfirmed transactions") {
                if let failure = feed.failure, feed.entries.isEmpty {
                    Text(
                        model.status?.source == .publicApi
                            ? "Only a local node supplies the mempool. The public API does not."
                            : failure
                    )
                    .foregroundStyle(.secondary)
                } else if !feed.loaded {
                    ProgressView("Loading the mempool…").frame(maxWidth: .infinity, minHeight: 200)
                } else if feed.entries.isEmpty {
                    VStack(spacing: 6) {
                        Text("The mempool is empty.").font(.title3)
                        Text(
                            model.syncFraction == nil
                                ? "All known transactions are in a block."
                                : "A node does not collect unconfirmed transactions during synchronization."
                        )
                        .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, minHeight: 200)
                } else if showsList {
                    MempoolList(entries: feed.entries, feed: feed, proxy: proxy)
                } else {
                    MempoolTiles(entries: feed.entries)
                }
            }
        }
        .navigationTitle("Mempool")
        .focusedSceneValue(
            \.placeActions,
            PlaceActions(refresh: { Task { await feed.refresh(model) } }, mempoolAsList: $showsList))
        .task(id: model.generation) {
            while !Task.isCancelled {
                await feed.refresh(model)
                try? await Task.sleep(for: MempoolFeed.interval)
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
        } catch {
            guard generation == model.generation else { return }
            failure = describe(error)
            provenance = nil
            loaded = true
            lastRefresh = .now
        }
    }

    private func merged(_ entry: MempoolEntry) -> MempoolEntry {
        guard let tags = classified[entry.txid] else { return entry }
        var entry = entry
        entry.classified = true
        entry.tags = tags
        return entry
    }
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
        .keyboardRows(rows.map(\.txid), selection: $feed.selection, proxy: proxy, open: open)
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

/// The mempool as a treemap of tiles.
private struct MempoolTiles: View {
    let entries: [MempoolEntry]
    @Environment(AppModel.self) private var model
    @State private var hovered: String?

    var body: some View {
        let rows = entries.sorted { ($0.size, $0.txid) > ($1.size, $1.txid) }
        TimelineView(.periodic(from: .now, by: 1)) { timeline in
            GeometryReader { geometry in
                let tiles = Self.layout(rows, in: geometry.size)
                Canvas { context, _ in
                    draw(context, tiles, now: timeline.date.timeIntervalSince1970)
                }
                .contentShape(Rectangle())
                .onContinuousHover { phase in
                    if case .active(let point) = phase {
                        hovered = tiles.first { $0.1.contains(point) }?.0.txid
                    } else {
                        hovered = nil
                    }
                }
                .onTapGesture(coordinateSpace: .local) { point in
                    if let tile = tiles.first(where: { $0.1.contains(point) }) {
                        model.open(.transaction(txid: tile.0.txid))
                    }
                }
            }
        }
        .frame(height: 280)
        .accessibilityLabel("Mempool tiles. Use the list for the same data as text.")
        Text(caption(rows))
            .font(.callout)
            .foregroundStyle(hovered == nil ? .secondary : .primary)
            .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
    }

    private func caption(_ rows: [MempoolEntry]) -> String {
        guard let row = rows.first(where: { $0.txid == hovered }) else {
            return "Each tile is an unconfirmed transaction. A larger tile has more bytes. A darker tile "
                + "is older. A dashed border means that it spends from another unconfirmed transaction. "
                + "Click a tile to open it."
        }
        var parts = [label(row), Format.bytes(row.size)]
        if let time = row.time { parts.append("arrived \(Format.age(time))") }
        if let fee = row.feeSats { parts.append("fee \(Format.coins(fee)) \(model.coin)") }
        if let height = row.height { parts.append("entry height \(Format.integer(height))") }
        if !row.parents.isEmpty {
            parts.append("\(row.parents.count) unconfirmed parent\(row.parents.count == 1 ? "" : "s")")
        }
        return parts.joined(separator: " · ")
    }

    private func label(_ row: MempoolEntry) -> String {
        row.classified ? (row.tags.first ?? "Transaction") : "Classification pending"
    }

    /// A strip treemap: rows of tiles, each row's height proportional to the bytes in it.
    static func layout(_ rows: [MempoolEntry], in size: CGSize) -> [(MempoolEntry, CGRect)] {
        let total = CGFloat(rows.map(\.size).reduce(0, +))
        guard total > 0, size.width > 0 else { return [] }
        let strips = min(max(Int((Double(rows.count) / 3).rounded(.up)), 1), 5)
        let perStrip = total / CGFloat(strips)
        var groups: [[MempoolEntry]] = [[]]
        var filled: CGFloat = 0
        for row in rows {
            if filled >= perStrip, groups.count < strips {
                groups.append([])
                filled = 0
            }
            groups[groups.count - 1].append(row)
            filled += CGFloat(row.size)
        }
        var tiles: [(MempoolEntry, CGRect)] = []
        var y: CGFloat = 0
        for group in groups where !group.isEmpty {
            let sum = CGFloat(group.map(\.size).reduce(0, +))
            let height = sum / total * size.height
            var x: CGFloat = 0
            for row in group {
                let width = CGFloat(row.size) / sum * size.width
                tiles.append((row, CGRect(x: x, y: y, width: width, height: height)))
                x += width
            }
            y += height
        }
        return tiles
    }

    private func draw(_ context: GraphicsContext, _ tiles: [(MempoolEntry, CGRect)], now: Double) {
        for (row, frame) in tiles {
            let box = frame.insetBy(dx: 1.5, dy: 1.5)
            guard box.width > 0, box.height > 0 else { continue }
            let age = row.time.map { max(now - Double($0), 0) } ?? 0
            let shape = Path(roundedRect: box, cornerRadius: 4)
            context.fill(shape, with: .color(.accentColor.opacity(min(0.25 + age / 90 * 0.7, 0.95))))
            if !row.parents.isEmpty {
                context.stroke(shape, with: .color(.primary), style: StrokeStyle(lineWidth: 1.6, dash: [4, 3]))
            }
            if hovered == row.txid {
                context.stroke(shape, with: .color(.primary), lineWidth: 2)
            }
            if box.width > 110, box.height > 30 {
                context.draw(
                    Text(label(row)).font(.caption.weight(.semibold)),
                    at: CGPoint(x: box.minX + 9, y: box.minY + 9), anchor: .topLeading)
            }
        }
    }
}
