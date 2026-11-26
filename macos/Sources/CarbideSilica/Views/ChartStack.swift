import CarbideCore
import SwiftUI

/// The five measures of a range as short panels on one time axis, with one cursor through
/// all of them. `macos/design/charts-research.html` has the reasons for the form.
///
/// Each panel has a reference for what is usual: process limits for the block rate, the
/// limits of chance for the staked share, and Carbide's trailing MA100 for the two
/// difficulties. The longest waits of the range are marked through all panels, so one
/// event can be seen in each measure.
struct ChartStack: View {
    /// Oldest first.
    let samples: [ChartSample]
    let spanSeconds: UInt64
    let fee: FeePoolState
    /// The native currency's name.
    let coin: String
    let selected: String?
    let select: (String) -> Void
    let open: (String) -> Void

    @State private var model: StackModel?
    @State private var hovered: Int?
    /// The panels drawn one time with more light. The light layer shows parts of it.
    @State private var lit: CGImage?
    /// The same data with much more light: for the light of the pointer and of a click.
    @State private var hot: CGImage?
    /// The place of the last click on the panels, and a count that changes with each click.
    @State private var wave: (count: Int, x: CGFloat) = (0, 0)

    private let leftInset: CGFloat = 66
    private let rightInset: CGFloat = 22
    private let headerHeight: CGFloat = 22
    private let panelHeight: CGFloat = 84
    private let gap: CGFloat = 10
    private var plotHeight: CGFloat { CGFloat(StackModel.Panel.allCases.count) * (headerHeight + panelHeight + gap) + 18 }

    /// Changes when the model must be built again.
    private var key: String {
        "\(samples.count)|\(samples.first?.hash ?? "")|\(samples.last?.hash ?? "")|\(spanSeconds)|"
            + "\(fee.checkpoints.count)|\(fee.scale)|\(fee.currency)"
    }

    var body: some View {
        // One display panel holds the readout, the five panels and the waits.
        DisplayPanel(padding: 0) {
            VStack(alignment: .leading, spacing: 0) {
                readout
                    .padding(.horizontal, 18)
                    .padding(.top, 14)
                    .padding(.bottom, 10)
                PanelRule()
                stack
                    .padding(.top, 8)
                    .padding(.bottom, 4)
                if let model {
                    WaitsStrip(model: model, leftInset: leftInset, rightInset: rightInset, wait: shownWait)
                }
            }
        }
        .onChange(of: key, initial: true) {
            model = StackModel(samples: samples, spanSeconds: spanSeconds, fee: fee, coin: coin)
        }
    }

    /// The five panels on one time axis.
    private var stack: some View {
        GeometryReader { geometry in
            let frame = plotFrame(geometry.size.width)
            Canvas { context, size in
                if let model { draw(context, size, model) }
            }
            .contentShape(Rectangle())
            .onContinuousHover { phase in
                switch phase {
                case .active(let point): hovered = model?.nearest(x: point.x, in: frame)
                case .ended: hovered = nil
                }
            }
            .onTapGesture(coordinateSpace: .local) { point in
                if let model, let index = model.nearest(x: point.x, in: frame), samples.indices.contains(index) {
                    select(samples[index].hash)
                    // A wave of light goes out from the selected block in the two directions.
                    wave = (wave.count + 1, model.x(Double(samples[index].time), in: frame))
                }
            }
            // Light on the panels: a slow front from the right to the left, light near the
            // block under the pointer, and the wave of a click. The layer shows parts of a
            // picture of the panels that has more light. The system moves the light.
            .overlay {
                ChartLight(
                    picture: lit, hot: hot, left: frame.lowerBound, right: frame.upperBound,
                    pointer: hovered.flatMap { index in
                        samples.indices.contains(index) ? model?.x(Double(samples[index].time), in: frame) : nil
                    },
                    wave: wave.count, waveAt: wave.x
                )
                .allowsHitTesting(false)
            }
            .task(id: "\(key)|\(Int(geometry.size.width))|\(model == nil)") {
                lit = litPicture(geometry.size, strong: false)
                hot = litPicture(geometry.size, strong: true)
            }
        }
        .frame(height: plotHeight)
        .accessibilityLabel("Block rate, mining difficulty, staking difficulty, staked share and fee pool on one time axis")
    }

    private func plotFrame(_ width: CGFloat) -> ClosedRange<CGFloat> { leftInset...(width - rightInset) }

    /// The data of the panels with more light, as a picture with nothing behind it: the lines
    /// and the dots only. It is made one time for each range and width.
    @MainActor
    private func litPicture(_ size: CGSize, strong: Bool) -> CGImage? {
        guard let model, size.width > 0 else { return nil }
        let renderer = ImageRenderer(
            content: Canvas { context, size in
                draw(context, size, model, cursors: false, marksOnly: true)
            }
            .frame(width: size.width, height: size.height)
            .brightness(strong ? 0.6 : 0.22)
            // The light around a lit line, as on a phosphor screen. The strong picture has
            // two lights: one near the line and one wide.
            .shadow(color: Tone.present.opacity(strong ? 0.95 : 0.55), radius: strong ? 3 : 2.5)
            .shadow(color: Tone.present.opacity(strong ? 0.6 : 0), radius: strong ? 8 : 0)
            .environment(\.colorScheme, .dark))
        renderer.scale = NSScreen.main?.backingScaleFactor ?? 2
        return renderer.cgImage
    }

    /// The block under the pointer, or the selected block.
    private var shownIndex: Int? {
        if let hovered { return hovered }
        guard let selected else { return samples.isEmpty ? nil : samples.count - 1 }
        return model?.index[selected] ?? (samples.isEmpty ? nil : samples.count - 1)
    }

    /// The wait that ended with the shown block, when that block started a burst.
    private var shownWait: Double? {
        guard let index = shownIndex, samples.indices.contains(index), let interval = samples[index].interval,
            interval > StackModel.burstSeconds
        else { return nil }
        return Double(interval)
    }

    // MARK: Readout

