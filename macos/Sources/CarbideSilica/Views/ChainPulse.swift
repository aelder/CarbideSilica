import CarbideCore
import SwiftUI

/// Recent blocks as bursts on a time axis, with the current wait set among recent waits.
///
/// Verus blocks arrive in bursts: several within seconds, then a wait. So one burst is one
/// column at one time, with one cell for each block. Staked cells go up from the line and
/// mined cells go down. What is unusual comes from the chain's own record of waits, not
/// from a fixed number of minutes. `macos/design/pulse-research.html` has the reasons.
struct ChainPulse: View {
    /// Newest first.
    let blocks: [BlockRow]
    /// True when the node is at the chain tip, so the right edge is the present moment.
    let live: Bool
    /// Unconfirmed transactions in the mempool.
    let waiting: UInt64?
    /// The block selected in the dashboard's list, drawn like a hovered block.
    var selected: String?

    @Environment(AppModel.self) private var model
    @State private var hovered: String?
    @State private var record = WaitRecord.shared

    private var history: PulseHistory? { record.history }
    private var historyProblem: String? { record.problem }

    private let span = WaitRecord.span
    /// The target time between blocks.
    private let target: Double = 60

    private let leftInset: CGFloat = 54
    private let rightInset: CGFloat = 16
    private let topPad: CGFloat = 8
    private let cell: CGFloat = 9
    private let cellStep: CGFloat = 11
    /// Room for four stacked cells on each side of the line. A larger burst is rare, and
    /// its cells then move closer together to stay inside the lane.
    private let laneCells = 4
    private var lane: CGFloat { 7 + CGFloat(laneCells) * cellStep }
    private var axisY: CGFloat { topPad + lane }
    private var timelineHeight: CGFloat { topPad + lane * 2 + 22 }
    private let waitsHeight: CGFloat = 80

    private struct Placed {
        let block: BlockRow
        let rect: CGRect
    }

    private struct Burst {
        let blocks: [BlockRow]
        let x: CGFloat
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            TimelineView(.periodic(from: .now, by: 1)) { timeline in
                let now = live
                    ? timeline.date.timeIntervalSince1970 : Double(blocks.first?.time ?? 0)
                VStack(alignment: .leading, spacing: 6) {
                    summary(now)
                    GeometryReader { geometry in
                        let bursts = bursts(width: geometry.size.width, now: now)
                        let marks = cells(in: bursts)
                        Canvas { context, size in
                            drawTimeline(context, size, now, bursts, marks)
                        }
                        .contentShape(Rectangle())
                        .onContinuousHover { phase in
                            switch phase {
                            case .active(let point): hovered = nearest(to: point, in: marks)?.hash
                            case .ended: hovered = nil
                            }
                        }
                        .onTapGesture(coordinateSpace: .local) { point in
                            if let block = nearest(to: point, in: marks) {
                                model.open(.block(id: block.hash))
                            }
                        }
                    }
                    .frame(height: timelineHeight)
                    .accessibilityLabel("Timeline of recent blocks in bursts")

                    waitSentence(now)
                    Canvas { context, size in
                        drawWaits(context, size, live ? currentWait(now) : nil)
                    }
                    .frame(height: waitsHeight)
                    .accessibilityLabel("Recent waits between bursts, with the current wait")
                }
            }

