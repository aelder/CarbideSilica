import CarbideCore
import Charts
import SwiftUI

/// The chain's recent behaviour as charts: block intervals, mining difficulty, staking
/// difficulty, and the share of blocks that were staked. Below them, the block estimator.
struct ChartsView: View {
    @Environment(AppModel.self) private var model
    @State private var span = Span.sixHours
    @State private var metric = Metric.intervals
    @State private var points: [BlockPoint] = []
    @State private var failure: String?
    @State private var loading = true
    @State private var limited = false

    enum Span: String, CaseIterable, Identifiable {
        case twoHours = "2 hours"
        case sixHours = "6 hours"
        case day = "1 day"
        var id: String { rawValue }
        var blocks: UInt32 {
            switch self {
            case .twoHours: 120
            case .sixHours: 360
            case .day: 1440
            }
        }
    }

    enum Metric: String, CaseIterable, Identifiable {
        case intervals = "Block intervals"
        case mining = "Mining difficulty"
        case staking = "Staking difficulty"
        case mix = "Staked share"
        var id: String { rawValue }
    }

    /// A public API answers one block at a time and slowly, so keep its requests modest.
    private let apiLimit: UInt32 = 240
    private let mixWindow = 100

    var body: some View {
        Page {
            Card("Charts", trailing: range) {
                HStack {
                    Picker("Metric", selection: $metric) {
                        ForEach(Metric.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    Spacer(minLength: 16)
                    Picker("Span", selection: $span) {
                        ForEach(Span.allCases) { Text($0.rawValue).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .labelsHidden()
                    .frame(maxWidth: 260)
                }
                if loading {
                    ProgressView("Loading blocks…").frame(maxWidth: .infinity, minHeight: 280)
                } else if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.secondary)
                } else {
                    chart.frame(height: 280)
                    Text(summary).font(.callout)
                    if limited {
                        Text("Showing the latest \(apiLimit) blocks. A public API serves blocks slowly; a local daemon loads the full span.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if model.syncFraction != nil {
                        Text("The node is still syncing, so this is history, not the present.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            EstimatorCard()
        }
        .navigationTitle("Charts")
        .task(id: "\(span.rawValue)|\(model.generation)|\(model.status != nil)") { await load() }
    }

    private var range: String? {
        guard let first = points.first, let last = points.last, !loading else { return nil }
        return "blocks \(Format.integer(first.height)) to \(Format.integer(last.height))"
    }

    private func load() async {
        guard let tip = model.status?.blocks, tip > 1 else { return }
        loading = true
        failure = nil
        let api = model.status?.source == .publicApi
        limited = api && span.blocks > apiLimit
        do {
            points = try await model.requireExplorer().blockSeries(
                endHeight: tip, count: api ? min(span.blocks, apiLimit) : span.blocks)
        } catch is CancellationError {
            return
        } catch {
            points = []
            failure = describe(error)
        }
        loading = false
    }

    private struct Sample: Identifiable {
        let id: UInt64
        let date: Date
        let value: Double
        let staked: Bool
    }

    private func date(_ point: BlockPoint) -> Date {
        Date(timeIntervalSince1970: TimeInterval(point.time))
    }

    /// Seconds between each block and the one before it.
    private var intervals: [Sample] {
        zip(points, points.dropFirst()).map { previous, block in
            Sample(
                id: block.height, date: date(block),
                value: Double(block.time) - Double(previous.time), staked: block.staked)
        }
    }

    private var series: [Sample] {
        switch metric {
        case .intervals:
            return intervals
        case .mining:
            return points.compactMap { point in
                point.miningDifficulty.map { Sample(id: point.height, date: date(point), value: $0, staked: false) }
            }
        case .staking:
            return points.compactMap { point in
                point.stakingDifficulty.map { Sample(id: point.height, date: date(point), value: $0, staked: true) }
            }
        case .mix:
            guard points.count >= mixWindow else { return [] }
            var staked = points.prefix(mixWindow).filter(\.staked).count
            var result: [Sample] = []
            for index in (mixWindow - 1)..<points.count {
                if index >= mixWindow {
                    staked += (points[index].staked ? 1 : 0) - (points[index - mixWindow].staked ? 1 : 0)
                }
                result.append(
                    Sample(
                        id: points[index].height, date: date(points[index]),
                        value: Double(staked) / Double(mixWindow), staked: true))
            }
            return result
        }
    }

    @ViewBuilder
    private var chart: some View {
        let samples = series
        if samples.isEmpty {
            Text(metric == .mix ? "This needs at least \(mixWindow) blocks. Choose a longer span." : "No blocks of this kind in the range.")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
            Chart {
                switch metric {
                case .intervals:
                    ForEach(samples) { sample in
                        BarMark(x: .value("Time", sample.date), y: .value("Seconds", sample.value), width: 2)
                            .foregroundStyle(sample.value > 180 ? Color.red : sample.staked ? Color.purple : Color.orange)
                    }
                    RuleMark(y: .value("Target", 60))
                        .foregroundStyle(.secondary)
                        .lineStyle(StrokeStyle(lineWidth: 1, dash: [4, 4]))
                        .annotation(position: .top, alignment: .trailing) {
                            Text("60 s target").font(.caption2).foregroundStyle(.secondary)
                        }
                case .mining, .staking:
                    ForEach(samples) { sample in
                        LineMark(x: .value("Time", sample.date), y: .value(metric.rawValue, sample.value))
                            .foregroundStyle(metric == .mining ? Color.orange : Color.purple)
                            .interpolationMethod(.monotone)
                    }
                case .mix:
                    ForEach(samples) { sample in
                        LineMark(x: .value("Time", sample.date), y: .value("Staked share", sample.value))
                            .foregroundStyle(Color.purple)
                    }
                    RuleMark(y: .value("Even", 0.5))
                        .foregroundStyle(.secondary)
                        .lineStyle(StrokeStyle(lineWidth: 1, dash: [4, 4]))
                }
            }
            .chartYScale(domain: metric == .mix ? .automatic(includesZero: true) : .automatic(includesZero: metric == .intervals))
            .chartYAxis {
                AxisMarks { value in
                    AxisGridLine()
                    AxisValueLabel {
                        if let number = value.as(Double.self) {
                            Text(axisLabel(number))
                        }
                    }
                }
            }
            .chartOverlay { proxy in
                GeometryReader { geometry in
                    Rectangle().fill(.clear).contentShape(Rectangle())
                        .onTapGesture(coordinateSpace: .local) { location in
                            guard let plot = proxy.plotFrame else { return }
                            let x = location.x - geometry[plot].origin.x
                            guard let tapped: Date = proxy.value(atX: x) else { return }
                            let nearest = samples.min {
                                abs($0.date.timeIntervalSince(tapped)) < abs($1.date.timeIntervalSince(tapped))
                            }
                            if let nearest { model.open(.block(id: "\(nearest.id)")) }
                        }
                }
            }
        }
    }

    private func axisLabel(_ value: Double) -> String {
        switch metric {
        case .intervals: "\(Int(value)) s"
        case .mix: Format.percent(value)
        case .mining, .staking: Format.difficulty(value)
        }
    }

    private var summary: String {
        let samples = series
        guard !samples.isEmpty else { return "" }
        let values = samples.map(\.value)
        let mean = values.reduce(0, +) / Double(values.count)
        switch metric {
        case .intervals:
            let slow = values.filter { $0 > 180 }.count
            let longest = samples.max { $0.value < $1.value }
            var text = "Blocks arrived every \(Int(mean.rounded())) seconds on average."
            if let longest {
                text += " The longest wait was \(Int(longest.value) / 60) min \(Int(longest.value) % 60) s, before block \(Format.integer(longest.id))."
            }
            text += slow == 0 ? " None took over three minutes." : " \(slow) took over three minutes."
            return text + " Click a bar to open its block."
        case .mining, .staking:
            let first = values.first ?? 0
            let last = values.last ?? 0
            let change = first > 0 ? (last - first) / first : 0
            let direction = abs(change) < 0.02 ? "about level" : change > 0 ? "up \(Format.percent(change))" : "down \(Format.percent(-change))"
            return "\(metric.rawValue) is \(Format.difficulty(last)), \(direction) across this range."
        case .mix:
            return "Over the last \(mixWindow) blocks, \(Format.percent(values.last ?? 0)) were staked. The dashed line is an even split with mining."
        }
    }
}

/// When a future block will arrive, or what height a future moment will be.
private struct EstimatorCard: View {
    @Environment(AppModel.self) private var model
    @State private var query = ""
    @State private var result: BlockEstimate?
    @State private var failure: String?
    @State private var running = false

    var body: some View {
        Card("Future block estimate") {
            HStack {
                TextField("Target", text: $query, prompt: Text("A future height or date: 5000000, 2028-11-08, in 2 hours"))
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { Task { await run() } }
                Button("Estimate") { Task { await run() } }
                    .disabled(running || query.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            if running {
                ProgressView().controlSize(.small)
            } else if let failure {
                Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
            } else if let result {
                Text(sentence(result)).font(.title3)
                Text(
                    "Based on \(result.secondsPerBlock.formatted(.number.precision(.fractionLength(2)))) seconds per block, "
                        + "measured from block \(Format.integer(result.measuredFrom)) to \(Format.integer(result.measuredTo)). \(result.source)."
                )
                .font(.callout).foregroundStyle(.secondary)
                Text("An estimate, not a schedule: it assumes blocks keep arriving at the measured rate.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private func sentence(_ estimate: BlockEstimate) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(estimate.time))
        let when = date.formatted(date: .abbreviated, time: .shortened)
        let relative = date.formatted(.relative(presentation: .named))
        return "Block \(Format.integer(estimate.height)) is expected around \(when) (\(relative))."
    }

    private func run() async {
        running = true
        failure = nil
        defer { running = false }
        do {
            result = try await model.requireExplorer().estimate(query: query)
        } catch {
            result = nil
            failure = describe(error)
        }
    }
}