    /// Each value at the block under the cursor, above the panels and not below the page.
    private var readout: some View {
        VStack(alignment: .leading, spacing: 7) {
            // The model is made a moment after the samples change. Until then it can be for a
            // shorter list: the figures show only for a block that the model has.
            if let model, let index = shownIndex, samples.indices.contains(index), model.has(index) {
                let sample = samples[index]
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Button("Block \(Format.integer(sample.height))") { open(sample.hash) }
                        .buttonStyle(.displayLink)
                        .font(Tone.figureSmall)
                        .help("Open this block")
                    Plate(ChartFormat.production(sample.production), fill: productionFill(sample.production))
                    Text(Format.date(sample.time)).foregroundStyle(Tone.value)
                    if let interval = sample.interval {
                        Text("· \(ChartFormat.signedSeconds(interval)) after the block before").foregroundStyle(Tone.label)
                    }
                    if hovered != nil {
                        Text("· click to select").foregroundStyle(Tone.label.opacity(0.7))
                    }
                    Spacer(minLength: 0)
                }
                HStack(alignment: .lastTextBaseline, spacing: 20) {
                    figure("Rate", model.rateText(at: index))
                    figure("Mining", model.miningAt[index].map { Format.difficulty($0) } ?? "–", tone: Tone.mined)
                    figure("Staking", model.stakingAt[index].map { Format.difficulty($0) } ?? "–", tone: Tone.staked)
                    figure("Staked share", sample.share100.map { Format.percent($0) } ?? "–")
                    figure("Fee pool", model.poolText(at: index))
                    Spacer(minLength: 0)
                }
            } else {
                Text(" ")
                Text(" ")
            }
        }
        .font(Tone.caption)
        .lineLimit(1)
        .monospacedDigit()
        .frame(minHeight: 40, alignment: .topLeading)
    }

    private func productionFill(_ production: BlockProduction) -> Color? {
        switch production {
        case .staked: return Tone.staked
        case .mined: return Tone.mined
        case .unknown: return nil
        }
    }

    /// A name and its figure at the shown block: "MINING 1.23T".
    private func figure(_ label: String, _ value: String, tone: Color = Tone.value) -> some View {
        HStack(alignment: .lastTextBaseline, spacing: 6) {
            NameText(label)
            Text(value).font(Tone.figureSmall).foregroundStyle(tone)
        }
    }

    // MARK: Drawing

    private func rect(_ panel: StackModel.Panel, _ size: CGSize) -> CGRect {
        let top = CGFloat(panel.rawValue) * (headerHeight + panelHeight + gap) + headerHeight
        return CGRect(x: leftInset, y: top, width: size.width - leftInset - rightInset, height: panelHeight)
    }

    /// `marksOnly` draws the data of the panels and nothing more: the lines and the dots, with
    /// no bands, no grid, no names and no cursor. The light layer uses that picture, so that
    /// light is on the chart elements only.
    private func draw(
        _ context: GraphicsContext, _ size: CGSize, _ model: StackModel, cursors: Bool = true, marksOnly: Bool = false
    ) {
        let frame = plotFrame(size.width)
        let x = { (time: Double) -> CGFloat in model.x(time, in: frame) }
        let bottom = rect(.pool, size).maxY

        // The longest waits of the range, through all panels.
        for wait in marksOnly ? [] : model.longWaits {
            let x0 = x(wait.start), x1 = max(x(wait.end), x0 + 1)
            context.fill(
                Path(CGRect(x: x0, y: headerHeight, width: x1 - x0, height: bottom - headerHeight)),
                with: .color(Tone.value.opacity(0.09)))
        }

        // Time grid and its labels.
        for tick in marksOnly ? [] : model.timeTicks {
            let tx = x(tick)
            context.rule(tx, headerHeight, tx, bottom, Tone.grid)
            context.rule(tx, bottom, tx, bottom + 3, Tone.axis)
            context.name(ChartFormat.axisTime(UInt64(tick), span: spanSeconds), tx, bottom + 11)
        }

        for panel in StackModel.Panel.allCases {
            let box = rect(panel, size)
            let range = model.range(panel)
            let y = { (value: Double) -> CGFloat in
                box.maxY - CGFloat((model.scaled(value, panel) - range.lowerBound) / (range.upperBound - range.lowerBound)) * box.height
            }
            if !marksOnly {
            // The name on a plate, and the answer, above the panel.
            let plateWidth = context.resolve(Text(panel.title.uppercased()).font(Tone.plate).tracking(0.8))
                .measure(in: CGSize(width: 400, height: 40)).width + 14
            context.plate(panel.title, x: leftInset, y: box.minY - 10, width: plateWidth, fill: plateFill(panel))
            context.draw(
                Text(model.answer(panel)).font(Tone.caption).foregroundStyle(model.signal(panel) ? Tone.alert : Tone.label),
                at: CGPoint(x: leftInset + plateWidth + 10, y: box.minY - 10), anchor: .leading)

            context.stroke(Path(box), with: .color(Tone.rule), lineWidth: 1)
            for tick in model.ticks(panel) {
                let ty = y(tick.value)
                guard ty >= box.minY - 0.5, ty <= box.maxY + 0.5 else { continue }
                context.rule(box.minX, ty, box.maxX, ty, Tone.grid)
                context.rule(box.minX - 3, ty, box.minX, ty, Tone.axis)
                context.name(tick.label, box.minX - 7, ty, .trailing)
            }
            }

            var inner = context
            inner.clip(to: Path(box))
            switch panel {
            case .rate: drawRate(inner, box, model, x, y, marksOnly)
            case .mining: drawDifficulty(inner, box, model.mining, model.miningAverage, Tone.mined, x, y)
            case .staking: drawDifficulty(inner, box, model.staking, model.stakingAverage, Tone.staked, x, y)
            case .share: drawShare(inner, box, model, x, y, marksOnly)
            case .pool: drawPool(inner, box, model, x, y, marksOnly)
            }
        }

        // The cursor: the selected block, and the block under the pointer.
        guard cursors, !marksOnly else { return }
        let selectedIndex = selected.flatMap { model.index[$0] }
        if let hovered, hovered != selectedIndex, samples.indices.contains(hovered) {
            cursor(context, size, model, at: hovered, strong: false)
        }
        if let index = selectedIndex ?? (hovered == nil ? samples.indices.last : nil) {
            cursor(context, size, model, at: index, strong: true)
        }
    }

    /// The colour of a panel's plate: the signal of its measure, or an outline.
    private func plateFill(_ panel: StackModel.Panel) -> Color? {
        switch panel {
        case .mining: return Tone.mined
        case .staking: return Tone.staked
        default: return nil
        }
    }

    /// The selected block is a line of the present with a little light. The block under the
    /// pointer is a quieter line.
    private func cursor(_ context: GraphicsContext, _ size: CGSize, _ model: StackModel, at index: Int, strong: Bool) {
        guard samples.indices.contains(index) else { return }
        let cx = model.x(Double(samples[index].time), in: plotFrame(size.width))
        let bottom = rect(.pool, size).maxY
        if strong {
            context.lit(
                Path(CGRect(x: cx - 0.75, y: headerHeight, width: 1.5, height: bottom - headerHeight)),
                Tone.present, strength: 0.6)
        } else {
            context.rule(cx, headerHeight, cx, bottom, Tone.present.opacity(0.45))
        }
    }

    private func line(_ points: [CGPoint?]) -> Path {
        Path { path in
            var open = false
            for point in points {
                guard let point else { open = false; continue }
                if open { path.addLine(to: point) } else { path.move(to: point) }
                open = true
            }
        }
    }

    private func drawRate(
        _ context: GraphicsContext, _ box: CGRect, _ model: StackModel, _ x: (Double) -> CGFloat, _ y: (Double) -> CGFloat,
        _ marksOnly: Bool = false
    ) {
        if !marksOnly, let limits = model.rateLimits {
            context.fill(
                Path(CGRect(x: box.minX, y: y(limits.high), width: box.width, height: y(limits.low) - y(limits.high))),
                with: .color(Tone.record.opacity(0.1)))
            context.rule(box.minX, y(limits.mean), box.maxX, y(limits.mean), Tone.record.opacity(0.6))
        }
        if !marksOnly {
            context.rule(
                box.minX, y(model.rateTarget), box.maxX, y(model.rateTarget), Tone.value.opacity(0.6), dash: [4, 3])
        }
        let points = model.buckets.map { CGPoint(x: x(($0.start + $0.end) / 2), y: y(Double($0.count))) }
        context.stroke(line(points), with: .color(Tone.value), lineWidth: 1)
        for (bucket, point) in zip(model.buckets, points) {
            let radius: CGFloat = bucket.flag == .none ? 2.2 : 3.5
            let color: Color = bucket.flag == .outside ? Tone.alert : bucket.flag == .run ? Tone.mined : Tone.value
            context.fill(
                Path(ellipseIn: CGRect(x: point.x - radius, y: point.y - radius, width: radius * 2, height: radius * 2)),
                with: .color(color))
        }
    }

    /// Dots for each block of the kind, or the range of each column when the dots would
    /// merge, with Carbide's trailing MA100.
    private func drawDifficulty(
        _ context: GraphicsContext, _ box: CGRect, _ points: [StackModel.Point], _ average: [StackModel.Point?],
        _ color: Color, _ x: (Double) -> CGFloat, _ y: (Double) -> CGFloat
    ) {
        if CGFloat(points.count) > box.width {
            context.fill(envelope(points, box, x, y), with: .color(color.opacity(0.45)))
        } else {
            for point in points {
                let center = CGPoint(x: x(point.time), y: y(point.value))
                context.fill(
                    Path(ellipseIn: CGRect(x: center.x - 1.4, y: center.y - 1.4, width: 2.8, height: 2.8)),
                    with: .color(color.opacity(0.8)))
            }
        }
        context.stroke(
            line(average.map { point in point.map { CGPoint(x: x($0.time), y: y($0.value)) } }),
            with: .color(Tone.value), lineWidth: 1)
    }

    /// The lowest and highest value of each two-point column, so that no peak is lost.
    private func envelope(
        _ points: [StackModel.Point], _ box: CGRect, _ x: (Double) -> CGFloat, _ y: (Double) -> CGFloat
    ) -> Path {
        var columns: [Int: (low: Double, high: Double)] = [:]
        for point in points {
            let column = Int((x(point.time) - box.minX) / 2)
            if let range = columns[column] {
                columns[column] = (min(range.low, point.value), max(range.high, point.value))
            } else {
                columns[column] = (point.value, point.value)
            }
        }
        return Path { path in
            for (column, range) in columns {
                let top = y(range.high), bottom = max(y(range.low), top + 1.5)
                path.addRect(CGRect(x: box.minX + CGFloat(column) * 2, y: top, width: 2, height: bottom - top))
            }
        }
    }

    private func drawShare(
        _ context: GraphicsContext, _ box: CGRect, _ model: StackModel, _ x: (Double) -> CGFloat, _ y: (Double) -> CGFloat,
        _ marksOnly: Bool = false
    ) {
        if !marksOnly {
            context.fill(
                Path(
                    CGRect(
                        x: box.minX, y: y(StackModel.chanceHigh), width: box.width,
                        height: y(StackModel.chanceLow) - y(StackModel.chanceHigh))),
                with: .color(Tone.record.opacity(0.1)))
            context.rule(box.minX, y(0.5), box.maxX, y(0.5), Tone.value.opacity(0.6), dash: [4, 3])
        }
        context.stroke(
            line(model.share1000.map { point in point.map { CGPoint(x: x($0.time), y: y($0.value)) } }),
            with: .color(Tone.record), lineWidth: 1)
        context.stroke(
            line(model.share100.map { point in point.map { CGPoint(x: x($0.time), y: y($0.value)) } }),
            with: .color(Tone.staked), lineWidth: 1)
    }

    private func drawPool(
        _ context: GraphicsContext, _ box: CGRect, _ model: StackModel, _ x: (Double) -> CGFloat, _ y: (Double) -> CGFloat,
        _ marksOnly: Bool = false
    ) {
        guard !model.pool.isEmpty else {
            if marksOnly { return }
            context.draw(
                Text(fee.loading ? "Reading fee pool checkpoints" : "No fee pool checkpoint is loaded for this range.")
                    .font(Tone.caption).foregroundStyle(Tone.label),
                at: CGPoint(x: box.midX, y: box.midY), anchor: .center)
            return
        }
        let points = model.pool.map { CGPoint(x: x($0.time), y: y($0.value)) }
        if !marksOnly, fee.scale != .fit, let first = points.first, let last = points.last {
            var area = line(points)
            area.addLine(to: CGPoint(x: last.x, y: box.maxY))
            area.addLine(to: CGPoint(x: first.x, y: box.maxY))
            area.closeSubpath()
            context.fill(area, with: .color(Tone.record.opacity(0.08)))
        }
        context.stroke(line(points), with: .color(Tone.record), lineWidth: 1)
        // A tick for each block where the pool rose. Known only between neighbouring blocks.
        for time in model.poolRises {
            let tx = x(time)
            context.rule(tx, box.minY, tx, box.minY + 6, Tone.value.opacity(0.75))
        }
        // The second currency of a PBaaS chain, on its own range.
        if !model.reserve.isEmpty, let top = model.reserve.map(\.value).max(), top > 0 {
            let reserve = model.reserve.map {
                CGPoint(x: x($0.time), y: box.maxY - CGFloat($0.value / (top * 1.08)) * box.height)
            }
            context.stroke(line(reserve), with: .color(Tone.good), lineWidth: 1)
        }
    }
}

