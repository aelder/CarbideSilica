import CarbideCore
import SwiftUI

/// A VerusID: its addresses, authorities and registration.
struct IdentityView: View {
    let name: String

    @Environment(AppModel.self) private var model

    var body: some View {
        LoadView(key: "identity:\(name)") { explorer in
            try await explorer.identity(name: name)
        } content: { identity in
            Page {
                PageHeader(
                    kind: "Identity",
                    title: identity.fullyQualifiedName ?? identity.name,
                    subtitle: identity.identityAddress,
                    tags: tags(identity)
                )
                if identity.definesCurrency, let address = identity.identityAddress {
                    Button {
                        model.open(.currency(name: address))
                    } label: {
                        Label("View its currency", systemImage: "circle.hexagongrid")
                    }
                }

                IdentityControlCard(identity: identity)

                Card("Identity") {
                    FieldRow(label: "Name", value: identity.name)
                    OptionalFieldRow(
                        label: "Identity address", value: identity.identityAddress,
                        monospaced: true)
                    OptionalFieldRow(
                        label: "Parent", value: identity.parent,
                        route: { .currency(name: $0) }, monospaced: true)
                    OptionalFieldRow(
                        label: "System", value: identity.systemId,
                        route: { .currency(name: $0) }, monospaced: true)
                    OptionalFieldRow(
                        label: "Private address", value: identity.privateAddress, monospaced: true)
                    if let timelock = identity.timelock, timelock > 0 {
                        FieldRow(label: "Timelock", value: Format.integer(timelock))
                    }
                }

                Card(
                    "Signing authority",
                    trailing: identity.minimumSignatures.map {
                        "\($0) of \(identity.primaryAddresses.count) required"
                    }
                ) {
                    ForEach(Array(identity.primaryAddresses.enumerated()), id: \.offset) {
                        index, address in
                        FieldRow(
                            label: "Primary address \(index + 1)", value: address,
                            route: .address(address: address), monospaced: true)
                    }
                    OptionalFieldRow(
                        label: "Revocation authority", value: identity.revocationAuthority,
                        route: { .identity(name: $0) }, monospaced: true)
                    OptionalFieldRow(
                        label: "Recovery authority", value: identity.recoveryAuthority,
                        route: { .identity(name: $0) }, monospaced: true)
                }

                IdentityHistoryCard(
                    name: identity.identityAddress ?? name, identityAddress: identity.identityAddress)

                Card("Latest update") {
                    if let height = identity.blockHeight {
                        FieldRow(
                            label: "Block", value: Format.integer(height),
                            route: .block(id: "\(height)"))
                    }
                    OptionalFieldRow(
                        label: "Transaction", value: identity.txid,
                        route: { .transaction(txid: $0) }, monospaced: true)
                }

                JSONSection(json: identity.json)
            }
            .task(id: identity.identityAddress) {
                await model.resolveNames([
                    identity.parent, identity.systemId, identity.revocationAuthority,
                    identity.recoveryAuthority,
                ])
            }
        }
        .navigationTitle("Identity")
    }

    private func tags(_ identity: IdentityDetail) -> [String] {
        var tags: [String] = []
        if let status = identity.status { tags.append(status.capitalized) }
        if identity.definesCurrency { tags.append("Currency defined") }
        return tags
    }
}

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
                    tags: [currency.launchStatus]
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
