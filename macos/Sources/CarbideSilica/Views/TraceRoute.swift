import CarbideCore
import SwiftUI

/// A traced transfer as a receipt, an axis of blocks and a batch strip.
///
/// Most transfers end in the same way: a conversion with a verified result. Thus the page
/// shows what differs between them. The receipt reads from the amount sent to the amount
/// received, with the fee and the rate. The axis has one tick for each block, so that the
/// wait for an export is a length, and stages in one block are one mark. The strip shows
/// the transfer as one member of its export batch.
struct TraceRoute: View {
    let trace: TransferTrace
    let live: Bool

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    var body: some View {
        let _ = showsNames
        VStack(alignment: .leading, spacing: 12) {
            ForEach(Array(trace.legs.enumerated()), id: \.offset) { index, leg in
                receipt(leg, number: trace.legs.count > 1 ? index + 1 : nil)
            }
            let marks = TraceAxis.marks(trace.moments)
            if !marks.isEmpty {
                Canvas { context, size in
                    TraceAxis.draw(context, size, marks: marks, reached: trace.confidence == .verified, name: stageName)
                }
                .frame(height: 74)
                .accessibilityLabel(spoken(marks))
            }
            batch
        }
    }

    // MARK: The receipt

    private func money(_ amount: CurrencyAmount) -> String {
        "\(Format.amount(amount.amount)) \(model.currencyName(amount.currency))"
    }

    private func list(_ amounts: [CurrencyAmount]) -> String {
        amounts.isEmpty ? "not known" : StatementText.list(amounts.map(money))
    }

    @ViewBuilder
    private func receipt(_ leg: TraceLeg, number: Int?) -> some View {
        let rate = leg.rates.first { $0.purpose == "Principal" } ?? leg.rates.first
        VStack(alignment: .leading, spacing: 6) {
            if let number {
                Text("Leg \(number)").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            }
            HStack(spacing: 6) {
                box("Sent", leg.principalKnown ? list(leg.principal) : "not known")
                if let rate {
                    arrow(feeShare(leg))
                    box("Converted in \(leg.pool.map { model.currencyName($0) } ?? "the pool")", money(rate.netInput))
                }
                arrow(nil)
                if let result = leg.result {
                    box(result, list(leg.resultAmounts), strong: true)
                } else if let burned = leg.burned {
                    box("Burned", money(burned), strong: true)
                } else {
                    box(leg.stage, live ? "The search continues." : "No result output is matched.", open: true)
                }
            }
            if let rate, let sats = rate.rateSats {
                Text(
                    "Rate of the batch: \(rate.exact ? "" : "approximately ")\(Format.coins(sats)) "
                        + "\(model.currencyName(rate.output.currency)) for 1 \(model.currencyName(rate.netInput.currency)).")
                    .font(.callout)
                    .textSelection(.enabled)
            }
            let fees = feeLine(leg)
            if !fees.isEmpty {
                Text(fees).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
            }
        }
    }

    private func feeShare(_ leg: TraceLeg) -> String? {
        guard let fee = leg.conversionFee, let sent = leg.principal.first(where: { $0.currency == fee.currency }),
            sent.amount > 0
        else { return nil }
        let percent = fee.amount / sent.amount * 100
        return "fee \(percent.formatted(.number.precision(.fractionLength(percent < 0.1 ? 3 : 2))))%"
    }

    private func feeLine(_ leg: TraceLeg) -> String {
        var parts: [String] = []
        if let fee = leg.conversionFee { parts.append("Conversion fee: \(money(fee)).") }
        if let budget = leg.transferFeeBudget {
            parts.append("Transfer fee budget, apart from the amount: \(money(budget)).")
        }
        return parts.joined(separator: " ")
    }

    private func box(_ title: String, _ value: String, strong: Bool = false, open: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title.uppercased()).font(.caption2.weight(.medium)).foregroundStyle(.secondary).lineLimit(1)
            Text(value)
                .font(.callout.weight(strong ? .semibold : .medium))
                .monospacedDigit()
                .foregroundStyle(open ? .secondary : .primary)
                .lineLimit(2)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(strong ? Color.accentColor.opacity(0.12) : .clear, in: RoundedRectangle(cornerRadius: 6))
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .stroke(
                    strong ? Color.accentColor : Color.secondary.opacity(0.4),
                    style: StrokeStyle(lineWidth: 1, dash: open ? [5, 4] : [])))
    }

    private func arrow(_ label: String?) -> some View {
        VStack(spacing: 1) {
            Text(label ?? " ").font(.caption2).foregroundStyle(.secondary).lineLimit(1).fixedSize()
            Image(systemName: "arrow.right").font(.caption).foregroundStyle(.secondary)
        }
        .accessibilityHidden(true)
    }

    // MARK: The axis

    private func stageName(_ stage: TraceStage) -> String {
        switch stage {
        case .sent: "Sent"
        case .exported: "Exported"
        case .imported: "Imported"
        case .result: trace.legs.last?.result ?? "Result"
        }
    }

    private func spoken(_ marks: [TraceAxis.Mark]) -> String {
        marks.map { mark in
            let names = StatementText.list(mark.stages.map(stageName))
            return mark.height.map { "\(names) in block \(Format.integer($0))" } ?? "\(names), block not known"
        }
        .joined(separator: ". ")
    }

    // MARK: The batch

    @ViewBuilder
    private var batch: some View {
        if let batch = trace.batch {
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 10) {
                    Text("Export batch").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    HStack(spacing: 3) {
                        ForEach(0..<Int(min(batch.size, 40)), id: \.self) { index in
                            RoundedRectangle(cornerRadius: 2)
                                .fill(index == Int(batch.place) ? Color.accentColor : Color.secondary.opacity(0.45))
                                .frame(width: 11, height: 11)
                        }
                    }
                    .accessibilityHidden(true)
                    Text(
                        batch.size == 1
                            ? "This transfer was alone in its export."
                            : "Transfer \(batch.place + 1) of \(batch.size) in the export.")
                        .font(.callout)
                }
                if trace.confidence == .verified, trace.legs.contains(where: { !$0.rates.isEmpty }) {
                    Text(candidateLine).font(.callout).foregroundStyle(.secondary)
                }
            }
        }
    }

    private var candidateLine: String {
        let same = "All conversions of the batch got the same rate."
        switch trace.candidates {
        case 0: return "\(same) No other output of the import went to the same address."
        case 1: return "\(same) 1 other output of the import went to the same address. The replay finds this one."
        default:
            return "\(same) \(trace.candidates) other outputs of the import went to the same address. The replay finds this one."
        }
    }
}

