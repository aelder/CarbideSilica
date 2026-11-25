import CarbideCore
import SwiftUI

/// A currency's definition and current state.
struct CurrencyView: View {
    /// A currency name or ID.
    let name: String

    var body: some View {
        LoadView(key: "currency:\(name)") { explorer in
            try await explorer.currency(name: name)
        } content: { currency in
            CurrencyPage(route: name, currency: currency)
        }
        .navigationTitle("Currency")
    }
}

private struct CurrencyPage: View {
    /// The name or ID the page was opened with.
    let route: String
    let currency: CurrencyDetail
    @Environment(AppModel.self) private var model
    /// A block chosen on the slider, sent to the exact state card.
    @State private var chosenHeight: UInt64?
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    var body: some View {
        // Read the display settings so the page redraws when they change.
        let _ = (showsNames, showsDates)
        ScrollViewReader { proxy in
            page
                .task(id: model.pendingCurrencyState) {
                    await takeRequest(proxy)
                }
        }
    }

    /// Another page asked for this currency's state at a height, such as a pool on a
    /// transfer trace. Run the exact-state request, bring it into view, then clear the
    /// request.
    private func takeRequest(_ proxy: ScrollViewProxy) async {
        guard let request = model.pendingCurrencyState else { return }
        let key = { (text: String) in
            text.trimmingCharacters(in: .whitespaces).trimmingCharacters(in: CharacterSet(charactersIn: "@"))
                .lowercased()
        }
        let names = [route, currency.currencyId, currency.name, currency.fullyQualifiedName].compactMap { $0 }.map(key)
        guard names.contains(key(request.currency)) else { return }
        model.pendingCurrencyState = nil
        chosenHeight = request.height
        // Let the page lay out before it scrolls.
        try? await Task.sleep(for: .milliseconds(150))
        withAnimation { proxy.scrollTo("exact-state", anchor: .top) }
    }

    private var page: some View {
        Page {
            PageHeader(
                kind: "Currency",
                title: currency.fullyQualifiedName ?? currency.name,
                subtitle: currency.currencyId,
                tags: [currency.launchStatus] + currency.optionTags
            )
            ProvenanceNote(provenance: currency.provenance)

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
                        label: "Start block", value: heightText(start, model: model),
                        route: .block(id: "\(start)"))
                }
                if let end = currency.endBlock, end > 0 {
                    FieldRow(label: "End block", value: heightText(end, model: model))
                }
                OptionalFieldRow(
                    label: "Definition transaction", value: currency.definitionTxid,
                    route: { .transaction(txid: $0) }, monospaced: true)
            }

            Card("Current state") {
                FieldRow(label: "Launch status", value: currency.launchStatus)
                if let supply = currency.supply {
                    FieldRow(label: "Supply", value: Format.amount(supply))
                }
                if let height = currency.lastConfirmedHeight {
                    FieldRow(
                        label: "Last confirmed height", value: heightText(height, model: model))
                }
            }

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
                                    text: showsNames ? reserve.name ?? reserve.currencyId : reserve.currencyId,
                                    route: .currency(name: reserve.currencyId),
                                    shorten: !showsNames || reserve.name == nil)
                                Text(reserve.weight.map(Format.percent) ?? "–")
                                Text(reserve.reserves.map(Format.amount) ?? "–")
                                Text(reserve.priceInReserve.map(Format.amount) ?? "–")
                            }
                            .monospacedDigit()
                        }
                    }
                }
            }

            CurrencyOverTimeCard(currency: currency) { height in
                chosenHeight = height
            }

            CurrencyStateCard(currency: currency, chosenHeight: $chosenHeight)
                .id("exact-state")

            NotarizationLanesCard(currency: currency, route: route)

            JSONSection(json: currency.json)
        }
        .task(id: currency.currencyId) {
            await model.resolveNames(
                [currency.parent, currency.systemId] + currency.reserves.map(\.currencyId))
        }
    }
}
