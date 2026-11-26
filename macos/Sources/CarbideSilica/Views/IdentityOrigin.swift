import CarbideCore
import SwiftUI

/// Where an identity came from: who possibly paid for its registration, and the payments
/// the registration made to other identities.
///
/// The payers are always candidates: they funded the inputs. The payments are drawn solid
/// when Carbide verified them against the issuer's rules, and dashed when they are only
/// inferred from the transaction.
struct IdentityOriginCard: View {
    let txid: String
    let height: UInt64
    let time: UInt64?
    let identityAddress: String?

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    fileprivate struct Payment: Sendable {
        let recipient: String
        let sats: Int64
        let currency: String
        /// Carbide's annotation, when the payment is verified.
        let annotation: String?
    }

    fileprivate struct Origin: Sendable {
        let payers: [String]
        let payments: [Payment]
        let verified: Bool
        let spent: Int64?
    }

    var body: some View {
        Card("Origin") {
            SectionLoader(key: "origin:\(txid)", loadingText: "Loading the registration…") {
                explorer in
                let transaction = try await explorer.transaction(txid: txid)
                let payments = try? await explorer.registrationPayments(txid: txid)
                return Self.origin(of: transaction, payments: payments, identity: identityAddress)
            } content: { origin in
                DisplayPanel {
                    VStack(alignment: .leading, spacing: 10) {
                        Canvas { context, size in
                            draw(context, size, origin)
                        }
                        .frame(height: CGFloat(max(origin.payers.count, origin.payments.count, 1)) * 52 + 26)
                        .accessibilityLabel(sentence(origin))
                        PanelRule()
                        Text(
                            "Dashed boxes and lines are inferences. Solid lines are payments that Carbide verified "
                                + "against the issuer's rules.")
                            .font(Tone.caption).foregroundStyle(Tone.label)
                            .task(id: txid) {
                                await model.resolveNames(
                                    origin.payers + origin.payments.flatMap { [$0.recipient, $0.currency] })
                            }
                    }
                }

                Text(sentence(origin)).font(.callout)
                HStack(spacing: 14) {
                    ConfidenceMark(level: .verified, label: "Registration")
                    ConfidenceMark(level: .candidate, label: "Payers are candidates")
                    ConfidenceMark(
                        level: origin.verified ? .verified : .candidate,
                        label: origin.verified ? "Payments verified" : "Payments not verified")
                    Spacer()
                    RouteLink(text: txid, route: .transaction(txid: txid))
                }
            }
        }
    }

    private static func origin(
        of transaction: TransactionDetail, payments: RegistrationPayments?, identity: String?
    ) -> Origin {
        var payers: [String] = []
        for address in transaction.inputs.flatMap(\.addresses) where !payers.contains(address) {
            payers.append(address)
        }
        let known = transaction.inputs.compactMap(\.valueSats)
        let change = transaction.outputs
            .filter { output in output.addresses.first.map(payers.contains) ?? false }
            .compactMap(\.valueSats).reduce(0, +)
        let spent = known.count == transaction.inputs.count && !known.isEmpty ? known.reduce(0, +) - change : nil

        if let payments, payments.status == .verified {
            var rows = payments.referrals.map {
                Payment(recipient: $0.recipient, sats: $0.amountSats, currency: $0.currency, annotation: $0.annotation)
            }
            if let charge = payments.charge {
                rows.insert(
                    Payment(recipient: charge.recipient, sats: charge.amountSats, currency: charge.currency,
                            annotation: charge.annotation), at: 0)
            }
            return Origin(payers: Array(payers.prefix(4)), payments: Array(rows.prefix(5)), verified: true, spent: spent)
        }

        let isIdentity = { (address: String) in address.hasPrefix("i") && address.count == 34 }
        var inferred: [Payment] = []
        for output in transaction.outputs {
            guard let address = output.addresses.first, isIdentity(address), address != identity,
                !payers.contains(address), let sats = output.valueSats, sats > 0
            else { continue }
            if let existing = inferred.firstIndex(where: { $0.recipient == address }) {
                let old = inferred[existing]
                inferred[existing] = Payment(recipient: address, sats: old.sats + sats, currency: old.currency, annotation: nil)
            } else {
                inferred.append(Payment(recipient: address, sats: sats, currency: "", annotation: nil))
            }
        }
        return Origin(payers: Array(payers.prefix(4)), payments: Array(inferred.prefix(4)), verified: false, spent: spent)
    }

