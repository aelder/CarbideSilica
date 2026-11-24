import CarbideCore
import SwiftUI

/// What the most recent conversion batch did to a pool, at the chosen moment.
///
/// The pool sits in the centre with its reserves around it. A line toward the pool is a
/// reserve being paid in; a line away is a reserve being paid out. Widths are compared in
/// the pool's own units, since the reserves' units are not comparable with each other.
///
/// Conversions are batched and all pass through the pool, so the chain records what went
/// in and out per reserve, not which reserve was swapped for which.
struct PoolBatchView: View {
    let currencyName: String
    let point: CurrencyPoint
    let name: (String) -> String

    private struct Flow {
        let id: String
        let amountIn: Double
        let amountOut: Double
        /// Larger of the two, in pool units.
        let weight: Double
    }

    private var flows: [Flow] {
        point.reserves.map { reserve in
            let price = max(reserve.priceInReserve ?? 0, 1e-12)
            return Flow(
                id: reserve.currencyId, amountIn: reserve.lastIn, amountOut: reserve.lastOut,
                weight: max(reserve.lastIn, reserve.lastOut) / price)
        }
    }

    private var active: Bool { flows.contains { $0.amountIn > 0 || $0.amountOut > 0 } }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Last conversion batch at this date").font(.subheadline.weight(.semibold))
            if active {
                Text(sentence).font(.callout)
                Canvas { context, size in
                    draw(context, size)
                }
                .frame(height: 220)
                .accessibilityLabel(sentence)
                Text("The pool does conversions in batches. The chain records the quantity in and out for each reserve. It does not record the pairs.")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                Text("The last batch before this block had no conversions.")
                    .font(.callout).foregroundStyle(.secondary)
            }
        }
    }

    private var sentence: String {
        let list = { (items: [(String, Double)]) in
            items.map { "\(Format.amount($0.1)) \($0.0)" }.joined(separator: " and ")
        }
        let ins = flows.filter { $0.amountIn > 0 }.map { (name($0.id), $0.amountIn) }
        let outs = flows.filter { $0.amountOut > 0 }.map { (name($0.id), $0.amountOut) }
        switch (ins.isEmpty, outs.isEmpty) {
        case (false, false): return "\(list(ins)) went into the pool. \(list(outs)) came out of the pool."
        case (false, true): return "\(list(ins)) went into the pool for \(currencyName)."
        case (true, false): return "\(list(outs)) came out of the pool for \(currencyName)."
        default: return ""
        }
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize) {
        let centre = CGPoint(x: size.width / 2, y: size.height / 2)
        let radiusX = min(size.width / 2 - 90, 260)
        let radiusY = size.height / 2 - 26
        let heaviest = max(flows.map(\.weight).max() ?? 1, 1e-12)
        for (index, flow) in flows.enumerated() {
            let angle = Double(index) / Double(max(flows.count, 1)) * 2 * .pi - .pi / 2 + (flows.count == 2 ? .pi / 2 : .pi / Double(flows.count))
            let node = CGPoint(x: centre.x + CGFloat(cos(angle)) * radiusX, y: centre.y + CGFloat(sin(angle)) * radiusY)
            let moving = flow.amountIn > 0 || flow.amountOut > 0
            let incoming = flow.amountIn >= flow.amountOut
            let color: Color = !moving ? .secondary : incoming ? .accentColor : .purple
            let width = moving ? 2 + CGFloat(flow.weight / heaviest) * 10 : 1
            let from = incoming ? node : centre
            let to = incoming ? centre : node
            context.stroke(
                Path { path in
                    path.move(to: from)
                    path.addLine(to: to)
                }, with: .color(color.opacity(moving ? 0.55 : 0.3)),
                style: StrokeStyle(lineWidth: width, lineCap: .round, dash: moving ? [] : [4, 4]))
            if moving {
                // An arrowhead two thirds of the way along, pointing the way the reserve moved.
                let t: CGFloat = 0.62
                let tip = CGPoint(x: from.x + (to.x - from.x) * t, y: from.y + (to.y - from.y) * t)
                let heading = atan2(to.y - from.y, to.x - from.x)
                let head = max(width, 5) + 5
                context.fill(
                    Path { path in
                        path.move(to: CGPoint(x: tip.x + cos(heading) * head, y: tip.y + sin(heading) * head))
                        path.addLine(to: CGPoint(x: tip.x + cos(heading + 2.4) * head, y: tip.y + sin(heading + 2.4) * head))
                        path.addLine(to: CGPoint(x: tip.x + cos(heading - 2.4) * head, y: tip.y + sin(heading - 2.4) * head))
                        path.closeSubpath()
                    }, with: .color(color))
            }
            let box = CGRect(x: node.x - 62, y: node.y - 19, width: 124, height: 38)
            let pill = Path(roundedRect: box, cornerRadius: 19)
            context.fill(pill, with: .color(Color(nsColor: .windowBackgroundColor)))
            context.stroke(pill, with: .color(moving ? color : .secondary.opacity(0.6)), lineWidth: moving ? 2 : 1.2)
            context.draw(
                Text(name(flow.id)).font(.callout.weight(.semibold)),
                at: CGPoint(x: node.x, y: node.y - (moving ? 7 : 0)), anchor: .center)
            if moving {
                context.draw(
                    Text(incoming ? "in \(Format.amount(flow.amountIn))" : "out \(Format.amount(flow.amountOut))")
                        .font(.caption2).foregroundStyle(color),
                    at: CGPoint(x: node.x, y: node.y + 9), anchor: .center)
            }
        }
        let hub = CGRect(x: centre.x - 60, y: centre.y - 22, width: 120, height: 44)
        let hubShape = Path(roundedRect: hub, cornerRadius: 9)
        context.fill(hubShape, with: .color(Color(nsColor: .windowBackgroundColor)))
        context.stroke(hubShape, with: .color(.primary), lineWidth: 2)
        context.draw(Text(currencyName).font(.headline), at: CGPoint(x: centre.x, y: centre.y - 6), anchor: .center)
        context.draw(
            Text("the pool").font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: centre.x, y: centre.y + 11), anchor: .center)
    }
}
