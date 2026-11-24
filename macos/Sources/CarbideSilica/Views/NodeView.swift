import CarbideCore
import Charts
import SwiftUI

/// What the local node is doing: how far it has validated, how fast, and who it is talking to.
struct NodeView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            if let node = model.node {
                Page {
                    SyncCard(node: node, speed: recentSpeed)
                    if isSyncing(node) {
                        SpeedCard(samples: model.speeds)
                    }
                    PeersCard(node: node)
                }
            } else if let error = model.nodeError ?? model.statusError {
                ContentUnavailableView {
                    Label("Node not reachable", systemImage: "server.rack")
                } description: {
                    Text(error)
                }
            } else {
                ProgressView("Asking the node…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .navigationTitle("Node")
        .task(id: model.generation) {
            while !Task.isCancelled {
                await model.refreshNode()
                try? await Task.sleep(for: .seconds(2))
            }
        }
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
        Card(syncing ? "Syncing the chain" : "Up to date") {
            if syncing, let tip = node.tipTime {
                (Text("Validating blocks from ")
                    + Text(
                        Date(timeIntervalSince1970: TimeInterval(tip))
                            .formatted(.dateTime.month(.wide).year())
                    ).fontWeight(.semibold))
                    .font(.title3)
            } else if !syncing {
                Text("Every block has been validated. New blocks are checked as they arrive.")
                    .foregroundStyle(.secondary)
            }

            Canvas { context, size in
                draw(context, size)
            }
            .frame(height: 62)
            .accessibilityLabel(
                "Sync progress: \(Format.percent(fraction)) of the chain validated")

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
                    Stat(label: "Time left", value: timeLeft, detail: "at the current speed")
                }
                if let waiting = node.mempoolCount, !syncing {
                    Stat(label: "Waiting", value: "\(waiting)", detail: "unconfirmed transactions")
                }
            }
            if syncing {
                Text(
                    "Year marks assume a steady block rate, so they are approximate. "
                        + "Busier periods of the chain take longer to validate."
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
        if hours >= 48 { return "about \(Int((hours / 24).rounded())) days" }
        if hours >= 1 { return "about \(Int(hours)) h \(Int(hours.truncatingRemainder(dividingBy: 1) * 60)) min" }
        return "about \(max(Int(seconds / 60), 1)) min"
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
                Text("Measuring. The trace appears after a few seconds.")
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

/// Connected peers, fastest first, with response time drawn as a bar.
private struct PeersCard: View {
    let node: NodeStatus

    var body: some View {
        Card("Peers", trailing: node.peersError == nil ? "\(node.peers.count) connected" : nil) {
            if let error = node.peersError {
                Label(
                    node.source == .publicApi
                        ? "Peers are only available from a local daemon." : error,
                    systemImage: "info.circle"
                )
                .foregroundStyle(.secondary)
            } else if node.peers.isEmpty {
                Label("No peers connected. The node cannot sync until it finds one.",
                      systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
            } else {
                VStack(spacing: 0) {
                    PeerColumns(
                        address: Text("Address"), direction: Text("Direction"),
                        response: Text("Response time"), height: Text("Its height"),
                        received: Text("Received"), sent: Text("Sent")
                    )
                    .font(.caption.weight(.medium))
                    .foregroundStyle(.secondary)
                    .padding(.bottom, 6)
                    Divider()
                    ForEach(sorted, id: \.id) { peer in
                        PeerColumns(
                            address: Text(peer.address).font(.callout.monospaced())
                                .textSelection(.enabled),
                            direction: Text(peer.inbound ? "Incoming" : "Outgoing")
                                .foregroundStyle(.secondary),
                            response: ResponseBar(milliseconds: peer.pingMs),
                            height: Text(peer.startingHeight.map { Format.integer(UInt64(max($0, 0))) } ?? "–")
                                .monospacedDigit(),
                            received: Text(Format.bytes(peer.bytesReceived)).monospacedDigit()
                                .foregroundStyle(.secondary),
                            sent: Text(Format.bytes(peer.bytesSent)).monospacedDigit()
                                .foregroundStyle(.secondary)
                        )
                        .padding(.vertical, 7)
                        Divider()
                    }
                }
                Text(
                    "\"Its height\" is the chain height the peer reported when it connected."
                        + (isSyncing(node)
                            ? " Response times read high during sync, because peers are busy sending blocks."
                            : ""))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    private var sorted: [PeerRow] {
        node.peers.sorted { ($0.pingMs ?? .infinity) < ($1.pingMs ?? .infinity) }
    }
}

private struct PeerColumns<A: View, B: View, C: View, D: View, E: View, F: View>: View {
    let address: A
    let direction: B
    let response: C
    let height: D
    let received: E
    let sent: F

    var body: some View {
        HStack(spacing: 12) {
            address.frame(width: 190, alignment: .leading)
            direction.frame(width: 76, alignment: .leading)
            response.frame(maxWidth: .infinity, alignment: .leading)
            height.frame(width: 90, alignment: .trailing)
            received.frame(width: 84, alignment: .trailing)
            sent.frame(width: 84, alignment: .trailing)
        }
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
