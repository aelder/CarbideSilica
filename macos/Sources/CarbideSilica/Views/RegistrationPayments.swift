import CarbideCore
import SwiftUI

/// How an identity registration's cost was split: payments to other identities, which are
/// usually referrals or the issuer's charge, and the transaction fee.
struct RegistrationPaymentsCard: View {
    let transaction: TransactionDetail
    @Environment(AppModel.self) private var model

    private var payers: Set<String> { Set(transaction.inputs.flatMap(\.addresses)) }

    private var payments: [(String, Int64)] {
        var result: [(String, Int64)] = []
        for output in transaction.outputs {
            guard let address = output.addresses.first, address.hasPrefix("i"), address.count == 34,
                !payers.contains(address), output.tags.isEmpty, let sats = output.valueSats, sats > 0
            else { continue }
            if let index = result.firstIndex(where: { $0.0 == address }) {
                result[index].1 += sats
            } else {
                result.append((address, sats))
            }
        }
        return result
    }

    private var fee: Int64? {
        let inputs = transaction.inputs.compactMap(\.valueSats)
        guard inputs.count == transaction.inputs.count, !inputs.isEmpty else { return nil }
        let out = transaction.outputs.compactMap(\.valueSats).reduce(0, +)
        return inputs.reduce(0, +) - out
    }

    var body: some View {
        if !payments.isEmpty {
            Card("Registration payments") {
                ForEach(payments, id: \.0) { payment in
                    HStack {
                        RouteLink(text: payment.0, route: .identity(name: payment.0), shorten: true)
                        Spacer()
                        CoinAmount(sats: payment.1)
                    }
                }
                if let fee, fee >= 0 {
                    Divider()
                    HStack {
                        Text("Transaction fee").foregroundStyle(.secondary)
                        Spacer()
                        CoinAmount(sats: fee)
                    }
                    HStack {
                        Text("Total").fontWeight(.semibold)
                        Spacer()
                        CoinAmount(sats: payments.map(\.1).reduce(0, +) + fee).fontWeight(.semibold)
                    }
                }
                HStack(spacing: 10) {
                    ConfidenceMark(level: .candidate, label: "Not confirmed")
                    Text("A payment to an identity in a registration is usually a referral or a charge from the issuer. The chain does not record the type.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .task(id: transaction.txid) { await model.resolveNames(payments.map(\.0)) }
        }
    }
}
