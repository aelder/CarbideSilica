import CarbideCore
import SwiftUI

/// A currency's definition and current state.
struct CurrencyView: View {
    /// A currency name or ID.
    let name: String
    @State private var reloads = 0

    var body: some View {
        LoadView(key: "currency:\(name)|\(reloads)") { explorer in
            try await explorer.currency(name: name)
        } content: { currency in
            CurrencyPage(route: name, currency: currency)
        }
        .navigationTitle("Currency")
        .publishPlace(PlaceActions(refresh: { reloads += 1 }))
    }
}

private struct CurrencyPage: View {
    /// The name or ID the page was opened with.
    let route: String
    let currency: CurrencyDetail
    @Environment(AppModel.self) private var model
    /// A block chosen on the slider, sent to the exact state card.
    @State private var chosenHeight: UInt64?
    /// The reserve that prices and values are shown in. Nil selects the default.
    @State private var chosenUnit: String?
    /// Incremented to put the keyboard in the field of the exact state card.
    @State private var stateFocus = 0
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
                // The terminal's keys on a currency: g asks for a past state, n goes to
                // the notarizations.
                .publishKeys([
                    "g": {
                        stateFocus += 1
                        withAnimation { proxy.scrollTo("exact-state", anchor: .top) }
                    },
                    "n": { withAnimation { proxy.scrollTo("notarizations", anchor: .top) } },
                ])
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

    // MARK: The unit

    /// The reserve that values are shown in: the choice, or the chain's coin when it is a
    /// reserve, or the first reserve.
    private var unit: String? {
        let ids = currency.reserves.map(\.currencyId)
        if let chosenUnit, ids.contains(chosenUnit) { return chosenUnit }
        if let coin = model.status?.chainId, ids.contains(coin) { return coin }
        return ids.first
    }

    private var unitPrice: Double? {
        currency.reserves.first { $0.currencyId == unit }?.priceInReserve.flatMap { $0 > 0 ? $0 : nil }
    }

    private func reserveName(_ reserve: ReserveRow) -> String {
        showsNames ? reserve.name ?? knownName(reserve.currencyId, model: model) : Format.short(reserve.currencyId, head: 6, tail: 4)
    }

    private var unitName: String {
        currency.reserves.first { $0.currencyId == unit }.map(reserveName) ?? ""
    }

    private func value(of reserve: ReserveRow, _ unitPrice: Double) -> Double? {
        guard let supply = currency.supply, let weight = reserve.weight else { return nil }
        return supply * unitPrice * weight
    }

    private var basketSentence: String? {
        guard let unitPrice, let supply = currency.supply else { return nil }
        let count = currency.reserves.count
        let weights = Set(currency.reserves.compactMap { $0.weight.map { ($0 * 1_000_000).rounded() } })
        var text = "\(currency.name) is a basket of \(count) \(count == 1 ? "reserve" : "reserves")"
        if count > 1, weights.count == 1, let weight = currency.reserves.first?.weight {
            text += " at \(Format.percent(weight)) each"
        }
        text += ". One \(currency.name) is worth \(BasketMath.text(unitPrice)) \(unitName). "
        text += "The pool holds the value of \(BasketMath.text(supply * unitPrice * totalWeight)) \(unitName)."
        return text
    }

    private var totalWeight: Double {
        currency.reserves.compactMap(\.weight).reduce(0, +)
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
                    if let sentence = basketSentence {
                        Text(sentence).font(.title3).fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
                    }
                    if currency.reserves.count > 1 {
                        Picker("Show values in", selection: Binding(get: { unit ?? "" }, set: { chosenUnit = $0 })) {
                            ForEach(currency.reserves, id: \.currencyId) { reserve in
                                Text(reserveName(reserve)).tag(reserve.currencyId)
                            }
                        }
                        .fixedSize()
                    }
                    Grid(alignment: .leading, horizontalSpacing: 20, verticalSpacing: 8) {
                        GridRow {
                            Text("Currency")
                            Text("Weight").gridColumnAlignment(.trailing)
                            Text("In reserve").gridColumnAlignment(.trailing)
                            Text("Price in reserve").gridColumnAlignment(.trailing)
                            if unitPrice != nil {
                                Text("Value in \(unitName)").gridColumnAlignment(.trailing)
                                Text("One is worth, in \(unitName)").gridColumnAlignment(.trailing)
                            }
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
                                if let unitPrice {
                                    Text(value(of: reserve, unitPrice).map(BasketMath.text) ?? "–")
                                    Text(
                                        reserve.currencyId == unit
                                            ? "1" : BasketMath.worth(reserve.priceInReserve, unitPrice).map(BasketMath.text) ?? "–")
                                }
                            }
                            .monospacedDigit()
                        }
                    }
                    Text(
                        "\"Price in reserve\" is the number of units of the reserve for one unit of \(currency.name). "
                            + (unitPrice == nil
                                ? ""
                                : "Each reserve holds its weight of the value of the pool. The values in \(unitName) are "
                                    + "the pool's own rates between its reserves. They are not market quotes."))
                        .font(.caption).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }

            CurrencyOverTimeCard(currency: currency, unit: unit) { height in
                chosenHeight = height
            }

            CurrencyStateCard(currency: currency, chosenHeight: $chosenHeight, focusRequests: stateFocus)
                .id("exact-state")

            NotarizationLanesCard(currency: currency, route: route)
                .id("notarizations")

            JSONSection(json: currency.json, filename: "currency-\(currency.currencyId ?? currency.name).json")
        }
        .task(id: currency.currencyId) {
            await model.resolveNames(
                [currency.parent, currency.systemId] + currency.reserves.map(\.currencyId))
        }
    }
}

/// The arithmetic of a basket: its prices are units of a reserve for one unit of the basket.
enum BasketMath {
    /// What one unit of a reserve is worth in the unit reserve: the price of the basket in
    /// the unit, divided by its price in the reserve.
    static func worth(_ price: Double?, _ unitPrice: Double?) -> Double? {
        guard let price, let unitPrice, price > 0, unitPrice > 0 else { return nil }
        return unitPrice / price
    }

    /// A figure with as many decimals as its size needs.
    static func text(_ value: Double) -> String {
        let size = abs(value)
        let digits = size >= 1000 ? 0 : size >= 10 ? 2 : size >= 1 ? 3 : size >= 0.01 ? 5 : 8
        return value.formatted(.number.precision(.fractionLength(0...digits)))
    }
}
