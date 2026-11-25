import CarbideCore
import SwiftUI

/// Colours shared by the chart plots. Each also has a shape and a word in the legend.
enum ChartColors {
    static let block = Color.blue
    static let average = Color.green
    static let long = Color.orange
    static let reserve = Color.green
    static let grid = Color.secondary.opacity(0.18)
    static let selection = Color.accentColor
}

/// A legend row: a small mark and the words for it.
struct ChartLegend: View {
    enum Shape { case dot, line, dash, bar }

    struct Item: Identifiable {
        let shape: Shape
        let color: Color
        let text: String
        var id: String { text }
    }

    let items: [Item]

    var body: some View {
        FlowRow(items: items) { item in
            HStack(spacing: 5) {
                Canvas { context, size in
                    let mid = size.height / 2
                    switch item.shape {
                    case .dot:
                        context.fill(Path(ellipseIn: CGRect(x: size.width / 2 - 3, y: mid - 3, width: 6, height: 6)), with: .color(item.color))
                    case .line, .dash:
                        var path = Path()
                        path.move(to: CGPoint(x: 0, y: mid))
                        path.addLine(to: CGPoint(x: size.width, y: mid))
                        context.stroke(
                            path, with: .color(item.color),
                            style: StrokeStyle(lineWidth: 2, dash: item.shape == .dash ? [3, 3] : []))
                    case .bar:
                        context.fill(Path(CGRect(x: size.width / 2 - 1, y: 1, width: 2, height: size.height - 2)), with: .color(item.color))
                    }
                }
                .frame(width: 16, height: 10)
                Text(item.text).font(.caption)
            }
        }
    }
}

/// Lays out short items in rows that wrap.
private struct FlowRow<Item: Identifiable, Content: View>: View {
    let items: [Item]
    @ViewBuilder let content: (Item) -> Content

    var body: some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: 16) { ForEach(items) { content($0) } }
            VStack(alignment: .leading, spacing: 4) { ForEach(items) { content($0) } }
        }
    }
}

/// The plot of one block metric. Block metrics use height on the horizontal axis. The PoW
/// ratio uses header time, as Carbide does. Each dot is one block. A click selects the
/// nearest block.
struct BlockPlot: View {
    let samples: [ChartSample]
    let first: UInt64
    let last: UInt64
    let metric: ChartState.Metric
    let selected: String?
    let select: (String) -> Void

    private let gutter: CGFloat = 76
    private let footer: CGFloat = 30

    var body: some View {
        let scale = PlotScale(samples: samples, first: first, last: last, metric: metric)
        GeometryReader { geometry in
            let plot = CGRect(
                x: gutter, y: 8,
                width: max(geometry.size.width - gutter - 12, 10),
                height: max(geometry.size.height - footer - 8, 10))
            Canvas { context, _ in
                drawGrid(&context, plot: plot, scale: scale)
                drawMarks(&context, plot: plot, scale: scale)
                drawSelection(&context, plot: plot, scale: scale)
            }
            .contentShape(Rectangle())
            .gesture(
                SpatialTapGesture().onEnded { tap in
                    if let hash = nearest(to: tap.location.x, plot: plot, scale: scale) { select(hash) }
                })
        }
        .accessibilityElement()
        .accessibilityLabel("\(metric.title) plot of \(samples.count) blocks")
    }

    // MARK: Drawing

