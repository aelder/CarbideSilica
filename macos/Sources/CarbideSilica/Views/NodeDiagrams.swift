import CarbideCore
import SwiftUI

/// Peers placed around the node, with distance from the centre as response time.
///
/// A slow connection sits at the edge and a fast one near the middle, so the quality of
/// the node's links is a shape. Direction carries no meaning. The peer map shows location.
struct PeersRadarCard: View {
    let node: NodeStatus
    @State private var hovered: Int64?

    var body: some View {
        if node.peersError == nil, !node.peers.isEmpty {
            Card("Peers by response time", trailing: "nearer is faster") {
                GeometryReader { geometry in
                    Canvas { context, size in
                        draw(context, size)
                    }
                    .contentShape(Rectangle())
                    .onContinuousHover { phase in
                        if case .active(let point) = phase {
                            hovered = nearest(to: point, in: geometry.size)?.id
                        } else {
                            hovered = nil
                        }
                    }
                }
                .frame(height: 300)
                Text(caption)
                    .font(.callout)
                    .foregroundStyle(hovered == nil ? .secondary : .primary)
                    .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
            }
        }
    }

    private var ordered: [PeerRow] { node.peers.sorted { $0.address < $1.address } }

    private var caption: String {
        guard let peer = node.peers.first(where: { $0.id == hovered }) else {
            return "Each dot is a peer. The distance from the centre shows the response time. "
                + "The size shows the quantity of data received."
        }
        let time = peer.pingMs.map { $0 < 1000 ? "\(Int($0.rounded())) ms" : String(format: "%.1f s", $0 / 1000) }
            ?? "not measured"
        return "\(peer.address) · \(time) · \(peer.inbound ? "incoming" : "outgoing") · "
            + "\(Format.bytes(peer.bytesReceived)) received"
    }

    /// 10 ms at the centre ring to 10 s at the edge, on a logarithmic scale.
    private func radius(_ ms: Double, outer: CGFloat) -> CGFloat {
        let fraction = min(max(log10(max(ms, 10) / 10) / 3, 0), 1)
        return 22 + CGFloat(fraction) * (outer - 22)
    }

    private func position(_ peer: PeerRow, index: Int, in size: CGSize) -> CGPoint {
        let outer = min(size.height / 2 - 14, size.width / 2 - 40)
        let angle = Double(index) / Double(max(ordered.count, 1)) * 2 * .pi - .pi / 2
        let distance = radius(peer.pingMs ?? 10_000, outer: outer)
        return CGPoint(
            x: size.width / 2 + CGFloat(cos(angle)) * distance,
            y: size.height / 2 + CGFloat(sin(angle)) * distance)
    }

    private func nearest(to location: CGPoint, in size: CGSize) -> PeerRow? {
        ordered.enumerated()
            .map { ($0.element, position($0.element, index: $0.offset, in: size)) }
            .filter { hypot($0.1.x - location.x, $0.1.y - location.y) < 16 }
            .min { hypot($0.1.x - location.x, $0.1.y - location.y) < hypot($1.1.x - location.x, $1.1.y - location.y) }?
            .0
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize) {
        let centre = CGPoint(x: size.width / 2, y: size.height / 2)
        let outer = min(size.height / 2 - 14, size.width / 2 - 40)
        for (ms, label) in [(10.0, "10 ms"), (100, "100 ms"), (1000, "1 s"), (10_000, "10 s")] {
            let r = radius(ms, outer: outer)
            context.stroke(
                Path(ellipseIn: CGRect(x: centre.x - r, y: centre.y - r, width: r * 2, height: r * 2)),
                with: .color(.secondary.opacity(0.25)), lineWidth: 1)
            context.draw(
                Text(label).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: centre.x + 4, y: centre.y - r - 2), anchor: .bottomLeading)
        }
        let most = Double(max(node.peers.map(\.bytesReceived).max() ?? 1, 1))
        for (index, peer) in ordered.enumerated() {
            let at = position(peer, index: index, in: size)
            let ms = peer.pingMs ?? 10_000
            let color: Color = ms < 150 ? .green : ms < 1000 ? .orange : .red
            context.stroke(
                Path { path in
                    path.move(to: centre)
                    path.addLine(to: at)
                }, with: .color(color.opacity(0.35)), lineWidth: 1)
            let dot = 5 + CGFloat(sqrt(Double(peer.bytesReceived) / most)) * 7
            let circle = Path(ellipseIn: CGRect(x: at.x - dot, y: at.y - dot, width: dot * 2, height: dot * 2))
            context.fill(circle, with: .color(color))
            if hovered == peer.id {
                context.stroke(circle, with: .color(.primary), lineWidth: 2)
            }
        }
        context.fill(
            Path(ellipseIn: CGRect(x: centre.x - 8, y: centre.y - 8, width: 16, height: 16)),
            with: .color(.primary))
        context.draw(
            Text("your node").font(.caption2).foregroundStyle(.primary),
            at: CGPoint(x: centre.x, y: centre.y + 18), anchor: .center)
    }
}

