import CarbideCore
import SwiftUI

/// How an identity registration's cost was split, checked as Carbide checks it: referral
/// payments, the issuer's charge, the native instruction budget and the transaction fee,
/// against the issuer's own fee rules. Nothing shows for a transaction that does not
/// register an identity.
struct RegistrationPaymentsCard: View {
    let txid: String
    /// False when the transaction has no reservation output, so there is nothing to check.
    private let candidate: Bool

    init(txid: String) {
        self.txid = txid
        self.candidate = true
    }

    init(transaction: TransactionDetail) {
        self.txid = transaction.txid
        self.candidate = transaction.json.contains("\"identityreservation\"")
            || transaction.json.contains("\"identityadvancedreservation\"")
    }

    @Environment(AppModel.self) private var model
    @State private var phase: Phase = .checking
    @State private var attempt = 0

    private enum Phase {
        case checking
        case failed(String)
        case done(RegistrationPayments)
    }

    var body: some View {
        if candidate { card }
    }

    private var card: some View {
        Group {
            switch phase {
            case .checking:
                Card("Registration payments") {
                    HStack(spacing: 8) {
                        ProgressView().controlSize(.small)
                        Text("Checking historical evidence").foregroundStyle(.secondary)
                    }
                }
            case .failed(let message):
                Card("Registration payments") {
                    Note(text: message, warning: true)
                    Button("Try Again") { attempt += 1 }
                }
            case .done(let payments):
                if payments.status != .notRegistration {
                    Card("Registration payments") { content(payments) }
                }
            }
        }
        .task(id: "\(txid)|\(model.generation)|\(attempt)") {
            phase = .checking
            do {
                let payments = try await model.requireExplorer().registrationPayments(txid: txid)
                phase = .done(payments)
                var ids: [String?] = payments.referrals.flatMap { [$0.recipient, $0.currency] }
                ids += [payments.issuer, payments.charge?.recipient]
                await model.resolveNames(ids)
            } catch is CancellationError {
            } catch {
                phase = .failed(describe(error))
            }
        }
    }

    @ViewBuilder
    private func content(_ payments: RegistrationPayments) -> some View {
        switch payments.status {
        case .unverified(let reason):
            ConfidenceMark(level: .unknown, label: "Unverified")
            Text(reason).foregroundStyle(.secondary)
            Text("Carbide marks the registration payments verified only when every payment matches the issuer's fee rules and the referral chain.")
                .font(.caption).foregroundStyle(.secondary)
        case .verified, .notRegistration:
            HStack(spacing: 10) {
                ConfidenceMark(level: .verified, label: "Verified against the issuer's rules")
                if let issuer = payments.issuer {
                    NameText("Issuer")
                    IDLink(id: issuer, route: .currency(name: issuer))
                }
            }
            VStack(alignment: .leading, spacing: 0) {
                row(
                    label: "Referrals (\(payments.referrals.count))",
                    amount: payments.referrals.first.map {
                        model.amountText(payments.referralTotalSats, currency: $0.currency)
                    } ?? model.amountText(0, currency: payments.issuer ?? model.coin))
                ForEach(payments.referrals, id: \.output) { referral in
                    paymentRow(referral).padding(.leading, 18)
                }
                if let charge = payments.charge {
                    Divider().padding(.vertical, 4)
                    paymentRow(charge)
                    row(label: "Native instruction budget", amount: "\(Format.coins(payments.budgetSats)) \(model.coin)")
                }
                Divider().padding(.vertical, 4)
                row(label: "Transaction fee", amount: "\(Format.coins(payments.feeSats)) \(model.coin)")
                row(label: "Total", amount: total(payments), bold: true)
            }
        }
    }

    private func total(_ payments: RegistrationPayments) -> String {
        let native = "\(Format.coins(payments.totalNativeSats)) \(model.coin)"
        guard let issuerSats = payments.totalIssuerSats, let issuer = payments.issuer else { return native }
        return "\(model.amountText(issuerSats, currency: issuer)) + \(native)"
    }

    private func paymentRow(_ payment: PaymentRow) -> some View {
        HStack(spacing: 10) {
            Text(payment.annotation).frame(width: 210, alignment: .leading)
            Text("output \(payment.output)").foregroundStyle(.secondary).monospacedDigit()
            IDLink(id: payment.recipient, route: .identity(name: payment.recipient))
            Spacer()
            Text(model.amountText(payment.amountSats, currency: payment.currency)).monospacedDigit()
        }
        .padding(.vertical, 4)
    }

    private func row(label: String, amount: String, bold: Bool = false) -> some View {
        HStack {
            if bold {
                Text(label.uppercased()).font(Tone.small).tracking(1.1).foregroundStyle(Tone.value)
            } else {
                NameText(label)
            }
            Spacer()
            Text(amount).monospacedDigit().fontWeight(bold ? .semibold : .regular)
                .foregroundStyle(bold ? Tone.value : Color.primary)
        }
        .padding(.vertical, 4)
    }
}
