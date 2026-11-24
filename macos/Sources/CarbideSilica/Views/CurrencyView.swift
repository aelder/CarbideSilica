import CarbideCore
import SwiftUI

/// A currency's definition and current state.
struct CurrencyView: View {
    /// A currency name or ID.
    let name: String
    @Environment(AppModel.self) private var model

    var body: some View {
        LoadView(key: "currency:\(name)") { explorer in
            try await explorer.currency(name: name)
        } content: { currency in
            Page {
                PageHeader(
                    kind: "Currency",
                    title: currency.fullyQualifiedName ?? currency.name,
                    subtitle: currency.currencyId,
                    tags: [currency.launchStatus] + currency.optionTags
                )

                Card("Definition") {
                    FieldRow(label: "Name", value: currency.name)
                    OptionalFieldRow(
                        label: "Currency ID", value: currency.currencyId,
                        route: { .identity(name: $0) }, monospaced: true)
                    OptionalFieldRow(
                        label: "Parent", value: currency.parent,
                        route: { .currency(name: $0) }, monospaced: true)
                    OptionalFieldRow(
                        label: "System", value: currency.systemId,
                        route: { .currency(name: $0) }, monospaced: true)
                    if let options = currency.options {
                        FieldRow(label: "Options", value: "\(options)")
                    }
                    if let protocolNumber = currency.proofProtocol {
                        FieldRow(label: "Proof protocol", value: "\(protocolNumber)")
                    }
                    if let start = currency.startBlock, start > 0 {
                        FieldRow(
                            label: "Start block", value: Format.integer(start),
                            route: .block(id: "\(start)"))
                    }
                    if let end = currency.endBlock, end > 0 {
                        FieldRow(label: "End block", value: Format.integer(end))
                    }
                    OptionalFieldRow(
                        label: "Definition transaction", value: currency.definitionTxid,
                        route: { .transaction(txid: $0) }, monospaced: true)
                }

                Card("State") {
                    FieldRow(label: "Launch status", value: currency.launchStatus)
                    if let supply = currency.supply {
                        FieldRow(label: "Supply", value: Format.amount(supply))
                    }
                    if let height = currency.lastConfirmedHeight {
                        FieldRow(
                            label: "Last confirmed height", value: Format.integer(height))
                    }
                }

                CurrencyOverTimeCard(currency: currency)

                NotarizationLanesCard(currency: currency)

                if !currency.reserves.isEmpty {
                    Card("Reserves", trailing: "\(currency.reserves.count) currencies") {
                        Grid(alignment: .leading, horizontalSpacing: 20, verticalSpacing: 8) {
                            GridRow {
                                Text("Currency")
                                Text("Weight").gridColumnAlignment(.trailing)
                                Text("In reserve").gridColumnAlignment(.trailing)
                                Text("Price in reserve").gridColumnAlignment(.trailing)
                            }
                            .font(.caption.weight(.medium))
                            .foregroundStyle(.secondary)
                            Divider()
                            ForEach(currency.reserves, id: \.currencyId) { reserve in
                                GridRow {
                                    RouteLink(
                                        text: reserve.name ?? reserve.currencyId,
                                        route: .currency(name: reserve.currencyId),
                                        shorten: reserve.name == nil)
                                    Text(reserve.weight.map(Format.percent) ?? "–")
                                    Text(reserve.reserves.map(Format.amount) ?? "–")
                                    Text(reserve.priceInReserve.map(Format.amount) ?? "–")
                                }
                                .monospacedDigit()
                            }
                        }
                    }
                }

                JSONSection(json: currency.json)
            }
            .task(id: currency.currencyId) {
                await model.resolveNames(
                    [currency.parent, currency.systemId] + currency.reserves.map(\.currencyId))
            }
        }
        .navigationTitle("Currency")
    }
}
