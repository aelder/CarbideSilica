import CarbideCore
import SwiftUI

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
                    DisplayPanel {
                        VStack(alignment: .leading, spacing: 10) {
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
                            PanelRule()
                            Text(caption)
                                .font(Tone.caption)
                                .foregroundStyle(hovered == nil ? Tone.label : Tone.value)
                                .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
                        }
                    }
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
        let tipX = x(active.height)
        // The active chain, and its tip: the present.
        context.rule(left, top, tipX, top, Tone.value, width: 1.5)
        context.lit(Path(CGRect(x: tipX - 0.75, y: top - 7, width: 1.5, height: 14)), Tone.present, strength: 0.7)
        context.name("tip", tipX + 6, top, .leading, color: Tone.present)
        context.name("\(window) blocks ago", left, top - 12, .leading)

        for (row, tip) in recent.enumerated() {
            let y = top + 26 + CGFloat(row) * 26
            let fork = tip.height >= tip.branchLength ? tip.height - tip.branchLength : 0
            let forkX = x(max(fork, start))
            let endX = max(x(tip.height), forkX + 6)
            let validated = tip.status == "valid-fork"
            let color: Color = tip.status == "invalid" ? Tone.alert : validated ? Tone.mined : Tone.axis
            let selected = hovered == tip.hash
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: forkX, y: top))
                    path.addLine(to: CGPoint(x: forkX, y: y))
                    path.addLine(to: CGPoint(x: endX, y: y))
                }, with: .color(selected ? Tone.present : color),
                style: StrokeStyle(lineWidth: selected ? 1.5 : 1, lineJoin: .miter, dash: validated ? [] : [4, 3]))
            let end = Path(CGRect(x: endX - 3.5, y: y - 3.5, width: 7, height: 7))
            context.lit(end, color, strength: selected ? 0.9 : 0)
            if selected {
                context.stroke(end, with: .color(Tone.present), lineWidth: 1.5)
            }
            context.figure("\(tip.branchLength)", endX + 9, y, .leading, color: selected ? Tone.present : Tone.value)
        }
    }
}