/// The stages of a transfer on an axis of blocks.
enum TraceAxis {
    /// The stages of one block. A stage with no known block is a mark of its own.
    struct Mark {
        var stages: [TraceStage]
        var height: UInt64?
        var time: UInt64?
    }

    /// Joins the stages that are in one block, in the order of the trail.
    static func marks(_ moments: [TraceMoment]) -> [Mark] {
        var marks: [Mark] = []
        for moment in moments {
            if let height = moment.height, let last = marks.indices.last, marks[last].height == height {
                if !marks[last].stages.contains(moment.stage) { marks[last].stages.append(moment.stage) }
            } else {
                marks.append(Mark(stages: [moment.stage], height: moment.height, time: moment.time))
            }
        }
        return marks
    }

    static func wait(blocks: UInt64, seconds: UInt64?) -> String {
        var text = "\(blocks) \(blocks == 1 ? "block" : "blocks")"
        if let seconds { text += " · \(MempoolText.span(Double(seconds)))" }
        return text
    }

    static func draw(
        _ context: GraphicsContext, _ size: CGSize, marks: [Mark], reached: Bool, name: (TraceStage) -> String
    ) {
        let placed = marks.filter { $0.height != nil }
        let open = marks.filter { $0.height == nil }
        let y: CGFloat = 36
        let left: CGFloat = 46
        // Room at the right for the label of the last mark, and for a dashed end.
        let right = size.width - (reached ? 230 : 150)
        guard let first = placed.first?.height, let last = placed.last?.height else {
            context.draw(
                Text(StatementText.list(marks.flatMap(\.stages).map(name)) + ": not in a block that this node has")
                    .font(.callout).foregroundStyle(.secondary),
                at: CGPoint(x: left, y: y), anchor: .leading)
            return
        }
        let span = max(Double(last - first), 1)
        func x(_ height: UInt64) -> CGFloat {
            last == first ? left : left + CGFloat(Double(height - first) / span) * (right - left)
        }
        // The line and one tick for each block.
        if last > first {
            context.fill(
                Path(CGRect(x: left, y: y - 1.5, width: right - left, height: 3)), with: .color(.accentColor))
            if last - first <= 60 {
                for height in first...last {
                    context.fill(
                        Path(CGRect(x: x(height) - 0.5, y: y - 5, width: 1, height: 10)),
                        with: .color(.secondary.opacity(0.8)))
                }
            }
        }
        if !reached || !open.isEmpty {
            var dashes = Path()
            dashes.move(to: CGPoint(x: x(last), y: y))
            dashes.addLine(to: CGPoint(x: size.width - 8, y: y))
            context.stroke(dashes, with: .color(.secondary), style: StrokeStyle(lineWidth: 1.5, dash: [5, 5]))
        }
        for (index, mark) in placed.enumerated() {
            guard let height = mark.height else { continue }
            let at = x(height)
            let final = index == placed.count - 1
            let radius: CGFloat = final ? 8 : 6.5
            let done = final && reached && mark.stages.contains(.result)
            context.fill(
                Path(ellipseIn: CGRect(x: at - radius, y: y - radius, width: radius * 2, height: radius * 2)),
                with: .color(done ? .green : .accentColor))
            var title = StatementText.list(mark.stages.map(name))
            if mark.stages.count > 1 { title = StatementText.capital(title.lowercased()) }
            let trailing = final && placed.count > 1
            context.draw(
                Text(title).font(.callout.weight(.semibold)),
                at: CGPoint(x: trailing ? at + 14 : at, y: y - 20), anchor: trailing ? .leading : .center)
            context.draw(
                Text("block \(Format.integer(height))" + (mark.stages.count > 1 ? ", one block" : ""))
                    .font(.caption).foregroundStyle(.secondary),
                at: CGPoint(x: trailing ? at + 14 : at, y: y + 20), anchor: trailing ? .leading : .center)
            // The wait from the mark before.
            if index > 0, let before = placed[index - 1].height, height > before {
                let seconds = mark.time.flatMap { end in placed[index - 1].time.map { end > $0 ? end - $0 : 0 } }
                context.draw(
                    Text(wait(blocks: height - before, seconds: seconds)).font(.caption),
                    at: CGPoint(x: (x(before) + at) / 2, y: y - 14), anchor: .center)
            }
        }
        if let pending = open.first {
            context.draw(
                Text(StatementText.list(pending.stages.map(name)) + ": on a system that this node does not have")
                    .font(.caption).foregroundStyle(.secondary),
                at: CGPoint(x: size.width - 8, y: y - 14), anchor: .trailing)
        }
    }
}
