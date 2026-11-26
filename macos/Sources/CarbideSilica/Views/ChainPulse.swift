import CarbideCore
import SwiftUI

/// The chain pulse: one instrument for the rhythm of the chain.
///
/// Verus blocks arrive in bursts: several within seconds, then a wait. The upper part shows
/// the last 25 minutes on an even time scale: one mark for each block, staked above the
/// line and mined below it. The lower part sets the present wait among the waits of the
/// last day, on a logarithmic scale. One curve joins the two parts: it is the time of the
/// lower axis on an even scale, and it ends at the present of the upper part.
///
/// The panel is a display: it is black in the light and the dark appearance. Colour is a
/// signal only: violet is staked, amber is mined, white is the present, red is a wait
/// longer than each wait in the record. `macos/design/pulse-research.html` has the reasons
/// for the forms.
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

    // MARK: The measures of the panel

    private let gutter: CGFloat = 78
    private let rightInset: CGFloat = 22
    private let cell: CGFloat = 9
    private let cellStep: CGFloat = 11
    /// Room for four stacked cells on each side of the line. A larger burst is rare, and
    /// its cells then move closer together to stay inside the lane.
    private let laneCells = 4
    private var lane: CGFloat { 7 + CGFloat(laneCells) * cellStep }

    /// The upper part: the top of the lanes, the line, and the bottom of the lanes.
    private let blocksTop: CGFloat = 20
    private var axisY: CGFloat { blocksTop + lane }
    private var blocksBottom: CGFloat { axisY + lane }
    /// The lower part: the line of the waits.
    private var waitsBase: CGFloat { blocksBottom + 96 }
    private var rulerTop: CGFloat { waitsBase + 26 }
    private var plotHeight: CGFloat { rulerTop + 30 }

    private struct Placed {
        let block: BlockRow
        let rect: CGRect
    }

    private struct Burst {
        let blocks: [BlockRow]
        let x: CGFloat
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            TimelineView(.periodic(from: .now, by: 1)) { timeline in
                readouts(moment(timeline.date))
            }
            .padding(.horizontal, 18)
            .padding(.top, 14)
            .padding(.bottom, 10)

            Rectangle().fill(Tone.rule).frame(height: 1)

            // The app draws each second. The mark of the present wait does not tick: it is
            // a layer that the system moves at the rate of the display.
            TimelineView(.periodic(from: .now, by: 1)) { timeline in
                let now = moment(timeline.date)
                let current = live ? currentWait(now) : nil
                GeometryReader { geometry in
                    let bursts = bursts(width: geometry.size.width, now: now)
                    let marks = cells(in: bursts)
                    Canvas { context, size in
                        drawBlocks(context, size, now, bursts, marks)
                        drawWaits(context, size, current)
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
                    .overlay(alignment: .topLeading) {
                        if let current {
                            presentMark(current, width: geometry.size.width, beat: Int(now))
                        }
                    }
                    // The same front crosses the lower part at the same time. It gives
                    // light to the time curve and to the dots. It moves nothing there.
                    .overlay(alignment: .topLeading) {
                        if live {
                            let shapes = waitShapes(width: geometry.size.width, current: current)
                            LightTrace(
                                parts: [
                                    .init(path: shapes.curve.cgPath, stroke: true, color: LightTrace.warm, glow: LightTrace.warm),
                                    .init(path: shapes.ahead.cgPath, stroke: false, color: LightTrace.warm, glow: LightTrace.warm),
                                    .init(path: shapes.passed.cgPath, stroke: false, color: LightTrace.pale, glow: LightTrace.blue),
                                ],
                                area: CGRect(
                                    x: gutter, y: blocksBottom,
                                    width: max(geometry.size.width - rightInset - gutter, 0),
                                    height: waitsBase - blocksBottom + 2),
                                beat: Int(now),
                                // The lower part is the quiet part: half the light.
                                strength: 0.5
                            )
                            .allowsHitTesting(false)
                        }
                    }
                    // The marks of the blocks are layers. With each update a front goes
                    // from the present to the left. The marks get their new places as the
                    // front passes, and the front gives light to the line and to the marks,
                    // as the trace of a vector monitor does.
                    .overlay(alignment: .topLeading) {
                        PulseTrace(
                            staked: marks.filter { $0.block.staked }.map(\.rect),
                            mined: marks.filter { !$0.block.staked }.map(\.rect),
                            lineY: axisY,
                            area: CGRect(
                                x: gutter, y: blocksTop - 6,
                                width: max(geometry.size.width - rightInset - gutter, 0),
                                height: lane * 2 + 12),
                            beat: Int(now), moves: live
                        )
                        .allowsHitTesting(false)
                    }
                }
            }
            .frame(height: plotHeight)
            .accessibilityLabel("The blocks of the last 25 minutes, and the present wait among the waits of the last day")

            Rectangle().fill(Tone.rule).frame(height: 1)

            Text(caption)
                .font(.system(size: 11.5))
                .foregroundStyle(hovered == nil ? Tone.label : Tone.value)
                .lineLimit(2)
                .frame(maxWidth: .infinity, minHeight: 16, alignment: .leading)
                .padding(.horizontal, 18)
                .padding(.vertical, 10)
        }
        .background(Tone.screen, in: RoundedRectangle(cornerRadius: 4))
        .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(Tone.rule, lineWidth: 1))
        .overlay(CornerMarks().stroke(Tone.value.opacity(0.55), lineWidth: 1).padding(5))
        // The panel is a display. It has the same colours in the light appearance.
        .environment(\.colorScheme, .dark)
        // The reader starts again when the node gets to the tip: it reads only then.
        .task(id: "\(model.generation)|\(live)") { await keepHistory() }
    }

    /// The present, or the time of the newest block when the node is not at the tip.
    private func moment(_ date: Date) -> Double {
        live ? date.timeIntervalSince1970 : Double(blocks.first?.time ?? 0)
    }

    // MARK: The chain's record

    /// The record of waits is shared with the Mempool page.
    private func keepHistory() async {
        await record.keep(model, live: live)
    }

    /// The time that the record of waits covers: "26 hours".
    private var recordSpan: String {
        guard let history else { return "day" }
        let hours = Double(history.coveredSeconds) / 3600
        return hours >= 1.5 ? "\(Int(hours.rounded())) hours" : "\(Int((hours * 60).rounded())) minutes"
    }

    private var burstSeconds: Int { Int(history?.burstSeconds ?? 10) }

    private func waitQuantile(_ share: Double) -> Double? {
        guard let waits = history?.waits, !waits.isEmpty else { return nil }
        return Double(waits[min(Int(Double(waits.count) * share), waits.count - 1)])
    }

    private var longestWait: Double? { history?.waits.last.map(Double.init) }

    /// Time since the newest block.
    private func currentWait(_ now: Double) -> Double? {
        blocks.first.map { max(now - Double($0.time), 0) }
    }

    /// True for a wait longer than each one in the record.
    private func isRecord(_ seconds: Double) -> Bool {
        longestWait.map { seconds > $0 } ?? false
    }

    private func waitColor(_ seconds: Double) -> Color {
        isRecord(seconds) ? Tone.alert : Tone.present
    }

    /// True for a wait in the longest tenth of the record.
    private func isLong(_ seconds: Double) -> Bool {
        guard let limit = waitQuantile(0.9) else { return false }
        return seconds >= limit
    }

    // MARK: Readouts

    /// The figures that answer first: how many blocks, against the target and against what
    /// is usual for this chain.
    private func readouts(_ now: Double) -> some View {
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
        func row(_ spacing: CGFloat) -> some View {
            HStack(alignment: .lastTextBaseline, spacing: spacing) {
                Readout("Blocks in \(minutes) min", "\(inWindow.count)", reserve: "00")
                BulletBar(value: inWindow.count, target: goal, usual: usual)
                    .frame(width: 120, height: 10)
                    .alignmentGuide(.lastTextBaseline) { $0[.bottom] - 4 }
                Readout("Target", "\(goal)", reserve: "00")
                if let usual {
                    Readout("Usual", "\(usual.0)–\(usual.1)", reserve: "00–00")
                        .help("Four in five 25-minute periods in the record had this number of blocks.")
                }
                Readout("Staked", "\(staked)", tone: Tone.staked, reserve: "00")
                Readout("Mined", "\(inWindow.count - staked)", tone: Tone.mined, reserve: "00")
                if live, let waiting {
                    Readout("Unconfirmed", "\(waiting)", reserve: "00")
                }
                Spacer(minLength: 0)
                if let historyProblem {
                    Text("The app cannot read the record of waits. \(historyProblem)")
                        .font(.system(size: 11.5)).foregroundStyle(Tone.alert).lineLimit(1)
                }
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

    // MARK: Caption

    private var caption: String {
        guard let hash = hovered, let index = blocks.firstIndex(where: { $0.hash == hash }) else {
            return "Top: one mark is one block. Bottom: one dot is 1 in 100 of the waits in the last "
                + "\(recordSpan). The curve is the time on the lower axis."
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

    /// "6:08" for a wait of six minutes and eight seconds.
    private static func clock(_ seconds: Double) -> String {
        let whole = Int(seconds)
        return whole >= 3600
            ? "\(whole / 3600):\(String(format: "%02d", whole / 60 % 60)):\(String(format: "%02d", whole % 60))"
            : "\(whole / 60):\(String(format: "%02d", whole % 60))"
    }

    // MARK: Layout

    private func x(_ time: Double, width: CGFloat, now: Double) -> CGFloat {
        let right = width - rightInset
        return right - CGFloat((now - time) / span) * (right - gutter)
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
            var bx = min(x(Double(group[0].time), width: width, now: now), right - cell)
            guard bx >= gutter else { continue }
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

    /// The longest time on the axis of the waits.
    private func waitsEnd(_ current: Double?) -> Double {
        max(1800, (longestWait ?? 0) * 1.15, (current ?? 0) * 1.15)
    }

    /// Where a wait is on the logarithmic axis.
    private func waitX(_ seconds: Double, width: CGFloat, high: Double) -> CGFloat {
        let low: Double = 10
        let right = width - rightInset
        let share = (log(max(seconds, low)) - log(low)) / (log(high) - log(low))
        return gutter + CGFloat(share) * (right - gutter)
    }

    // MARK: Drawing the blocks

    private func drawBlocks(
        _ context: GraphicsContext, _ size: CGSize, _ now: Double, _ bursts: [Burst], _ marks: [Placed]
    ) {
        let right = size.width - rightInset
        let top = blocksTop - 6
        let bottom = blocksBottom + 6

        context.plate("Staked", y: axisY - lane / 2, fill: Tone.staked)
        context.plate("Mined", y: axisY + lane / 2, fill: Tone.mined)

        // A tick for each minute, and a line for each five.
        var minute = 1
        while Double(minute * 60) <= span {
            let gx = x(now - Double(minute * 60), width: size.width, now: now)
            if minute % 5 == 0 {
                context.rule(gx, top, gx, bottom, Tone.grid)
                context.name("\(minute) min", gx, bottom + 11)
            } else {
                context.rule(gx, axisY - 2.5, gx, axisY + 2.5, Tone.rule)
            }
            minute += 1
        }

        // The window can start before the oldest block that the app has loaded. Say so:
        // an empty stretch there is not a stretch without blocks.
        if let oldest = blocks.last {
            let ox = x(Double(oldest.time), width: size.width, now: now)
            if ox > gutter + 40 {
                var hatch = Path()
                var hx = gutter
                while hx < ox - 8 {
                    hatch.move(to: CGPoint(x: hx, y: bottom))
                    hatch.addLine(to: CGPoint(x: min(hx + (bottom - top), ox - 8), y: top))
                    hx += 9
                }
                context.stroke(hatch, with: .color(Tone.grid), lineWidth: 1)
                context.name("not loaded", (gutter + ox - 8) / 2, top + 9)
            }
        }

        // The line. It has no ticks for the target: this chain has no steady rhythm.
        context.rule(gutter, axisY, right, axisY, Tone.axis)

        // Waits between bursts that are long for this chain.
        for (before, burst) in zip(bursts, bursts.dropFirst()) {
            guard let last = before.blocks.last, let first = burst.blocks.first else { continue }
            let wait = Double(first.time) - Double(last.time)
            guard isLong(wait) else { continue }
            waitSpan(context, from: before.x + 8, to: burst.x - 8, seconds: wait, dashed: false)
        }

        // The wait since the newest block. Only the present has one.
        if live, let newest = bursts.last, let wait = currentWait(now), isLong(wait) {
            waitSpan(context, from: newest.x + 8, to: right, seconds: wait, dashed: true)
        }

        // Blocks.
        for mark in marks {
            let block = mark.block
            let color = block.staked ? Tone.staked : Tone.mined
            let chosen = hovered == block.hash || selected == block.hash
            let box = chosen ? mark.rect.insetBy(dx: -1.5, dy: -1.5) : mark.rect
            // The marks are layers: see `PulseTrace`. The canvas draws only the ring of
            // the mark that the pointer or the list has.
            guard chosen else { continue }
            let shape = block.staked ? Path(ellipseIn: box) : Path(box)
            context.lit(shape, color, strength: 0.9)
            context.stroke(shape, with: .color(Tone.present), lineWidth: 1.5)
        }

        // The right edge: the present, or the newest block the node has validated.
        var spine = Path()
        spine.addRect(CGRect(x: right - 0.75, y: top, width: 1.5, height: blocksBottom - top))
        context.lit(spine, Tone.present, strength: live ? 0.7 : 0)
        context.name(live ? "now" : "newest", right, top - 8, .trailing, color: Tone.present)
    }

    /// A long wait: a stronger stretch of the line with its duration above it.
    private func waitSpan(
        _ context: GraphicsContext, from x0: CGFloat, to x1: CGFloat, seconds: Double, dashed: Bool
    ) {
        guard x1 > x0 else { return }
        let color = waitColor(seconds)
        context.rule(x0, axisY, x1, axisY, color, width: 2, dash: dashed ? [5, 4] : [])
        guard x1 - x0 > 56 else { return }
        context.draw(
            Text(Self.clock(seconds)).font(Tone.figureSmall).monospacedDigit().foregroundStyle(color),
            at: CGPoint(x: (x0 + x1) / 2, y: axisY - 11), anchor: .center)
    }

    // MARK: Drawing the waits

    /// The recorded waits as dots on a logarithmic time axis. Each dot is an equal share of
    /// the waits, so a person can count dots: the dots left of the mark are the share of
    /// waits that were shorter than the present one.
    private func drawWaits(_ context: GraphicsContext, _ size: CGSize, _ current: Double?) {
        let right = size.width - rightInset
        let base = waitsBase
        let high = waitsEnd(current)
        let position = { (seconds: Double) -> CGFloat in
            waitX(seconds, width: size.width, high: high)
        }

        context.plate("Waits", y: base - 12, fill: nil)
        context.plate("Time", y: rulerTop + 5, fill: nil)

        // The scale is logarithmic. The curve shows it: the height of the curve is the time
        // on this axis, on an even scale. It is flat where the axis gives much room to a
        // short time, and it goes up where the axis puts much time in a small room. It ends
        // at the present of the upper part, and so the two parts are one instrument.
        let shapes = waitShapes(width: size.width, current: current)
        let curve = shapes.curve
        var area = curve
        area.addLine(to: CGPoint(x: right, y: base))
        area.addLine(to: CGPoint(x: gutter, y: base))
        area.closeSubpath()
        context.fill(
            area,
            with: .linearGradient(
                Gradient(colors: [Tone.present.opacity(0), Tone.present.opacity(0.07)]),
                startPoint: CGPoint(x: gutter, y: base), endPoint: CGPoint(x: right, y: base)))
        context.stroke(curve, with: .color(Tone.present.opacity(0.5)), lineWidth: 1)

        // The line of the waits and its times.
        context.rule(gutter, base, right, base, Tone.axis)
        let ticks: [(Double, String)] = [
            (10, "10 s"), (30, "30 s"), (60, "1 min"), (120, "2 min"), (300, "5 min"), (600, "10 min"),
            (1200, "20 min"), (3600, "1 h"),
        ]
        for (seconds, name) in ticks where seconds <= high {
            let tx = position(seconds)
            context.rule(tx, base, tx, base + 4, Tone.axis)
            context.name(name, tx, base + 13)
        }

        // A ruler of equal steps of time. The steps get closer together to the right, and
        // then the ruler changes to a longer step.
        let steps: [(from: Double, to: Double, step: Double, name: String)] = [
            (10, 300, 10, "10 s"), (300, 1800, 60, "1 min"), (1800, 36_000, 600, "10 min"),
        ]
        for part in steps where part.from < high {
            let end = min(part.to, high)
            var marks = Path()
            var seconds = part.from
            while seconds <= end {
                let mx = position(seconds)
                marks.move(to: CGPoint(x: mx, y: rulerTop))
                marks.addLine(to: CGPoint(x: mx, y: rulerTop + (seconds == part.from ? 11 : 6)))
                seconds += part.step
            }
            context.stroke(marks, with: .color(Tone.axis), lineWidth: 1)
            if position(end) - position(part.from) > 40 {
                context.name("step \(part.name)", position(part.from) + 5, rulerTop + 17, .leading)
            }
        }

        // One hundred dots at most. Each dot is the wait at the middle of an equal share of
        // the record. The dots that the present wait has passed have light.
        if history?.waits.isEmpty == false {
            context.fill(shapes.ahead, with: .color(Tone.axis))
            context.lit(shapes.passed, Tone.record, strength: 0.4)
        } else if live {
            context.name("reading the record", (gutter + right) / 2, base - 12)
        }
    }

    /// The shapes of the lower part that get light from the front: the time curve, the dots
    /// that the present wait has passed, and the dots that it has not.
    private func waitShapes(width: CGFloat, current: Double?) -> (curve: Path, passed: Path, ahead: Path) {
        let right = width - rightInset
        let base = waitsBase
        let dot: CGFloat = 5
        let high = waitsEnd(current)
        var curve = Path()
        var cx = gutter
        while cx <= right {
            let share = Double((cx - gutter) / (right - gutter))
            let seconds = 10 * exp(share * log(high / 10))
            let point = CGPoint(x: cx, y: base - CGFloat(seconds / high) * (base - blocksBottom))
            if cx == gutter { curve.move(to: point) } else { curve.addLine(to: point) }
            cx += 2
        }
        curve.addLine(to: CGPoint(x: right, y: blocksBottom))

        var passed = Path()
        var ahead = Path()
        if let waits = history?.waits, !waits.isEmpty {
            let count = min(100, waits.count)
            var stacks: [Int: Int] = [:]
            for index in 0..<count {
                let rank = Int((Double(index) + 0.5) / Double(count) * Double(waits.count))
                let value = Double(waits[min(rank, waits.count - 1)])
                let bin = Int((waitX(value, width: width, high: high) - gutter) / (dot + 1.5))
                let level = stacks[bin, default: 0]
                stacks[bin] = level + 1
                let center = CGPoint(
                    x: gutter + (CGFloat(bin) + 0.5) * (dot + 1.5),
                    y: base - 3 - dot / 2 - CGFloat(level) * (dot + 1.5))
                let box = CGRect(x: center.x - dot / 2, y: center.y - dot / 2, width: dot, height: dot)
                if current.map({ value < $0 }) ?? true {
                    passed.addEllipse(in: box)
                } else {
                    ahead.addEllipse(in: box)
                }
            }
        }
        return (curve, passed, ahead)
    }

    /// The present wait as a mark that moves along the axis without steps. With a new block
    /// it starts again at the left with no movement back.
    private func presentMark(_ current: Double, width: CGFloat, beat: Int) -> some View {
        let right = width - rightInset
        let position = { (wait: Double) -> CGFloat in
            min(waitX(wait, width: width, high: waitsEnd(wait)), right)
        }
        return GlidingMark(
            blockTime: Double(blocks.first?.time ?? 0), position: position, expected: position(current),
            // The mark can be as much as the movement of two seconds from the last drawn second.
            tolerance: max(4, abs(position(current + 2) - position(current))),
            top: waitsBase - 38, bottom: waitsBase, text: Self.clock(current), red: isRecord(current),
            beat: beat, sweepFrom: right + 12, sweepTo: gutter - 12,
            key: "\(blocks.first?.hash ?? "")|\(Int(width))|\(Int(longestWait ?? 0))"
        )
        .allowsHitTesting(false)
    }
}

/// Light from the front of the trace on a set of shapes. The front goes from the right edge
/// of the area to the left edge in one second with each beat. A shape has light when the
/// front is at it, and the light goes down behind the front. The shapes do not move.
///
/// It is Core Animation: the system moves the front, and the app does no work for a frame.
struct LightTrace: NSViewRepresentable {
    struct Part {
        let path: CGPath
        /// True for a line, false for a filled shape.
        let stroke: Bool
        let color: CGColor
        let glow: CGColor
        /// The colour of the shape itself. With it, the view also draws the shape, and the
        /// shape gets its new place when the front goes across it. Nil: the view draws only
        /// the light, and a different view draws the shape.
        var base: CGColor?
        /// How strong the light around the shape itself is, from 0 to 1.
        var baseGlow: Float = 0
    }

    static let warm = CGColor(red: 0.84, green: 0.88, blue: 0.8, alpha: 1)
    static let pale = CGColor(red: 0.78, green: 0.9, blue: 0.88, alpha: 1)
    static let blue = CGColor(red: 0.3, green: 0.74, blue: 0.7, alpha: 1)

    let parts: [Part]
    let area: CGRect
    /// Changes with each update. A new value starts one pass of the front.
    let beat: Int
    /// How strong the light is, from 0 to 1.
    var strength: Float = 1

    final class LightView: NSView {
        let light = CALayer()
        let mask = CAGradientLayer()
        var shapes: [CAShapeLayer] = []
        /// The shapes ahead of the front, at the places of the beat before.
        let before = CALayer()
        let beforeMask = CALayer()
        var beforeShapes: [CAShapeLayer] = []
        /// The shapes behind the front, at their new places.
        let after = CALayer()
        let afterMask = CALayer()
        var afterShapes: [CAShapeLayer] = []
        var beat: Int?
        override var isFlipped: Bool { true }

        override init(frame: NSRect) {
            super.init(frame: frame)
            wantsLayer = true
            for (group, mask) in [(before, beforeMask), (after, afterMask)] {
                mask.backgroundColor = CGColor(gray: 0, alpha: 1)
                group.mask = mask
                layer?.addSublayer(group)
            }
            beforeMask.anchorPoint = CGPoint(x: 1, y: 0)
            afterMask.anchorPoint = CGPoint(x: 0, y: 0)
            mask.colors = [
                CGColor(gray: 0, alpha: 0), CGColor(gray: 0, alpha: 1), CGColor(gray: 0, alpha: 0.35),
                CGColor(gray: 0, alpha: 0),
            ]
            mask.locations = [0, 0.08, 0.4, 1]
            mask.startPoint = CGPoint(x: 0, y: 0.5)
            mask.endPoint = CGPoint(x: 1, y: 0.5)
            mask.anchorPoint = CGPoint(x: 0.08, y: 0)
            mask.opacity = 0
            light.mask = mask
            layer?.addSublayer(light)
        }

        required init?(coder: NSCoder) { nil }
    }

    func makeNSView(context: Context) -> LightView { LightView() }

    func updateNSView(_ view: LightView, context: Context) {
        let newBeat = view.beat != beat
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        view.light.opacity = strength
        while view.shapes.count < parts.count {
            let shape = CAShapeLayer()
            shape.shadowOffset = .zero
            shape.shadowOpacity = 1
            view.light.addSublayer(shape)
            view.shapes.append(shape)
            for (group, list) in [(view.before, \LightView.beforeShapes), (view.after, \LightView.afterShapes)] {
                let own = CAShapeLayer()
                own.shadowOffset = .zero
                own.shadowRadius = 4.5
                own.lineWidth = 1
                group.addSublayer(own)
                view[keyPath: list].append(own)
            }
        }
        for (index, part) in parts.enumerated() {
            let before = view.beforeShapes[index]
            let after = view.afterShapes[index]
            // The shapes of the beat before stay ahead of the front.
            if newBeat { before.path = after.path ?? part.path }
            after.path = part.path
            for own in [before, after] {
                own.isHidden = part.base == nil
                own.fillColor = part.stroke ? nil : part.base
                own.strokeColor = part.stroke ? part.base : nil
                own.shadowColor = part.glow
                own.shadowOpacity = part.baseGlow
            }
        }
        for mask in [view.beforeMask, view.afterMask] {
            mask.bounds = CGRect(x: 0, y: 0, width: area.width + 60, height: area.height + 24)
            mask.position = CGPoint(x: area.minX - 12, y: area.minY - 12)
        }
        for (shape, part) in zip(view.shapes, parts) {
            shape.path = part.path
            shape.fillColor = part.stroke ? nil : part.color
            shape.strokeColor = part.stroke ? part.color : nil
            shape.lineWidth = 1.5
            shape.shadowColor = part.glow
            shape.shadowRadius = part.stroke ? 5 : 6
        }
        let end = area.minX - 12
        view.mask.bounds = CGRect(x: 0, y: 0, width: 190, height: area.height + 24)
        view.mask.position = CGPoint(x: end, y: area.minY - 12)
        CATransaction.commit()

        guard newBeat else { return }
        view.beat = beat
        // A person who asked the system for less movement gets no front.
        let still = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion || area.width <= 0
        view.mask.opacity = still ? 0 : 1
        for mask in [view.mask as CALayer, view.beforeMask, view.afterMask] {
            mask.removeAnimation(forKey: "front")
            guard !still else { continue }
            let move = CABasicAnimation(keyPath: "position.x")
            move.fromValue = area.maxX + 12
            move.toValue = end
            move.duration = 1
            move.timingFunction = CAMediaTimingFunction(name: .linear)
            mask.add(move, forKey: "front")
        }
    }
}

/// The marks of the blocks, with the front of the trace.
///
/// With each beat a front goes from the right edge of the area to the left edge in one
/// second, at a constant speed. Ahead of the front the marks are where they were. Behind
/// the front they are at their new places, so the picture updates along the front, as the
/// line of a monitor does. At the front the centre line and the marks have a strong light,
/// which goes down behind the front as the light of a phosphor does. The front does not
/// fade: it crosses the full width.
///
/// All of it is Core Animation: the system moves the front at the rate of the display, and
/// the app does no work for a frame.
private struct PulseTrace: NSViewRepresentable {
    let staked: [CGRect]
    let mined: [CGRect]
    /// The centre line between the staked and the mined marks.
    let lineY: CGFloat
    /// The area that the front crosses, in the coordinates of the view.
    let area: CGRect
    /// Changes with each update. A new value starts one pass of the front.
    let beat: Int
    /// False when the node is not at the tip: the marks then get their places at once.
    let moves: Bool

    // A mark is a pale core, and its colour is in the light around it, as on film.
    private static let stakedColor = CGColor(red: 0.72, green: 0.68, blue: 0.8, alpha: 1)
    private static let minedColor = CGColor(red: 0.74, green: 0.67, blue: 0.44, alpha: 1)
    private static let stakedGlow = CGColor(red: 0.56, green: 0.3, blue: 0.86, alpha: 1)
    private static let minedGlow = CGColor(red: 1, green: 0.5, blue: 0.12, alpha: 1)
    private static let stakedLight = CGColor(red: 0.88, green: 0.87, blue: 0.9, alpha: 1)
    private static let minedLight = CGColor(red: 0.9, green: 0.86, blue: 0.68, alpha: 1)
    private static let lineLight = CGColor(red: 0.84, green: 0.88, blue: 0.8, alpha: 1)
    /// The length of the light behind the front.
    private static let tail: CGFloat = 190

    final class TraceView: NSView {
        /// The marks ahead of the front, at the places of the beat before.
        let before = CALayer()
        let beforeStaked = CAShapeLayer()
        let beforeMined = CAShapeLayer()
        let beforeMask = CALayer()
        /// The marks behind the front, at their new places.
        let after = CALayer()
        let afterStaked = CAShapeLayer()
        let afterMined = CAShapeLayer()
        let afterMask = CALayer()
        /// The light at the front.
        let light = CALayer()
        let lightStaked = CAShapeLayer()
        let lightMined = CAShapeLayer()
        let lightLine = CAShapeLayer()
        let lightMask = CAGradientLayer()
        var beat: Int?
        override var isFlipped: Bool { true }

        override init(frame: NSRect) {
            super.init(frame: frame)
            wantsLayer = true
            for (group, mask, shapes) in [
                (before, beforeMask as CALayer, [beforeStaked, beforeMined]),
                (after, afterMask as CALayer, [afterStaked, afterMined]),
                (light, lightMask as CALayer, [lightLine, lightStaked, lightMined]),
            ] {
                shapes.forEach(group.addSublayer)
                group.mask = mask
                layer?.addSublayer(group)
            }
            for mask in [beforeMask, afterMask] { mask.backgroundColor = .black }
            // The marks, with the soft light that each mark of the display has.
            for (shape, color, glow) in [
                (beforeStaked, PulseTrace.stakedColor, PulseTrace.stakedGlow),
                (afterStaked, PulseTrace.stakedColor, PulseTrace.stakedGlow),
                (beforeMined, PulseTrace.minedColor, PulseTrace.minedGlow),
                (afterMined, PulseTrace.minedColor, PulseTrace.minedGlow),
            ] {
                shape.fillColor = color
                shape.shadowColor = glow
                shape.shadowOpacity = 0.9
                shape.shadowRadius = 4.5
                shape.shadowOffset = .zero
            }
            // The light at the front: the marks and the line, lighter, with a wider light.
            for (shape, color, glow) in [
                (lightStaked, PulseTrace.stakedLight, PulseTrace.stakedGlow),
                (lightMined, PulseTrace.minedLight, PulseTrace.minedGlow),
            ] {
                shape.fillColor = color
                shape.shadowColor = glow
                shape.shadowOpacity = 1
                shape.shadowRadius = 7
                shape.shadowOffset = .zero
            }
            lightLine.fillColor = nil
            lightLine.strokeColor = PulseTrace.lineLight
            lightLine.lineWidth = 1.5
            lightLine.shadowColor = PulseTrace.lineLight
            lightLine.shadowOpacity = 0.9
            lightLine.shadowRadius = 5
            lightLine.shadowOffset = .zero
            // The front is at the left of the band. The light is full at the front and goes
            // down to the right, behind the front.
            lightMask.colors = [
                CGColor(gray: 0, alpha: 0), CGColor(gray: 0, alpha: 1), CGColor(gray: 0, alpha: 0.35),
                CGColor(gray: 0, alpha: 0),
            ]
            lightMask.locations = [0, 0.08, 0.4, 1]
            lightMask.startPoint = CGPoint(x: 0, y: 0.5)
            lightMask.endPoint = CGPoint(x: 1, y: 0.5)
            lightMask.anchorPoint = CGPoint(x: 0.08, y: 0)
            beforeMask.anchorPoint = CGPoint(x: 1, y: 0)
            afterMask.anchorPoint = CGPoint(x: 0, y: 0)
        }

        required init?(coder: NSCoder) { nil }
    }

    func makeNSView(context: Context) -> TraceView { TraceView() }

    func updateNSView(_ view: TraceView, context: Context) {
        let stakedPath = CGMutablePath()
        staked.forEach { stakedPath.addEllipse(in: $0) }
        let minedPath = CGMutablePath()
        mined.forEach { minedPath.addRect($0) }
        let linePath = CGMutablePath()
        linePath.move(to: CGPoint(x: area.minX, y: lineY))
        linePath.addLine(to: CGPoint(x: area.maxX, y: lineY))

        let newBeat = view.beat != beat
        // The places where the front starts and stops: a little outside the area.
        let start = area.maxX + 12
        let end = area.minX - 12
        let tall = area.height + 24
        let top = area.minY - 12

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        if newBeat {
            // The marks of the beat before stay ahead of the front.
            view.beforeStaked.path = view.afterStaked.path ?? stakedPath
            view.beforeMined.path = view.afterMined.path ?? minedPath
        }
        view.afterStaked.path = stakedPath
        view.afterMined.path = minedPath
        view.lightStaked.path = stakedPath
        view.lightMined.path = minedPath
        view.lightLine.path = linePath
        let wide = area.width + 60
        view.beforeMask.bounds = CGRect(x: 0, y: 0, width: wide, height: tall)
        view.afterMask.bounds = CGRect(x: 0, y: 0, width: wide, height: tall)
        view.lightMask.bounds = CGRect(x: 0, y: 0, width: Self.tail, height: tall)
        // With no movement, the front is at its end: all marks are at their new places.
        for mask in [view.beforeMask, view.afterMask, view.lightMask as CALayer] {
            mask.position = CGPoint(x: end, y: top)
        }
        CATransaction.commit()

        guard newBeat else { return }
        view.beat = beat
        // A person who asked the system for less movement gets no front.
        let still = !moves || NSWorkspace.shared.accessibilityDisplayShouldReduceMotion || area.width <= 0
        view.lightMask.opacity = still ? 0 : 1
        for mask in [view.beforeMask, view.afterMask, view.lightMask as CALayer] {
            mask.removeAnimation(forKey: "front")
            guard !still else { continue }
            let move = CABasicAnimation(keyPath: "position.x")
            move.fromValue = start
            move.toValue = end
            // One second: the front gets to the left edge when the next front starts.
            move.duration = 1
            move.timingFunction = CAMediaTimingFunction(name: .linear)
            mask.add(move, forKey: "front")
        }
    }
}

/// A vertical mark with the present wait as a time, which moves along the axis of the waits.
///
/// It is a Core Animation layer with one animation for the full wait: the app calculates
/// the path of the mark one time for each block, and the system moves the layer at the rate
/// of the display. The app does no work for a frame. A faster timeline or a SwiftUI
/// animation does the work in the app for each frame, and that used a quarter of one
/// processor core or more on this page.
private struct GlidingMark: NSViewRepresentable {
    /// The time of the newest block. The wait is the time since then.
    let blockTime: Double
    /// Where a wait of a number of seconds is on the axis.
    let position: (Double) -> CGFloat
    /// Where the mark must be now. The view starts the animation again if the layer is not
    /// there, such as after the computer was asleep.
    let expected: CGFloat
    let tolerance: CGFloat
    let top: CGFloat
    let bottom: CGFloat
    /// The wait as a time, written above the mark.
    let text: String
    let red: Bool
    /// Changes with each update. The front of the trace then crosses from `sweepFrom` to
    /// `sweepTo` in one second, and the mark has more light when the front is at it.
    let beat: Int
    let sweepFrom: CGFloat
    let sweepTo: CGFloat
    /// Changes when the path changes: a new block, a new width, or a new longest wait.
    let key: String

    /// The animation has the path of the mark for this time.
    private static let covered: Double = 3600
    private static let steps = 720

    final class MarkView: NSView {
        let line = CALayer()
        let label = CATextLayer()
        var key: String?
        var beat: Int?
        override var isFlipped: Bool { true }

        override init(frame: NSRect) {
            super.init(frame: frame)
            wantsLayer = true
            label.fontSize = 13
            label.font = NSFont(name: "Futura-Medium", size: 13) ?? NSFont.systemFont(ofSize: 13, weight: .medium)
            label.alignmentMode = .center
            label.bounds = CGRect(x: 0, y: 0, width: 64, height: 17)
            for layer in [line, label] {
                layer.shadowOpacity = 0.9
                layer.shadowRadius = 5
                layer.shadowOffset = .zero
            }
            layer?.addSublayer(line)
            layer?.addSublayer(label)
        }

        required init?(coder: NSCoder) { nil }
    }

    func makeNSView(context: Context) -> MarkView { MarkView() }

    func updateNSView(_ view: MarkView, context: Context) {
        let color = red
            ? CGColor(red: 0.82, green: 0.33, blue: 0.24, alpha: 1) : CGColor(red: 0.8, green: 0.84, blue: 0.77, alpha: 1)
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        view.line.backgroundColor = color
        view.line.shadowColor = color
        view.label.foregroundColor = color
        view.label.shadowColor = color
        view.label.string = text
        view.label.contentsScale = view.window?.backingScaleFactor ?? 2
        view.line.bounds = CGRect(x: 0, y: 0, width: 1.5, height: max(bottom - top, 0))
        let shown = view.line.presentation()?.position.x
        let moved = shown.map { abs($0 - expected) > tolerance } ?? true
        if view.key != key || moved {
            view.key = key
            start(view)
        }
        CATransaction.commit()
        if view.beat != beat {
            view.beat = beat
            flare(view)
        }
    }

    /// More light on the mark at the moment when the front of the trace is at it, which
    /// then goes down as the light of a phosphor does.
    ///
    /// The front crosses from `sweepFrom` to `sweepTo` in one second. The flare starts at
    /// the time when the front is at the mark, and it is a short animation of its own, so
    /// that the flare of one beat does not stop the flare of the beat before.
    private func flare(_ view: MarkView) {
        guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion, sweepFrom > sweepTo else { return }
        let reach = Double(min(max((sweepFrom - expected) / (sweepFrom - sweepTo), 0), 1))
        let begin = CACurrentMediaTime() + reach
        for layer in [view.line as CALayer, view.label as CALayer] {
            let glow = CAKeyframeAnimation(keyPath: "shadowRadius")
            glow.values = [5, 9, 5]
            glow.keyTimes = [0, 0.1, 1]
            glow.duration = 0.4
            glow.beginTime = begin
            layer.add(glow, forKey: "flare-\(beat)")
        }
        let wide = CAKeyframeAnimation(keyPath: "transform.scale.x")
        wide.values = [1, 1.6, 1]
        wide.keyTimes = [0, 0.1, 1]
        wide.duration = 0.4
        wide.beginTime = begin
        view.line.add(wide, forKey: "flareWidth-\(beat)")
    }

    /// Gives the layers the path of the mark from now, as one animation.
    private func start(_ view: MarkView) {
        let wait = max(Date().timeIntervalSince1970 - blockTime, 0)
        var values: [CGFloat] = []
        var times: [NSNumber] = []
        for step in 0...Self.steps {
            // More points at the start, where the mark moves fast on the logarithmic axis.
            let part = Double(step) / Double(Self.steps)
            values.append(position(wait + Self.covered * part * part))
            times.append(NSNumber(value: part * part))
        }
        let path = CAKeyframeAnimation(keyPath: "position.x")
        path.values = values
        path.keyTimes = times
        path.duration = Self.covered
        path.calculationMode = .linear
        path.fillMode = .forwards
        path.isRemovedOnCompletion = false
        let middle = (top + bottom) / 2
        for (layer, y) in [(view.line as CALayer, middle), (view.label as CALayer, top - 11)] {
            layer.position = CGPoint(x: values[0], y: y)
            layer.removeAnimation(forKey: "path")
            layer.add(path, forKey: "path")
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
            context.fill(Path(CGRect(origin: .zero, size: size)), with: .color(.white.opacity(0.08)))
            if let usual {
                context.fill(
                    Path(
                        CGRect(
                            x: position(usual.0), y: 0, width: position(usual.1) - position(usual.0),
                            height: size.height)),
                    with: .color(.white.opacity(0.2)))
            }
            context.fill(
                Path(CGRect(x: 0, y: size.height / 2 - 1.5, width: position(value), height: 3)),
                with: .color(.white.opacity(0.92)))
            context.fill(
                Path(CGRect(x: position(target) - 1, y: -2, width: 2, height: size.height + 4)),
                with: .color(.white))
        }
        .accessibilityLabel(
            "\(value) blocks. Target \(target)."
                + (usual.map { " Usual \($0.0) to \($0.1)." } ?? ""))
        .help("The bar is the number of blocks. The tick is the target. The band is the usual range.")
    }
}