    private func drawGrid(_ context: inout GraphicsContext, plot: CGRect, scale: PlotScale) {
        for tick in scale.yTicks {
            let y = scale.y(tick.value, in: plot)
            var line = Path()
            line.move(to: CGPoint(x: plot.minX, y: y))
            line.addLine(to: CGPoint(x: plot.maxX, y: y))
            context.stroke(line, with: .color(ChartColors.grid), lineWidth: 1)
            context.draw(
                Text(tick.label).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: plot.minX - 6, y: y), anchor: .trailing)
        }
        if metric == .ratio {
            for (level, label) in [(1.0 / 3.0, "Mined"), (2.0 / 3.0, "Staked")] {
                context.draw(
                    Text(label).font(.caption2).foregroundStyle(ChartColors.block),
                    at: CGPoint(x: plot.maxX + 2, y: plot.maxY - plot.height * level), anchor: .bottomTrailing)
            }
        }
        if metric == .intervals {
            let y = scale.y(60, in: plot)
            if y >= plot.minY {
                var line = Path()
                line.move(to: CGPoint(x: plot.minX, y: y))
                line.addLine(to: CGPoint(x: plot.maxX, y: y))
                context.stroke(line, with: .color(.secondary), style: StrokeStyle(lineWidth: 1, dash: [4, 4]))
            }
        }
        var axis = Path()
        axis.move(to: CGPoint(x: plot.minX, y: plot.minY))
        axis.addLine(to: CGPoint(x: plot.minX, y: plot.maxY))
        axis.addLine(to: CGPoint(x: plot.maxX, y: plot.maxY))
        context.stroke(axis, with: .color(.secondary.opacity(0.5)), lineWidth: 1)
        for (index, tick) in scale.xTicks.enumerated() {
            let x = scale.x(tick, in: plot)
            var mark = Path()
            mark.move(to: CGPoint(x: x, y: plot.maxY))
            mark.addLine(to: CGPoint(x: x, y: plot.maxY + 4))
            context.stroke(mark, with: .color(.secondary), lineWidth: 1)
            let anchor: UnitPoint = index == 0 ? .topLeading : index == scale.xTicks.count - 1 ? .topTrailing : .top
            context.draw(
                Text(scale.xLabel(tick)).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: x, y: plot.maxY + 6), anchor: anchor)
        }
    }

    private func drawMarks(_ context: inout GraphicsContext, plot: CGRect, scale: PlotScale) {
        let radius: CGFloat = samples.count > 3_000 ? 1.2 : samples.count > 500 ? 1.6 : 2.2
        switch metric {
        case .intervals:
            var bars = Path()
            let base = scale.y(0, in: plot)
            for sample in samples {
                // A negative interval is not drawn, and never as zero.
                guard let interval = sample.interval, interval >= 0 else { continue }
                let x = scale.x(scale.xValue(sample), in: plot)
                let y = scale.y(Double(interval), in: plot)
                bars.addRect(CGRect(x: x - 0.6, y: min(y, base - 1), width: 1.2, height: max(base - y, 1)))
            }
            context.fill(bars, with: .color(ChartColors.block.opacity(0.8)))
        case .mining, .staking:
            var dots = Path()
            for sample in samples {
                guard let value = metric == .mining ? sample.mining : sample.staking else { continue }
                let point = CGPoint(x: scale.x(scale.xValue(sample), in: plot), y: scale.y(value, in: plot))
                dots.addEllipse(in: CGRect(x: point.x - radius, y: point.y - radius, width: radius * 2, height: radius * 2))
            }
            context.fill(dots, with: .color(ChartColors.block))
            let averages = line(plot: plot, scale: scale, joins: { $0.linked }) { sample in
                (metric == .mining ? sample.miningAverage : sample.stakingAverage)?.value
            }
            context.stroke(averages, with: .color(ChartColors.average), lineWidth: 2)
        case .ratio:
            var dots = Path()
            for sample in samples {
                let level: Double
                switch sample.production {
                case .mined: level = 1.0 / 3.0
                case .staked: level = 2.0 / 3.0
                case .unknown: continue
                }
                let point = CGPoint(x: scale.x(scale.xValue(sample), in: plot), y: plot.maxY - plot.height * level)
                dots.addEllipse(in: CGRect(x: point.x - radius, y: point.y - radius, width: radius * 2, height: radius * 2))
            }
            context.fill(dots, with: .color(ChartColors.block.opacity(0.7)))
            let center = scale.y(0.5, in: plot)
            var half = Path()
            half.move(to: CGPoint(x: plot.minX, y: center))
            half.addLine(to: CGPoint(x: plot.maxX, y: center))
            context.stroke(half, with: .color(.secondary), style: StrokeStyle(lineWidth: 1, dash: [4, 4]))
            let joins: (ChartSample) -> Bool = { $0.linked && $0.timeIncreases }
            context.stroke(line(plot: plot, scale: scale, joins: joins) { $0.share1000 }, with: .color(ChartColors.long), lineWidth: 2)
            context.stroke(line(plot: plot, scale: scale, joins: joins) { $0.share100 }, with: .color(ChartColors.average), lineWidth: 1.5)
        case .feePool:
            break
        }
    }

    /// A line through the values, broken where a value is missing or a sample does not join
    /// the one before it.
    private func line(
        plot: CGRect, scale: PlotScale, joins: (ChartSample) -> Bool, value: (ChartSample) -> Double?
    ) -> Path {
        var path = Path()
        var open = false
        for sample in samples {
            guard let value = value(sample) else {
                open = false
                continue
            }
            let point = CGPoint(x: scale.x(scale.xValue(sample), in: plot), y: scale.y(value, in: plot))
            if open && joins(sample) {
                path.addLine(to: point)
            } else {
                path.move(to: point)
            }
            open = true
        }
        return path
    }

    private func drawSelection(_ context: inout GraphicsContext, plot: CGRect, scale: PlotScale) {
        guard let selected, let sample = samples.first(where: { $0.hash == selected }) else { return }
        let x = scale.x(scale.xValue(sample), in: plot)
        var rule = Path()
        rule.move(to: CGPoint(x: x, y: plot.minY))
        rule.addLine(to: CGPoint(x: x, y: plot.maxY))
        context.stroke(rule, with: .color(ChartColors.selection.opacity(0.7)), lineWidth: 1)
        let value: Double?
        switch metric {
        case .intervals: value = sample.interval.map { Double(max($0, 0)) }
        case .mining: value = sample.mining
        case .staking: value = sample.staking
        case .ratio: value = sample.production == .unknown ? nil : (sample.production == .staked ? 2.0 / 3.0 : 1.0 / 3.0)
        case .feePool: value = nil
        }
        if let value {
            let y = metric == .ratio ? plot.maxY - plot.height * value : scale.y(value, in: plot)
            let ring = CGRect(x: x - 5, y: y - 5, width: 10, height: 10)
            context.stroke(Path(ellipseIn: ring), with: .color(ChartColors.selection), lineWidth: 2)
        }
    }

    private func nearest(to location: CGFloat, plot: CGRect, scale: PlotScale) -> String? {
        let target = scale.xFromPosition(location, in: plot)
        let candidates = samples.filter { sample in
            switch metric {
            case .intervals: sample.interval != nil
            case .mining: sample.mining != nil
            case .staking: sample.staking != nil
            case .ratio: sample.production != .unknown
            case .feePool: false
            }
        }
        return candidates.min { abs(scale.xValue($0) - target) < abs(scale.xValue($1) - target) }?.hash
    }
}

