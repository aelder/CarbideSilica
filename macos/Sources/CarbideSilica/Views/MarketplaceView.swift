import CarbideCore
import SwiftUI

/// Open offers involving the chain's own coin, as Carbide requests them.
///
/// The board places each offer by how long it has left and what it costs. Both things
/// that matter are on the axes at once: an offer about to lapse sits in the shaded band on
/// the left, and one priced far from the rest stands apart. Both axes are logarithmic,
/// because expiry runs from minutes to years and prices span many magnitudes. The list
/// below is the text form of every offer, in Carbide's four sort orders.
struct MarketplaceView: View {
    @Environment(AppModel.self) private var model
    @State private var market: Marketplace?
    @State private var failure: String?
    @State private var loading = true
    @State private var filter = Filter.all
    @State private var hovered: String?
    @State private var sort = OfferSort.expiry
    @State private var descending = false
    @State private var sorted: [Offer] = []
    @State private var shown = 100
    /// From each offer's funding transaction: when it was listed and its addresses.
    @State private var funding: [String: OfferFunding] = [:]
    @State private var fundingTask: Task<Void, Never>?
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    enum Filter: String, CaseIterable, Identifiable {
        case all = "All"
        case sale = "For sale"
        case wanted = "Wanted"
        case swap = "Swaps"
        var id: String { rawValue }
    }

    private struct Point {
        let offer: Offer
        let hours: Double
        let price: Double
    }

    var body: some View {
        let _ = showsNames
        ScrollViewReader { proxy in
            Page {
                Card("Open offers", trailing: market.map { "\($0.offers.count) involving \($0.subject)" }) {
                    if loading && market == nil {
                        ProgressView("Loading offers…").frame(maxWidth: .infinity, minHeight: 240)
                    } else if let failure, market == nil {
                        Note(text: failure, warning: true)
                    } else {
                        board
                    }
                }
                if let market {
                    Card("Offers", trailing: "\(sorted.count) shown") {
                        listControls
                            .id("offers-top")
                        if market.truncated {
                            Note(text: "The node returned more offers than the app keeps. The app shows the first 5,000.", warning: true)
                        }
                        if sorted.isEmpty {
                            Text("No open offers were returned for this currency.").foregroundStyle(.secondary)
                        }
                        LazyVStack(spacing: 0) {
                            ForEach(sorted.prefix(shown), id: \.txid) { offer in
                                Divider()
                                OfferRow(
                                    offer: offer, funding: funding[offer.txid], ticker: name,
                                    loadAddresses: { Task { await loadFunding([offer.txid]) } })
                            }
                        }
                        if sorted.count > shown {
                            Button("Show \(min(100, sorted.count - shown)) More") { shown += 100 }
                        }
                    }
                }
            }
            .onChange(of: SortKey(sort: sort, descending: descending, filter: filter)) {
                // As in Carbide, a new sort order returns to the top.
                shown = 100
                resort()
                proxy.scrollTo("offers-top", anchor: .top)
            }
        }
        .navigationTitle("Marketplace")
        .task(id: model.generation) { await load() }
        .onDisappear {
            fundingTask?.cancel()
            fundingTask = nil
        }
    }

    private struct SortKey: Equatable {
        let sort: OfferSort
        let descending: Bool
        let filter: Filter
    }

    // MARK: Board

