import CarbideCore
import Charts
import SwiftUI

/// A currency's supply and the value of its reserves across time, with one cursor through
/// all of them.
///
/// The node gives the price of a basket in each reserve. A line of that price goes up
/// when the reserve loses value, which a person reads the other way. Thus the card shows
/// the basket and each reserve in one unit that the person selects, as lines on one
/// scale that start at 100. Dragging the slider moves every figure together. Each point
/// is the currency's recorded state at a real block.
struct CurrencyOverTimeCard: View {
    let currency: CurrencyDetail
    /// The reserve that values are shown in. Nil for a currency with no reserves.
    var unit: String?
    /// Asks for the exact state at the block under the cursor.
    var onChoose: ((UInt64) -> Void)?

    @Environment(AppModel.self) private var model
    @State private var span = Span.year
    @State private var points: [CurrencyPoint] = []
    @State private var index: Double = 0
    @State private var failure: String?
    @State private var loading = true
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    enum Span: String, CaseIterable, Identifiable {
        case month = "30 days"
        case quarter = "90 days"
        case year = "1 year"
        case all = "All"
        var id: String { rawValue }

        /// Blocks back from the tip, at Verus's one-minute block target.
        var blocks: UInt64? {
            switch self {
            case .month: 30 * 1440
            case .quarter: 90 * 1440
            case .year: 365 * 1440
            case .all: nil
            }
        }
    }

    private let samples: UInt32 = 90

