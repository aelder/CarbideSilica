import CarbideCore
import SwiftUI

/// A transparent address: balances and recent transactions.
struct AddressView: View {
    let address: String

    var body: some View {
        LoadView(key: "address:\(address)") { explorer in
            try await explorer.address(address: address)
        } content: { detail in
            Page {
                PageHeader(kind: "Address", title: detail.address)

                Card("Balance") {
                    if let sats = detail.balanceSats {
                        HStack {
                            Text("Balance")
                                .foregroundStyle(.secondary)
                                .frame(width: 170, alignment: .leading)
                            CoinAmount(sats: sats).font(.title3.weight(.semibold))
                        }
                    }
                    if let sats = detail.receivedSats {
                        HStack {
                            Text("Total received")
                                .foregroundStyle(.secondary)
                                .frame(width: 170, alignment: .leading)
                            CoinAmount(sats: sats)
                        }
                    }
                    let others = detail.currencyBalances.filter { $0.currency != coin }
                    ForEach(others, id: \.currency) { balance in
                        HStack {
                            RouteLink(
                                text: balance.currency,
                                route: .currency(name: balance.currency))
                                .frame(width: 170, alignment: .leading)
                            Text(Format.amount(balance.amount)).monospacedDigit()
                        }
                    }
                }

                Card("Transactions", trailing: activitySummary(detail)) {
                    if let error = detail.activityError {
                        Label(error, systemImage: "exclamationmark.triangle")
                            .foregroundStyle(.secondary)
                    } else if detail.recentTxids.isEmpty {
                        Text("No transactions.").foregroundStyle(.secondary)
                    } else {
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(Array(detail.recentTxids.enumerated()), id: \.element) {
                                index, txid in
                                if index > 0 { Divider() }
                                RouteLink(
                                    text: txid, route: .transaction(txid: txid), shorten: false
                                )
                                .padding(.vertical, 6)
                            }
                        }
                    }
                }

                JSONSection(json: detail.json)
            }
        }
        .navigationTitle("Address")
    }

    @Environment(AppModel.self) private var model
    private var coin: String { model.coin }

    private func activitySummary(_ detail: AddressDetail) -> String? {
        guard detail.activityError == nil else { return nil }
        let shown = UInt64(detail.recentTxids.count)
        if detail.transactionCount > shown {
            return "Newest \(shown) of \(Format.integer(detail.transactionCount))"
        }
        return "\(detail.transactionCount)"
    }
}
