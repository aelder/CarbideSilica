import CarbideCore
import SwiftUI

/// Open offers involving the chain's own coin, as Carbide requests them, shown as a
/// catalogue: an answer line, the prices as steps and as one curve, and a list with
/// filters that show their counts. `macos/design/market-research.html` has the reasons.
///
/// The offers on the chain are mostly identity names for sale at a few exact prices, so a
/// plot of points puts hundreds of offers on one spot. A person looks for a name, a kind
/// of name, a price or a seller. Each of those is a filter here, and each filter option
/// shows how many offers it leaves.
struct MarketplaceView: View {
    @Environment(AppModel.self) private var model
    @State private var market: Marketplace?
    @State private var failure: String?
    @State private var loading = true
    @State private var items: [MarketItem] = []
    @State private var store = MarketStore.shared
    @State private var selected: [Facet: String] = [:]
    @State private var search = ""
    @State private var sort = OfferSort.askAmount
    @State private var descending = false
    @State private var ordered: [MarketItem] = []
    @State private var shown = 60
    @State private var hoveredPrice: Double?
    /// Worked out once for each change of the data, not at each redraw: the option of each
    /// filter that each offer belongs to, the sellers, and how many offers have details.
    @State private var values: [String: [Facet: String]] = [:]
    @State private var sellers: [(address: String, count: Int)] = []
    @State private var detailed = 0
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    var body: some View {
        let _ = showsNames
        ScrollViewReader { proxy in
            Page {
                Card("Open offers", trailing: market.map { "\(Format.integer(UInt64($0.offers.count))) involving \($0.subject)" }) {
                    if loading && market == nil {
                        ProgressView("Loading offers…").frame(maxWidth: .infinity, minHeight: 160)
                    } else if let failure, market == nil {
                        Note(text: failure, warning: true)
                    } else if let market {
                        summary(market)
                    }
                }
                if market != nil, !priced.isEmpty {
                    Card("Prices", trailing: "ask in \(priceTicker)") {
                        prices
                    }
                }
                if let market {
                    Card("Catalogue") {
                        if market.truncated {
                            Note(text: "The node returned more offers than the app keeps. The app shows the first 5,000.", warning: true)
                        }
                        HStack(alignment: .top, spacing: 22) {
                            facets.frame(width: 232)
                            results.id("catalogue-top")
                        }
                    }
                    smallerSellers
                }
            }
            .onChange(of: ListKey(sort: sort, descending: descending, selected: selected, search: search)) {
                reorder()
            }
            .onChange(of: store.details.count) {
                rebuild()
            }
            .onChange(of: ScrollKey(sort: sort, descending: descending, selected: selected)) {
                // As in Carbide, a new sort order returns to the top.
                shown = 60
                proxy.scrollTo("catalogue-top", anchor: .top)
            }
        }
        .navigationTitle("Marketplace")
        .task(id: model.generation) {
            await load()
            await loadDetails(force: false)
        }
        .publishPlace(PlaceActions(refresh: { Task { await load() } }))
    }

    private struct ListKey: Equatable {
        let sort: OfferSort
        let descending: Bool
        let selected: [Facet: String]
        let search: String
    }

    private struct ScrollKey: Equatable {
        let sort: OfferSort
        let descending: Bool
        let selected: [Facet: String]
    }

    // MARK: Summary

    /// The answer first: how many offers, from how many sellers, at what prices.
    @ViewBuilder
    private func summary(_ market: Marketplace) -> some View {
        HStack {
            ProvenanceNote(provenance: market.provenance)
            Spacer()
            if loading { ProgressView().controlSize(.small) }
            Button("Refresh") { Task { await load() } }.disabled(loading)
        }
        if let failure {
            Note(text: failure, warning: true)
        }
        let prices = priced.map(\.price!).sorted()
        HStack(spacing: 20) {
            figure("Offers", Format.integer(UInt64(items.count)))
            if let top = sellers.first {
                figure("Sellers", "\(sellers.count)")
                figure("Largest seller", "\(Int((Double(top.count) / Double(max(detailed, 1)) * 100).rounded()))%")
            }
            if let low = prices.first {
                figure("Lowest", "\(Format.amount(low)) \(priceTicker)")
                figure("Median", "\(Format.amount(prices[prices.count / 2])) \(priceTicker)")
            }
            if let newest = items.compactMap({ listed($0) }).max() {
                figure("Newest offer", Self.age(since: newest) + " ago")
            }
            Spacer(minLength: 0)
        }
        .font(.callout)
        .lineLimit(1)
        detailProgress
        Text("Request: getoffers for \(market.subject). The seller of an identity is its first primary address now.")
            .font(.caption).foregroundStyle(.secondary)
    }