    @ViewBuilder
    private var board: some View {
        HStack {
            Picker("Show", selection: $filter) {
                ForEach(Filter.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 360)
            Spacer()
            if loading { ProgressView().controlSize(.small) }
            Button("Refresh") { Task { await load() } }.disabled(loading)
        }
        if let market {
            ProvenanceNote(provenance: market.provenance)
            Text("Request: getoffers for \(market.subject).")
                .font(.caption).foregroundStyle(.secondary)
        }
        if let failure {
            Note(text: failure, warning: true)
        }
        if points.isEmpty {
            Text("No offer priced in \(model.coin) with time left matches the filter.").foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, minHeight: 120)
        } else {
            GeometryReader { geometry in
                Canvas { context, size in
                    draw(context, size)
                }
                .contentShape(Rectangle())
                .onContinuousHover { phase in
                    if case .active(let point) = phase {
                        hovered = nearest(to: point, in: geometry.size)?.offer.txid
                    } else {
                        hovered = nil
                    }
                }
                .onTapGesture(coordinateSpace: .local) { point in
                    if let found = nearest(to: point, in: geometry.size) {
                        model.open(.transaction(txid: found.offer.txid))
                    }
                }
            }
            .frame(height: 340)
            Text(caption)
                .font(.callout)
                .foregroundStyle(hovered == nil ? .secondary : .primary)
                .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
        }
        let excluded = filtered.count - points.count
        if excluded > 0 {
            Text("\(excluded) offer\(excluded == 1 ? " is" : "s are") not on the board. The ask is not one amount of \(model.coin), or the offer has no time left. The list below shows every offer.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    private var listControls: some View {
        HStack {
            Picker("Sort by", selection: $sort) {
                Text("Expiry").tag(OfferSort.expiry)
                Text("Name").tag(OfferSort.name)
                Text("Ask amount").tag(OfferSort.askAmount)
                Text("Listing age").tag(OfferSort.listingAge)
            }
            .pickerStyle(.segmented)
            .frame(maxWidth: 420)
            Toggle("Reverse", isOn: $descending).toggleStyle(.button)
            Spacer()
            Text(sortNote).font(.caption).foregroundStyle(.secondary)
        }
    }

    private var sortNote: String {
        switch sort {
        case .expiry: return descending ? "Latest expiry first" : "Soonest expiry first"
        case .name: return descending ? "Names Z to A" : "Names A to Z"
        case .askAmount: return "Grouped by ask currency, \(descending ? "largest" : "smallest") first. Offers with no single ask last."
        case .listingAge:
            let known = sorted.filter { funding[$0.txid]?.blockTime != nil }.count
            let order = descending ? "Oldest listing first" : "Newest listing first"
            return known < sorted.count ? "\(order). Listing times: \(known) of \(sorted.count) loaded." : order
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
            resort()
            let currencies = Set(found.offers.flatMap { $0.links.filter { $0.kind == .currency }.map(\.value) })
            await model.resolveNames(Array(currencies))
            resort()
        } catch is CancellationError {
        } catch {
            failure = describe(error)
        }
    }

    private var filtered: [Offer] {
        (market?.offers ?? []).filter { offer in
            switch filter {
            case .all: true
            case .sale: offer.kind == .identityForSale
            case .wanted: offer.kind == .identityWanted
            case .swap: offer.kind == .currencySwap
            }
        }
    }

    private func resort() {
        sorted = sortOffers(
            offers: filtered, sort: sort, descending: descending, names: sortNames,
            times: funding.compactMapValues(\.blockTime))
        if sort == .listingAge { startFundingLoad() }
    }

    /// Listing age needs each funding transaction. Carbide requests them in the
    /// background while that sort is active, and so does this view.
    private func startFundingLoad() {
        guard fundingTask == nil else { return }
        fundingTask = Task {
            let missing = sorted.map(\.txid).filter { funding[$0] == nil }
            for start in stride(from: 0, to: missing.count, by: 64) {
                if Task.isCancelled || sort != .listingAge { break }
                await loadFunding(Array(missing[start..<min(start + 64, missing.count)]))
                sorted = sortOffers(
                    offers: filtered, sort: sort, descending: descending, names: sortNames,
                    times: funding.compactMapValues(\.blockTime))
            }
            fundingTask = nil
        }
    }

    /// Resolved names for the name sort, without the trailing "@".
    private var sortNames: [String: String] {
        model.names.mapValues { $0.hasSuffix("@") ? String($0.dropLast()) : $0 }
    }

    private func loadFunding(_ txids: [String]) async {
        guard let explorer = model.explorer else { return }
        for item in await explorer.offerFunding(txids: txids) {
            funding[item.txid] = item
        }
    }

    private func name(_ id: String) -> String {
        knownName(id, model: model)
    }

    // MARK: Board geometry

    /// Offers whose ask is one amount of the chain's own coin and that have time left, so
    /// they share one price axis.
    private var points: [Point] {
        guard let tip = model.status?.blocks, let chain = model.status?.chainId else { return [] }
        return filtered.compactMap { offer in
            guard offer.askCurrency == chain, let sats = offer.askSats, sats > 0, offer.expiryHeight > tip
            else { return nil }
            return Point(offer: offer, hours: Double(offer.expiryHeight - tip) / 60, price: Double(sats) / 1e8)
        }
    }

    private func label(_ kind: OfferKind) -> String {
        switch kind {
        case .identityForSale: "For sale"
        case .identityWanted: "Wanted"
        case .currencySwap: "Swap"
        }
    }

    private var caption: String {
        guard let point = points.first(where: { $0.offer.txid == hovered }) else {
            return "Offers on the left expire sooner. Offers at the top cost more. A circle is a sale. "
                + "A square is a request to buy. A diamond is a swap. Click an offer to open its funding transaction."
        }
        return "\(OfferRow.subject(point.offer, name: name)) · \(label(point.offer.kind).lowercased()) at "
            + "\(Format.amount(point.price)) \(model.coin) · \(OfferRow.timeLeft(point.hours))"
    }

    // Both axes take their range from the offers shown, rounded out to whole powers of ten,
    // so the points use the space instead of crowding one corner.
    private var hourRange: (Double, Double) {
        let hours = points.map(\.hours)
        let low = pow(10, floor(log10(max(hours.min() ?? 1, 0.1))))
        let high = pow(10, ceil(log10(max(hours.max() ?? 10, low * 10))))
        return (low, high)
    }
    private var minHours: Double { hourRange.0 }
    private var maxHours: Double { hourRange.1 }
    private let left: CGFloat = 70
    private let bottomInset: CGFloat = 38

    private var priceRange: (Double, Double) {
        let prices = points.map(\.price)
        let low = pow(10, floor(log10(max(prices.min() ?? 1, 1e-8))))
        let high = pow(10, ceil(log10(max(prices.max() ?? 10, low * 10))))
        return (low, high)
    }

    private func position(hours: Double, price: Double, in size: CGSize) -> CGPoint {
        let (low, high) = priceRange
        let fx = (log10(min(max(hours, minHours), maxHours)) - log10(minHours))
            / (log10(maxHours) - log10(minHours))
        let fy = (log10(price) - log10(low)) / (log10(high) - log10(low))
        return CGPoint(
            x: left + CGFloat(fx) * (size.width - left - 12),
            y: (size.height - bottomInset) - CGFloat(fy) * (size.height - bottomInset - 14))
    }

    private func nearest(to location: CGPoint, in size: CGSize) -> Point? {
        var best: Point?
        var distance: CGFloat = 12
        for point in points {
            let at = position(hours: point.hours, price: point.price, in: size)
            let gap = hypot(at.x - location.x, at.y - location.y)
            if gap < distance {
                distance = gap
                best = point
            }
        }
        return best
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize) {
        let (low, high) = priceRange
        let plotBottom = size.height - bottomInset
        let xFor = { (hours: Double) -> CGFloat in
            position(hours: hours, price: low, in: size).x
        }
        // Expiring within a day, when the range reaches that far left.
        if minHours < 24 {
            context.fill(
                Path(CGRect(x: left, y: 14, width: xFor(24) - left, height: plotBottom - 14)),
                with: .color(.red.opacity(0.1)))
            context.draw(
                Text("expires within a day").font(.caption2).foregroundStyle(.red),
                at: CGPoint(x: left + 4, y: 22), anchor: .leading)
        }

        var price = low
        while price <= high * 1.0001 {
            let fy = (log10(price) - log10(low)) / (log10(high) - log10(low))
            let y = plotBottom - CGFloat(fy) * (plotBottom - 14)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: left, y: y))
                    path.addLine(to: CGPoint(x: size.width - 12, y: y))
                }, with: .color(.secondary.opacity(0.18)), lineWidth: 1)
            context.draw(
                Text(price.formatted(.number.precision(.fractionLength(0...8)))).font(.caption2)
                    .foregroundStyle(.secondary),
                at: CGPoint(x: left - 8, y: y), anchor: .trailing)
            price *= 10
        }
        let ticks: [(Double, String)] = [
            (1, "1 h"), (24, "1 day"), (168, "1 week"), (720, "1 month"), (8760, "1 year"),
            (87600, "10 years"),
        ]
        for (hours, label) in ticks where hours >= minHours && hours <= maxHours {
            context.draw(
                Text(label).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: xFor(hours), y: plotBottom + 12), anchor: .center)
        }
        context.draw(
            Text("time before the offer expires (log scale)").font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: size.width - 12, y: plotBottom + 28), anchor: .trailing)
        context.draw(
            Text("Ask in \(model.coin) (log scale)").font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: 0, y: 4), anchor: .leading)

        for point in points {
            let at = position(hours: point.hours, price: point.price, in: size)
            let selected = hovered == point.offer.txid
            let radius: CGFloat = selected ? 7 : 4.5
            let box = CGRect(x: at.x - radius, y: at.y - radius, width: radius * 2, height: radius * 2)
            switch point.offer.kind {
            case .identityForSale:
                context.fill(Path(ellipseIn: box), with: .color(.accentColor.opacity(selected ? 1 : 0.6)))
            case .identityWanted:
                context.stroke(Path(box), with: .color(.purple), lineWidth: 2)
            case .currencySwap:
                let diamond = Path { path in
                    path.move(to: CGPoint(x: at.x, y: box.minY))
                    path.addLine(to: CGPoint(x: box.maxX, y: at.y))
                    path.addLine(to: CGPoint(x: at.x, y: box.maxY))
                    path.addLine(to: CGPoint(x: box.minX, y: at.y))
                    path.closeSubpath()
                }
                context.fill(diamond, with: .color(.orange.opacity(selected ? 1 : 0.75)))
            }
            if selected {
                context.stroke(Path(ellipseIn: box.insetBy(dx: -2, dy: -2)), with: .color(.primary), lineWidth: 1.5)
            }
        }
    }
}

