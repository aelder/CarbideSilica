import CarbideCore
import SwiftUI

/// The fee pool's balance over the chart range, read from the checkpoint in sampled blocks'
/// coinbases. Native currency is pale blue. On a PBaaS chain the network's Verus currency is
/// green. Lines join observed checkpoints. They do not claim that every block between two
/// checkpoints was measured.
struct FeePoolPlot: View {
    let state: ChartState

    private let gutter: CGFloat = 84
    private let footer: CGFloat = 30

    var body: some View {
        let model = FeePoolPlotModel(state: state)
        DisplayPanel(padding: 0) {
            plotView(model)
        }
        .accessibilityElement()
        .accessibilityLabel("Fee pool plot of \(model.points.count) checkpoints")
    }

    private func plotView(_ model: FeePoolPlotModel) -> some View {
        GeometryReader { geometry in
            let plot = CGRect(
                x: gutter, y: 8,
                width: max(geometry.size.width - gutter - 12, 10),
                height: max(geometry.size.height - footer - 8, 10))
            Canvas { context, _ in
                model.draw(&context, plot: plot)
            }
            .contentShape(Rectangle())
            .gesture(
                SpatialTapGesture().onEnded { tap in
                    if let hash = model.nearest(to: tap.location.x, plot: plot) { state.select(hash: hash) }
                })
        }
        .overlay {
            if model.points.isEmpty {
                Text(state.fee.loading ? "Reading fee pool checkpoints…" : "No fee pool checkpoints have loaded for this range.")
                    .font(Tone.caption)
                    .foregroundStyle(Tone.label)
            }
        }
    }
}

/// One loaded checkpoint at a sampled block of the range.
struct FeePoolPoint {
    let sample: ChartSample
    let checkpoint: FeePoolCheckpoint
}

/// The fee pool plot's points and axes. A checkpoint without a known balance breaks a line.
/// A checkpoint that has not loaded yet is left out.
struct FeePoolPlotModel {
    let points: [FeePoolPoint]
    let currency: FeePoolState.Currency
    let scale: FeePoolState.Scale
    let hasReserve: Bool
    let selected: String?
    let xLow: Double
    let xHigh: Double
    let yLow: Double
    let yHigh: Double
    let ticks: [(Double, String)]

    @MainActor
    init(state: ChartState) {
        let fee = state.fee
        let samples = state.page?.samples ?? []
        points = samples.compactMap { sample in
            guard fee.scheduled.contains(sample.hash), let checkpoint = fee.checkpoints[sample.hash] else { return nil }
            return FeePoolPoint(sample: sample, checkpoint: checkpoint)
        }
        hasReserve = fee.hasReserve
        currency = hasReserve ? fee.currency : .native
        scale = fee.scale
        selected = state.selectedSample?.hash
        let times = samples.map { Double($0.time) }
        xLow = times.min() ?? 0
        xHigh = max(times.max() ?? 1, xLow + 1)
        var values: [Double] = []
        for point in points {
            if currency != .reserve, let sats = point.checkpoint.nativeSats { values.append(Double(sats) / 1e8) }
            if currency != .native, let sats = point.checkpoint.reserveSats { values.append(Double(sats) / 1e8) }
        }
        switch scale {
        case .log:
            let top = max((values.map(FeePoolPlotModel.transform).max() ?? 1).rounded(.up), 1)
            let step = max((top / 5).rounded(.up), 1)
            var list: [(Double, String)] = []
            var tick = 0.0
            while tick < top {
                list.append((tick, FeePoolPlotModel.logLabel(tick)))
                tick += step
            }
            list.append((top, FeePoolPlotModel.logLabel(top)))
            ticks = list
            yLow = 0
            yHigh = top
        case .zero, .fit:
            let nice = PlotScale.niceTicks(values: values, fromZero: scale == .zero, count: 5)
            let step = nice.count > 1 ? nice[1] - nice[0] : 1
            ticks = nice.map { ($0, FeePoolPlotModel.linearLabel($0, step: step)) }
            yLow = nice.first ?? 0
            yHigh = max(nice.last ?? 1, yLow + 1e-8)
        }
    }

    // Carbide's log-like scale (`chart_ui::fee_pool::scale`): linear from zero to the
    // protocol's 0.0001 minimum share, then logarithmic, so that zero stays defined.
    static let knee = 0.0001