/// The branches the node knows about near the tip of the chain, drawn as a tree.
///
/// The active chain runs along the top. Each side branch leaves it at the block where the
/// two diverged and runs for its own length. This is the daemon's own list of chain tips.
struct BranchesCard: View {
    @Environment(AppModel.self) private var model
    @State private var tips: [ChainTip] = []
    @State private var failure: String?
    @State private var loaded = false
    @State private var hovered: String?

    /// How far back from the tip to draw.
    private let window: UInt64 = 300
    private let maxDrawn = 8

    var body: some View {
        Card("Branches", trailing: loaded && failure == nil ? known : nil) {
            if let failure {
                Text(failure).foregroundStyle(.secondary)
            } else if !loaded {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text("Reading the chain tips from the node. This can take some seconds.")
                        .foregroundStyle(.secondary)
                }
            } else if let active {
                Text(summary(active))
                if !recent.isEmpty {
                    GeometryReader { geometry in
                        Canvas { context, size in
                            draw(context, size, active)
                        }
                        .contentShape(Rectangle())
                        .onContinuousHover { phase in
                            if case .active(let point) = phase {
                                let row = Int((point.y - 44) / 26 + 0.5)
                                hovered = recent.indices.contains(row) && point.y > 30 ? recent[row].hash : nil
                            } else {
                                hovered = nil
                            }
                        }
                        .onTapGesture(coordinateSpace: .local) { point in
                            let row = Int((point.y - 44) / 26 + 0.5)
                            if recent.indices.contains(row), point.y > 30 {
                                model.open(.block(id: recent[row].hash))
                            }
                        }
                    }
                    .frame(height: CGFloat(52 + recent.count * 26))
                    Text(caption)
                        .font(.callout)
                        .foregroundStyle(hovered == nil ? .secondary : .primary)
                        .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
                }
                Button("Refresh") {
                    loaded = false
                    Task { await load() }
                }
                if let ahead = headersAhead(active) {
                    Text("The node has headers to block \(Format.integer(ahead)). It has validated blocks to block \(Format.integer(active.height)).")
                        .font(.callout).foregroundStyle(.secondary)
                }
            } else {
                Text("The node reported no active chain.").foregroundStyle(.secondary)
            }
        }
        // Loaded once, and again on request. On a full node this request takes several
        // seconds and makes the node's other answers wait, so it is not repeated on a timer.
        .task(id: model.generation) { await load() }
    }

    private var active: ChainTip? { tips.first { $0.status == "active" } }

    /// Side branches the node has on record, not counting headers it has merely downloaded ahead.
    private var known: String {
        let ahead = active.flatMap(headersAhead) == nil ? 0 : 1
        let count = max(tips.count - 1 - ahead, 0)
        return "\(count) side branch\(count == 1 ? "" : "es") known"
    }

    /// Side branches that diverged within the window behind the tip.
    private var recent: [ChainTip] {
        guard let active else { return [] }
        let start = active.height > window ? active.height - window : 0
        return Array(
            tips.filter {
                $0.status != "active" && $0.height >= start && $0.height <= active.height + 2
                    && $0.branchLength <= window
            }.prefix(maxDrawn))
    }

    /// A syncing node lists its downloaded headers as a tip far ahead; that is not a fork.
    private func headersAhead(_ active: ChainTip) -> UInt64? {
        tips.first { $0.status == "headers-only" && $0.height > active.height + window }?.height
    }

    private func summary(_ active: ChainTip) -> String {
        if recent.isEmpty {
            return "The last \(window) blocks have no side branches."
        }
        let longest = recent.map(\.branchLength).max() ?? 0
        return "\(recent.count) side branch\(recent.count == 1 ? "" : "es") in the last \(window) blocks. "
            + "The longest side branch has \(longest) block\(longest == 1 ? "" : "s")."
    }

    private func meaning(_ status: String) -> String {
        switch status {
        case "valid-fork": "validated, then replaced"
        case "valid-headers": "headers valid, some blocks not downloaded"
        case "headers-only": "headers only, blocks not downloaded"
        case "invalid": "contains an invalid block"
        default: status
        }
    }

    private var caption: String {
        guard let tip = recent.first(where: { $0.hash == hovered }) else {
            return "The top line is the active chain. Each lower line is a side branch. Click a side branch to open its last block."
        }
        return "Branch ending at block \(Format.integer(tip.height)) · \(tip.branchLength) block"
            + "\(tip.branchLength == 1 ? "" : "s") long · \(meaning(tip.status))"
    }

    private func load() async {
        guard let explorer = model.explorer else { return }
        do {
            tips = try await explorer.chainTips()
            failure = nil
        } catch {
            failure = describe(error)
        }
        loaded = true
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize, _ active: ChainTip) {
        let start = active.height > window ? active.height - window : 0
        let left: CGFloat = 8
        let right = size.width - 44
        let x = { (height: UInt64) -> CGFloat in
            left + CGFloat(Double(height - min(height, start)) / Double(window)) * (right - left)
        }
        let top: CGFloat = 18
        context.stroke(
            Path { path in
                path.move(to: CGPoint(x: left, y: top))
                path.addLine(to: CGPoint(x: x(active.height), y: top))
            }, with: .color(.accentColor), style: StrokeStyle(lineWidth: 4, lineCap: .round))
        context.draw(
            Text("tip").font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: x(active.height) + 6, y: top), anchor: .leading)
        context.draw(
            Text("\(window) blocks ago").font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: left, y: top - 12), anchor: .leading)

        for (row, tip) in recent.enumerated() {
            let y = top + 26 + CGFloat(row) * 26
            let fork = tip.height >= tip.branchLength ? tip.height - tip.branchLength : 0
            let forkX = x(max(fork, start))
            let endX = max(x(tip.height), forkX + 6)
            let validated = tip.status == "valid-fork"
            let color: Color = tip.status == "invalid" ? .red : validated ? .orange : .secondary
            let selected = hovered == tip.hash
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: forkX, y: top))
                    path.addLine(to: CGPoint(x: forkX, y: y))
                    path.addLine(to: CGPoint(x: endX, y: y))
                }, with: .color(color),
                style: StrokeStyle(lineWidth: selected ? 3 : 2, lineJoin: .round, dash: validated ? [] : [4, 3]))
            context.fill(
                Path(ellipseIn: CGRect(x: endX - 4, y: y - 4, width: 8, height: 8)), with: .color(color))
            context.draw(
                Text("\(tip.branchLength)").font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: endX + 8, y: y), anchor: .leading)
        }
    }
}