// MARK: - Model

/// The figures of one range, worked out once for each load and not at each pointer move.
struct StackModel {
    enum Panel: Int, CaseIterable {
        case rate, mining, staking, share, pool

        var title: String {
            switch self {
            case .rate: "Block rate"
            case .mining: "Mining difficulty"
            case .staking: "Staking difficulty"
            case .share: "Staked share"
            case .pool: "Fee pool"
            }
        }
    }

    enum Flag { case none, outside, run }

    struct Point {
        let time: Double
        let value: Double
    }

    struct Bucket {
        let start: Double
        let end: Double
        let count: Int
        var flag = Flag.none
    }

    struct Tick {
        let value: Double
        let label: String
    }

    /// Three standard deviations of chance for the staked share of 100 blocks at 50%.
    static let chanceLow = 0.5 - 3 * 0.05
    static let chanceHigh = 0.5 + 3 * 0.05
    /// Blocks no more than this many seconds after the block before are one burst.
    static let burstSeconds: Int64 = 10

    let start: Double
    let end: Double
    let index: [String: Int]
    private let times: [Double]

    let bucketSeconds: Double
    let bucketName: String
    let rateTarget: Double
    private(set) var buckets: [Bucket] = []
    private(set) var rateLimits: (mean: Double, low: Double, high: Double)?