    static func transform(_ value: Double) -> Double {
        value <= knee ? value / knee : 1 + log10(value / knee)
    }

    static func logLabel(_ value: Double) -> String {
        let coins = value <= 1 ? value * knee : knee * pow(10, value - 1)
        if coins >= 1_000_000 { return coins.formatted(.number.notation(.scientific).precision(.significantDigits(3))) }
        return coins.formatted(.number.precision(.fractionLength(0...8)))
    }

    static func linearLabel(_ value: Double, step: Double) -> String {
        let digits = max(0, min(8, Int((-log10(step)).rounded(.up)) + 1))
        return value.formatted(.number.precision(.fractionLength(0...digits)))
    }

    func y(_ coins: Double, in plot: CGRect) -> CGFloat {
        let value = scale == .log ? FeePoolPlotModel.transform(coins) : coins
        let span = yHigh - yLow
        guard span > 0 else { return plot.midY }
        return plot.maxY - CGFloat((value - yLow) / span) * plot.height
    }

    func x(_ time: Double, in plot: CGRect) -> CGFloat {
        plot.minX + CGFloat((time - xLow) / (xHigh - xLow)) * plot.width
    }

    func draw(_ context: inout GraphicsContext, plot: CGRect) {
        for (value, label) in ticks {
            let y = plot.maxY - CGFloat((value - yLow) / max(yHigh - yLow, 1e-12)) * plot.height
            var line = Path()
            line.move(to: CGPoint(x: plot.minX, y: y))
            line.addLine(to: CGPoint(x: plot.maxX, y: y))
            context.stroke(line, with: .color(ChartColors.grid), lineWidth: 1)
            context.name(label, plot.minX - 6, y, .trailing)
        }
        var axis = Path()
        axis.move(to: CGPoint(x: plot.minX, y: plot.minY))
        axis.addLine(to: CGPoint(x: plot.minX, y: plot.maxY))
        axis.addLine(to: CGPoint(x: plot.maxX, y: plot.maxY))
        context.stroke(axis, with: .color(Tone.axis), lineWidth: 1)
        let xTicks = PlotScale.xTicks(low: xLow, high: xHigh, count: 5, integral: false)
        for (index, tick) in xTicks.enumerated() {
            let x = x(tick, in: plot)
            let anchor: UnitPoint = index == 0 ? .topLeading : index == xTicks.count - 1 ? .topTrailing : .top
            context.rule(x, plot.maxY, x, plot.maxY + 4, Tone.axis)
            context.name(ChartFormat.axisTime(UInt64(max(tick, 0)), span: UInt64(xHigh - xLow)), x, plot.maxY + 6, anchor)
        }
        if currency != .reserve {
            series(&context, plot: plot, color: ChartColors.block) { $0.nativeSats }
        }
        if currency != .native && hasReserve {
            series(&context, plot: plot, color: ChartColors.reserve) { $0.reserveSats }
        }
        if let selected, let point = points.first(where: { $0.sample.hash == selected }) {
            let x = x(Double(point.sample.time), in: plot)
            context.lit(
                Path(CGRect(x: x - 0.75, y: plot.minY, width: 1.5, height: plot.height)), ChartColors.selection,
                strength: 0.6)
            let sats = currency == .reserve ? point.checkpoint.reserveSats : point.checkpoint.nativeSats
            if let sats {
                let y = y(Double(sats) / 1e8, in: plot)
                context.stroke(Path(ellipseIn: CGRect(x: x - 5, y: y - 5, width: 10, height: 10)), with: .color(ChartColors.selection), lineWidth: 1.5)
            }
        }
    }

    private func series(
        _ context: inout GraphicsContext, plot: CGRect, color: Color, value: (FeePoolCheckpoint) -> Int64?
    ) {
        var line = Path()
        var dots = Path()
        var open = false
        var previousTime: UInt64?
        for point in points {
            guard point.checkpoint.known, let sats = value(point.checkpoint) else {
                open = false
                continue
            }
            let location = CGPoint(x: x(Double(point.sample.time), in: plot), y: y(Double(sats) / 1e8, in: plot))
            // A line does not join a time that does not increase.
            if open, let previousTime, point.sample.time > previousTime {
                line.addLine(to: location)
            } else {
                line.move(to: location)
            }
            open = true
            previousTime = point.sample.time
            dots.addEllipse(in: CGRect(x: location.x - 1.5, y: location.y - 1.5, width: 3, height: 3))
        }
        context.stroke(line, with: .color(color), lineWidth: 1)
        context.fill(dots, with: .color(color))
    }

