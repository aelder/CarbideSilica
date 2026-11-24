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
            Text("Latest conversion batch at this moment").font(.subheadline.weight(.semibold))
            if active {
                Text(sentence).font(.callout)
                Canvas { context, size in
                    draw(context, size)
                }
                .frame(height: 220)
                .accessibilityLabel(sentence)
                Text("Conversions run in batches through the pool. The chain records what each reserve paid in and received, not which was swapped for which.")
                    .font(.caption).foregroundStyle(.secondary)
            } else {
                Text("The most recent batch before this block moved nothing through the pool.")
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
        case (false, false): return "\(list(ins)) went in, and \(list(outs)) came out."
        case (false, true): return "\(list(ins)) went in, converted to \(currencyName)."
        case (true, false): return "\(list(outs)) came out, converted from \(currencyName)."
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

/// Where an identity came from: who probably paid for its registration, and the payments
/// the registration made to other identities.
///
/// Both are drawn dashed, because both are inferences from the registration transaction.
/// The payer funded its inputs; payments to identities are usually referrals or the
/// issuer's charge, but the chain does not label them.
struct IdentityOriginCard: View {
    let registration: IdentityVersion
    let identityAddress: String?

    @Environment(AppModel.self) private var model

    private struct Origin: Sendable {
        let payers: [String]
        let payments: [(String, Int64)]
        let spent: Int64?
    }

    var body: some View {
        if let txid = registration.txid {
            LoadView(key: "origin:\(txid)") { explorer in
                let transaction = try await explorer.transaction(txid: txid)
                return Self.origin(of: transaction, identity: identityAddress)
            } content: { origin in
                Card("Where it came from") {
                    Canvas { context, size in
                        draw(context, size, origin)
                    }
                    .frame(height: CGFloat(max(origin.payers.count, origin.payments.count, 1)) * 52 + 26)
                    .accessibilityLabel(sentence(origin))

                    Text(sentence(origin)).font(.callout)
                    HStack(spacing: 14) {
                        ConfidenceMark(level: .verified, label: "Registration")
                        ConfidenceMark(level: .candidate, label: "Payer and payments are inferred")
                        Spacer()
                        RouteLink(text: txid, route: .transaction(txid: txid))
                    }
                }
                .task(id: txid) {
                    await model.resolveNames(origin.payers + origin.payments.map(\.0))
                }
            }
            .frame(minHeight: 80)
        }
    }

    private static func origin(of transaction: TransactionDetail, identity: String?) -> Origin {
        var payers: [String] = []
        for address in transaction.inputs.flatMap(\.addresses) where !payers.contains(address) {
            payers.append(address)
        }
        let isIdentity = { (address: String) in address.hasPrefix("i") && address.count == 34 }
        var payments: [(String, Int64)] = []
        for output in transaction.outputs {
            guard let address = output.addresses.first, isIdentity(address), address != identity,
                !payers.contains(address), let sats = output.valueSats, sats > 0
            else { continue }
            if let existing = payments.firstIndex(where: { $0.0 == address }) {
                payments[existing].1 += sats
            } else {
                payments.append((address, sats))
            }
        }
        let known = transaction.inputs.compactMap(\.valueSats)
        let change = transaction.outputs
            .filter { output in output.addresses.first.map(payers.contains) ?? false }
            .compactMap(\.valueSats).reduce(0, +)
        let spent = known.count == transaction.inputs.count && !known.isEmpty ? known.reduce(0, +) - change : nil
        return Origin(payers: Array(payers.prefix(4)), payments: Array(payments.prefix(4)), spent: spent)
    }

    private func display(_ address: String) -> String {
        model.names[address] ?? Format.short(address, head: 8, tail: 6)
    }

    private var when: String {
        registration.time.map {
            Date(timeIntervalSince1970: TimeInterval($0)).formatted(date: .abbreviated, time: .omitted)
        } ?? "block \(Format.integer(registration.height))"
    }

    private func sentence(_ origin: Origin) -> String {
        var text = "Registered on \(when)"
        if let payer = origin.payers.first {
            text += ", probably paid for by \(display(payer))"
            if origin.payers.count > 1 { text += " and \(origin.payers.count - 1) more" }
        }
        if let spent = origin.spent, spent > 0 {
            text += ", at a cost of \(Format.coins(spent)) \(model.coin)"
        }
        text += "."
        if !origin.payments.isEmpty {
            let list = origin.payments.map { "\(Format.coins($0.1)) \(model.coin) to \(display($0.0))" }
            text += " The registration paid \(list.joined(separator: ", ")), which is usually a referral or the issuer's charge."
        }
        return text
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize, _ origin: Origin) {
        let midY = size.height / 2
        let centre = CGRect(x: size.width / 2 - 80, y: midY - 28, width: 160, height: 56)
        let sideWidth = min(size.width * 0.27, 220)
        let slot = { (index: Int, count: Int) -> CGFloat in
            size.height * (CGFloat(index) + 0.5) / CGFloat(max(count, 1))
        }
        let dashed = StrokeStyle(lineWidth: 1.6, dash: [5, 4])
        let box = { (frame: CGRect, title: String, subtitle: String) in
            let shape = Path(roundedRect: frame, cornerRadius: 8)
            context.fill(shape, with: .color(Color(nsColor: .windowBackgroundColor)))
            context.stroke(shape, with: .color(.secondary), style: dashed)
            context.draw(
                Text(title).font(.callout.weight(.semibold)),
                at: CGPoint(x: frame.midX, y: frame.midY - 7), anchor: .center)
            context.draw(
                Text(subtitle).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: frame.midX, y: frame.midY + 9), anchor: .center)
        }
        for (index, payer) in origin.payers.enumerated() {
            let y = slot(index, origin.payers.count)
            let frame = CGRect(x: 0, y: y - 20, width: sideWidth, height: 40)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: frame.maxX, y: y))
                    path.addLine(to: CGPoint(x: centre.minX, y: midY))
                }, with: .color(.secondary), style: dashed)
            box(frame, display(payer), "probably paid")
        }
        for (index, payment) in origin.payments.enumerated() {
            let y = slot(index, origin.payments.count)
            let frame = CGRect(x: size.width - sideWidth, y: y - 20, width: sideWidth, height: 40)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: centre.maxX, y: midY))
                    path.addLine(to: CGPoint(x: frame.minX, y: y))
                }, with: .color(.secondary), style: dashed)
            box(frame, display(payment.0), "was paid \(Format.coins(payment.1)) \(model.coin)")
        }
        let shape = Path(roundedRect: centre, cornerRadius: 10)
        context.fill(shape, with: .color(.accentColor.opacity(0.14)))
        context.stroke(shape, with: .color(.accentColor), lineWidth: 2.2)
        context.draw(Text("Registered").font(.headline), at: CGPoint(x: centre.midX, y: centre.midY - 8), anchor: .center)
        context.draw(
            Text(when).font(.caption).foregroundStyle(.secondary),
            at: CGPoint(x: centre.midX, y: centre.midY + 11), anchor: .center)
    }
}