    let mining: [Point]
    let staking: [Point]
    let miningAverage: [Point?]
    let stakingAverage: [Point?]
    /// The value at the block or at the nearest block of the kind before it.
    let miningAt: [Double?]
    let stakingAt: [Double?]
    let share100: [Point?]
    let share1000: [Point?]

    let pool: [Point]
    let reserve: [Point]
    let poolRises: [Double]
    private let poolAt: [(value: Double, exact: Bool)?]
    private let poolScale: FeePoolState.Scale
    private let coin: String
    private let checkpointsRead: Int
    private let checkpointsWanted: Int

    let longWaits: [(start: Double, end: Double)]
    /// The waits between bursts of the range in seconds, shortest first.
    let waits: [Double]
    let timeTicks: [Double]

    init(samples: [ChartSample], spanSeconds: UInt64, fee: FeePoolState, coin: String) {
        self.coin = coin
        times = samples.map { Double($0.time) }
        start = times.first ?? 0
        end = max(times.last ?? 1, start + 1)
        index = Dictionary(samples.enumerated().map { ($1.hash, $0) }, uniquingKeysWith: { first, _ in first })

        // The block rate: complete periods that end at the newest block.
        switch spanSeconds {
        case ...7_200: (bucketSeconds, bucketName) = (600, "10 minutes")
        case ...21_600: (bucketSeconds, bucketName) = (1_800, "30 minutes")
        case ...86_400: (bucketSeconds, bucketName) = (3_600, "hour")
        default: (bucketSeconds, bucketName) = (21_600, "6 hours")
        }
        rateTarget = bucketSeconds / 60
        var periods: [Bucket] = []
        var periodEnd = end
        let sorted = times.sorted()
        while periodEnd - bucketSeconds >= start {
            let low = periodEnd - bucketSeconds
            let from = sorted.partitionIndex { $0 > low }
            let to = sorted.partitionIndex { $0 > periodEnd }
            periods.append(Bucket(start: low, end: periodEnd, count: to - from))
            periodEnd = low
        }
        periods.reverse()
        // A process behaviour chart: the mean, and limits of 2.66 times the mean moving range.
        if periods.count >= 10 {
            let counts = periods.map { Double($0.count) }
            let mean = counts.reduce(0, +) / Double(counts.count)
            let moving = zip(counts, counts.dropFirst()).map { abs($1 - $0) }
            let range = moving.reduce(0, +) / Double(moving.count)
            let low = max(mean - 2.66 * range, 0), high = mean + 2.66 * range
            rateLimits = (mean, low, high)
            // A run of seven periods on one side of the mean is a signal too.
            var run = 0
            var side = 0.0
            for position in periods.indices {
                let now = counts[position] > mean ? 1.0 : counts[position] < mean ? -1.0 : 0
                run = now != 0 && now == side ? run + 1 : 1
                side = now
                if run >= 7, now != 0 {
                    for back in (position - run + 1)...position { periods[back].flag = .run }
                }
            }
            for position in periods.indices where counts[position] > high || counts[position] < low {
                periods[position].flag = .outside
            }
        }
        buckets = periods

        mining = samples.compactMap { sample in sample.mining.map { Point(time: Double(sample.time), value: $0) } }
        staking = samples.compactMap { sample in sample.staking.map { Point(time: Double(sample.time), value: $0) } }
        miningAverage = samples.map { sample in
            sample.miningAverage.map { Point(time: Double(sample.time), value: $0.value) }
        }
        stakingAverage = samples.map { sample in
            sample.stakingAverage.map { Point(time: Double(sample.time), value: $0.value) }
        }
        var lastMining: Double?
        var lastStaking: Double?
        var miningCarried: [Double?] = []
        var stakingCarried: [Double?] = []
        for sample in samples {
            if let value = sample.mining { lastMining = value }
            if let value = sample.staking { lastStaking = value }
            miningCarried.append(lastMining)
            stakingCarried.append(lastStaking)
        }
        miningAt = miningCarried
        stakingAt = stakingCarried
        share100 = samples.map { sample in sample.share100.map { Point(time: Double(sample.time), value: $0) } }
        share1000 = samples.map { sample in sample.share1000.map { Point(time: Double(sample.time), value: $0) } }

        // The fee pool: the checkpoints that are loaded, in block order.
        poolScale = fee.scale
        var native: [Point] = []
        var second: [Point] = []
        var rises: [Double] = []
        var carried: [(value: Double, exact: Bool)?] = []
        var before: (height: UInt64, value: Double)?
        var last: Double?
        for sample in samples {
            if let checkpoint = fee.checkpoints[sample.hash], checkpoint.known, let sats = checkpoint.nativeSats {
                let value = Double(sats) / 100_000_000
                if fee.currency != .reserve { native.append(Point(time: Double(sample.time), value: value)) }
                if let before, before.height + 1 == sample.height, value > before.value { rises.append(Double(sample.time)) }
                before = (sample.height, value)
                last = value
                carried.append((value, true))
                if fee.hasReserve, fee.currency != .native, let reserveSats = checkpoint.reserveSats {
                    second.append(Point(time: Double(sample.time), value: Double(reserveSats) / 100_000_000))
                }
            } else {
                carried.append(last.map { ($0, false) })
            }
        }
        pool = native
        reserve = second
        poolRises = fee.currency == .reserve ? [] : rises
        poolAt = carried
        checkpointsRead = samples.filter { fee.checkpoints[$0.hash] != nil }.count
        checkpointsWanted = max(fee.requested, checkpointsRead)

        // The longest gaps of the range: the longest hundredth, and 12 at most, so that
        // a long range does not fill with bands.
        let gaps = samples.compactMap { sample in sample.interval.flatMap { $0 > 0 ? Double($0) : nil } }
        if gaps.count >= 200 {
            let ranked = gaps.sorted()
            let limit = max(ranked[min(Int(Double(gaps.count) * 0.99), gaps.count - 1)], ranked[max(gaps.count - 12, 0)])
            longWaits = samples.compactMap { sample in
                guard let interval = sample.interval, Double(interval) >= limit else { return nil }
                return (Double(sample.time) - Double(interval), Double(sample.time))
            }
        } else {
            longWaits = []
        }
        waits = samples.compactMap { sample in
            sample.interval.flatMap { $0 > StackModel.burstSeconds ? Double($0) : nil }
        }.sorted()

        // Six to eight time labels, on round times.
        let length = end - start
        let step = [600.0, 1_800, 3_600, 10_800, 21_600, 43_200, 86_400].first { length / $0 <= 9 } ?? 86_400
        var ticks: [Double] = []
        var tick = (start / step).rounded(.up) * step
        while tick <= end {
            ticks.append(tick)
            tick += step
        }
        timeTicks = ticks
    }