            Text(caption)
                .font(.callout)
                .foregroundStyle(hovered == nil ? .secondary : .primary)
                .frame(maxWidth: .infinity, minHeight: 54, alignment: .topLeading)
        }
        .task(id: model.generation) { await keepHistory() }
    }

    // MARK: The chain's record

    /// The record of waits is shared with the Mempool page.
    private func keepHistory() async {
        await record.keep(model, live: live)
    }

    private var burstSeconds: Int { Int(history?.burstSeconds ?? 10) }

    /// The share of recent waits that are shorter than `seconds`, from 0 to 1.
    private func shareShorter(than seconds: Double) -> Double? {
        guard let waits = history?.waits, !waits.isEmpty else { return nil }
        var low = 0
        var high = waits.count
        while low < high {
            let middle = (low + high) / 2
            if Double(waits[middle]) < seconds { low = middle + 1 } else { high = middle }
        }
        return Double(low) / Double(waits.count)
    }

    private func waitQuantile(_ share: Double) -> Double? {
        guard let waits = history?.waits, !waits.isEmpty else { return nil }
        return Double(waits[min(Int(Double(waits.count) * share), waits.count - 1)])
    }

    private var longestWait: Double? { history?.waits.last.map(Double.init) }

    /// Time since the newest block.
    private func currentWait(_ now: Double) -> Double? {
        blocks.first.map { max(now - Double($0.time), 0) }
    }

    /// Neutral for an ordinary wait. Red only for a wait longer than each one in the record.
    private func waitColor(_ seconds: Double) -> Color {
        if let longestWait, seconds > longestWait { return .red }
        return .primary
    }

    /// True for a wait in the longest tenth of the record.
    private func isLong(_ seconds: Double) -> Bool {
        guard let limit = waitQuantile(0.9) else { return false }
        return seconds >= limit
    }

    // MARK: Summary

    /// The figures that answer first: how many blocks, against the target and against what
    /// is usual for this chain.
    private func summary(_ now: Double) -> some View {
        let inWindow = blocks.filter { Double($0.time) > now - span && Double($0.time) <= now + 120 }
        let oldest = blocks.last.map { Double($0.time) } ?? now
        // The list can be shorter than the window. Count against the time it covers.
        let covered = min(span, max(now - oldest, target))
        let minutes = Int((covered / 60).rounded())
        let staked = inWindow.filter(\.staked).count
        let goal = Int((covered / target).rounded())
        let counts = history?.windowCounts ?? []
        let usual: (Int, Int)? = counts.count >= 10 && covered >= span
            ? (Int(counts[counts.count / 10]), Int(counts[counts.count * 9 / 10])) : nil
        return HStack(spacing: 18) {
            figure("Blocks", "\(inWindow.count)", "in \(minutes) min")
            BulletBar(value: inWindow.count, target: goal, usual: usual)
                .frame(width: 110, height: 12)
            figure("Target", "\(goal)")
            if let usual {
                figure("Usual", "\(usual.0) to \(usual.1)")
                    .help("Four in five 25-minute periods in the record had this number of blocks.")
            }
            figure("Staked", "\(staked)")
            figure("Mined", "\(inWindow.count - staked)")
            if live, let waiting {
                figure("Unconfirmed", "\(waiting)")
            }
            Spacer(minLength: 0)
        }
        .font(.callout)
        .lineLimit(1)
    }

    private func figure(_ label: String, _ value: String, _ unit: String? = nil, color: Color = .primary)
        -> some View
    {
        HStack(spacing: 5) {
            Text(label).foregroundStyle(.secondary)
            Text(value).fontWeight(.semibold).monospacedDigit().foregroundStyle(color)
            if let unit { Text(unit).foregroundStyle(.secondary) }
        }
    }

    /// One sentence that places the current wait in the chain's record.
    private func waitSentence(_ now: Double) -> some View {
        HStack(spacing: 5) {
            if live, let wait = currentWait(now) {
                Text("Last block").foregroundStyle(.secondary)
                Text(Self.duration(wait)).fontWeight(.semibold).monospacedDigit()
                    .foregroundStyle(waitColor(wait))
                Text("ago.").foregroundStyle(.secondary)
                if let share = shareShorter(than: wait) {
                    if let longestWait, wait > longestWait {
                        Text("This wait is longer than each wait in the record.")
                            .foregroundStyle(.red)
                    } else {
                        Text("Longer than").foregroundStyle(.secondary)
                        Text("\(Int((share * 100).rounded())) of 100").fontWeight(.semibold).monospacedDigit()
                        Text("recent waits.").foregroundStyle(.secondary)
                    }
                }
            }
            Text(recordText).foregroundStyle(.secondary)
            Spacer(minLength: 0)
        }
        .font(.callout)
        .lineLimit(1)
        .padding(.top, 2)
    }

    private var recordText: String {
        if let history {
            let hours = Double(history.coveredSeconds) / 3600
            let period = hours >= 1.5
                ? "approximately \(Int(hours.rounded())) hours" : "approximately \(Int((hours * 60).rounded())) minutes"
            return "The record is the last \(Format.integer(UInt64(history.blocks))) blocks, \(period)."
        }
        if let historyProblem { return "The app cannot read the record of waits. \(historyProblem)" }
        return live ? "The app reads the record of recent waits." : ""
    }

    // MARK: Caption

    private var caption: String {
        guard let hash = hovered, let index = blocks.firstIndex(where: { $0.hash == hash }) else {
            return "Each cell is one block. Blocks within \(burstSeconds) seconds of each other form one column, at the "
                + "time of the first. Staked blocks are above the line and mined blocks are below it. A gap shows its "
                + "time when it is in the longest tenth of the record, and it is red when the record has no wait as "
                + "long. In the lower plot, each dot is 1 in 100 of the recorded waits. Positions are block "
                + "timestamps. Click a block to open it."
        }
        let block = blocks[index]
        var text = "Block \(Format.integer(block.height)) · \(block.staked ? "staked" : "mined") · "
            + "\(block.transactionCount) transaction\(block.transactionCount == 1 ? "" : "s") · "
            + Format.date(block.time)
        if index + 1 < blocks.count {
            let gap = Int(block.time) - Int(blocks[index + 1].time)
            text += gap >= 0
                ? " · \(Self.duration(Double(gap))) after the previous block"
                : " · timestamp \(-gap)s before the previous block"
        }
        if let seen = model.arrivals[hash] {
            text += " · this app saw it at \(seen.formatted(date: .omitted, time: .standard))"
        }
        return text
    }

    private static func duration(_ seconds: Double) -> String {
        if seconds < 60 { return "\(Int(seconds))s" }
        return "\(Int(seconds) / 60)m \(String(format: "%02d", Int(seconds) % 60))s"
    }

    // MARK: Layout

    private func x(_ time: Double, width: CGFloat, now: Double) -> CGFloat {
        let right = width - rightInset
        return right - CGFloat((now - time) / span) * (right - leftInset)
    }

    /// Groups the blocks into bursts, in chain order, and gives each burst one position.
    /// The bridge uses the same rule for the record: a block no more than the burst time
    /// after the block before is in the same burst.
    private func bursts(width: CGFloat, now: Double) -> [Burst] {
        var groups: [[BlockRow]] = []
        for block in blocks.reversed() {
            if let previous = groups.last?.last, Int(block.time) - Int(previous.time) <= burstSeconds {
                groups[groups.count - 1].append(block)
            } else {
                groups.append([block])
            }
        }
        let right = width - rightInset
        var placed: [Burst] = []
        for group in groups {
            // A timestamp can be a little later than this Mac's clock.
            var bx = min(x(Double(group[0].time), width: width, now: now), right)
            guard bx >= leftInset else { continue }
            // Two bursts a few seconds apart must not draw on each other.
            if let before = placed.last, bx - before.x < cellStep { bx = before.x + cellStep }
            placed.append(Burst(blocks: group, x: bx))
        }
        return placed
    }

    /// One cell for each block: staked cells up from the line, mined cells down.
    private func cells(in bursts: [Burst]) -> [Placed] {
        var placed: [Placed] = []
        for burst in bursts {
            var up = 0
            var down = 0
            let stakedCount = burst.blocks.filter(\.staked).count
            let upStep = step(for: stakedCount)
            let downStep = step(for: burst.blocks.count - stakedCount)
            for block in burst.blocks {
                let y: CGFloat
                if block.staked {
                    y = axisY - 7 - cell - CGFloat(up) * upStep
                    up += 1
                } else {
                    y = axisY + 7 + CGFloat(down) * downStep
                    down += 1
                }
                placed.append(
                    Placed(block: block, rect: CGRect(x: burst.x - cell / 2, y: y, width: cell, height: cell)))
            }
        }
        return placed
    }

    /// The distance between stacked cells: the normal step, or less when a burst has more
    /// cells of one kind than the lane holds.
    private func step(for count: Int) -> CGFloat {
        count <= laneCells ? cellStep : (lane - 7 - cell) / CGFloat(count - 1)
    }

    private func nearest(to point: CGPoint, in marks: [Placed]) -> BlockRow? {
        var best: BlockRow?
        var distance: CGFloat = 9
        for mark in marks {
            let gap = hypot(mark.rect.midX - point.x, mark.rect.midY - point.y)
            if gap < distance {
                distance = gap
                best = mark.block
            }
        }
        return best
    }

    // MARK: Drawing the timeline

    private func drawTimeline(
        _ context: GraphicsContext, _ size: CGSize, _ now: Double, _ bursts: [Burst], _ marks: [Placed]
    ) {
        let right = size.width - rightInset
        let top = topPad - 2
        let bottom = axisY + lane

        // Five-minute grid.
        var minutes = 5
        while Double(minutes * 60) <= span {
            let gx = x(now - Double(minutes * 60), width: size.width, now: now)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: gx, y: top))
                    path.addLine(to: CGPoint(x: gx, y: bottom))
                }, with: .color(.secondary.opacity(0.14)), lineWidth: 1)
            context.draw(
                Text(live ? "\(minutes) min ago" : "\(minutes) min earlier")
                    .font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: gx, y: bottom + 12), anchor: .center)
            minutes += 5
        }

        // The window can start before the oldest block that the app has loaded. Say so:
        // an empty stretch there is not a stretch without blocks.
        if let oldest = blocks.last {
            let ox = x(Double(oldest.time), width: size.width, now: now)
            if ox > leftInset + 40 {
                context.fill(
                    Path(CGRect(x: leftInset, y: top, width: ox - 8 - leftInset, height: bottom - top)),
                    with: .color(.secondary.opacity(0.08)))
                context.draw(
                    Text("not loaded").font(.caption2).foregroundStyle(.secondary),
                    at: CGPoint(x: (leftInset + ox - 8) / 2, y: top + 8), anchor: .center)
            }
        }

        // The line. It has no ticks for the target: this chain has no steady rhythm.
        context.stroke(
            Path { path in
                path.move(to: CGPoint(x: leftInset, y: axisY))
                path.addLine(to: CGPoint(x: right, y: axisY))
            }, with: .color(.secondary.opacity(0.4)), lineWidth: 1)
        context.draw(
            Text("Staked").font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: 0, y: axisY - 14), anchor: .leading)
        context.draw(
            Text("Mined").font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: 0, y: axisY + 14), anchor: .leading)

        // Waits between bursts that are long for this chain.
        for (before, burst) in zip(bursts, bursts.dropFirst()) {
            guard let last = before.blocks.last, let first = burst.blocks.first else { continue }
            let wait = Double(first.time) - Double(last.time)
            guard isLong(wait) else { continue }
            waitSpan(context, from: before.x + 7, to: burst.x - 7, seconds: wait, dashed: false)
        }

        // The wait since the newest block. Only the present has one.
        if live, let newest = bursts.last, let wait = currentWait(now), isLong(wait) {
            waitSpan(context, from: newest.x + 7, to: right, seconds: wait, dashed: true)
        }

        // Blocks.
        for mark in marks {
            let block = mark.block
            let color: Color = block.staked ? .purple : .orange
            let chosen = hovered == block.hash || selected == block.hash
            let box = chosen ? mark.rect.insetBy(dx: -1.5, dy: -1.5) : mark.rect
            let shape = block.staked ? Path(ellipseIn: box) : Path(roundedRect: box, cornerRadius: 1.5)
            context.fill(shape, with: .color(color))
            if chosen {
                context.stroke(shape, with: .color(.primary), lineWidth: 1.5)
            }
        }

        // The right edge: the present, or the newest block the node has validated.
        context.stroke(
            Path { path in
                path.move(to: CGPoint(x: right, y: top))
                path.addLine(to: CGPoint(x: right, y: bottom))
            }, with: .color(.primary.opacity(0.6)), lineWidth: 1)
        context.draw(
            Text(live ? "now" : "newest").font(.caption2).foregroundStyle(.primary),
            at: CGPoint(x: right, y: bottom + 12), anchor: .trailing)
    }

    /// A long wait: a stronger stretch of the line with its duration above it.
    private func waitSpan(
        _ context: GraphicsContext, from x0: CGFloat, to x1: CGFloat, seconds: Double, dashed: Bool
    ) {
        guard x1 > x0 else { return }
        let color = waitColor(seconds).opacity(0.85)
        context.stroke(
            Path { path in
                path.move(to: CGPoint(x: x0, y: axisY))
                path.addLine(to: CGPoint(x: x1, y: axisY))
            }, with: .color(color),
            style: StrokeStyle(lineWidth: 2, lineCap: .butt, dash: dashed ? [4, 3] : []))
        guard x1 - x0 > 56 else { return }
        context.draw(
            Text(Self.duration(seconds)).font(.caption2.weight(.semibold)).foregroundStyle(color),
            at: CGPoint(x: (x0 + x1) / 2, y: axisY - 10), anchor: .center)
    }

    // MARK: Drawing the waits

    /// The recorded waits as dots on a logarithmic time axis. Each dot is an equal share of
    /// the waits, so a person can count dots: the dots left of the marker are the share of
    /// waits that were shorter than the current one.
    private func drawWaits(_ context: GraphicsContext, _ size: CGSize, _ current: Double?) {
        let right = size.width - rightInset
        let base = size.height - 24
        let dot: CGFloat = 6
        let low: Double = 10
        let high = max(1800, (longestWait ?? 0) * 1.15, (current ?? 0) * 1.15)
        let position = { (seconds: Double) -> CGFloat in
            let share = (log(max(seconds, low)) - log(low)) / (log(high) - log(low))
            return leftInset + CGFloat(share) * (right - leftInset)
        }

        context.draw(
            Text("Waits").font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: 0, y: base - 8), anchor: .leading)
        context.stroke(
            Path { path in
                path.move(to: CGPoint(x: leftInset, y: base))
                path.addLine(to: CGPoint(x: right, y: base))
            }, with: .color(.secondary.opacity(0.4)), lineWidth: 1)
        let ticks: [(Double, String)] = [
            (10, "10 s"), (30, "30 s"), (60, "1 min"), (120, "2 min"), (300, "5 min"), (600, "10 min"),
            (1200, "20 min"), (3600, "1 h"),
        ]
        for (seconds, label) in ticks where seconds <= high {
            let tx = position(seconds)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: tx, y: base))
                    path.addLine(to: CGPoint(x: tx, y: base + 4))
                }, with: .color(.secondary.opacity(0.6)), lineWidth: 1)
            context.draw(
                Text(label).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: tx, y: base + 13), anchor: .center)
        }

        // One hundred dots at most, so "58 of 100" in the sentence is 58 dots left of the
        // marker. Each dot is the wait at the middle of an equal share of the record.
        if let waits = history?.waits, !waits.isEmpty {
            let count = min(100, waits.count)
            var stacks: [Int: Int] = [:]
            for index in 0..<count {
                let rank = Int((Double(index) + 0.5) / Double(count) * Double(waits.count))
                let value = Double(waits[min(rank, waits.count - 1)])
                let bin = Int((position(value) - leftInset) / (dot + 1))
                let level = stacks[bin, default: 0]
                stacks[bin] = level + 1
                let center = CGPoint(
                    x: leftInset + (CGFloat(bin) + 0.5) * (dot + 1),
                    y: base - 2 - dot / 2 - CGFloat(level) * (dot + 1))
                let shorter = current.map { value < $0 } ?? true
                context.fill(
                    Path(ellipseIn: CGRect(x: center.x - dot / 2, y: center.y - dot / 2, width: dot, height: dot)),
                    with: .color(shorter ? Color.accentColor : Color.secondary.opacity(0.45)))
            }
        }

        // The current wait moves along the axis.
        if let current {
            let cx = min(position(current), right)
            let color = waitColor(current)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: cx, y: 12))
                    path.addLine(to: CGPoint(x: cx, y: base))
                }, with: .color(color), lineWidth: 1.5)
            context.draw(
                Text("now").font(.caption2.weight(.semibold)).foregroundStyle(color),
                at: CGPoint(x: cx, y: 5), anchor: cx > right - 20 ? .trailing : .center)
        }
    }
}

