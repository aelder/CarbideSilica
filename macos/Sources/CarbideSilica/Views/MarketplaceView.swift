import CarbideCore
import SwiftUI

/// Open offers placed by how long they have left and what they cost.
///
/// Both things that matter are on the axes at once: an offer about to lapse sits in the
/// shaded band on the left, and one priced far from the rest stands apart. Both axes are
/// logarithmic, because expiry runs from minutes to years and prices span many magnitudes.
struct MarketplaceView: View {
    @Environment(AppModel.self) private var model
    @State private var offers: [MarketOffer] = []
    @State private var failure: String?
    @State private var loading = true
    @State private var filter = Filter.all
    @State private var hovered: String?

    enum Filter: String, CaseIterable, Identifiable {
        case all = "All"
        case sale = "For sale"
        case wanted = "Wanted"
        case swap = "Swaps"
        var id: String { rawValue }
    }

    private struct Point {
        let offer: MarketOffer
        let hours: Double
    }

    var body: some View {
        Page {
            Card("Open offers", trailing: loading ? nil : "\(points.count) priced in \(model.coin)") {
                if loading {
                    ProgressView("Loading offers…").frame(maxWidth: .infinity, minHeight: 240)
                } else if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.secondary)
                } else {
                    HStack {
                        Picker("Show", selection: $filter) {
                            ForEach(Filter.allCases) { Text($0.rawValue).tag($0) }
                        }
                        .pickerStyle(.segmented)
                        .labelsHidden()
                        .frame(maxWidth: 360)
                        Spacer()
                        Button("Refresh") { Task { await load() } }
                    }
                    if points.isEmpty {
                        Text("No open offers match the filter.").foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity, minHeight: 200)
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
                    if excluded > 0 {
                        Text("\(excluded) offer\(excluded == 1 ? " is" : "s are") not shown. The price is in a different currency, or the offer has expired.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                }
            }

            if !soonest.isEmpty {
                Card("Expiring soonest") {
                    VStack(spacing: 0) {
                        ForEach(Array(soonest.enumerated()), id: \.element.offer.txid) { index, point in
                            if index > 0 { Divider() }
                            HStack(spacing: 12) {
                                Text(label(point.offer.kind)).foregroundStyle(.secondary)
                                    .frame(width: 70, alignment: .leading)
                                Text(point.offer.subject).fontWeight(.medium).lineLimit(1)
                                Spacer()
                                Text("\(Format.amount(point.offer.price)) \(model.coin)").monospacedDigit()
                                Text(timeLeft(point.hours)).foregroundStyle(.secondary)
                                    .frame(width: 90, alignment: .trailing)
                                RouteLink(text: point.offer.txid, route: .transaction(txid: point.offer.txid))
                            }
                            .padding(.vertical, 7)
                        }
                    }
                }
            }
        }
        .navigationTitle("Marketplace")
        .task(id: model.generation) { await load() }
    }

    private func load() async {
        guard let explorer = model.explorer else { return }
        loading = true
        do {
            offers = try await explorer.offers()
            failure = nil
        } catch {
            offers = []
            failure = describe(error)
        }
        loading = false
    }

    /// Offers priced in the chain's own coin with time left, so they share one price axis.
    private var plottable: [Point] {
        guard let tip = model.status?.blocks, let chain = model.status?.chainId else { return [] }
        return offers.compactMap { offer in
            guard offer.priceCurrency == chain, offer.price > 0, let expiry = offer.expiryHeight, expiry > tip
            else { return nil }
            return Point(offer: offer, hours: Double(expiry - tip) / 60)
        }
    }

    private var points: [Point] {
        plottable.filter { point in
            switch filter {
            case .all: true
            case .sale: point.offer.kind == .identityForSale
            case .wanted: point.offer.kind == .identityWanted
            case .swap: point.offer.kind == .currencySwap
            }
        }
    }

    private var excluded: Int { offers.count - plottable.count }

    private var soonest: [Point] {
        Array(points.sorted { $0.hours < $1.hours }.prefix(12))
    }

    private func label(_ kind: OfferKind) -> String {
        switch kind {
        case .identityForSale: "For sale"
        case .identityWanted: "Wanted"
        case .currencySwap: "Swap"
        }
    }

    private func timeLeft(_ hours: Double) -> String {
        if hours < 1 { return "\(max(Int(hours * 60), 1)) min left" }
        if hours < 48 { return "\(Int(hours)) h left" }
        if hours < 24 * 365 { return "\(Int(hours / 24)) days left" }
        return "\((hours / 24 / 365).formatted(.number.precision(.fractionLength(1)))) years left"
    }

    private var caption: String {
        guard let point = points.first(where: { $0.offer.txid == hovered }) else {
            return "Offers on the left expire sooner. Offers at the top cost more. A circle is a sale. "
                + "A square is a request to buy. A diamond is a swap. Click an offer to open it."
        }
        return "\(point.offer.subject) · \(label(point.offer.kind).lowercased()) at "
            + "\(Format.amount(point.offer.price)) \(model.coin) · \(timeLeft(point.hours))"
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
        let prices = points.map(\.offer.price)
        let low = pow(10, floor(log10(max(prices.min() ?? 1, 1e-8))))
        let high = pow(10, ceil(log10(max(prices.max() ?? 10, low * 10))))
        return (low, high)
    }

    private func position(_ point: Point, in size: CGSize) -> CGPoint {
        let (low, high) = priceRange
        let fx = (log10(min(max(point.hours, minHours), maxHours)) - log10(minHours))
            / (log10(maxHours) - log10(minHours))
        let fy = (log10(point.offer.price) - log10(low)) / (log10(high) - log10(low))
        return CGPoint(
            x: left + CGFloat(fx) * (size.width - left - 12),
            y: (size.height - bottomInset) - CGFloat(fy) * (size.height - bottomInset - 14))
    }

    private func nearest(to location: CGPoint, in size: CGSize) -> Point? {
        var best: Point?
        var distance: CGFloat = 12
        for point in points {
            let at = position(point, in: size)
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
            position(Point(offer: points[0].offer, hours: hours), in: size).x
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
            Text("Price in \(model.coin) (log scale)").font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: 0, y: 4), anchor: .leading)

        for point in points {
            let at = position(point, in: size)
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
