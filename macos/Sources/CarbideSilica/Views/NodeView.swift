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
                        PeersRadarCard(node: node)
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
        .focusedSceneValue(
            \.placeActions,
            PlaceActions(
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
/// The list grows with the page. Up and Down select, Return or a double-click opens a peer.
private struct PeersCard: View {
    let node: NodeStatus
    @Bindable var feed: PeerFeed
    let proxy: ScrollViewProxy
    let open: (PeerRow) -> Void

    var body: some View {
        Card("Peers", trailing: summary) {
            if node.peers.isEmpty {
                Note(text: "No peers are connected. The node cannot synchronize without a peer.", warning: true)
            }
            if !items.isEmpty {
                VStack(alignment: .leading, spacing: 0) {
                    PeerColumns(
                        peer: Text("Peer"), address: Text("Address"), state: Text("Direction or ban"),
                        location: Text("Location"), response: Text("Response time"),
                        height: Text("Peer height"), connected: Text("Connected"),
                        received: Text("Received"), sent: Text("Sent"))
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 8)
                        .padding(.bottom, 6)
                    Divider()
                    TimelineView(.periodic(from: .now, by: 5)) { context in
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(items) { item in
                                SelectableRow(id: item.id, selection: $feed.selection, open: { openItem(item.id) }) {
                                    line(item, now: context.date).padding(.vertical, 5)
                                }
                                Divider()
                            }
                        }
                    }
                }
                .keyboardRows(items.map(\.id), selection: $feed.selection, proxy: proxy, open: openItem)
            }
            if let error = feed.bannedError {
                Text("The ban list is not available: \(error)").font(.callout).foregroundStyle(.secondary)
            }
            totals
            Text(
                "\"Peer height\" is the chain height that the peer reported when it connected."
                    + (isSyncing(node)
                        ? " Response times are high during synchronization because the peers send many blocks."
                        : ""))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private func openItem(_ id: String) {
        if let peer = items.first(where: { $0.id == id })?.peer { open(peer) }
    }

    private func line(_ item: PeerListItem, now: Date) -> some View {
        PeerColumns(
            peer: Group {
                if let peer = item.peer {
                    Button("Peer \(peer.id)") { open(peer) }.buttonStyle(.link)
                } else {
                    Label("Banned", systemImage: "nosign").foregroundStyle(.red)
                }
            },
            address: Text(item.peer?.address ?? item.banned?.address ?? "")
                .font(.callout.monospaced())
                .truncationMode(.middle)
                .help(item.peer?.address ?? item.banned?.address ?? ""),
            state: Text(item.peer.map { $0.inbound ? "Inbound" : "Outbound" } ?? item.banned.map(banText) ?? "")
                .foregroundStyle(.secondary),
            location: Text(item.peer.map(location) ?? "").foregroundStyle(.secondary),
            response: Group { if let peer = item.peer { ResponseBar(milliseconds: peer.pingMs) } },
            height: Text(item.peer?.startingHeight.map { Format.integer(UInt64(max($0, 0))) } ?? ""),
            connected: Text(
                item.peer?.connectedSince.map {
                    Format.age($0, now: now).replacingOccurrences(of: " ago", with: "")
                } ?? ""),
            received: Text(item.peer.map { Format.bytes($0.bytesReceived) } ?? ""),
            sent: Text(item.peer.map { Format.bytes($0.bytesSent) } ?? ""))
            .monospacedDigit()
    }

    private var summary: String {
        let bans = feed.bannedError == nil ? "\(feed.banned.count) banned" : "bans not available"
        return "\(node.peers.count) connected · \(bans)"
    }

    private var items: [PeerListItem] {
        node.peers.sorted { ($0.pingMs ?? .infinity) < ($1.pingMs ?? .infinity) }
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

    private func location(_ peer: PeerRow) -> String {
        guard let locations = feed.locations,
            let location = locations.locations.first(where: { $0.address == peer.address })
        else { return "" }
        switch location.place {
        case .located:
            return [location.city, location.countryCode].compactMap { $0 }.joined(separator: ", ")
        case .private: return "private or local"
        case .unresolved:
            return locations.state == .current || locations.state == .updateAvailable ? "not found" : "no location data"
        }
    }
}

/// One column layout for the peer list's heading and rows. Flexible columns share the
/// page width, so every column fits without a horizontal scroll.
private struct PeerColumns<A: View, B: View, C: View, D: View, E: View, F: View, G: View, H: View, I: View>: View {
    let peer: A
    let address: B
    let state: C
    let location: D
    let response: E
    let height: F
    let connected: G
    let received: H
    let sent: I

    var body: some View {
        HStack(spacing: 8) {
            peer.frame(width: 64, alignment: .leading)
            address.frame(minWidth: 110, maxWidth: 230, alignment: .leading)
            state.frame(minWidth: 70, maxWidth: 140, alignment: .leading)
            location.frame(minWidth: 60, maxWidth: 130, alignment: .leading)
            response.frame(minWidth: 100, maxWidth: .infinity, alignment: .leading)
            height.frame(width: 76, alignment: .trailing)
            connected.frame(width: 64, alignment: .trailing)
            received.frame(width: 64, alignment: .trailing)
            sent.frame(width: 64, alignment: .trailing)
        }
        .lineLimit(1)
    }
}

/// A bar whose length is the peer's response time on a logarithmic scale from 10 ms to 10 s.
private struct ResponseBar: View {
    let milliseconds: Double?

    var body: some View {
        if let milliseconds {
            HStack(spacing: 8) {
                GeometryReader { geometry in
                    Capsule()
                        .fill(color(milliseconds))
                        .frame(width: max(geometry.size.width * fraction(milliseconds), 4))
                        .frame(maxHeight: .infinity, alignment: .center)
                }
                .frame(height: 8)
                Text(label(milliseconds))
                    .monospacedDigit()
                    .frame(width: 64, alignment: .trailing)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Response time \(label(milliseconds))")
        } else {
            Text("not measured yet").foregroundStyle(.secondary)
        }
    }

    private func fraction(_ ms: Double) -> Double {
        min(max(log10(max(ms, 10) / 10) / 3, 0.02), 1)
    }

    private func color(_ ms: Double) -> Color {
        ms < 150 ? .green : ms < 1000 ? .orange : .red
    }

    private func label(_ ms: Double) -> String {
        ms < 1000 ? "\(Int(ms.rounded())) ms" : (ms / 1000).formatted(.number.precision(.fractionLength(1))) + " s"
    }
}