/// Stephen Few's bullet graph in one line: a bar for the count, a tick for the target, and
/// a band for the range that is usual for this chain.
private struct BulletBar: View {
    let value: Int
    let target: Int
    let usual: (Int, Int)?

    var body: some View {
        Canvas { context, size in
            let top = Double(max(value, target, usual?.1 ?? 0)) * 1.2
            guard top > 0 else { return }
            let position = { (count: Int) -> CGFloat in CGFloat(Double(count) / top) * size.width }
            context.fill(
                Path(roundedRect: CGRect(origin: .zero, size: size), cornerRadius: 2),
                with: .color(.secondary.opacity(0.12)))
            if let usual {
                context.fill(
                    Path(
                        CGRect(
                            x: position(usual.0), y: 0, width: position(usual.1) - position(usual.0),
                            height: size.height)),
                    with: .color(.secondary.opacity(0.3)))
            }
            context.fill(
                Path(CGRect(x: 0, y: size.height / 2 - 2, width: position(value), height: 4)),
                with: .color(.primary.opacity(0.85)))
            context.fill(
                Path(CGRect(x: position(target) - 1, y: 0, width: 2, height: size.height)),
                with: .color(.primary))
        }
        .accessibilityLabel(
            "\(value) blocks. Target \(target)."
                + (usual.map { " Usual \($0.0) to \($0.1)." } ?? ""))
        .help("The bar is the number of blocks. The tick is the target. The band is the usual range.")
    }
}
