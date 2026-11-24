import CarbideCore
import SwiftUI

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
                Card("Origin") {
                    Canvas { context, size in
                        draw(context, size, origin)
                    }
                    .frame(height: CGFloat(max(origin.payers.count, origin.payments.count, 1)) * 52 + 26)
                    .accessibilityLabel(sentence(origin))

                    Text(sentence(origin)).font(.callout)
                    HStack(spacing: 14) {
                        ConfidenceMark(level: .verified, label: "Registration")
                        ConfidenceMark(level: .candidate, label: "Payer and payments are not confirmed")
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
            text += ", possibly paid for by \(display(payer))"
            if origin.payers.count > 1 { text += " and \(origin.payers.count - 1) more" }
        }
        if let spent = origin.spent, spent > 0 {
            text += ", at a cost of \(Format.coins(spent)) \(model.coin)"
        }
        text += "."
        if !origin.payments.isEmpty {
            let list = origin.payments.map { "\(Format.coins($0.1)) \(model.coin) to \(display($0.0))" }
            text += " The registration paid \(list.joined(separator: ", ")). Such a payment is usually a referral or a charge from the issuer."
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
            box(frame, display(payer), "possible payer")
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