/// The axes of a block plot: horizontal by height or time, vertical with rounded ticks.
struct PlotScale {
    struct Tick {
        let value: Double
        let label: String
    }

    let metric: ChartState.Metric
    let xLow: Double
    let xHigh: Double
    let yLow: Double
    let yHigh: Double
    let yTicks: [Tick]
    let xTicks: [Double]
    private let timeAxis: Bool

    init(samples: [ChartSample], first: UInt64, last: UInt64, metric: ChartState.Metric) {
        self.metric = metric
        timeAxis = metric == .ratio
        if timeAxis {
            let times = samples.map { Double($0.time) }
            xLow = times.min() ?? 0
            xHigh = max(times.max() ?? 1, xLow + 1)
        } else {
            xLow = Double(first)
            xHigh = max(Double(last), xLow + 1)
        }
        xTicks = PlotScale.xTicks(low: xLow, high: xHigh, count: 5, integral: !timeAxis)
        switch metric {
        case .ratio:
            yLow = 0
            yHigh = 1
            yTicks = [
                Tick(value: 0, label: "0% staked"), Tick(value: 0.5, label: "50/50"),
                Tick(value: 1, label: "100% staked"),
            ]
        default:
            var values: [Double] = []
            for sample in samples {
                switch metric {
                case .intervals:
                    if let interval = sample.interval, interval >= 0 { values.append(Double(interval)) }
                case .mining:
                    if let value = sample.mining { values.append(value) }
                    if let average = sample.miningAverage { values.append(average.value) }
                case .staking:
                    if let value = sample.staking { values.append(value) }
                    if let average = sample.stakingAverage { values.append(average.value) }
                default: break
                }
            }
            let ticks = PlotScale.niceTicks(values: values, fromZero: metric == .intervals, count: 5)
            yLow = ticks.first ?? 0
            yHigh = max(ticks.last ?? 1, yLow + Double.leastNonzeroMagnitude)
            let step = ticks.count > 1 ? ticks[1] - ticks[0] : 1
            yTicks = ticks.map { Tick(value: $0, label: PlotScale.label($0, step: step, metric: metric)) }
        }
    }