    // MARK: Positions

    func x(_ time: Double, in frame: ClosedRange<CGFloat>) -> CGFloat {
        frame.lowerBound + CGFloat((time - start) / (end - start)) * (frame.upperBound - frame.lowerBound)
    }

    /// The block nearest in time to a horizontal position.
    func nearest(x: CGFloat, in frame: ClosedRange<CGFloat>) -> Int? {
        guard !times.isEmpty else { return nil }
        let share = Double((x - frame.lowerBound) / (frame.upperBound - frame.lowerBound))
        let time = start + min(max(share, 0), 1) * (end - start)
        var best = 0
        var distance = Double.infinity
        for (position, value) in times.enumerated() where abs(value - time) < distance {
            distance = abs(value - time)
            best = position
        }
        return best
    }

    /// The fee pool's log-like scale keeps zero defined. The other panels are linear.
    func scaled(_ value: Double, _ panel: Panel) -> Double {
        panel == .pool && poolScale == .log ? FeePoolPlotModel.transform(value) : value
    }

    func range(_ panel: Panel) -> ClosedRange<Double> {
        func fitted(_ values: [Double], floor: Double = 0) -> ClosedRange<Double> {
            guard let low = values.min(), let high = values.max(), high > low else {
                let value = values.first ?? 1
                return max(value * 0.9, floor)...(value * 1.1 + 1e-9)
            }
            let pad = (high - low) * 0.08
            return max(low - pad, floor)...(high + pad)
        }
        switch panel {
        case .rate:
            var values = buckets.map { Double($0.count) } + [rateTarget]
            if let rateLimits { values += [rateLimits.low, rateLimits.high] }
            return fitted(values)
        case .mining:
            return fitted(mining.map(\.value) + miningAverage.compactMap { $0?.value })
        case .staking:
            return fitted(staking.map(\.value) + stakingAverage.compactMap { $0?.value })
        case .share:
            let values = (share100 + share1000).compactMap { $0?.value }
            return min(0.3, (values.min() ?? 0.3) - 0.03)...max(0.7, (values.max() ?? 0.7) + 0.03)
        case .pool:
            let values = pool.map(\.value)
            switch poolScale {
            case .zero: return 0...max((values.max() ?? 1) * 1.08, 1e-8)
            case .fit: return fitted(values)
            case .log: return 0...max(FeePoolPlotModel.transform((values.max() ?? 1) * 1.08), 1)
            }
        }
    }

