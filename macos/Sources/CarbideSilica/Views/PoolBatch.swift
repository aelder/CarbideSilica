import CarbideCore
import SwiftUI

/// What the most recent conversion batch did to a pool, at the chosen moment, as a
/// statement: one row for each reserve that moved.
///
/// A hollow bar to the left is a reserve that went out of the pool. A solid bar to the
/// right is a reserve that came in. The bars are the value of each amount in the unit
/// reserve, at the pool's own rate, so that amounts of different currencies share one
/// scale. Conversions are batched and all pass through the pool, so the chain records what
/// went in and out for each reserve, not which reserve was swapped for which.
struct PoolBatchView: View {
    let currencyName: String
    let point: CurrencyPoint
    /// The reserve that amounts are valued in.
    var unit: String?
    let name: (String) -> String

    private struct Flow: Identifiable {
        let id: String
        let amountIn: Double
        let amountOut: Double
        /// The value of the larger amount in the unit. Nil when a price is missing.
        let value: Double?
    }

    private var flows: [Flow] {
        let unitPrice = point.reserves.first { $0.currencyId == unit }?.priceInReserve
        return point.reserves.filter { $0.lastIn > 0 || $0.lastOut > 0 }.map { reserve in
            let worth = reserve.currencyId == unit ? 1 : BasketMath.worth(reserve.priceInReserve, unitPrice)
            return Flow(
                id: reserve.currencyId, amountIn: reserve.lastIn, amountOut: reserve.lastOut,
                value: worth.map { max(reserve.lastIn, reserve.lastOut) * $0 })
        }
    }

    var body: some View {
        let flows = flows
        let largest = max(flows.compactMap(\.value).max() ?? 0, 1e-12)
        VStack(alignment: .leading, spacing: 8) {
            Text("Last conversion batch at this date").font(.subheadline.weight(.semibold))
            if flows.isEmpty {
                Text("The last batch before this block had no conversions.")
                    .font(.callout).foregroundStyle(.secondary)
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(flows.enumerated()), id: \.element.id) { position, flow in
                        if position > 0 { Divider() }
                        HStack(spacing: 14) {
                            Text(name(flow.id)).fontWeight(.semibold).lineLimit(1)
                                .frame(width: 130, alignment: .leading)
                            Canvas { context, size in
                                let middle = size.width / 2
                                context.fill(
                                    Path(CGRect(x: middle - 0.5, y: 0, width: 1, height: size.height)),
                                    with: .color(.secondary.opacity(0.7)))
                                let length = max(CGFloat((flow.value ?? 0) / largest) * (middle - 2), 2)
                                let incoming = flow.amountIn >= flow.amountOut
                                let bar = CGRect(
                                    x: incoming ? middle : middle - length, y: 1, width: length, height: size.height - 2)
                                if incoming {
                                    context.fill(Path(bar), with: .color(.accentColor))
                                } else {
                                    context.stroke(Path(bar), with: .color(.primary), lineWidth: 1.3)
                                }
                            }
                            .frame(height: 11)
                            .accessibilityHidden(true)
                            Text(line(flow)).font(.callout).monospacedDigit().lineLimit(1)
                                .frame(width: 330, alignment: .trailing)
                        }
                        .padding(.vertical, 6)
                        .accessibilityElement(children: .combine)
                    }
                }
                Text(
                    "A solid bar to the right came into the pool. A hollow bar to the left went out. "
                        + (unit == nil ? "" : "The bars are the value of each amount in \(name(unit ?? "")), at the pool's own rate. ")
                        + "The pool does conversions in batches. The chain records the quantity in and out for each "
                        + "reserve. It does not record the pairs.")
                    .font(.caption).foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func line(_ flow: Flow) -> String {
        let incoming = flow.amountIn >= flow.amountOut
        var text = "\(incoming ? "in" : "out") \(Format.amount(incoming ? flow.amountIn : flow.amountOut)) \(name(flow.id))"
        if let value = flow.value, let unit, flow.id != unit {
            text += " · \(BasketMath.text(value)) \(name(unit))"
        }
        return text
    }
}