    func xValue(_ sample: ChartSample) -> Double {
        timeAxis ? Double(sample.time) : Double(sample.height)
    }

    func x(_ value: Double, in plot: CGRect) -> CGFloat {
        plot.minX + CGFloat((value - xLow) / (xHigh - xLow)) * plot.width
    }

    func xFromPosition(_ position: CGFloat, in plot: CGRect) -> Double {
        xLow + Double((position - plot.minX) / plot.width) * (xHigh - xLow)
    }

    func y(_ value: Double, in plot: CGRect) -> CGFloat {
        let span = yHigh - yLow
        guard span > 0 else { return plot.midY }
        return plot.maxY - CGFloat((value - yLow) / span) * plot.height
    }

    func xLabel(_ value: Double) -> String {
        if timeAxis {
            return ChartFormat.axisTime(UInt64(max(value, 0)), span: UInt64(xHigh - xLow))
        }
        return "#" + Format.integer(UInt64(max(value, 0)))
    }

    /// Evenly spaced positions between the ends, both included. Ported from Carbide's
    /// `chart_ui::x_ticks`.
    static func xTicks(low: Double, high: Double, count: Int, integral: Bool) -> [Double] {
        let intervals = integral ? min(Double(count - 1), (high - low).rounded(.down)) : Double(count - 1)
        guard intervals > 0 else { return [low] }
        return (0...Int(intervals)).map { index in
            let value = low + (high - low) * Double(index) / intervals
            return integral ? value.rounded(.down) : value
        }
    }

    /// Rounded, uniform ticks so that labels and grid lines share one scale. Ported from
    /// Carbide's `chart_ui::y_ticks`.
    static func niceTicks(values: [Double], fromZero: Bool, count: Int) -> [Double] {
        var low = fromZero ? 0 : (values.min() ?? 0)
        var high = max(values.max() ?? 1, 0)
        if high <= low {
            if high > 0 {
                low *= 0.95
                high *= 1.05
            } else {
                low = 0
                high = 1
            }
        }
        if fromZero { low = 0 }
        let rough = max((high - low) / Double(max(count - 1, 1)), Double.leastNormalMagnitude)
        let base = pow(10, floor(log10(rough)))
        let factor = [1.0, 2.0, 2.5, 5.0, 10.0].first { $0 * base >= rough } ?? 10
        let step = base * factor
        let start = fromZero ? 0 : floor(low / step) * step
        let intervals = Int(min(max(((high - start) / step).rounded(.up), 1), 8))
        return (0...intervals).map { start + step * Double($0) }
    }

    static func label(_ value: Double, step: Double, metric: ChartState.Metric) -> String {
        if metric == .intervals { return "\(Int(value.rounded())) s" }
        if value == 0 { return "0" }
        if abs(value) >= 1_000 { return Format.difficulty(value) }
        let digits = max(0, min(8, Int((-log10(step)).rounded(.up)) + 1))
        return value.formatted(.number.precision(.fractionLength(0...digits)))
    }
}