    func ticks(_ panel: Panel) -> [Tick] {
        let range = range(panel)
        if panel == .pool, poolScale == .log {
            return stride(from: 0.0, through: range.upperBound, by: 1).map {
                Tick(value: $0 <= 1 ? $0 * FeePoolPlotModel.knee : FeePoolPlotModel.knee * pow(10, $0 - 1), label: FeePoolPlotModel.logLabel($0))
            }
        }
        if panel == .share {
            return stride(from: 0.0, through: 1.0, by: 0.1)
                .filter { $0 > range.lowerBound + 0.02 && $0 < range.upperBound - 0.02 }
                .map { Tick(value: $0, label: "\(Int(($0 * 100).rounded()))%") }
        }
        let values = PlotScale.niceTicks(values: [range.lowerBound, range.upperBound], fromZero: false, count: 5)
        let step = values.count > 1 ? values[1] - values[0] : 1
        return values.filter { $0 >= range.lowerBound && $0 <= range.upperBound }.map { value in
            switch panel {
            case .rate: Tick(value: value, label: "\(Int(value.rounded()))")
            case .mining, .staking: Tick(value: value, label: Format.difficulty(value))
            case .share: Tick(value: value, label: "\(Int((value * 100).rounded()))%")
            case .pool: Tick(value: value, label: FeePoolPlotModel.linearLabel(value, step: step))
            }
        }
    }

    // MARK: Answers

    /// True when the panel has a signal that a rule found.
    func signal(_ panel: Panel) -> Bool {
        switch panel {
        case .rate: buckets.contains { $0.flag == .outside }
        case .share: share100.last.flatMap { $0 }.map { $0.value < Self.chanceLow || $0.value > Self.chanceHigh } ?? false
        default: false
        }
    }

    /// The value now, the change in the range, and a word for what is usual.
    func answer(_ panel: Panel) -> String {
        switch panel {
        case .rate:
            guard !buckets.isEmpty else { return "The range is shorter than one period of \(bucketName)." }
            let mean = Double(buckets.map(\.count).reduce(0, +)) / Double(buckets.count)
            var text = "\(mean.formatted(.number.precision(.fractionLength(0...1)))) blocks each \(bucketName), target \(Int(rateTarget))"
            let outside = buckets.filter { $0.flag == .outside }.count
            let run = buckets.contains { $0.flag == .run }
            if rateLimits == nil {
                text += ". Limits need 10 periods."
            } else if outside > 0 {
                text += ". \(outside) period\(outside == 1 ? "" : "s") outside the limits."
            } else if run {
                text += ". A run of 7 periods on one side of the mean."
            } else {
                text += ". In its usual range."
            }
            return text
        case .mining: return change(miningAverage, raw: mining)
        case .staking: return change(stakingAverage, raw: staking)
        case .share:
            guard let now = share100.last.flatMap({ $0 })?.value else { return "A share needs 100 known blocks." }
            let inside = now >= Self.chanceLow && now <= Self.chanceHigh
            return "\(Format.percent(now)) staked over 100 blocks. "
                + (inside ? "Inside the limits of chance." : "Outside the limits of chance.")
        case .pool:
            guard let now = pool.last?.value else { return coin }
            var text = "\(now.formatted(.number.precision(.fractionLength(0...8)))) \(coin)"
            if let first = pool.first?.value, first > 0 {
                text += ", \(Self.percentChange(from: first, to: now)) in this range"
            }
            text += checkpointsRead >= times.count
                ? ". Each block of the range is read."
                : ". \(Format.integer(UInt64(checkpointsRead))) of \(Format.integer(UInt64(checkpointsWanted))) checkpoints read."
            return text
        }
    }

    private func change(_ average: [Point?], raw: [Point]) -> String {
        let values = average.compactMap { $0?.value }
        guard let first = values.first, let last = values.last, first > 0 else {
            return raw.last.map { "\(Format.difficulty($0.value)) at the newest block of this kind" } ?? "No block of this kind in the range."
        }
        return "\(Format.difficulty(last)), \(Self.percentChange(from: first, to: last)) in this range. The line is the trailing MA100."
    }

    private static func percentChange(from first: Double, to last: Double) -> String {
        let change = (last / first - 1) * 100
        if abs(change) < 0.5 { return "no change" }
        return "\(change > 0 ? "up" : "down") \(abs(change).formatted(.number.precision(.fractionLength(0...1))))%"
    }

    /// True when the model has figures for the block at this place of the list.
    func has(_ index: Int) -> Bool {
        index >= 0 && index < times.count && index < miningAt.count && index < stakingAt.count
            && index < poolAt.count
    }

    func rateText(at index: Int) -> String {
        guard times.indices.contains(index) else { return "–" }
        let time = times[index]
        guard let bucket = buckets.first(where: { time > $0.start && time <= $0.end }) else { return "–" }
        return "\(bucket.count) in its \(bucketName == "hour" ? "hour" : "period")"
    }

    func poolText(at index: Int) -> String {
        guard poolAt.indices.contains(index), let found = poolAt[index] else { return "–" }
        let text = "\(found.value.formatted(.number.precision(.fractionLength(8)))) \(coin)"
        return found.exact ? text : text + " (nearest earlier checkpoint)"
    }
}

extension Array where Element == Double {
    /// The first position whose value passes the test, in an array sorted so that all
    /// values that fail come first.
    func partitionIndex(where test: (Double) -> Bool) -> Int {
        var low = 0
        var high = count
        while low < high {
            let middle = (low + high) / 2
            if test(self[middle]) { high = middle } else { low = middle + 1 }
        }
        return low
    }
}

// MARK: - Waits

/// The waits between bursts in the range, in the same form as the dashboard's chain pulse.
/// Each dot is an equal share of the waits. The marker is the wait before the shown block.
private struct WaitsStrip: View {
    let model: StackModel
    let leftInset: CGFloat
    let rightInset: CGFloat
    /// The wait before the shown block, in seconds.
    let wait: Double?