    func nearest(to location: CGFloat, plot: CGRect) -> String? {
        let time = xLow + Double((location - plot.minX) / plot.width) * (xHigh - xLow)
        return points.filter(\.checkpoint.known)
            .min { abs(Double($0.sample.time) - time) < abs(Double($1.sample.time) - time) }?.sample.hash
    }
}

/// Fee pool controls, the selected checkpoint, the current balance and a table.
struct FeePoolSection: View {
    @Bindable var state: ChartState
    @Environment(AppModel.self) private var model
    @State private var tableShown = false

    private var fee: FeePoolState { state.fee }

    var body: some View {
        Card("Fee pool") {
            controls
            legend
            progress
        }
        Card("Selected checkpoint") {
            selectedCheckpoint
        }
        Card("Current fee pool") {
            current
        }
        Card("Checkpoints as a table") {
            DisclosureGroup(isExpanded: $tableShown) {
                table
            } label: {
                Text(tableShown ? "Hide" : "Show the loaded checkpoints").foregroundStyle(Tone.label)
            }
        }
    }

    private var labels: (String, String) {
        (fee.current?.currencies.nativeLabel ?? model.coin, fee.current?.currencies.reserveLabel ?? "VRSC")
    }

    private var controls: some View {
        HStack(spacing: 16) {
            // A chain without a reserve currency has one series, so there is nothing to choose.
            if fee.hasReserve {
                Picker("Currency", selection: $state.fee.currency) {
                    Text("Both").tag(FeePoolState.Currency.both)
                    Text(labels.0).tag(FeePoolState.Currency.native)
                    Text(labels.1).tag(FeePoolState.Currency.reserve)
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 260)
                .help("Key v cycles the currencies. Each choice has its own vertical range.")
            }
            Picker("Scale", selection: $state.fee.scale) {
                Text("Linear from zero").tag(FeePoolState.Scale.zero)
                Text("Linear, fitted").tag(FeePoolState.Scale.fit)
                Text("Log-like").tag(FeePoolState.Scale.log)
            }
            .pickerStyle(.segmented)
            .frame(maxWidth: 340)
            .help("Key z cycles the scales.")
            Toggle("Detail", isOn: $state.fee.detail)
                .help("Up to 2,000 checkpoints instead of 100. Key t switches.")
        }
        .controlSize(.small)
    }

    private var legendItems: [ChartLegend.Item] {
        var items: [ChartLegend.Item] = [.init(shape: .line, color: ChartColors.block, text: "\(labels.0) in the fee pool")]
        if fee.hasReserve {
            items.append(.init(shape: .line, color: ChartColors.reserve, text: "\(labels.1) in the fee pool"))
        }
        return items
    }