    var body: some View {
        let _ = showsNames
        Card("Over time", trailing: "\(samples) samples") {
            HStack {
                Picker("Span", selection: $span) {
                    ForEach(Span.allCases) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .frame(maxWidth: 340)
                Spacer()
                if let point = selected {
                    VStack(alignment: .trailing, spacing: 1) {
                        Text(dateLabel(point)).font(.headline)
                        Text("block \(Format.integer(point.height))")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    if let onChoose {
                        Button("Show Exact State") { onChoose(point.height) }
                            .help("Read the state at block \(Format.integer(point.height)) and check its imports and exports")
                    }
                }
            }

            if loading {
                ProgressView().frame(maxWidth: .infinity, minHeight: 120)
            } else if let failure {
                Label(failure, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.secondary)
            } else if points.count < 2 {
                Text("The range contains no recorded data.").foregroundStyle(.secondary)
            } else {
                Slider(
                    value: Binding(get: { index }, set: { index = $0.rounded() }),
                    in: 0...Double(points.count - 1))
                    .accessibilityLabel("Date")
                VStack(spacing: 0) {
                    row(title: "Supply", subtitle: currency.name, values: points.map(\.supply), digits: 2)
                    if let unit, reserveIDs.contains(unit) {
                        Divider()
                        indexed(unit)
                    }
                }
                Divider()
                activityRow
                if flatSince != nil {
                    Text(flatSince!).font(.caption).foregroundStyle(.secondary)
                }
                if let point = selected, !point.reserves.isEmpty {
                    Divider().padding(.vertical, 4)
                    PoolBatchView(currencyName: currency.name, point: point, unit: unit, name: reserveName)
                }
            }
        }
        .task(id: "\(span.rawValue)|\(model.generation)|\(model.status?.blocks ?? 0 > 0)") {
            await load()
        }
        .task(id: reserveIDs) {
            await model.resolveNames(reserveIDs)
        }
    }

    private var selected: CurrencyPoint? {
        guard !points.isEmpty else { return nil }
        return points[min(Int(index), points.count - 1)]
    }

    private var reserveIDs: [String] {
        points.last?.reserves.map(\.currencyId) ?? []
    }

    private func reserveName(_ id: String) -> String {
        // A currency's name is its identity's name without the trailing "@". With names
        // off in the View menu, the ID.
        guard showsNames else { return Format.short(id, head: 6, tail: 4) }
        return currency.reserves.first { $0.currencyId == id }?.name
            ?? knownName(id, model: model)
    }

    private func dateLabel(_ point: CurrencyPoint) -> String {
        guard let time = point.time else { return "Block \(Format.integer(point.height))" }
        let date = Date(timeIntervalSince1970: TimeInterval(time))
            .formatted(date: .abbreviated, time: .omitted)
        return Int(index) == points.count - 1 ? "\(date) · latest" : date
    }

    /// Says so when nothing has changed for the latter part of the range, which a flat
    /// line alone leaves ambiguous.
    private var flatSince: String? {
        guard let last = points.last, let lastSupply = last.supply else { return nil }
        guard let firstSame = points.firstIndex(where: { $0.supply == lastSupply }),
            firstSame < points.count - 3, firstSame > 0, let time = points[firstSame].time
        else { return nil }
        let date = Date(timeIntervalSince1970: TimeInterval(time))
            .formatted(date: .abbreviated, time: .omitted)
        return "The supply has not changed since approximately \(date). The sampled blocks after that date show no conversions."
    }

    /// A sample counts as activity when the latest batch differs from the previous sample's.
    private var changed: [Bool] {
        let marks = points.map { point in
            point.reserves.map { "\($0.lastIn)/\($0.lastOut)" }.joined(separator: "|") + "|\(point.supply ?? 0)"
        }
        return marks.indices.map { $0 > 0 && marks[$0] != marks[$0 - 1] }
    }

    /// Marks where conversions happened between samples, on the same axis as the traces.
    private var activityRow: some View {
        let flags = changed
        let position = min(Int(index), max(points.count - 1, 0))
        return HStack(spacing: 14) {
            VStack(alignment: .leading, spacing: 1) {
                Text("Activity").fontWeight(.semibold)
                Text("conversions found").font(.caption).foregroundStyle(.secondary)
            }
            .frame(width: 130, alignment: .leading)
            Canvas { context, size in
                let count = max(flags.count - 1, 1)
                for (offset, active) in flags.enumerated() where active {
                    let x = size.width * CGFloat(offset) / CGFloat(count)
                    context.fill(
                        Path(CGRect(x: x - 1.5, y: 6, width: 3, height: size.height - 12)),
                        with: .color(.purple))
                }
                let cursor = size.width * CGFloat(position) / CGFloat(count)
                context.fill(
                    Path(CGRect(x: cursor - 0.75, y: 0, width: 1.5, height: size.height)),
                    with: .color(.primary.opacity(0.55)))
            }
            .frame(height: 30)
            Text("\(flags.filter { $0 }.count) of \(max(flags.count - 1, 0)) intervals")
                .font(.caption).foregroundStyle(.secondary)
                .frame(width: 130, alignment: .trailing)
        }
        .padding(.vertical, 7)
    }

    // MARK: The lines from 100

    private struct IndexLine: Identifiable {
        let id: String
        let name: String
        let basket: Bool
        let color: Color
        /// The value in the unit at each sample, as a part of the first known value, times 100.
        let values: [Double?]
    }

    private static let palette: [Color] = [.orange, .purple, .green, .red, .cyan, .pink, .brown, .mint, .indigo, .yellow]

    private func lines(_ unit: String) -> [IndexLine] {
        func price(_ point: CurrencyPoint, _ id: String) -> Double? {
            point.reserves.first { $0.currencyId == id }?.priceInReserve.flatMap { $0 > 0 ? $0 : nil }
        }
        func rebased(_ raw: [Double?]) -> [Double?] {
            guard let base = raw.compactMap({ $0 }).first, base > 0 else { return raw.map { _ in nil } }
            return raw.map { $0.map { $0 / base * 100 } }
        }
        var result = [
            IndexLine(
                id: "basket", name: currency.name, basket: true, color: .primary,
                values: rebased(points.map { price($0, unit) }))
        ]
        for (position, id) in reserveIDs.enumerated() where id != unit {
            result.append(
                IndexLine(
                    id: id, name: reserveName(id), basket: false,
                    color: Self.palette[position % Self.palette.count],
                    values: rebased(points.map { BasketMath.worth(price($0, id), price($0, unit)) })))
        }
        return result
    }

    private func indexed(_ unit: String) -> some View {
        let lines = lines(unit)
        let position = min(Int(index), max(points.count - 1, 0))
        let known = lines.flatMap { $0.values.compactMap { $0 } }
        let low = min(known.min() ?? 90, 95)
        let high = max(known.max() ?? 110, 105)
        return VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .top, spacing: 14) {
                VStack(alignment: .leading, spacing: 1) {
                    Text("Value").fontWeight(.semibold)
                    Text("in \(reserveName(unit))").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                    Text("start of range = 100").font(.caption).foregroundStyle(.secondary)
                }
                .frame(width: 130, alignment: .leading)
                Chart {
                    RuleMark(y: .value("Start", 100)).foregroundStyle(Color.secondary.opacity(0.6))
                    ForEach(lines) { line in
                        ForEach(Array(line.values.enumerated()), id: \.offset) { offset, value in
                            if let value {
                                LineMark(
                                    x: .value("Sample", offset), y: .value("Value", value),
                                    series: .value("Currency", line.id))
                                    .foregroundStyle(line.color)
                                    .lineStyle(StrokeStyle(lineWidth: line.basket ? 2.6 : 1.5))
                            }
                        }
                    }
                    RuleMark(x: .value("Sample", position))
                        .foregroundStyle(Color.primary.opacity(0.55))
                        .lineStyle(StrokeStyle(lineWidth: 1.5))
                }
                .chartXAxis(.hidden)
                .chartYAxis {
                    // The labels are inside the plot, so that the plot has the width of the
                    // lines above and below it and one cursor goes through all of them.
                    AxisMarks(preset: .inset, position: .leading)
                }
                .chartXScale(domain: 0...max(points.count - 1, 1))
                .chartYScale(domain: (low - 3)...(high + 3))
                .chartLegend(.hidden)
                .frame(height: 220)
                .accessibilityLabel("The value of \(currency.name) and of each reserve in \(reserveName(unit)), from 100 at the start")
                VStack(alignment: .leading, spacing: 5) {
                    ForEach(lines) { line in
                        HStack(spacing: 6) {
                            RoundedRectangle(cornerRadius: 1.5).fill(line.color)
                                .frame(width: 14, height: line.basket ? 4 : 2.5)
                            Text(line.name).fontWeight(line.basket ? .semibold : .regular).lineLimit(1)
                            Spacer(minLength: 4)
                            Text(line.values[position].map(Self.changeText) ?? "–").monospacedDigit()
                        }
                        .font(.caption)
                    }
                    Text("at the cursor").font(.caption).foregroundStyle(.secondary)
                }
                .frame(width: 130)
            }
            Text(
                "A line at 80 means that the currency lost 20 of each 100 of its value in \(reserveName(unit)) since "
                    + "the start of the range. The heavy line is \(currency.name). The values are the pool's own rates.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, 7)
    }

    private static func changeText(_ value: Double) -> String {
        let change = value - 100
        return "\(change >= 0 ? "+" : "−")\(abs(change).formatted(.number.precision(.fractionLength(1))))"
    }

    @ViewBuilder
    private func row(title: String, subtitle: String, values: [Double?], digits: Int) -> some View {
        let position = min(Int(index), values.count - 1)
        let known = values.compactMap { $0 }
        let low = known.min() ?? 0
        let high = known.max() ?? 1
        let pad = high > low ? (high - low) * 0.12 : max(abs(high) * 0.05, 0.0001)
        HStack(spacing: 14) {
            VStack(alignment: .leading, spacing: 1) {
                Text(title).fontWeight(.semibold).lineLimit(1)
                Text(subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            .frame(width: 130, alignment: .leading)

            Chart {
                ForEach(Array(values.enumerated()), id: \.offset) { offset, value in
                    if let value {
                        LineMark(x: .value("Sample", offset), y: .value(title, value))
                            .foregroundStyle(Color.accentColor)
                    }
                }
                RuleMark(x: .value("Sample", position))
                    .foregroundStyle(Color.primary.opacity(0.55))
                    .lineStyle(StrokeStyle(lineWidth: 1.5))
            }
            .chartXAxis(.hidden)
            .chartYAxis(.hidden)
            .chartXScale(domain: 0...max(values.count - 1, 1))
            .chartYScale(domain: (low - pad)...(high + pad))
            .frame(height: 42)

            VStack(alignment: .trailing, spacing: 1) {
                if let value = values[position] {
                    Text(value.formatted(.number.precision(.fractionLength(0...digits))))
                        .monospacedDigit()
                    Text(change(from: value, to: values.last ?? nil, latest: position == values.count - 1))
                        .font(.caption).foregroundStyle(.secondary).monospacedDigit()
                } else {
                    Text("–").foregroundStyle(.secondary)
                }
            }
            .frame(width: 130, alignment: .trailing)
        }
        .padding(.vertical, 7)
    }

    private func change(from value: Double, to latest: Double?, latest isLatest: Bool) -> String {
        guard !isLatest, let latest, value != 0 else { return "latest" }
        let percent = (latest - value) / value
        return "\(percent >= 0 ? "+" : "")\(percent.formatted(.percent.precision(.fractionLength(1)))) since"
    }

    private func load() async {
        guard let tip = model.status?.blocks, tip > 0 else { return }
        loading = true
        failure = nil
        let start = max(currency.startBlock ?? 1, 1)
        let from = span.blocks.map { tip > $0 ? max(tip - $0, start) : start } ?? start
        do {
            let loaded = try await model.requireExplorer().currencyHistory(
                name: currency.currencyId ?? currency.name, fromHeight: from, toHeight: tip,
                samples: samples)
            points = loaded
            index = Double(max(loaded.count - 1, 0))
        } catch is CancellationError {
            return
        } catch {
            points = []
            failure = describe(error)
        }
        loading = false
    }
}