/// One offer as Carbide's tile shows it: the funding transaction and expiry, what is
/// offered for what, the subject and the ask, and every association as a link.
private struct OfferRow: View {
    let offer: Offer
    let funding: OfferFunding?
    let ticker: (String) -> String
    let loadAddresses: () -> Void
    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    var body: some View {
        let _ = (showsNames, showsDates)
        VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 10) {
                Text(kindWord).font(.callout.weight(.semibold)).foregroundStyle(kindColor)
                    .frame(width: 70, alignment: .leading)
                Text(Self.subject(offer, name: ticker)).fontWeight(.medium).lineLimit(1)
                Spacer()
                Text(ask).monospacedDigit()
                Text(expiry).foregroundStyle(.secondary).frame(minWidth: 150, alignment: .trailing)
            }
            HStack(spacing: 6) {
                Text(asset(offer.offered)).foregroundStyle(.green)
                Image(systemName: "arrow.right").foregroundStyle(.secondary).accessibilityLabel("for")
                Text(asset(offer.accepted)).foregroundStyle(.orange)
                if let time = funding?.blockTime {
                    Spacer()
                    Text("listed \(Format.date(time))").font(.caption).foregroundStyle(.secondary)
                }
            }
            .font(.callout)
            .lineLimit(1)
            // The associations as a row of labelled fields, separated by dots.
            HStack(spacing: 8) {
                Text("Links").foregroundStyle(.secondary).frame(width: 70, alignment: .leading)
                ForEach(Array(links.enumerated()), id: \.element) { index, link in
                    if index > 0 {
                        Text("·").foregroundStyle(.tertiary).accessibilityHidden(true)
                    }
                    HStack(spacing: 4) {
                        Text(fieldName(link.kind)).foregroundStyle(.secondary)
                        linkView(link)
                    }
                    .fixedSize()
                }
                if funding == nil {
                    Text("·").foregroundStyle(.tertiary).accessibilityHidden(true)
                    Button("Show Addresses", action: loadAddresses)
                        .buttonStyle(.borderless)
                        .controlSize(.small)
                } else if let error = funding?.error {
                    Text(error).font(.caption).foregroundStyle(.orange).lineLimit(1)
                }
                Spacer(minLength: 0)
            }
            .font(.callout)
            .lineLimit(1)
        }
        .padding(.vertical, 8)
    }

    private func fieldName(_ kind: OfferLinkKind) -> String {
        switch kind {
        case .funding: "Funding"
        case .identity: "Identity"
        case .currency: "Currency"
        case .address: "Address"
        }
    }

    private var links: [OfferLink] {
        offerLinksWith(offer: offer, addresses: funding?.addresses ?? [])
    }

    @ViewBuilder
    private func linkView(_ link: OfferLink) -> some View {
        switch link.kind {
        case .funding:
            Button(Format.short(link.value, head: 6, tail: 4)) { model.open(.transaction(txid: link.value)) }
                .buttonStyle(.link).help(link.value)
        case .identity:
            Button(identityName(link.value)) { model.open(.identity(name: link.value)) }
                .buttonStyle(.link).help(link.value)
        case .currency:
            Button(ticker(link.value)) { model.open(.currency(name: link.value)) }
                .buttonStyle(.link).help(link.value)
        case .address:
            Button(Format.short(link.value, head: 6, tail: 4)) {
                model.open(Route.holder(link.value))
            }
            .buttonStyle(.link).help(link.value)
        }
    }

    private func identityName(_ id: String) -> String {
        guard showsNames else { return Format.short(id, head: 6, tail: 4) }
        for side in [offer.offered, offer.accepted] where side.identityId == id {
            if let name = side.identityName { return "\(name)@" }
        }
        return model.names[id] ?? Format.short(id, head: 6, tail: 4)
    }

    private var kindWord: String {
        switch offer.kind {
        case .identityForSale: "For sale"
        case .identityWanted: "Wanted"
        case .currencySwap: "Swap"
        }
    }

    private var kindColor: Color {
        switch offer.kind {
        case .identityForSale: .accentColor
        case .identityWanted: .purple
        case .currencySwap: .orange
        }
    }

    /// As Carbide: the exact asking amount, or what kind of ask it is.
    private var ask: String {
        if let currency = offer.askCurrency, let sats = offer.askSats {
            return "Ask \(Format.coins(sats)) \(ticker(currency))"
        }
        if offer.accepted.identityId != nil || offer.accepted.identityName != nil {
            return "Ask identity exchange"
        }
        return "Ask asset bundle"
    }

    private func asset(_ side: OfferAsset) -> String {
        if let id = side.identityId { return "ID \(identityName(id))" }
        if let name = side.identityName { return "ID \(name)@" }
        if side.amounts.isEmpty { return "asset" }
        return side.amounts.map { "\(Format.amount($0.amount)) \(ticker($0.currency))" }.joined(separator: " + ")
    }

    private var expiry: String {
        guard offer.expiryHeight > 0 else { return "no expiry" }
        let at = heightText(offer.expiryHeight, model: model)
        guard let tip = model.status?.blocks else { return "expires at \(at)" }
        if offer.expiryHeight <= tip { return "expired at \(at)" }
        return "expires in \(Self.timeLeft(Double(offer.expiryHeight - tip) / 60)), at \(at)"
    }

    /// The identity's name for an identity offer, otherwise the subject's name.
    static func subject(_ offer: Offer, name: (String) -> String) -> String {
        for side in [offer.offered, offer.accepted] where side.identityId == offer.subjectId {
            if let identity = side.identityName { return "\(identity)@" }
        }
        return offer.subjectId.map(name) ?? offer.category
    }

    static func timeLeft(_ hours: Double) -> String {
        if hours < 1 { return "\(max(Int(hours * 60), 1)) min" }
        if hours < 48 { return "\(Int(hours)) h" }
        if hours < 24 * 365 { return "\(Int(hours / 24)) days" }
        return "\((hours / 24 / 365).formatted(.number.precision(.fractionLength(1)))) years"
    }
}