    private func display(_ address: String) -> String {
        return model.displayID(address)
    }

    private func amount(_ payment: Payment) -> String {
        payment.currency.isEmpty
            ? "\(Format.coins(payment.sats)) \(model.coin)"
            : model.amountText(payment.sats, currency: payment.currency)
    }

    private var when: String {
        time.map {
            Date(timeIntervalSince1970: TimeInterval($0)).formatted(date: .abbreviated, time: .omitted)
        } ?? model.heightText(height)
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
            let list = origin.payments.map { payment in
                let role = payment.annotation.map { " (\($0))" } ?? ""
                return "\(amount(payment)) to \(display(payment.recipient))\(role)"
            }
            text += " The registration paid \(list.joined(separator: ", "))."
            if !origin.verified {
                text += " Such a payment is usually a referral or a charge from the issuer. Carbide did not verify it."
            }
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
        let dashed = StrokeStyle(lineWidth: 1, lineCap: .butt, dash: [5, 4])
        let solid = StrokeStyle(lineWidth: 1)
        let box = { (frame: CGRect, title: String, subtitle: String, style: StrokeStyle, color: Color) in
            let shape = Path(frame.insetBy(dx: 0.5, dy: 0.5))
            context.fill(shape, with: .color(Tone.screen))
            context.stroke(shape, with: .color(color), style: style)
            context.draw(
                Text(title).font(.callout.weight(.semibold)).foregroundStyle(Tone.value),
                at: CGPoint(x: frame.midX, y: frame.midY - 7), anchor: .center)
            context.draw(
                Text(subtitle).font(.caption2).foregroundStyle(Tone.label),
                at: CGPoint(x: frame.midX, y: frame.midY + 9), anchor: .center)
        }
        for (index, payer) in origin.payers.enumerated() {
            let y = slot(index, origin.payers.count)
            let frame = CGRect(x: 0, y: y - 20, width: sideWidth, height: 40)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: frame.maxX, y: y))
                    path.addLine(to: CGPoint(x: centre.minX, y: midY))
                }, with: .color(Tone.axis), style: dashed)
            box(frame, display(payer), "possible payer", dashed, Tone.axis)
        }
        let style = origin.verified ? solid : dashed
        let color: Color = origin.verified ? Tone.good : Tone.axis
        for (index, payment) in origin.payments.enumerated() {
            let y = slot(index, origin.payments.count)
            let frame = CGRect(x: size.width - sideWidth, y: y - 20, width: sideWidth, height: 40)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: centre.maxX, y: midY))
                    path.addLine(to: CGPoint(x: frame.minX, y: y))
                }, with: .color(color), style: style)
            let subtitle = payment.annotation.map { "\($0): \(amount(payment))" }
                ?? "was paid \(amount(payment))"
            box(frame, display(payment.recipient), subtitle, style, color)
        }
        let shape = Path(centre.insetBy(dx: 0.5, dy: 0.5))
        context.fill(shape, with: .color(Tone.screen))
        context.fill(shape, with: .color(Tone.record.opacity(0.08)))
        context.stroke(shape, with: .color(Tone.record), lineWidth: 1)
        context.draw(
            Text("REGISTERED").font(Tone.heading).tracking(1.1).foregroundStyle(Tone.record),
            at: CGPoint(x: centre.midX, y: centre.midY - 8), anchor: .center)
        context.figure(when, centre.midX, centre.midY + 11, color: Tone.label)
    }
}
