import CarbideCore
import SwiftUI

/// Recent blocks on a time axis. Each block sits at the moment it was produced, so the
/// spacing between marks is the time between blocks.
///
/// Squares are mined, circles are staked, and the bar above each mark is its transaction
/// count. A gap longer than three minutes is drawn in red.
struct ChainPulse: View {
    /// Newest first.
    let blocks: [BlockRow]
    /// True when the node is at the chain tip, so the right edge is the present moment.
    let live: Bool
    /// Unconfirmed transactions waiting for the next block.
    let waiting: UInt64?

    @Environment(AppModel.self) private var model
    @State private var hovered: String?

    private let span: Double = 25 * 60
    private let slowGap: Double = 180
    private let rightInset: CGFloat = 78
    private let height: CGFloat = 190

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            TimelineView(.periodic(from: .now, by: 1)) { timeline in
                let now = live
                    ? timeline.date.timeIntervalSince1970 : Double(blocks.first?.time ?? 0)
                GeometryReader { geometry in
                    Canvas { context, size in
                        draw(context, size, now)
                    }
                    .contentShape(Rectangle())
                    .onContinuousHover { phase in
                        switch phase {
                        case .active(let point):
                            hovered = nearest(to: point, width: geometry.size.width, now: now)?.hash
                        case .ended:
                            hovered = nil
                        }
                    }
                    .onTapGesture(coordinateSpace: .local) { point in
                        if let block = nearest(to: point, width: geometry.size.width, now: now) {
                            model.open(.block(id: block.hash))
                        }
                    }
                }
            }
            .frame(height: height)
            .accessibilityLabel("Timeline of recent blocks")

            Text(caption)
                .font(.callout)
                .foregroundStyle(hovered == nil ? .secondary : .primary)
                .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
        }
    }

    private var baseline: CGFloat { height - 52 }

    private func x(_ time: Double, width: CGFloat, now: Double) -> CGFloat {
        let right = width - rightInset
        return right - CGFloat((now - time) / span) * (right - 8)
    }

    private func nearest(to point: CGPoint, width: CGFloat, now: Double) -> BlockRow? {
        var best: BlockRow?
        var distance: CGFloat = 10
        for block in blocks {
            let gap = abs(x(Double(block.time), width: width, now: now) - point.x)
            if gap < distance {
                distance = gap
                best = block
            }
        }
        return best
    }

    private var caption: String {
        guard let hash = hovered, let index = blocks.firstIndex(where: { $0.hash == hash }) else {
            return "A square is a mined block. A circle is a staked block. The bar shows the number of "
                + "transactions. Click a block to open it."
        }
        let block = blocks[index]
        var text = "Block \(Format.integer(block.height)) · \(block.staked ? "staked" : "mined") · "
            + "\(block.transactionCount) transaction\(block.transactionCount == 1 ? "" : "s")"
        if index + 1 < blocks.count {
            let gap = Int(block.time) - Int(blocks[index + 1].time)
            text += " · \(Self.duration(Double(gap))) after the previous block"
        }
        return text
    }

    private static func duration(_ seconds: Double) -> String {
        if seconds < 0 { return "\(Int(seconds))s" }
        if seconds < 60 { return "\(Int(seconds))s" }
        return "\(Int(seconds) / 60)m \(String(format: "%02d", Int(seconds) % 60))s"
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize, _ now: Double) {
        let right = size.width - rightInset
        let base = baseline
        let ordered = blocks.reversed().filter { Double($0.time) > now - span - 240 }

        // Five-minute grid.
        var minutes = 5
        while Double(minutes * 60) < span {
            let gx = x(now - Double(minutes * 60), width: size.width, now: now)
            if gx < 30 { break }
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: gx, y: 12))
                    path.addLine(to: CGPoint(x: gx, y: base + 22))
                }, with: .color(.secondary.opacity(0.18)), lineWidth: 1)
            context.draw(
                Text(live ? "\(minutes) min ago" : "\(minutes) min earlier")
                    .font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: gx, y: base + 34), anchor: .center)
            minutes += 5
        }

        // Gaps between consecutive blocks.
        for (previous, block) in zip(ordered, ordered.dropFirst()) {
            let gap = Double(block.time) - Double(previous.time)
            let slow = gap > slowGap
            let x0 = max(x(Double(previous.time), width: size.width, now: now), 0)
            let x1 = x(Double(block.time), width: size.width, now: now)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: x0, y: base))
                    path.addLine(to: CGPoint(x: x1, y: base))
                }, with: .color(slow ? .red : .secondary.opacity(0.45)), lineWidth: slow ? 3 : 1)
            if slow, x1 - x0 > 44 {
                context.draw(
                    Text(Self.duration(gap)).font(.caption2.weight(.semibold)).foregroundStyle(.red),
                    at: CGPoint(x: (x0 + x1) / 2, y: base - 12), anchor: .center)
            }
        }

        // Time since the newest block, only meaningful when live.
        if live, let newest = ordered.last {
            let wait = now - Double(newest.time)
            let late = wait > slowGap
            context.stroke(
                Path { path in
                    path.move(
                        to: CGPoint(x: x(Double(newest.time), width: size.width, now: now), y: base))
                    path.addLine(to: CGPoint(x: right, y: base))
                }, with: .color(late ? .red : .secondary),
                style: StrokeStyle(lineWidth: late ? 2.5 : 1, dash: [3, 3]))
            context.draw(
                Text("waiting \(Self.duration(max(wait, 0)))")
                    .font(.caption2).foregroundStyle(late ? .red : .secondary),
                at: CGPoint(x: right - 5, y: base + 14), anchor: .trailing)
        }

        // Blocks.
        for block in ordered {
            let bx = x(Double(block.time), width: size.width, now: now)
            if bx < -8 { continue }
            let color: Color = block.staked ? .purple : .orange
            let selected = hovered == block.hash
            let barHeight = 5 + CGFloat(min(block.transactionCount, 30)) * 3.4
            context.fill(
                Path(CGRect(x: bx - 2.5, y: base - 8 - barHeight, width: 5, height: barHeight)),
                with: .color(color.opacity(selected ? 1 : 0.8)))
            let radius: CGFloat = selected ? 7 : 5
            let box = CGRect(x: bx - radius, y: base - radius, width: radius * 2, height: radius * 2)
            let mark = block.staked ? Path(ellipseIn: box) : Path(box)
            context.fill(mark, with: .color(color))
            if selected {
                context.stroke(mark, with: .color(.primary), lineWidth: 1.5)
            }
        }

        // The right edge: the present, or the newest block the node has validated.
        context.stroke(
            Path { path in
                path.move(to: CGPoint(x: right, y: 8))
                path.addLine(to: CGPoint(x: right, y: base + 22))
            }, with: .color(.primary.opacity(0.7)), lineWidth: 1)
        context.draw(
            Text(live ? "now" : "newest").font(.caption2).foregroundStyle(.primary),
            at: CGPoint(x: right - 5, y: 14), anchor: .trailing)

        // Waiting transactions stack up beside the present.
        if live, let waiting {
            let px = size.width - 40
            for index in 0..<Int(min(waiting, 26)) {
                let dot = CGRect(
                    x: px + (index % 2 == 0 ? -10 : 2), y: base - 8 - CGFloat(index / 2) * 11,
                    width: 8, height: 8)
                context.fill(Path(ellipseIn: dot), with: .color(.accentColor))
            }
            context.draw(
                Text("\(waiting) waiting").font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: px, y: base + 14), anchor: .center)
        }
    }
}
