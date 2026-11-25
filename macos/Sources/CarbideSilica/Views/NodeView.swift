import CarbideCore
import Charts
import SwiftUI

/// What the local node is doing: how far it has validated, how fast, and who it is talking to.
struct NodeView: View {
    @Environment(AppModel.self) private var model
    @State private var feed = PeerFeed.shared
    @State private var commands = PeerCommands()
    @State private var detailPeer: Int64?
    @AppStorage("peerMapShown") private var showsMap = true

    var body: some View {
        Group {
            if let node = model.node {
                PlacePage(key: "node") { proxy in
                    SourceLine(provenance: node.provenance).font(.callout)
                    if node.provenance.retained {
                        ProvenanceNote(provenance: node.provenance)
                    }
                    SyncCard(node: node, speed: recentSpeed)
                    if isSyncing(node) {
                        SpeedCard(samples: model.speeds)
                    }
                    if node.peersError == nil {
                        PeersCard(node: node, feed: feed, proxy: proxy, open: { detailPeer = $0.id })
                        if showsMap {
                            PeerMapCard(peers: node.peers, feed: feed, open: { detailPeer = $0.id })
                        }
                        PeerControlsCard(banned: feed.banned.count, commands: commands, reload: reload)
                    } else {
                        Card("Peers") {
                            Text(
                                node.source == .publicApi
                                    ? "Only a local node supplies peer data. The public API does not."
                                    : node.peersError ?? ""
                            )
                            .foregroundStyle(.secondary)
                        }
                    }
                    BranchesCard()
                }
            } else if let error = model.nodeError ?? model.statusError {
                ContentUnavailableView {
                    Label("No connection to the node", systemImage: "server.rack")
                } description: {
                    Text(error)
                }
            } else {
                ProgressView("Connecting to the node…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationTitle("Node and Peers")
        .modifier(PeerCommandDialogs(commands: commands, enabled: detailPeer == nil, reload: reload))
        .sheet(
            item: Binding(
                get: { detailPeer.map(PeerID.init) }, set: { detailPeer = $0?.id })
        ) { peer in
            PeerDetailView(peerID: peer.id, commands: commands)
                .environment(model)
                .modifier(PeerCommandDialogs(commands: commands, reload: reload))
        }
        .publishPlace(PlaceActions(
                refresh: { Task { await model.refreshNode(); await reload() } },
                addNode: model.node?.peersError == nil ? { commands.startAddNode() } : nil,
                pingAll: model.node?.peersError == nil
                    ? {
                        commands.run(
                            .pingAll, done: "The node sends a ping to all connected peers.",
                            model: model, after: reload)
                    } : nil,
                clearBans: feed.banned.isEmpty ? nil : { commands.confirmClearBans(count: feed.banned.count) },
                showMap: $showsMap))
        .task(id: model.generation) {
            // The traffic totals are a snapshot, read once per visit as in the terminal.
            await feed.loadTotals(model)
            while !Task.isCancelled {
                await model.refreshNode()
                feed.note(model.node?.peers ?? [])
                await feed.loadBans(model)
                try? await Task.sleep(for: .seconds(2))
            }
        }
        .task(id: "\(model.generation)|\(peerAddresses.joined(separator: ","))") {
            await feed.loadLocations(model, addresses: peerAddresses)
        }
    }

    private var peerAddresses: [String] { (model.node?.peers ?? []).map(\.address).sorted() }

    private func reload() async {
        await feed.loadBans(model)
    }

    /// Average speed over roughly the last half minute.
    private var recentSpeed: Double? {
        let recent = model.speeds.suffix(15)
        guard !recent.isEmpty else { return nil }
        return recent.map(\.blocksPerSecond).reduce(0, +) / Double(recent.count)
    }
}

private func isSyncing(_ node: NodeStatus) -> Bool {
    guard let longest = node.longestChain else { return false }
    return longest > node.blocks + 20
}

/// The chain's whole history as one bar that fills as blocks are validated.
private struct SyncCard: View {
    let node: NodeStatus
    let speed: Double?

    /// The first Verus block.
    private static let genesis = Date(timeIntervalSince1970: 1_526_889_600)

    var body: some View {
        let syncing = isSyncing(node)
        Card(syncing ? "Synchronization in progress" : "Synchronized") {
            if syncing, let tip = node.tipTime {
                (Text("Validating blocks from ")
                    + Text(
                        Date(timeIntervalSince1970: TimeInterval(tip))
                            .formatted(.dateTime.month(.wide).year())
                    ).fontWeight(.semibold))
                    .font(.title3)
            } else if !syncing {
                Text("The node has validated all blocks. It validates each new block on arrival.")
                    .foregroundStyle(.secondary)
            }

            Canvas { context, size in
                draw(context, size)
            }
            .frame(height: 62)
            .accessibilityLabel(
                "Synchronization progress: \(Format.percent(fraction)) of the chain validated")

            HStack(spacing: 28) {
                Stat(
                    label: "Block",
                    value: Format.integer(node.blocks),
                    detail: node.longestChain.map { "of \(Format.integer($0))" })
                Stat(
                    label: "Validated",
                    value: fraction.formatted(.percent.precision(.fractionLength(1))),
                    detail: headerDetail)
                if syncing {
                    Stat(
                        label: "Speed",
                        value: speed.map { "\(Int($0.rounded())) blocks/s" } ?? "Measuring…",
                        detail: nil)
                    Stat(label: "Time remaining", value: timeLeft, detail: "at the current speed")
                }
                if let waiting = node.mempoolCount, !syncing {
                    Stat(label: "Waiting", value: "\(waiting)", detail: "unconfirmed transactions")
                }
            }
            if syncing {
                Text(
                    "The year marks are approximate. They assume a constant block rate. "
                        + "Blocks with more transactions take longer to validate."
                )
                .font(.caption)
                .foregroundStyle(.secondary)
            }
        }
    }

    private var fraction: Double {
        guard let longest = node.longestChain, longest > 0 else { return 1 }
        return min(Double(node.blocks) / Double(longest), 1)
    }

    private var headerFraction: Double? {
        guard let headers = node.syncedHeaders, let longest = node.longestChain, longest > 0
        else { return nil }
        return min(Double(headers) / Double(longest), 1)
    }

    private var headerDetail: String? {
        guard let headers = headerFraction, headers > fraction + 0.005, headers < 0.999 else {
            return nil
        }
        return "headers at \(Format.percent(headers))"
    }

    private var timeLeft: String {
        guard let speed, speed > 0.5, let longest = node.longestChain else { return "–" }
        let seconds = Double(longest - node.blocks) / speed
        let hours = seconds / 3600
        if hours >= 48 { return "approximately \(Int((hours / 24).rounded())) days" }
        if hours >= 1 { return "approximately \(Int(hours)) h \(Int(hours.truncatingRemainder(dividingBy: 1) * 60)) min" }
        return "approximately \(max(Int(seconds / 60), 1)) min"
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize) {
        let bar = CGRect(x: 0, y: 4, width: size.width, height: 30)
        let shape = Path(roundedRect: bar, cornerRadius: 5)
        context.fill(shape, with: .color(.secondary.opacity(0.14)))
        var clipped = context
        clipped.clip(to: shape)
        if let headers = headerFraction {
            clipped.fill(
                Path(CGRect(x: 0, y: bar.minY, width: bar.width * headers, height: bar.height)),
                with: .color(.accentColor.opacity(0.22)))
        }
        clipped.fill(
            Path(CGRect(x: 0, y: bar.minY, width: bar.width * fraction, height: bar.height)),
            with: .color(.accentColor))
        context.stroke(shape, with: .color(.secondary.opacity(0.4)), lineWidth: 1)

        // Year marks: height is close to linear in time because blocks target one minute.
        let total = Date.now.timeIntervalSince(Self.genesis)
        let calendar = Calendar(identifier: .gregorian)
        let thisYear = calendar.component(.year, from: .now)
        for year in 2019...max(thisYear, 2019) {
            guard let start = calendar.date(from: DateComponents(timeZone: .gmt, year: year)) else {
                continue
            }
            let x = bar.width * start.timeIntervalSince(Self.genesis) / total
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: x, y: bar.maxY))
                    path.addLine(to: CGPoint(x: x, y: bar.maxY + 6))
                }, with: .color(.secondary), lineWidth: 1)
            context.draw(
                Text(String(year)).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: x, y: bar.maxY + 16), anchor: .center)
        }
    }
}