    @ViewBuilder
    private var legend: some View {
        ChartLegend(items: legendItems)
        if fee.scale == .log {
            Text("Log-like scale: linear from 0 to 0.0001, then logarithmic, so that zero stays on the axis. Labels are whole-currency amounts at powers of ten.")
                .font(.caption).foregroundStyle(.secondary)
        } else if fee.scale == .fit {
            Text("Fitted scale: the vertical axis does not start at zero. It shows small changes.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var progress: some View {
        let loaded = fee.scheduled.filter { fee.checkpoints[$0]?.known == true }.count
        let read = fee.scheduled.filter { fee.checkpoints[$0] != nil }.count
        HStack {
            if fee.loading { ProgressView().controlSize(.small) }
            Text("\(loaded) of \(fee.requested) sampled checkpoints have a balance. \(read) of \(fee.requested) are read.")
                .monospacedDigit()
        }
        .font(.callout)
        let blocks = state.page?.samples.count ?? 0
        Text(
            fee.detail && blocks <= 2_000
                ? "Detail mode includes every block of this range."
                : "The chart samples at most \(fee.detail ? "2,000" : "100") of \(blocks) blocks. It can miss a peak between two samples. Shorten the span or turn on detail to look closer."
        )
        .font(.caption).foregroundStyle(.secondary)
        Text("The endpoints load first, then finer passes in random order. A block without a checkpoint, such as before activation, has no balance and breaks the line. This is a balance chart. It does not separate inflow from payout.")
            .font(.caption).foregroundStyle(.secondary)
        if let error = fee.error {
            Note(text: error, warning: true)
        }
    }

    @ViewBuilder
    private var selectedCheckpoint: some View {
        if let sample = state.selectedSample {
            HStack {
                Button {
                    state.move(-1)
                } label: {
                    Label("Older", systemImage: "chevron.left")
                }
                Button {
                    state.move(1)
                } label: {
                    Label("Newer", systemImage: "chevron.right")
                }
                Button("Open block") { model.open(.block(id: sample.hash)) }
                Spacer()
            }
            .buttonStyle(.lamp)
            .controlSize(.small)
            FieldRow(label: "Height", value: Format.integer(sample.height), route: .block(id: sample.hash))
            FieldRow(label: "Time", value: Format.date(sample.time))
            if let checkpoint = fee.checkpoints[sample.hash] {
                amounts(checkpoint)
            } else {
                Text("This block's checkpoint has not loaded.").foregroundStyle(.secondary)
            }
        } else {
            Text("No checkpoint with a balance has loaded yet.").foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private func amounts(_ checkpoint: FeePoolCheckpoint) -> some View {
        if checkpoint.known {
            FieldRow(label: labels.0, value: ChartFormat.sats(checkpoint.nativeSats) + " " + labels.0)
            if fee.hasReserve {
                FieldRow(label: labels.1, value: ChartFormat.sats(checkpoint.reserveSats) + " " + labels.1)
            }
        } else if let problem = checkpoint.problem {
            Note(text: "The checkpoint could not be read: \(problem). No zero balance is assumed.", warning: true)
        } else {
            Text("This block's coinbase has no fee pool checkpoint. Blocks before activation have none. No zero balance is assumed.")
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var current: some View {
        if let current = fee.current {
            let checkpoint = current.checkpoint
            FieldRow(label: "Latest observed block", value: Format.integer(checkpoint.height), route: .block(id: checkpoint.hash))
            FieldRow(label: "Time", value: Format.date(checkpoint.time))
            amounts(checkpoint)
            Text("This balance is at the latest block when the app last read it, not at the end of the chart range.")
                .font(.caption).foregroundStyle(.secondary)
        } else if let error = fee.currentError {
            Note(text: "The current fee pool is not available: \(error). No zero balance is assumed.", warning: true)
        } else {
            Text("Reading the latest block's checkpoint.").foregroundStyle(.secondary)
        }
        Button("Read again") { state.fee.reset(keepCheckpoints: true) }
            .controlSize(.small)
    }

    private var table: some View {
        let rows = (state.page?.samples ?? []).reversed().compactMap { sample in
            fee.checkpoints[sample.hash].map { FeePoolPoint(sample: sample, checkpoint: $0) }
        }
        return Table(rows) {
            TableColumn(ChartFormat.columnName("Height")) { Text(String($0.sample.height)).monospacedDigit() }
                .width(min: 70, ideal: 80)
            TableColumn(ChartFormat.columnName("Time")) { Text(Format.date($0.sample.time)).monospacedDigit() }
                .width(min: 150, ideal: 180)
            TableColumn(ChartFormat.columnName(labels.0)) { Text($0.checkpoint.known ? ChartFormat.sats($0.checkpoint.nativeSats) : "No checkpoint").monospacedDigit() }
            TableColumn(ChartFormat.columnName(labels.1)) { point in
                Text(point.checkpoint.known && fee.hasReserve ? ChartFormat.sats(point.checkpoint.reserveSats) : "–").monospacedDigit()
            }
        }
        .frame(height: 280)
    }
}

extension FeePoolPoint: Identifiable {
    var id: String { sample.hash }
}

extension ChartFormat {
    /// Satoshis as an exact amount with eight decimals. A missing amount says so.
    static func sats(_ value: Int64?) -> String {
        guard let value else { return "Unknown" }
        let whole = value / 100_000_000
        let fraction = String(format: "%08lld", value % 100_000_000)
        return "\(whole.formatted(.number)).\(fraction)"
    }
}