    private func figure(_ label: String, _ value: String) -> some View {
        HStack(spacing: 5) {
            Text(label).foregroundStyle(.secondary)
            Text(value).fontWeight(.semibold).monospacedDigit()
        }
    }

    /// The sellers and the listing dates need two requests for each offer.
    @ViewBuilder
    private var detailProgress: some View {
        if store.loading {
            HStack(spacing: 8) {
                ProgressView().controlSize(.small)
                Text("Reading sellers and listing dates: \(Format.integer(UInt64(detailed))) of \(Format.integer(UInt64(items.count)))")
                    .font(.caption).foregroundStyle(.secondary).monospacedDigit()
            }
        } else if detailed < items.count {
            HStack(spacing: 8) {
                Text(
                    store.problem
                        ?? "The sellers and the listing dates are not read. They need two requests for each offer."
                )
                .font(.caption).foregroundStyle(.secondary)
                Button("Read Sellers and Dates") { Task { await loadDetails(force: true) } }
                    .controlSize(.small)
            }
        }
    }

    // MARK: Prices

    /// The exact prices that many offers share, as bars, and one curve of how many offers
    /// are available at or below each price.
    private var prices: some View {
        let steps = priceSteps
        return VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .top, spacing: 26) {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Prices that many offers share").font(.callout.weight(.semibold))
                    let top = Double(steps.map(\.count).max() ?? 1)
                    ForEach(steps, id: \.label) { step in
                        HStack(spacing: 8) {
                            Text(step.label).monospacedDigit().frame(width: 104, alignment: .trailing)
                            GeometryReader { geometry in
                                RoundedRectangle(cornerRadius: 2)
                                    .fill(Color.accentColor.opacity(step.other ? 0.4 : 0.85))
                                    .frame(width: max(geometry.size.width * CGFloat(Double(step.count) / top), 2))
                            }
                            .frame(height: 12)
                            Text(Format.integer(UInt64(step.count))).monospacedDigit().frame(width: 44, alignment: .leading)
                        }
                        .font(.callout)
                    }
                }
                .frame(maxWidth: 330)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Offers at or below each price").font(.callout.weight(.semibold))
                    GeometryReader { geometry in
                        Canvas { context, size in drawSupply(context, size) }
                            .contentShape(Rectangle())
                            .onContinuousHover { phase in
                                if case .active(let point) = phase {
                                    hoveredPrice = price(atX: point.x, width: geometry.size.width)
                                } else {
                                    hoveredPrice = nil
                                }
                            }
                    }
                    .frame(height: 190)
                    .accessibilityLabel("The number of offers available at or below each price")
                    Text(supplyReadout)
                        .font(.callout)
                        .foregroundStyle(hoveredPrice == nil ? .secondary : .primary)
                        .monospacedDigit()
                }
            }
            let unpriced = items.count - priced.count
            Text(
                "Each step up in the curve is a price that many offers share. The price scale is logarithmic. "
                    + (unpriced > 0 ? "\(unpriced) offer\(unpriced == 1 ? " does" : "s do") not ask for one amount of \(priceTicker) and \(unpriced == 1 ? "is" : "are") not in these figures." : "")
            )
            .font(.caption).foregroundStyle(.secondary)
        }
    }

    private struct Step {
        let label: String
        let count: Int
        let other: Bool
    }

    /// The exact prices with the most offers, cheapest first, and one row for the rest.
    private var priceSteps: [Step] {
        var counts: [Double: Int] = [:]
        for item in priced { counts[item.price!, default: 0] += 1 }
        let floor = max(priced.count / 60, 3)
        let common = counts.filter { $0.value >= floor }.sorted { $0.value > $1.value }.prefix(9).sorted { $0.key < $1.key }
        var steps = common.map { Step(label: "\(Format.amount($0.key)) \(priceTicker)", count: $0.value, other: false) }
        let rest = priced.count - common.reduce(0) { $0 + $1.value }
        if rest > 0 { steps.append(Step(label: "each other price", count: rest, other: true)) }
        return steps
    }

    private var priceBounds: (low: Double, high: Double) {
        let values = priced.map(\.price!)
        let low = max(values.min() ?? 1, 1e-8), high = max(values.max() ?? 10, low * 10)
        return (pow(10, floor(log10(low))), pow(10, ceil(log10(high))))
    }

    private func price(atX x: CGFloat, width: CGFloat) -> Double {
        let bounds = priceBounds, left: CGFloat = 46, right = width - 10
        let share = Double(min(max((x - left) / (right - left), 0), 1))
        return pow(10, log10(bounds.low) + share * (log10(bounds.high) - log10(bounds.low)))
    }

    private var supplyReadout: String {
        let sortedPrices = priced.map(\.price!).sorted()
        guard let hoveredPrice else {
            guard let middle = sortedPrices.isEmpty ? nil : sortedPrices[sortedPrices.count / 2] else { return "" }
            return "Half of the offers ask \(Format.amount(middle)) \(priceTicker) or less. Move the pointer over the curve to read it."
        }
        let count = sortedPrices.filter { $0 <= hoveredPrice }.count
        return "\(Format.integer(UInt64(count))) offer\(count == 1 ? "" : "s") at \(Format.amount(hoveredPrice.rounded(toSignificant: 3))) \(priceTicker) or less."
    }

    private func drawSupply(_ context: GraphicsContext, _ size: CGSize) {
        let sortedPrices = priced.map(\.price!).sorted()
        guard !sortedPrices.isEmpty else { return }
        let bounds = priceBounds
        let left: CGFloat = 46, right = size.width - 10, top: CGFloat = 8, base = size.height - 22
        let x = { (price: Double) -> CGFloat in
            left + CGFloat((log10(max(price, bounds.low)) - log10(bounds.low)) / (log10(bounds.high) - log10(bounds.low))) * (right - left)
        }
        let y = { (count: Int) -> CGFloat in base - CGFloat(Double(count) / Double(sortedPrices.count)) * (base - top) }
        for share in [0.0, 0.5, 1.0] {
            let count = Int(Double(sortedPrices.count) * share), gy = y(count)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: left, y: gy))
                    path.addLine(to: CGPoint(x: right, y: gy))
                }, with: .color(.secondary.opacity(0.14)), lineWidth: 1)
            context.draw(
                Text(Format.integer(UInt64(count))).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: left - 6, y: gy), anchor: .trailing)
        }
        var decade = bounds.low
        while decade <= bounds.high * 1.0001 {
            let last = decade * 10 > bounds.high * 1.0001
            context.draw(
                Text(Format.amount(decade)).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: x(decade), y: base + 11), anchor: last ? .trailing : .center)
            decade *= 10
        }
        var curve = Path()
        curve.move(to: CGPoint(x: left, y: base))
        for (index, price) in sortedPrices.enumerated() {
            curve.addLine(to: CGPoint(x: x(price), y: y(index)))
            curve.addLine(to: CGPoint(x: x(price), y: y(index + 1)))
        }
        curve.addLine(to: CGPoint(x: right, y: top))
        var area = curve
        area.addLine(to: CGPoint(x: right, y: base))
        area.closeSubpath()
        context.fill(area, with: .color(Color.accentColor.opacity(0.14)))
        context.stroke(curve, with: .color(Color.accentColor), lineWidth: 1.6)
        if let hoveredPrice {
            let hx = x(hoveredPrice)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: hx, y: top))
                    path.addLine(to: CGPoint(x: hx, y: base))
                }, with: .color(.primary.opacity(0.7)), lineWidth: 1)
        }
    }

    // MARK: Filters

    /// Each option shows its count with the other filters applied, and a bar for it.
    private var facets: some View {
        VStack(alignment: .leading, spacing: 14) {
            ForEach(Facet.allCases, id: \.self) { facet in
                let options = options(for: facet)
                if !options.isEmpty {
                    VStack(alignment: .leading, spacing: 3) {
                        Text(facet.title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                        let counts = counts(for: facet)
                        let top = max(counts.values.max() ?? 1, 1)
                        ForEach(options, id: \.self) { option in
                            FacetOption(
                                label: option, count: counts[option] ?? 0, share: Double(counts[option] ?? 0) / Double(top),
                                on: selected[facet] == option
                            ) {
                                selected[facet] = selected[facet] == option ? nil : option
                            }
                        }
                    }
                }
            }
            if detailed < items.count, !store.loading {
                Text("The Seller and Offered filters appear when the sellers and dates are read.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    /// How many offers each option of a filter leaves, with the other filters applied.
    private func counts(for facet: Facet) -> [String: Int] {
        var counts: [String: Int] = [:]
        for item in items where matches(item, skipping: facet) {
            if let option = values[item.offer.txid]?[facet] { counts[option, default: 0] += 1 }
        }
        return counts
    }

    private func options(for facet: Facet) -> [String] {
        switch facet {
        case .offer:
            let kinds = Set(items.map { $0.kindLabel })
            return kinds.count > 1 ? ["For sale", "Wanted", "Swap"].filter(kinds.contains) : []
        case .name: return NameKind.allCases.map(\.rawValue)
        case .length: return ["1", "2", "3", "4", "5 to 7", "8 or more"]
        case .price: return MarketItem.priceBands.map(\.label)
        case .seller:
            guard detailed > 0 else { return [] }
            var options = namedSellers.map { Format.short($0, head: 6, tail: 4) }
            if sellers.count > options.count { options.append("Other sellers") }
            return options
        case .listed:
            return detailed > 0 ? ["In the last 90 days", "90 days to 1 year", "1 to 2 years", "More than 2 years"] : []
        }
    }

    /// The sellers that the Seller filter names: the five largest with five offers or more.
    private var namedSellers: [String] {
        sellers.filter { $0.count >= 5 }.prefix(5).map(\.address)
    }

    /// The option of a filter that an offer belongs to. `rebuild` keeps the results.
    private func value(_ facet: Facet, _ item: MarketItem, named: [String]) -> String? {
        switch facet {
        case .offer: return item.kindLabel
        case .name: return item.traits?.kind.rawValue
        case .length: return item.traits.map { MarketItem.lengthBand($0.length) }
        case .price: return item.price.flatMap { MarketItem.priceBand($0) }
        case .seller:
            guard let holder = store.details[item.offer.txid]?.holder else { return nil }
            return named.contains(holder) ? Format.short(holder, head: 6, tail: 4) : "Other sellers"
        case .listed:
            guard let time = listed(item) else { return nil }
            let days = Date.now.timeIntervalSince1970 / 86_400 - Double(time) / 86_400
            if days <= 90 { return "In the last 90 days" }
            if days <= 365 { return "90 days to 1 year" }
            if days <= 730 { return "1 to 2 years" }
            return "More than 2 years"
        }
    }

    private func matches(_ item: MarketItem, skipping: Facet?) -> Bool {
        let query = search.trimmingCharacters(in: .whitespaces)
        if !query.isEmpty, !item.title.localizedCaseInsensitiveContains(query) { return false }
        let own = values[item.offer.txid]
        for (facet, option) in selected where facet != skipping {
            if own?[facet] != option { return false }
        }
        return true
    }

    // MARK: Results

    private var results: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                TextField("Search the names", text: $search)
                    .textFieldStyle(.roundedBorder)
                    .frame(maxWidth: 240)
                Text("\(Format.integer(UInt64(ordered.count))) of \(Format.integer(UInt64(items.count))) offers")
                    .font(.headline).monospacedDigit()
                Spacer(minLength: 0)
            }
            HStack(spacing: 8) {
                Text(appliedText).font(.callout).foregroundStyle(.secondary).lineLimit(2)
                if !selected.isEmpty || !search.isEmpty {
                    Button("Clear Filters") {
                        selected = [:]
                        search = ""
                    }
                    .controlSize(.small)
                }
                Spacer(minLength: 0)
            }
            HStack {
                Picker("Sort by", selection: $sort) {
                    Text("Ask amount").tag(OfferSort.askAmount)
                    Text("Name").tag(OfferSort.name)
                    Text("Expiry").tag(OfferSort.expiry)
                    Text("Listing age").tag(OfferSort.listingAge)
                }
                .pickerStyle(.segmented)
                .frame(maxWidth: 400)
                Toggle("Reverse", isOn: $descending).toggleStyle(.button)
                Spacer(minLength: 0)
            }
            Text(sortNote).font(.caption).foregroundStyle(.secondary)
            if ordered.isEmpty {
                Text(items.isEmpty ? "No open offers were returned for this currency." : "No offer matches the filters.")
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 12)
            }
            LazyVStack(spacing: 0) {
                ForEach(ordered.prefix(shown)) { item in
                    Divider()
                    OfferLine(
                        item: item, detail: store.details[item.offer.txid], ticker: name, priceTicker: priceTicker,
                        priceBounds: priceBounds)
                }
            }
            if ordered.count > shown {
                Button("Show \(min(100, ordered.count - shown)) More") { shown += 100 }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var appliedText: String {
        var parts = Facet.allCases.compactMap { facet in selected[facet].map { "\(facet.title): \($0)" } }
        let query = search.trimmingCharacters(in: .whitespaces)
        if !query.isEmpty { parts.insert("Name contains \"\(query)\"", at: 0) }
        return parts.isEmpty ? "No filter is applied." : parts.joined(separator: " · ")
    }

    private var sortNote: String {
        switch sort {
        case .expiry: return descending ? "Latest expiry first." : "Soonest expiry first."
        case .name: return descending ? "Names Z to A." : "Names A to Z."
        case .askAmount: return "Grouped by ask currency, \(descending ? "largest" : "smallest") first. Offers with no single ask last."
        case .listingAge:
            let order = descending ? "Oldest listing first." : "Newest listing first."
            return detailed < items.count ? "\(order) Listing dates: \(detailed) of \(items.count) read." : order
        }
    }

    // MARK: The smaller sellers

    /// When one seller has most of the offers, the offers of the others are the only
    /// independent prices. They get their own group so that they are not lost.
    @ViewBuilder
    private var smallerSellers: some View {
        if sellers.count > 2, detailed > 0,
            Double(sellers[0].count + sellers[1].count) / Double(detailed) >= 0.8
        {
            let large = Set(sellers.prefix(2).map(\.address))
            let rest = items.filter { item in
                store.details[item.offer.txid]?.holder.map { !large.contains($0) } ?? false
            }.sorted { ($0.price ?? .infinity) < ($1.price ?? .infinity) }
            Card("Offers of the smaller sellers", trailing: "\(rest.count) from \(sellers.count - 2) sellers") {
                Text("Two sellers have \(Int((Double(sellers[0].count + sellers[1].count) / Double(detailed) * 100).rounded()))% of the offers. These are the offers of the others.")
                    .font(.callout).foregroundStyle(.secondary)
                LazyVStack(spacing: 0) {
                    ForEach(rest.prefix(60)) { item in
                        Divider()
                        OfferLine(
                            item: item, detail: store.details[item.offer.txid], ticker: name, priceTicker: priceTicker,
                            priceBounds: priceBounds)
                    }
                }
            }
        }
    }

    // MARK: Data

    private func load() async {
        guard let explorer = model.explorer else { return }
        loading = true
        defer { loading = false }
        do {
            let found = try await explorer.marketplace()
            market = found
            failure = nil
            // The currency that most offers ask for is the price currency.
            let asked = Dictionary(grouping: found.offers.compactMap(\.askCurrency), by: { $0 }).mapValues(\.count)
            let main = asked.max { $0.value < $1.value }?.key
            items = found.offers.map { MarketItem(offer: $0, priceCurrency: main, name: name) }
            rebuild()
            let currencies = Set(found.offers.flatMap { $0.links.filter { $0.kind == .currency }.map(\.value) })
            await model.resolveNames(Array(currencies))
            reorder()
        } catch is CancellationError {
        } catch {
            failure = describe(error)
        }
    }

    /// Reads the seller and the listing block of each offer, in groups, and keeps them
    /// for the session. A public API sees each request, so the person starts it there.
    private func loadDetails(force: Bool) async {
        guard let explorer = model.explorer, !store.loading else { return }
        if store.generation != model.generation {
            store.details = [:]
            store.generation = model.generation
        }
        guard force || model.status?.source != .publicApi else { return }
        let missing = items.filter { store.details[$0.offer.txid] == nil }
        guard !missing.isEmpty else { return }
        store.loading = true
        store.problem = nil
        defer { store.loading = false }
        for start in stride(from: 0, to: missing.count, by: 128) {
            if Task.isCancelled { return }
            let group = missing[start..<min(start + 128, missing.count)]
            let keys = group.map { OfferKey(txid: $0.offer.txid, identityId: $0.offer.offered.identityId) }
            let found = await explorer.offerDetails(offers: keys)
            for detail in found { store.details[detail.txid] = detail }
            if let problem = found.first(where: { $0.error != nil && $0.holder == nil && $0.blockHeight == nil })?.error {
                // A source that refuses these requests refuses all of them.
                store.problem = "The source did not answer for the sellers and dates. \(problem)"
                return
            }
        }
    }

    /// Works out the sellers and each offer's filter options again, then the list.
    private func rebuild() {
        let holders = items.compactMap { store.details[$0.offer.txid]?.holder }
        sellers = Dictionary(grouping: holders, by: { $0 }).map { (address: $0.key, count: $0.value.count) }
            .sorted { $0.count != $1.count ? $0.count > $1.count : $0.address < $1.address }
        detailed = items.filter { store.details[$0.offer.txid] != nil }.count
        let named = namedSellers
        var found: [String: [Facet: String]] = [:]
        for item in items {
            var own: [Facet: String] = [:]
            for facet in Facet.allCases {
                if let option = value(facet, item, named: named) { own[facet] = option }
            }
            found[item.offer.txid] = own
        }
        values = found
        reorder()
    }

    private func reorder() {
        let visible = items.filter { matches($0, skipping: nil) }
        let times = store.details.compactMapValues(\.blockTime)
        let sorted = sortOffers(
            offers: visible.map(\.offer), sort: sort, descending: descending, names: sortNames, times: times)
        let byTxid = Dictionary(visible.map { ($0.offer.txid, $0) }, uniquingKeysWith: { first, _ in first })
        ordered = sorted.compactMap { byTxid[$0.txid] }
    }

    /// Resolved names for the name sort, without the trailing "@".
    private var sortNames: [String: String] {
        model.names.mapValues { $0.hasSuffix("@") ? String($0.dropLast()) : $0 }
    }

    private func name(_ id: String) -> String {
        knownName(id, model: model)
    }

    private var priced: [MarketItem] { items.filter { $0.price != nil } }

    private var priceTicker: String {
        items.first { $0.price != nil }?.offer.askCurrency.map(name) ?? model.coin
    }

    private func listed(_ item: MarketItem) -> UInt64? { store.details[item.offer.txid]?.blockTime }

    static func age(since unix: UInt64) -> String {
        timeLeft((Date.now.timeIntervalSince1970 - Double(unix)) / 3_600)
    }

    static func timeLeft(_ hours: Double) -> String {
        if hours < 1 { return "\(max(Int(hours * 60), 1)) min" }
        if hours < 48 { return "\(Int(hours)) h" }
        if hours < 24 * 365 * 2 { return "\(Int(hours / 24)) days" }
        return "\((hours / 24 / 365).formatted(.number.precision(.fractionLength(1)))) years"
    }
}

// MARK: - Model

/// The sellers and listing dates of the session, so that a return to the page does not
/// read them again.
@MainActor
@Observable
final class MarketStore {
    static let shared = MarketStore()
    var details: [String: OfferDetail] = [:]
    var generation = -1
    var loading = false
    var problem: String?
}

enum Facet: CaseIterable, Hashable {
    case offer, name, length, price, seller, listed

    var title: String {
        switch self {
        case .offer: "Kind of offer"
        case .name: "Kind of name"
        case .length: "Length of name"
        case .price: "Price"
        case .seller: "Seller"
        case .listed: "Offered"
        }
    }
}

enum NameKind: String, CaseIterable {
    case latin = "Latin letters"
    case emoji = "Emoji and symbols"
    case other = "Other scripts"
    case digits = "Digits"
}

/// What a name is made of. The length is in characters as a person sees them: one emoji
/// built from several code points is one character.
struct NameTraits {
    let kind: NameKind
    let length: Int
    let scripts: [String]
    let codePoints: String

    init(_ name: String) {
        length = name.count
        codePoints = name.unicodeScalars.map { String(format: "U+%04X", $0.value) }.joined(separator: " ")
        let letters = name.unicodeScalars.filter { $0.properties.isAlphabetic }
        var found: [String] = []
        for scalar in letters {
            let script = Self.script(scalar.value)
            if !found.contains(script) { found.append(script) }
        }
        scripts = found
        if !name.isEmpty, name.unicodeScalars.allSatisfy({ (0x30...0x39).contains($0.value) }) {
            kind = .digits
        } else if letters.isEmpty {
            kind = .emoji
        } else if found == ["Latin"] {
            kind = .latin
        } else {
            kind = .other
        }
    }

    /// The script of a letter, for the scripts that names on the chain use.
    private static func script(_ value: UInt32) -> String {
        switch value {
        case 0x0041...0x024F, 0x1E00...0x1EFF, 0xFF21...0xFF5A: "Latin"
        case 0x0370...0x03FF, 0x1F00...0x1FFF: "Greek"
        case 0x0400...0x052F: "Cyrillic"
        case 0x0530...0x058F: "Armenian"
        case 0x0590...0x05FF: "Hebrew"
        case 0x0600...0x06FF, 0x0750...0x077F, 0x08A0...0x08FF: "Arabic"
        case 0x0900...0x097F: "Devanagari"
        case 0x0980...0x09FF: "Bengali"
        case 0x0B80...0x0BFF: "Tamil"
        case 0x0E00...0x0E7F: "Thai"
        case 0x10A0...0x10FF: "Georgian"
        case 0x1100...0x11FF, 0x3130...0x318F, 0xAC00...0xD7AF: "Hangul"
        case 0x3040...0x309F: "Hiragana"
        case 0x30A0...0x30FF, 0x31F0...0x31FF: "Katakana"
        case 0x3400...0x4DBF, 0x4E00...0x9FFF, 0x20000...0x2A6DF: "Han"
        case 0x13000...0x1342F: "Egyptian hieroglyphs"
        default: "another script"
        }
    }
}

/// One offer with the figures that the catalogue filters by.
struct MarketItem: Identifiable {
    let offer: Offer
    /// The identity's name for an identity offer, otherwise the subject's name.
    let title: String
    /// The ask, when it is one amount of the price currency.
    let price: Double?
    /// For an offer of a named identity.
    let traits: NameTraits?

    var id: String { offer.txid }

    init(offer: Offer, priceCurrency: String?, name: (String) -> String) {
        self.offer = offer
        var identity: String?
        for side in [offer.offered, offer.accepted] where side.identityId == offer.subjectId {
            if let found = side.identityName { identity = found }
        }
        title = identity ?? offer.subjectId.map(name) ?? offer.category
        traits = identity.map { NameTraits($0) }
        if let currency = offer.askCurrency, currency == priceCurrency, let sats = offer.askSats {
            price = Double(sats) / 100_000_000
        } else {
            price = nil
        }
    }

    var isIdentity: Bool { traits != nil }

    var kindLabel: String {
        switch offer.kind {
        case .identityForSale: "For sale"
        case .identityWanted: "Wanted"
        case .currencySwap: "Swap"
        }
    }

    static let priceBands: [(label: String, low: Double, high: Double)] = [
        ("21 or less", 0, 21), ("22 to 100", 21, 100), ("101 to 225", 100, 225), ("226 to 500", 225, 500),
        ("501 to 2,500", 500, 2_500), ("More than 2,500", 2_500, .infinity),
    ]

    static func priceBand(_ price: Double) -> String? {
        priceBands.first { price > $0.low && price <= $0.high }?.label
    }

    static func lengthBand(_ length: Int) -> String {
        length >= 8 ? "8 or more" : length >= 5 ? "5 to 7" : "\(length)"
    }
}

extension Double {
    func rounded(toSignificant digits: Int) -> Double {
        guard self != 0, isFinite else { return self }
        let scale = pow(10, Double(digits) - ceil(log10(abs(self))))
        return (self * scale).rounded() / scale
    }
}

// MARK: - Rows

/// One filter option: its name, its count for the current selection, and a bar for the
/// count. An option with no offers stays in place and cannot be selected.
private struct FacetOption: View {
    let label: String
    let count: Int
    let share: Double
    let on: Bool
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(spacing: 6) {
                Image(systemName: on ? "checkmark.circle.fill" : "circle")
                    .foregroundStyle(on ? Color.accentColor : .secondary)
                    .imageScale(.small)
                Text(label).fontWeight(on ? .semibold : .regular).lineLimit(1)
                Spacer(minLength: 4)
                Text(Format.integer(UInt64(count))).foregroundStyle(.secondary).monospacedDigit()
            }
            .font(.callout)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(alignment: .leading) {
                GeometryReader { geometry in
                    RoundedRectangle(cornerRadius: 5)
                        .fill(Color.accentColor.opacity(on ? 0.28 : 0.14))
                        .frame(width: geometry.size.width * CGFloat(share))
                }
            }
            .overlay {
                if on { RoundedRectangle(cornerRadius: 5).stroke(Color.accentColor, lineWidth: 1) }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(count == 0 && !on)
        .opacity(count == 0 && !on ? 0.5 : 1)
        .accessibilityLabel("\(label), \(count) offers")
        .accessibilityValue(on ? "selected" : "not selected")
    }
}

/// One offer on one line: the name, the price with a bar, when it was offered and when it
/// expires, the seller, and the links to each object that the offer refers to.
private struct OfferLine: View {
    let item: MarketItem
    let detail: OfferDetail?
    let ticker: (String) -> String
    let priceTicker: String
    let priceBounds: (low: Double, high: Double)

    @Environment(AppModel.self) private var model
    @State private var addresses: [String]?
    @State private var showsPoints = false
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    private var offer: Offer { item.offer }

    var body: some View {
        let _ = (showsNames, showsDates)
        HStack(alignment: .center, spacing: 14) {
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 8) {
                    Button {
                        open()
                    } label: {
                        // The isolate marks keep the "@" after a name in a right-to-left script.
                        Text(item.isIdentity ? "\u{2068}\(item.title)\u{2069}@" : item.title)
                            .font(.title3.weight(.medium))
                            .lineLimit(2)
                            .multilineTextAlignment(.leading)
                    }
                    .buttonStyle(.plain)
                    .help(offer.subjectId ?? offer.txid)
                    .cursorStop(token: offer.txid, copy: offer.subjectId ?? offer.txid) { open() }
                    if offer.kind != .identityForSale {
                        Text(item.kindLabel)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(offer.kind == .identityWanted ? Color.purple : .orange)
                    }
                }
                if let traits = item.traits, traits.kind == .other || traits.kind == .emoji {
                    Button {
                        showsPoints.toggle()
                    } label: {
                        Text(traits.scripts.isEmpty ? "no letters" : traits.scripts.joined(separator: ", "))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .help("Show the code points of this name")
                    .popover(isPresented: $showsPoints) {
                        VStack(alignment: .leading, spacing: 6) {
                            Text("\(traits.length) character\(traits.length == 1 ? "" : "s"), code points:")
                            Text(traits.codePoints).font(.callout.monospaced()).textSelection(.enabled)
                        }
                        .padding(12)
                        .frame(maxWidth: 420)
                    }
                }
                if offer.kind != .identityForSale {
                    Text("\(asset(offer.offered)) for \(asset(offer.accepted))")
                        .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            VStack(alignment: .leading, spacing: 3) {
                Text(ask).fontWeight(.semibold).monospacedDigit().lineLimit(1)
                if let price = item.price {
                    GeometryReader { geometry in
                        RoundedRectangle(cornerRadius: 2)
                            .fill(Color.accentColor)
                            .frame(width: max(geometry.size.width * barShare(price), 3))
                    }
                    .frame(height: 4)
                }
            }
            .frame(width: 132, alignment: .leading)

            VStack(alignment: .leading, spacing: 1) {
                if let time = detail?.blockTime {
                    Text("offered \(MarketplaceView.age(since: time)) ago").help(Format.date(time))
                }
                Text(expiry)
            }
            .font(.caption)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .frame(width: 166, alignment: .leading)

            Group {
                if let holder = detail?.holder {
                    Button(Format.short(holder, head: 6, tail: 4)) { model.open(Route.holder(holder)) }
                        .buttonStyle(.link)
                        .help("The seller: \(holder)")
                } else {
                    Text(" ")
                }
            }
            .font(.callout)
            .frame(width: 104, alignment: .leading)

            Menu {
                Button("Funding Transaction") { model.open(.transaction(txid: offer.txid)) }
                ForEach(offer.links.filter { $0.kind == .identity }, id: \.self) { link in
                    Button("Identity \(identityName(link.value))") { model.open(.identity(name: link.value)) }
                }
                ForEach(offer.links.filter { $0.kind == .currency }, id: \.self) { link in
                    Button("Currency \(ticker(link.value))") { model.open(.currency(name: link.value)) }
                }
                if let addresses {
                    ForEach(addresses, id: \.self) { address in
                        Button("Address \(Format.short(address, head: 6, tail: 4))") { model.open(Route.holder(address)) }
                    }
                } else {
                    Button("Read Funding Addresses") { Task { await loadAddresses() } }
                }
            } label: {
                Image(systemName: "link").accessibilityLabel("Links")
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("The objects that this offer refers to")
        }
        .padding(.vertical, 7)
    }

    private func open() {
        if let id = offer.subjectId, item.isIdentity {
            model.open(.identity(name: id))
        } else {
            model.open(.transaction(txid: offer.txid))
        }
    }

    private func loadAddresses() async {
        guard let explorer = model.explorer else { return }
        addresses = await explorer.offerFunding(txids: [offer.txid]).first?.addresses ?? []
    }

    /// The bar's length on the same logarithmic scale as the price curve.
    private func barShare(_ price: Double) -> CGFloat {
        let low = log10(priceBounds.low), high = log10(priceBounds.high)
        guard high > low else { return 1 }
        return CGFloat(min(max((log10(max(price, priceBounds.low)) - low) / (high - low), 0.02), 1))
    }

    private func identityName(_ id: String) -> String {
        guard showsNames else { return Format.short(id, head: 6, tail: 4) }
        for side in [offer.offered, offer.accepted] where side.identityId == id {
            if let name = side.identityName { return "\(name)@" }
        }
        return model.names[id] ?? Format.short(id, head: 6, tail: 4)
    }

    /// As Carbide: the exact asking amount, or what kind of ask it is.
    private var ask: String {
        if let currency = offer.askCurrency, let sats = offer.askSats {
            return "\(Format.coins(sats)) \(ticker(currency))"
        }
        if offer.accepted.identityId != nil || offer.accepted.identityName != nil {
            return "Identity exchange"
        }
        return "Asset bundle"
    }

    private func asset(_ side: OfferAsset) -> String {
        if let id = side.identityId { return "ID \(identityName(id))" }
        if let name = side.identityName { return "ID \(name)@" }
        if side.amounts.isEmpty { return "asset" }
        return side.amounts.map { "\(Format.amount($0.amount)) \(ticker($0.currency))" }.joined(separator: " + ")
    }

    private var expiry: String {
        guard offer.expiryHeight > 0 else { return "no expiry" }
        guard let tip = model.status?.blocks else { return "expires at \(heightText(offer.expiryHeight, model: model))" }
        if offer.expiryHeight <= tip { return "expired at \(heightText(offer.expiryHeight, model: model))" }
        let left = MarketplaceView.timeLeft(Double(offer.expiryHeight - tip) / 60)
        return showsDates
            ? "expires in \(left), \(heightText(offer.expiryHeight, model: model))"
            : "expires in \(left), at \(Format.integer(offer.expiryHeight))"
    }
}