private struct Stat: View {
    let label: String
    let value: String
    let detail: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label).font(.caption.weight(.medium)).foregroundStyle(.secondary)
            Text(value).font(.title3.weight(.semibold)).monospacedDigit()
            Text(detail ?? " ").font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// Validation speed as a live trace.
private struct SpeedCard: View {
    let samples: [SpeedSample]

    var body: some View {
        Card("Validation speed", trailing: "blocks per second") {
            if samples.count < 2 {
                Text("Measurement in progress.")
                    .foregroundStyle(.secondary)
                    .frame(height: 120)
            } else {
                Chart(samples) { sample in
                    AreaMark(
                        x: .value("Time", sample.time),
                        y: .value("Blocks per second", sample.blocksPerSecond)
                    )
                    .foregroundStyle(Color.accentColor.opacity(0.15))
                    LineMark(
                        x: .value("Time", sample.time),
                        y: .value("Blocks per second", sample.blocksPerSecond)
                    )
                    .foregroundStyle(Color.accentColor)
                    .interpolationMethod(.monotone)
                }
                .chartYScale(domain: .automatic(includesZero: true))
                .frame(height: 140)
            }
        }
    }
}

/// Identifies the peer shown in the detail sheet.
struct PeerID: Identifiable {
    let id: Int64
}