    var body: some View {
        if model.waits.count >= 20 {
            VStack(alignment: .leading, spacing: 0) {
                PanelRule()
                Plate("Waits between bursts in this range")
                    .padding(.leading, leftInset)
                    .padding(.top, 12)
                // The canvas has the full width, so that its axis has the insets of the panels above.
                Canvas { context, size in draw(context, size) }
                    .frame(height: 62)
                    .accessibilityLabel("Dot plot of the waits between bursts in this range")
                Text(
                    (model.waits.count <= 100
                        ? "Each dot is one of the \(model.waits.count) waits. "
                        : "Each dot is 1 in 100 of the \(Format.integer(UInt64(model.waits.count))) waits. ")
                        + "Blocks within \(StackModel.burstSeconds) seconds of each other are one burst. "
                        + (wait.map { "The line is the wait before the shown block: \(ChartFormat.duration(Int64($0))), longer than \(shareShorter($0)) of 100." }
                            ?? "The shown block followed its parent within the burst time.")
                )
                .font(Tone.caption).foregroundStyle(Tone.label)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 18)
                .padding(.top, 4)
                .padding(.bottom, 10)
            }
        }
    }

    /// How many of 100 waits in the range are shorter than this one.
    private func shareShorter(_ seconds: Double) -> Int {
        let shorter = model.waits.partitionIndex { $0 >= seconds }
        return Int((Double(shorter) / Double(max(model.waits.count, 1)) * 100).rounded())
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize) {
        let right = size.width - rightInset
        let base = size.height - 20
        let dot: CGFloat = 6
        let low = 10.0
        let high = max(1_800, (model.waits.last ?? 0) * 1.15)
        let position = { (seconds: Double) -> CGFloat in
            leftInset + CGFloat((log(max(seconds, low)) - log(low)) / (log(high) - log(low))) * (right - leftInset)
        }
        context.rule(leftInset, base, right, base, Tone.axis)
        let ticks: [(Double, String)] = [
            (10, "10 s"), (30, "30 s"), (60, "1 min"), (120, "2 min"), (300, "5 min"), (600, "10 min"),
            (1200, "20 min"), (3600, "1 h"),
        ]
        for (seconds, label) in ticks where seconds <= high {
            let tx = position(seconds)
            context.rule(tx, base, tx, base + 4, Tone.axis)
            context.name(label, tx, base + 12)
        }
        let count = min(100, model.waits.count)
        var stacks: [Int: Int] = [:]
        // The waits shorter than the shown wait have light, as in the chain pulse.
        var shorterDots = Path()
        var longerDots = Path()
        for index in 0..<count {
            let rank = Int((Double(index) + 0.5) / Double(count) * Double(model.waits.count))
            let value = model.waits[min(rank, model.waits.count - 1)]
            let bin = Int((position(value) - leftInset) / (dot + 1))
            let level = stacks[bin, default: 0]
            stacks[bin] = level + 1
            let center = CGPoint(
                x: leftInset + (CGFloat(bin) + 0.5) * (dot + 1), y: base - 2 - dot / 2 - CGFloat(level) * (dot + 1))
            let shorter = wait.map { value < $0 } ?? true
            let box = CGRect(x: center.x - dot / 2, y: center.y - dot / 2, width: dot, height: dot)
            if shorter {
                shorterDots.addEllipse(in: box)
            } else {
                longerDots.addEllipse(in: box)
            }
        }
        context.fill(longerDots, with: .color(Tone.axis))
        context.lit(shorterDots, Tone.record, strength: 0.4)
        if let wait {
            let cx = min(position(wait), right)
            context.lit(Path(CGRect(x: cx - 0.75, y: 2, width: 1.5, height: base - 2)), Tone.present, strength: 0.6)
        }
    }
}


/// Light on the chart panels. It shows parts of a picture of the panels that has more light.
/// There are three lights, and each has its own layer, so that one does not change another:
///
/// - The scan: a quiet front that goes from the right to the left in 10 seconds. The next
///   front starts at the right at the moment when the front gets to the left.
/// - The pointer: the stretch that the pointer moved across keeps its light for a short time
///   and then loses it, as a phosphor does. There is no light ahead of the pointer.
/// - The wave of a click: two fronts go out from the selected block to the two edges. Each is
///   strong at the start, loses light with the square of its distance, and loses speed.
///
/// It is Core Animation: the system moves the light, and the app does no work for a frame.
private struct ChartLight: NSViewRepresentable {
    let picture: CGImage?
    /// The data with much more light: the pointer and the click use it.
    let hot: CGImage?
    /// The left and right edges of the plot, in the coordinates of the view.
    let left: CGFloat
    let right: CGFloat
    /// The place of the block under the pointer.
    let pointer: CGFloat?
    /// Changes with each click. `waveAt` is the place of the selected block.
    let wave: Int
    let waveAt: CGFloat

    /// The length of the light behind a front.
    private static let tail: CGFloat = 300
    /// The time that a place keeps its light after the pointer leaves it.
    private static let afterglow: CFTimeInterval = 0.9
    /// The number of marks of light that the pointer can leave at one time.
    private static let marks = 48

    final class LightView: NSView {
        // Each light is a copy of the bright picture with its own window.
        let scan = CALayer()
        let scanWindow = CALayer()
        let scanFronts = [CAGradientLayer(), CAGradientLayer()]
        let trace = CALayer()
        let traceWindow = CALayer()
        let traceMarks: [CALayer] = (0..<ChartLight.marks).map { _ in CALayer() }
        let burst = CALayer()
        let burstWindow = CALayer()
        let burstLeft = CAGradientLayer()
        let burstRight = CAGradientLayer()

        var layout: String?
        var wave: Int?
        var lastPointer: CGFloat?
        var nextMark = 0
        override var isFlipped: Bool { true }