/// One line of the peer list: a connected peer or an entry in the ban list.
private struct PeerListItem: Identifiable {
    let peer: PeerRow?
    let banned: BannedPeer?
    var id: String { peer.map { "peer-\($0.id)" } ?? "ban-\(banned?.address ?? "")" }
}

/// Connected peers, fastest first, and the node's ban list, in one list as in the terminal.
///
/// A node has a small number of peers, and they stay for hours. Thus the list is the one
/// view of them: a sentence, a count by network operator and by country, then one row for
/// each peer with its measures on scales that all rows share. The response time is a dot
/// at the last measure and a line for the range in this session. Colour is not a
/// judgement of speed. The list grows with the page. Up and Down select, Return or a
/// double-click opens a peer.
private struct PeersCard: View {
    let node: NodeStatus
    @Bindable var feed: PeerFeed
    let proxy: ScrollViewProxy
    let open: (PeerRow) -> Void

    var body: some View {
        Card("Peers", trailing: summary) {
            if node.peers.isEmpty {
                Note(text: "No peers are connected. The node cannot synchronize without a peer.", warning: true)
            } else {
                Text(sentence)
                    .font(.title3)
                    .fixedSize(horizontal: false, vertical: true)
                counts
            }
            if !items.isEmpty {
                TimelineView(.periodic(from: .now, by: 5)) { context in
                    let scale = PeerScale(peers: node.peers, ranges: feed.responseRange, now: context.date)
                    VStack(alignment: .leading, spacing: 0) {
                        PeerColumns(
                            peer: Text("Peer"),
                            response: Text("Response time, 0 to \(PeerScale.time(scale.slowest))"),
                            data: Text("Data received"), connected: Text("Connected"))
                            .font(.caption.weight(.medium))
                            .foregroundStyle(.secondary)
                            .padding(.horizontal, 8)
                            .padding(.bottom, 6)
                        Divider()
                        ForEach(items) { item in
                            SelectableRow(id: item.id, selection: $feed.selection, open: { openItem(item.id) }) {
                                line(item, scale: scale, now: context.date).padding(.vertical, 6)
                            }
                            Divider()
                        }
                    }
                }
            }
            if let error = feed.bannedError {
                Text("The ban list is not available: \(error)").font(.callout).foregroundStyle(.secondary)
            }
            totals
            Text(
                "The dot is the last response time that the node measured. The line is the range since this "
                    + "page opened. The bar is the data received for each second of the connection."
                    + (isSyncing(node)
                        ? " Response times are high during synchronization because the peers send many blocks."
                        : ""))
                .font(.caption)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func openItem(_ id: String) {
        if let peer = items.first(where: { $0.id == id })?.peer { open(peer) }
    }

    // MARK: The sentence and the counts

    private func place(_ peer: PeerRow) -> PeerLocation? {
        guard let location = feed.location(of: peer.address), location.place == .located else { return nil }
        return location
    }

    /// The peers in groups by one property, the largest group first.
    private func groups(_ key: (PeerLocation) -> String?) -> [(name: String, count: Int)] {
        let names = node.peers.compactMap { peer in place(peer).flatMap(key) }
        return Dictionary(grouping: names, by: { $0 }).map { ($0.key, $0.value.count) }
            .sorted { ($0.1, $1.0) > ($1.1, $0.0) }
    }

    /// How many blocks the peer is behind the node. Nil when it is at the tip or not known.
    private func behind(_ peer: PeerRow) -> UInt64? {
        guard !isSyncing(node), let synced = peer.syncedBlocks, synced > 0 else { return nil }
        let gap = Int64(node.blocks) - synced
        return gap > 1 ? UInt64(gap) : nil
    }

    private var sentence: String {
        let peers = node.peers
        let count = peers.count
        let incoming = peers.filter(\.inbound).count
        var parts: [String] = []
        let noun = count == 1 ? "peer" : "peers"
        if count == 1 {
            parts.append("1 peer, \(incoming == 1 ? "incoming" : "outgoing").")
        } else if incoming == 0 {
            parts.append("\(count) \(noun), all outgoing.")
        } else if incoming == count {
            parts.append("\(count) \(noun), all incoming.")
        } else {
            parts.append("\(count) \(noun): \(count - incoming) outgoing and \(incoming) incoming.")
        }
        let operators = groups(\.organization)
        if count > 1, let largest = operators.first, largest.count * 2 > count {
            parts.append(
                largest.count == count
                    ? "All \(count) use one network operator, \(largest.name)."
                    : "\(largest.count) of the \(count) use one network operator, \(largest.name).")
        }
        if !isSyncing(node), peers.allSatisfy({ ($0.syncedBlocks ?? 0) > 0 }) {
            let late = peers.compactMap(behind).count
            var tip = late == 0
                ? (count == 1 ? "It is at the tip of the chain" : "All \(count) are at the tip of the chain")
                : "\(late) of the \(count) \(late == 1 ? "is" : "are") behind the node"
            let versions = Set(peers.compactMap(\.version))
            if count > 1, versions.count == 1 { tip += late == 0 ? " and run the same version" : ". All run the same version" }
            parts.append(tip + ".")
        }
        return parts.joined(separator: " ")
    }

    @ViewBuilder
    private var counts: some View {
        let operators = groups(\.organization)
        let countries = groups(\.country)
        if !operators.isEmpty || !countries.isEmpty {
            VStack(alignment: .leading, spacing: 5) {
                if !operators.isEmpty { countLine("Network operator", operators) }
                if !countries.isEmpty { countLine("Country", countries) }
            }
        }
    }

    private func countLine(_ title: String, _ groups: [(name: String, count: Int)]) -> some View {
        HStack(alignment: .center, spacing: 10) {
            Text(title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                .frame(width: 110, alignment: .leading)
            FlowLayout(spacing: 14) {
                ForEach(groups, id: \.name) { group in
                    let most = group.count * 2 > node.peers.count
                    HStack(spacing: 5) {
                        HStack(spacing: 3) {
                            ForEach(0..<group.count, id: \.self) { _ in
                                RoundedRectangle(cornerRadius: 2)
                                    .fill(most ? Color.accentColor : Color.secondary.opacity(0.6))
                                    .frame(width: 11, height: 11)
                            }
                        }
                        Text("\(group.name) \(group.count)").font(.callout)
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("\(group.name), \(group.count)")
                }
            }
        }
    }

    // MARK: The rows

    private func line(_ item: PeerListItem, scale: PeerScale, now: Date) -> some View {
        PeerColumns(
            peer: VStack(alignment: .leading, spacing: 2) {
                if let peer = item.peer {
                    HStack(spacing: 6) {
                        Button("Peer \(peer.id)") { open(peer) }.buttonStyle(.link)
                        Text(placeName(peer)).lineLimit(1)
                    }
                    HStack(spacing: 0) {
                        Text(detail(peer) + " · ").lineLimit(1).layoutPriority(1)
                        Text(peer.address).lineLimit(1).truncationMode(.middle)
                    }
                    .font(.callout).foregroundStyle(.secondary)
                    .help("\(peer.address) · height at connection \(peer.startingHeight.map { Format.integer(UInt64(max($0, 0))) } ?? "unknown")")
                    if let gap = behind(peer) {
                        Text("\(Format.integer(gap)) blocks behind the node").font(.callout).foregroundStyle(.orange)
                    }
                } else if let banned = item.banned {
                    Label("Banned", systemImage: "nosign").foregroundStyle(.red)
                    Text("\(banned.address) · \(banText(banned))")
                        .font(.callout).foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                }
            },
            response: Group {
                if let peer = item.peer {
                    ResponseScale(milliseconds: peer.pingMs, range: feed.responseRange[peer.id], slowest: scale.slowest)
                }
            },
            data: Group {
                if let peer = item.peer {
                    VStack(alignment: .leading, spacing: 3) {
                        GeometryReader { geometry in
                            Rectangle()
                                .fill(Color.accentColor.opacity(0.85))
                                .frame(width: max(geometry.size.width * scale.share(of: peer), 1.5))
                        }
                        .frame(height: 8)
                        .accessibilityHidden(true)
                        Text("\(Format.bytes(peer.bytesReceived)) · sent \(Format.bytes(peer.bytesSent))")
                            .font(.callout).foregroundStyle(.secondary).lineLimit(1)
                    }
                    .help(scale.rate(of: peer).map { "\(PeerScale.rateText($0)) for each second of the connection." } ?? "")
                }
            },
            connected: Text(
                item.peer?.connectedSince.map {
                    Format.age($0, now: now).replacingOccurrences(of: " ago", with: "")
                } ?? ""))
            .monospacedDigit()
    }

    private func placeName(_ peer: PeerRow) -> String {
        guard let locations = feed.locations,
            let location = locations.locations.first(where: { $0.address == peer.address })
        else { return "" }
        switch location.place {
        case .located:
            return [location.city, location.countryCode].compactMap { $0 }.joined(separator: ", ")
        case .private: return "private or local address"
        case .unresolved:
            return locations.state == .current || locations.state == .updateAvailable
                ? "no location found" : "no location data"
        }
    }

    private func detail(_ peer: PeerRow) -> String {
        var parts: [String] = []
        if let organization = place(peer)?.organization { parts.append(organization) }
        parts.append(peer.inbound ? "incoming" : "outgoing")
        return parts.joined(separator: " · ")
    }

    private var summary: String {
        let bans = feed.bannedError == nil ? "\(feed.banned.count) banned" : "bans not available"
        return "\(node.peers.count) connected · \(bans)"
    }

    private var items: [PeerListItem] {
        node.peers.sorted { ($0.pingMs ?? .infinity, $0.id) < ($1.pingMs ?? .infinity, $1.id) }
            .map { PeerListItem(peer: $0, banned: nil) }
            + feed.banned.map { PeerListItem(peer: nil, banned: $0) }
    }

    /// The terminal's traffic tile: totals read once when the page opened.
    @ViewBuilder
    private var totals: some View {
        HStack(spacing: 18) {
            Text("Network totals").fontWeight(.semibold)
            if let totals = feed.totals {
                Text("Received \(Format.bytes(totals.received))").monospacedDigit()
                Text("Sent \(Format.bytes(totals.sent))").monospacedDigit()
                Text("Read at \(Format.date(totals.timeMillis / 1000))").foregroundStyle(.secondary)
            } else if feed.totalsError != nil {
                Text("Not available.").foregroundStyle(.secondary)
            } else {
                Text("Reading…").foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        .font(.callout)
        .help("The node's traffic since it started. The app reads it once each time this page opens.")
    }

    private func banText(_ banned: BannedPeer) -> String {
        guard let until = banned.bannedUntil, until > 0 else { return banned.reason ?? "banned" }
        return "until \(Format.date(UInt64(until)))"
    }
}

/// The scales that all rows of the peer list share.
struct PeerScale {
    /// The top of the response-time scale, in milliseconds: the slowest peer, rounded up.
    let slowest: Double
    private let rates: [Int64: Double]
    private let fastestRate: Double

    init(peers: [PeerRow], ranges: [Int64: ClosedRange<Double>], now: Date) {
        let times = peers.compactMap(\.pingMs) + peers.compactMap { ranges[$0.id]?.upperBound }
        slowest = Self.round(times.max() ?? 100)
        var rates: [Int64: Double] = [:]
        for peer in peers {
            guard let since = peer.connectedSince else { continue }
            let seconds = now.timeIntervalSince1970 - Double(since)
            if seconds >= 1 { rates[peer.id] = Double(peer.bytesReceived) / seconds }
        }
        self.rates = rates
        fastestRate = max(rates.values.max() ?? 0, 1)
    }

    /// The next step above a time: 100, 200, 500, then 1, 2 and 5 seconds, to 10 seconds.
    static func round(_ milliseconds: Double) -> Double {
        for step in [100.0, 200, 300, 400, 500, 750, 1000, 2000, 5000, 10_000] where milliseconds <= step {
            return step
        }
        return 10_000
    }

    static func time(_ milliseconds: Double) -> String {
        milliseconds < 1000
            ? "\(Int(milliseconds.rounded())) ms"
            : (milliseconds / 1000).formatted(.number.precision(.fractionLength(0...1))) + " s"
    }

    static func rateText(_ bytes: Double) -> String {
        bytes >= 10 ? Format.bytes(UInt64(bytes)) : "\(bytes.formatted(.number.precision(.fractionLength(1)))) bytes"
    }

    func rate(of peer: PeerRow) -> Double? { rates[peer.id] }

    /// The data rate of a peer as a part of the highest rate, from 0 to 1.
    func share(of peer: PeerRow) -> Double {
        min((rates[peer.id] ?? 0) / fastestRate, 1)
    }
}

/// One column layout for the peer list's heading and rows. The flexible columns share the
/// page width, so each column fits without a horizontal scroll.
private struct PeerColumns<A: View, B: View, C: View, D: View>: View {
    let peer: A
    let response: B
    let data: C
    let connected: D

    var body: some View {
        HStack(alignment: .top, spacing: 16) {
            peer.frame(minWidth: 220, maxWidth: .infinity, alignment: .leading)
            response.frame(minWidth: 180, maxWidth: 300, alignment: .leading)
            data.frame(width: 170, alignment: .leading)
            connected.frame(width: 70, alignment: .trailing)
        }
    }
}

/// A response time on a straight scale from zero to the slowest peer. The dot is the last
/// measure. The line is the range of the measures in this session.
private struct ResponseScale: View {
    let milliseconds: Double?
    let range: ClosedRange<Double>?
    let slowest: Double

    var body: some View {
        if let milliseconds {
            VStack(alignment: .leading, spacing: 3) {
                Canvas { context, size in
                    let middle = size.height / 2
                    func x(_ ms: Double) -> CGFloat { 5 + CGFloat(min(ms / slowest, 1)) * (size.width - 10) }
                    context.fill(
                        Path(CGRect(x: 5, y: middle - 0.5, width: size.width - 10, height: 1)),
                        with: .color(.secondary.opacity(0.35)))
                    context.fill(
                        Path(CGRect(x: 5 - 0.5, y: middle - 4, width: 1, height: 8)), with: .color(.secondary.opacity(0.6)))
                    if let range, range.upperBound - range.lowerBound > 0 {
                        let from = x(range.lowerBound)
                        let band = CGRect(x: from, y: middle - 2, width: max(x(range.upperBound) - from, 2), height: 4)
                        context.fill(Path(roundedRect: band, cornerRadius: 2), with: .color(.primary.opacity(0.35)))
                    }
                    let at = x(milliseconds)
                    context.fill(
                        Path(ellipseIn: CGRect(x: at - 4.5, y: middle - 4.5, width: 9, height: 9)),
                        with: .color(.accentColor))
                }
                .frame(height: 12)
                .accessibilityHidden(true)
                Text(label(milliseconds)).font(.callout).lineLimit(1)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Response time \(label(milliseconds))")
        } else {
            Text("not measured yet").foregroundStyle(.secondary)
        }
    }

    private func label(_ ms: Double) -> String {
        var text = PeerScale.time(ms)
        if let range, range.upperBound - range.lowerBound >= 5 {
            text += " · \(PeerScale.time(range.lowerBound)) to \(PeerScale.time(range.upperBound))"
        }
        return text
    }
}