        override init(frame: NSRect) {
            super.init(frame: frame)
            wantsLayer = true
            let clear = CGColor(gray: 0, alpha: 0)
            for (light, window) in [(scan, scanWindow), (trace, traceWindow), (burst, burstWindow)] {
                light.contentsGravity = .resize
                window.masksToBounds = true
                light.mask = window
                layer?.addSublayer(light)
            }
            // The scan: the front is at the left of the band, and the light goes down behind it.
            for front in scanFronts {
                // Full light at the front, and a third of it at the middle of the tail.
                front.colors = [clear, CGColor(gray: 0, alpha: 1), CGColor(gray: 0, alpha: 0.32), clear]
                front.locations = [0, 0.06, 0.45, 1]
                front.anchorPoint = CGPoint(x: 0.06, y: 0)
                scanWindow.addSublayer(front)
            }
            // A mark of the pointer: the stretch of the plot that the pointer moved across.
            for mark in traceMarks {
                mark.backgroundColor = .black
                mark.anchorPoint = .zero
                mark.opacity = 0
                traceWindow.addSublayer(mark)
            }
            // The wave: each front is at the outer side of its band.
            burstLeft.colors = [clear, CGColor(gray: 0, alpha: 1), CGColor(gray: 0, alpha: 0.35), clear]
            burstLeft.locations = [0, 0.06, 0.4, 1]
            burstLeft.anchorPoint = CGPoint(x: 0.06, y: 0)
            burstRight.colors = [clear, CGColor(gray: 0, alpha: 0.35), CGColor(gray: 0, alpha: 1), clear]
            burstRight.locations = [0, 0.6, 0.94, 1]
            burstRight.anchorPoint = CGPoint(x: 0.94, y: 0)
            for band in [burstLeft, burstRight] {
                band.opacity = 0
                burstWindow.addSublayer(band)
            }
            for band in scanFronts + [burstLeft, burstRight] {
                band.startPoint = CGPoint(x: 0, y: 0.5)
                band.endPoint = CGPoint(x: 1, y: 0.5)
            }
        }

        required init?(coder: NSCoder) { nil }
    }

    func makeNSView(context: Context) -> LightView { LightView() }

    func updateNSView(_ view: LightView, context: Context) {
        let bounds = view.bounds
        let width = max(right - left, 0)
        let still = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion

        // The layers get their places only when the picture or the size changes. A change of
        // the pointer does nothing to the scan.
        let layout = "\(Int(left))|\(Int(width))|\(Int(bounds.height))|\(still)|"
            + "\(picture.map { "\($0.width)x\($0.height)|\(Unmanaged.passUnretained($0).toOpaque())" } ?? "none")|"
            + "\(hot.map { "\(Unmanaged.passUnretained($0).toOpaque())" } ?? "none")"
        if view.layout != layout {
            view.layout = layout
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            let plot = CGRect(x: left, y: 0, width: width, height: bounds.height)
            for (light, window) in [(view.scan, view.scanWindow), (view.trace, view.traceWindow), (view.burst, view.burstWindow)] {
                light.frame = bounds
                light.contents = light === view.scan ? picture : (hot ?? picture)
                window.frame = plot
            }
            let tall = bounds.height
            for front in view.scanFronts {
                front.bounds = CGRect(x: 0, y: 0, width: Self.tail / 0.94, height: tall)
                front.removeAnimation(forKey: "scan")
                front.opacity = still || picture == nil ? 0 : 1
            }
            view.burstLeft.bounds = CGRect(x: 0, y: 0, width: Self.tail / 0.94, height: tall)
            view.burstRight.bounds = CGRect(x: 0, y: 0, width: Self.tail / 0.94, height: tall)
            CATransaction.commit()

            // Two fronts, one plot width apart. When the first gets to the left edge it
            // starts again at the right edge, and the second has the light that the first
            // left behind. Thus there is always one front in the plot, with no stop.
            if !still, width > 0, picture != nil {
                for (number, front) in view.scanFronts.enumerated() {
                    let move = CABasicAnimation(keyPath: "position.x")
                    move.fromValue = width - CGFloat(number) * width
                    move.toValue = -CGFloat(number) * width
                    move.duration = 10
                    move.repeatCount = .infinity
                    move.timingFunction = CAMediaTimingFunction(name: .linear)
                    front.add(move, forKey: "scan")
                }
            }
        }

        // The pointer leaves light on the stretch that it moved across, and no light ahead
        // of it. The light of a stretch goes down slowly: the afterglow of a phosphor.
        if let pointer, picture != nil {
            let at = pointer - left
            if let before = view.lastPointer, abs(before - at) >= 0.5 {
                let mark = view.traceMarks[view.nextMark]
                view.nextMark = (view.nextMark + 1) % Self.marks
                CATransaction.begin()
                CATransaction.setDisableActions(true)
                mark.frame = CGRect(x: min(before, at), y: 0, width: abs(before - at), height: bounds.height)
                CATransaction.commit()
                let glow = CAKeyframeAnimation(keyPath: "opacity")
                // A strong light at the moment of the pointer, which goes down quickly to the
                // quiet afterglow and then to nothing.
                glow.values = [1, 0.34, 0.11, 0]
                glow.keyTimes = [0, 0.12, 0.35, 1]
                glow.duration = Self.afterglow
                mark.removeAnimation(forKey: "glow")
                mark.add(glow, forKey: "glow")
            }
            view.lastPointer = at
        } else {
            view.lastPointer = nil
        }

        // A click: two fronts go out from the selected block. Each front is strong at the
        // start. Its light goes down with the square of its distance from the block, and
        // its speed goes down as it goes out, as a wave that spreads does.
        guard view.wave != wave else { return }
        let first = view.wave == nil
        view.wave = wave
        guard !first, !still, width > 0, picture != nil else { return }
        let origin = waveAt - left
        // The distance at which the light is half of its start.
        let half = max(width * 0.16, 60)
        let steps = 24
        for (band, target) in [(view.burstLeft, -Self.tail), (view.burstRight, width + Self.tail)] {
            let distance = target - origin
            var places: [CGFloat] = []
            var lights: [Double] = []
            var times: [NSNumber] = []
            for step in 0...steps {
                let time = Double(step) / Double(steps)
                // Quick at the start and slower to the end.
                let gone = 1 - pow(1 - time, 2.4)
                let travelled = distance * CGFloat(gone)
                places.append(origin + travelled)
                lights.append(1 / (1 + pow(Double(abs(travelled)) / Double(half), 2)))
                times.append(NSNumber(value: time))
            }
            let move = CAKeyframeAnimation(keyPath: "position.x")
            move.values = places
            move.keyTimes = times
            let light = CAKeyframeAnimation(keyPath: "opacity")
            light.values = lights
            light.keyTimes = times
            let group = CAAnimationGroup()
            group.animations = [move, light]
            // A wave that has more plot to cross has more time.
            group.duration = 0.9 + 1.5 * Double(abs(distance) / (width + Self.tail))
            band.removeAnimation(forKey: "wave")
            band.removeAnimation(forKey: "show")
            band.add(group, forKey: "wave")
        }
    }
}
