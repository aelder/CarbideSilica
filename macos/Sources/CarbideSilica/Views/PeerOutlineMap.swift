import CarbideCore
import SwiftUI

/// The coast points of the world, read from the bridge once.
enum WorldOutline {
    static let points: [WorldPoint] = worldOutline()
}

/// The part of the world that a map shows, and where a position is in a view of that part.
///
/// The frame holds all the positions with room around them. It is never smaller than a
/// large region, so that the coast points are near enough to read as land, and it has the
/// proportions of the view. East and west are scaled by the cosine of the middle latitude,
/// which keeps shapes near that latitude correct.
struct MapFrame: Equatable {
    var west: Double
    var east: Double
    var south: Double
    var north: Double

    static let minimumWidth = 110.0
    static let minimumHeight = 40.0

    init(positions: [(longitude: Double, latitude: Double)], aspect: Double) {
        let longitudes = positions.map(\.longitude)
        let latitudes = positions.map(\.latitude)
        var west = longitudes.min() ?? -180
        var east = longitudes.max() ?? 180
        var south = latitudes.min() ?? -60
        var north = latitudes.max() ?? 80
        func grow(_ low: inout Double, _ high: inout Double, to span: Double) {
            let add = (span - (high - low)) / 2
            if add > 0 {
                low -= add
                high += add
            }
        }
        grow(&west, &east, to: max((east - west) * 1.25, (east - west) + 24, Self.minimumWidth))
        grow(&south, &north, to: max((north - south) * 1.25, (north - south) + 12, Self.minimumHeight))
        // The proportions of the view.
        let middle = (south + north) / 2
        let squeeze = max(cos(middle * .pi / 180), 0.3)
        let shown = (east - west) * squeeze / (north - south)
        if shown > aspect {
            grow(&south, &north, to: (east - west) * squeeze / aspect)
        } else {
            grow(&west, &east, to: (north - south) * aspect / squeeze)
        }
        // Stay on the world.
        if north - south > 170 {
            south = -85
            north = 85
        } else {
            if north > 85 {
                south -= north - 85
                north = 85
            }
            if south < -85 {
                north += -85 - south
                south = -85
            }
        }
        if east - west >= 360 {
            west = -180
            east = 180
        } else {
            if east > 180 {
                west -= east - 180
                east = 180
            }
            if west < -180 {
                east += -180 - west
                west = -180
            }
        }
        self.west = west
        self.east = east
        self.south = south
        self.north = north
    }

    func contains(longitude: Double, latitude: Double) -> Bool {
        longitude >= west && longitude <= east && latitude >= south && latitude <= north
    }

    func point(longitude: Double, latitude: Double, in size: CGSize) -> CGPoint {
        CGPoint(
            x: (longitude - west) / (east - west) * size.width,
            y: (north - latitude) / (north - south) * size.height)
    }
}

/// Peers that are one mark on the map, because their positions are near.
struct PeerMark: Identifiable {
    var peers: [(peer: PeerRow, location: PeerLocation)]
    var centre: CGPoint
    var id: Int64 { peers.first?.peer.id ?? 0 }

    var radius: CGFloat { min(9 + CGFloat(peers.count - 1) * 2.5, 18) }

    /// "Walla Walla, US" for one peer. "3 peers · Germany" for more.
    var label: String {
        if peers.count == 1, let location = peers.first?.location {
            return [location.city, location.countryCode].compactMap { $0 }.joined(separator: ", ")
        }
        let countries = Set(peers.compactMap(\.location.country))
        let place = countries.count == 1 ? countries.first ?? "" : "\(countries.count) countries"
        return "\(peers.count) peers · \(place)"
    }
}

/// The line from this node to one mark, along a grid as on a transit map.
struct PeerRoute: Equatable {
    /// The peer of the line.
    let id: Int64
    /// The mark that the line goes to. A mark with three peers has three lines.
    let mark: Int64
    /// The corners of the line, from this node to the mark.
    let points: [CGPoint]
    /// The time of one trip out and back on the map, in seconds.
    let round: Double
    /// True when the peer made the connection: the point then starts at the peer.
    let inbound: Bool

    /// The line with round corners.
    var path: CGPath {
        let path = CGMutablePath()
        guard let first = points.first else { return path }
        path.move(to: first)
        for index in 1..<points.count {
            if index < points.count - 1 {
                path.addArc(tangent1End: points[index], tangent2End: points[index + 1], radius: 7)
            } else {
                path.addLine(to: points[index])
            }
        }
        return path
    }
}

/// A map that the app draws: the coasts as points, a light grid, this node, one mark for
/// each place that has peers, and a line from this node to each mark. A mark is an area,
/// because the place of an address is an estimate.
struct PeerOutlineMap: View {
    let pins: [(PeerRow, PeerLocation)]
    /// The place of this node, when it is known.
    var home: PeerLocation?
    let open: (PeerRow) -> Void

    /// How many times slower than the response time the point on a line goes.
    static let slower: Double = 12

    @State private var hovered: Int64?
    private let height: CGFloat = 360
    /// Peers nearer than this on the map are one mark.
    private let joinDistance: CGFloat = 22

    var body: some View {
        DisplayPanel(padding: 0) {
            VStack(spacing: 0) {
                readouts
                    .padding(.horizontal, 18)
                    .padding(.vertical, 12)
                PanelRule()
                GeometryReader { geometry in
                    let size = geometry.size
                    let frame = frame(for: size)
                    let marks = marks(frame, size)
                    let origin = home.map {
                        frame.point(longitude: $0.longitude ?? 0, latitude: $0.latitude ?? 0, in: size)
                    }
                    let routes = origin.map { routes(from: $0, to: marks) } ?? []
                    Canvas { context, size in
                        draw(context, size, frame: frame, marks: marks, origin: origin, routes: routes)
                    }
                    .contentShape(Rectangle())
                    .onContinuousHover { phase in
                        if case .active(let point) = phase {
                            hovered = mark(at: point, in: marks)?.id
                        } else {
                            hovered = nil
                        }
                    }
                    .onTapGesture(coordinateSpace: .local) { point in
                        if let mark = mark(at: point, in: marks), mark.peers.count == 1 {
                            open(mark.peers[0].peer)
                        }
                    }
                    .accessibilityLabel(spoken(marks))
                    // The points that go out and come back on the lines.
                    .overlay { RoutePulses(routes: routes).allowsHitTesting(false) }
                    .overlay(alignment: .bottomLeading) {
                        Text(caption(marks))
                            .font(Tone.caption)
                            .foregroundStyle(Tone.value)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 4)
                            .background(Tone.screen.opacity(0.85))
                            .overlay(Rectangle().strokeBorder(Tone.rule, lineWidth: 1))
                            .padding(12)
                            .opacity(hovered == nil ? 0 : 1)
                            .allowsHitTesting(false)
                    }
                }
                .frame(height: height)
                .clipped()
            }
        }
    }

    /// The figures of the connections.
    private var readouts: some View {
        let times = pins.compactMap(\.0.pingMs)
        let inbound = pins.filter(\.0.inbound).count
        return HStack(alignment: .lastTextBaseline, spacing: 26) {
            Readout("Placed", "\(pins.count)", reserve: "00")
            Readout("Outbound", "\(pins.count - inbound)", reserve: "00")
            Readout("Inbound", "\(inbound)", reserve: "00")
            if let fastest = times.min(), let slowest = times.max() {
                Readout("Fastest", PeerScale.time(fastest), tone: Tone.record)
                Readout("Slowest", PeerScale.time(slowest))
            }
            Spacer(minLength: 0)
            if let home {
                Readout("This node", [home.city, home.countryCode].compactMap { $0 }.joined(separator: ", "), tone: Tone.present)
            }
        }
        .frame(minWidth: 0, maxWidth: .infinity, alignment: .leading)
        .clipped()
    }

    private func frame(for size: CGSize) -> MapFrame {
        var positions = pins.map { (longitude: $0.1.longitude ?? 0, latitude: $0.1.latitude ?? 0) }
        if let home { positions.append((longitude: home.longitude ?? 0, latitude: home.latitude ?? 0)) }
        return MapFrame(positions: positions, aspect: Double(size.width / max(size.height, 1)))
    }

    /// Joins the peers whose positions on the map are near, from west to east.
    private func marks(_ frame: MapFrame, _ size: CGSize) -> [PeerMark] {
        let placed = pins.map { peer, location in
            (peer, location, frame.point(longitude: location.longitude ?? 0, latitude: location.latitude ?? 0, in: size))
        }
        .sorted { ($0.2.x, $0.0.id) < ($1.2.x, $1.0.id) }
        var marks: [PeerMark] = []
        for (peer, location, point) in placed {
            if let index = marks.firstIndex(where: {
                hypot($0.centre.x - point.x, $0.centre.y - point.y) < joinDistance
            }) {
                let count = CGFloat(marks[index].peers.count)
                marks[index].centre = CGPoint(
                    x: (marks[index].centre.x * count + point.x) / (count + 1),
                    y: (marks[index].centre.y * count + point.y) / (count + 1))
                marks[index].peers.append((peer, location))
            } else {
                marks.append(PeerMark(peers: [(peer, location)], centre: point))
            }
        }
        return marks
    }

    /// One line from this node to each mark. A line has three parts, as on a transit map:
    /// straight along the longer direction, then at 45 degrees, then straight to the mark.
    /// Lines that go the same way start side by side and turn at different places.
    private func routes(from origin: CGPoint, to marks: [PeerMark]) -> [PeerRoute] {
        // The lines by the side of this node that they leave from.
        var sides: [Int: [PeerMark]] = [:]
        for mark in marks {
            let dx = mark.centre.x - origin.x, dy = mark.centre.y - origin.y
            guard hypot(dx, dy) > 14 else { continue }
            let side = abs(dx) >= abs(dy) ? (dx >= 0 ? 0 : 1) : (dy >= 0 ? 2 : 3)
            sides[side, default: []].append(mark)
        }
        var routes: [PeerRoute] = []
        let pipe: CGFloat = 4
        for (side, group) in sides {
            let level = side < 2
            // In the order across the direction of the line, so that lines do not cross at the start.
            let ordered = group.sorted {
                level ? $0.centre.y < $1.centre.y : $0.centre.x < $1.centre.x
            }
            // Each peer has a pipe. The pipes of all the marks of a side are side by side at
            // this node, with a little more room between two marks.
            let total = ordered.reduce(CGFloat(0)) { $0 + CGFloat($1.peers.count) * pipe } + CGFloat(ordered.count - 1) * 3
            var across = -total / 2
            for (rank, mark) in ordered.enumerated() {
                let count = mark.peers.count
                let lane = across + CGFloat(count) * pipe / 2
                across += CGFloat(count) * pipe + 3
                let start = CGPoint(x: origin.x + (level ? 0 : lane), y: origin.y + (level ? lane : 0))
                let dx = mark.centre.x - start.x, dy = mark.centre.y - start.y
                let slant = min(abs(dx), abs(dy))
                let straight = max(abs(dx), abs(dy)) - slant
                // Lines nearer the middle of the group turn later.
                let middle = abs(CGFloat(rank) - CGFloat(ordered.count - 1) / 2)
                let part = min(0.2 + 0.22 * (CGFloat(ordered.count) / 2 - middle), 0.8)
                let first = straight * part
                let sx: CGFloat = dx >= 0 ? 1 : -1, sy: CGFloat = dy >= 0 ? 1 : -1
                let turn = CGPoint(x: start.x + (level ? sx * first : 0), y: start.y + (level ? 0 : sy * first))
                let after = CGPoint(x: turn.x + sx * slant, y: turn.y + sy * slant)
                var points = [start, turn, after, mark.centre]
                // A part with no length is not a corner.
                points = points.enumerated().filter { index, point in
                    index == 0 || hypot(point.x - points[index - 1].x, point.y - points[index - 1].y) > 0.5
                }.map(\.element)
                // One pipe for each peer of the mark, side by side along the full line.
                let peers = mark.peers.sorted { $0.peer.id < $1.peer.id }
                for (index, entry) in peers.enumerated() {
                    let shift = (CGFloat(index) - CGFloat(count - 1) / 2) * pipe
                    let ms = entry.peer.pingMs ?? 200
                    routes.append(PeerRoute(
                        id: entry.peer.id, mark: mark.id, points: Self.beside(points, by: shift),
                        round: min(max(ms / 1000 * Self.slower, 0.5), 8), inbound: entry.peer.inbound))
                }
            }
        }
        return routes.sorted { $0.id < $1.id }
    }

    /// The line that is a constant distance to one side of a line with corners.
    private static func beside(_ points: [CGPoint], by distance: CGFloat) -> [CGPoint] {
        guard points.count > 1, distance != 0 else { return points }
        // The direction to the side of each part.
        let sides: [CGPoint] = (0..<points.count - 1).map { index in
            let dx = points[index + 1].x - points[index].x, dy = points[index + 1].y - points[index].y
            let length = max(hypot(dx, dy), 0.001)
            return CGPoint(x: -dy / length, y: dx / length)
        }
        return points.indices.map { index in
            let before = sides[max(index - 1, 0)], after = sides[min(index, sides.count - 1)]
            // At a corner the two parts meet: the point goes out along the mean direction.
            let sum = CGPoint(x: before.x + after.x, y: before.y + after.y)
            let scale = distance / max((sum.x * before.x + sum.y * before.y), 0.3)
            return CGPoint(x: points[index].x + sum.x * scale, y: points[index].y + sum.y * scale)
        }
    }

    private func mark(at point: CGPoint, in marks: [PeerMark]) -> PeerMark? {
        marks.first { hypot($0.centre.x - point.x, $0.centre.y - point.y) <= $0.radius + 6 }
    }

    private func caption(_ marks: [PeerMark]) -> String {
        guard let mark = marks.first(where: { $0.id == hovered }) else { return " " }
        return mark.peers.map { peer, location in
            var parts = ["Peer \(peer.id)", peer.inbound ? "inbound" : "outbound", PeerMapCard.placeName(location)]
            if let organization = location.organization { parts.append(organization) }
            if let ms = peer.pingMs { parts.append(PeerScale.time(ms)) }
            return parts.joined(separator: " · ")
        }
        .joined(separator: "\n")
    }

    private func spoken(_ marks: [PeerMark]) -> String {
        "A map with \(marks.count) \(marks.count == 1 ? "place" : "places"): "
            + marks.map(\.label).joined(separator: "; ") + ". The peer list has the same peers as text."
    }

    private func draw(
        _ context: GraphicsContext, _ size: CGSize, frame: MapFrame, marks: [PeerMark], origin: CGPoint?,
        routes: [PeerRoute]
    ) {
        // A light grid each 30 degrees, with its degrees at the edges.
        for longitude in stride(from: -180.0, through: 180, by: 30) where longitude > frame.west && longitude < frame.east {
            let x = frame.point(longitude: longitude, latitude: 0, in: size).x
            context.rule(x, 0, x, size.height, Tone.grid)
            let word = longitude == 0 ? "0" : "\(Int(abs(longitude)))\(longitude < 0 ? "W" : "E")"
            context.name(word, x + 4, size.height - 9, .leading, color: Tone.label.opacity(0.7))
        }
        for latitude in stride(from: -60.0, through: 60, by: 30) where latitude > frame.south && latitude < frame.north {
            let y = frame.point(longitude: 0, latitude: latitude, in: size).y
            context.rule(0, y, size.width, y, Tone.grid)
            let word = latitude == 0 ? "0" : "\(Int(abs(latitude)))\(latitude < 0 ? "S" : "N")"
            context.name(word, 6, y - 8, .leading, color: Tone.label.opacity(0.7))
        }
        // The coasts.
        var coast = Path()
        for point in WorldOutline.points where frame.contains(longitude: point.longitude, latitude: point.latitude) {
            let at = frame.point(longitude: point.longitude, latitude: point.latitude, in: size)
            coast.addRect(CGRect(x: at.x - 0.9, y: at.y - 0.9, width: 1.8, height: 1.8))
        }
        context.fill(coast, with: .color(Tone.axis))

        // The lines to the peers. The line of the mark under the pointer has light.
        for route in routes {
            let line = Path(route.path)
            if hovered == route.mark {
                context.lit(line.strokedPath(StrokeStyle(lineWidth: 2, lineJoin: .round)), Tone.present, strength: 0.7)
            } else {
                context.stroke(
                    line, with: .color(Tone.record.opacity(0.55)), style: StrokeStyle(lineWidth: 1.3, lineJoin: .round))
            }
        }

        // The peers: a square as a station of the line. A hovered mark is the selection.
        for mark in marks {
            let chosen = hovered == mark.id
            let color = chosen ? Tone.present : Tone.record
            let side: CGFloat = mark.peers.count > 1 ? max(16, CGFloat(mark.peers.count) * 4 + 6) : 9
            let box = CGRect(x: mark.centre.x - side / 2, y: mark.centre.y - side / 2, width: side, height: side)
            context.fill(Path(box), with: .color(Tone.screen))
            if chosen {
                context.lit(Path(box).strokedPath(StrokeStyle(lineWidth: 1.5)), Tone.present, strength: 0.9)
            } else {
                context.stroke(Path(box), with: .color(color), lineWidth: 1.5)
            }
            if mark.peers.count > 1 {
                context.name("\(mark.peers.count)", mark.centre.x, mark.centre.y, .center, color: chosen ? Tone.present : Tone.value)
            } else {
                context.fill(Path(box.insetBy(dx: 2.5, dy: 2.5)), with: .color(color))
            }
            let right = mark.centre.x < size.width - 150
            context.name(
                mark.label, mark.centre.x + (right ? side / 2 + 7 : -side / 2 - 7), mark.centre.y - 9,
                right ? .leading : .trailing, color: chosen ? Tone.present : Tone.value)
        }

        // This node: a lit square in corner marks.
        if let origin {
            let arm: CGFloat = 5, reach: CGFloat = 12
            var corners = Path()
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] as [(CGFloat, CGFloat)] {
                let corner = CGPoint(x: origin.x + sx * reach, y: origin.y + sy * reach)
                corners.move(to: CGPoint(x: corner.x - sx * arm, y: corner.y))
                corners.addLine(to: corner)
                corners.addLine(to: CGPoint(x: corner.x, y: corner.y - sy * arm))
            }
            context.fill(
                Path(CGRect(x: origin.x - reach, y: origin.y - reach, width: reach * 2, height: reach * 2)),
                with: .color(Tone.screen.opacity(0.7)))
            context.stroke(corners, with: .color(Tone.present), lineWidth: 1.2)
            context.lit(Path(CGRect(x: origin.x - 4, y: origin.y - 4, width: 8, height: 8)), Tone.present, strength: 0.9)
            context.plate("This node", x: origin.x - 33, y: origin.y + reach + 12, width: 66, fill: Tone.present)
        }
    }
}

/// The packets that go along the lines of the map, in the two directions at the same time.
///
/// Each line has some packets that go out and some that come in. One trip takes half the
/// response time of the peer, made slower. Each packet has its own speed, its own rest
/// after a trip, and its own start, and some go in pairs, so that the traffic does not
/// repeat as a pattern. The numbers come from the ID of the line, so that the traffic of a
/// line is the same each time the map is drawn.
///
/// It is Core Animation: the system moves the packets, and the app does no work for a frame.
private struct RoutePulses: NSViewRepresentable {
    let routes: [PeerRoute]

    final class PulseView: NSView {
        var shown: [PeerRoute] = []
        override var isFlipped: Bool { true }

        override init(frame: NSRect) {
            super.init(frame: frame)
            wantsLayer = true
        }

        required init?(coder: NSCoder) { nil }
    }

    /// A generator with a fixed start.
    private struct Steps {
        var state: UInt64
        mutating func unit() -> Double {
            state = state &* 6364136223846793005 &+ 1442695040888963407
            return Double(state >> 40) / Double(1 << 24)
        }
        mutating func between(_ low: Double, _ high: Double) -> Double { low + (high - low) * unit() }
    }

    func makeNSView(context: Context) -> PulseView { PulseView() }

    func updateNSView(_ view: PulseView, context: Context) {
        // The packets start again only when a line changes.
        guard view.shown != routes, let layer = view.layer else { return }
        view.shown = routes
        layer.sublayers?.forEach { $0.removeFromSuperlayer() }
        guard !NSWorkspace.shared.accessibilityDisplayShouldReduceMotion else { return }
        let out = CGColor(red: 0.96, green: 0.93, blue: 0.82, alpha: 1)
        let back = CGColor(red: 0.6, green: 0.92, blue: 0.86, alpha: 1)
        for route in routes where route.points.count > 1 {
            var random = Steps(state: UInt64(bitPattern: route.id) &* 0x9E37_79B9_7F4A_7C15 &+ 0x5EED)
            let forward = route.path
            let reversed = PeerRoute(
                    id: route.id, mark: route.mark, points: route.points.reversed(), round: route.round,
                inbound: route.inbound
            ).path
            let leg = route.round / 2
            for outward in [true, false] {
                let color = outward ? out : back
                for _ in 0..<3 {
                    // A packet is a little faster or slower than the mean.
                    let trip = leg * random.between(0.82, 1.25)
                    // Most rests are short. Some are long: a line is quiet for a time.
                    let rest = random.unit() < 0.3 ? random.between(3.5, 8) : random.between(0.3, 2.6)
                    let cycle = trip + rest
                    let start = random.between(0, cycle)
                    // Some packets go as two or three, one after the other.
                    let train = random.unit() < 0.4 ? (random.unit() < 0.4 ? 3 : 2) : 1
                    for car in 0..<train {
                        let packet = CALayer()
                        packet.bounds = CGRect(x: 0, y: 0, width: 5.5, height: 2.6)
                        packet.cornerRadius = 1.3
                        packet.backgroundColor = color
                        packet.shadowColor = color
                        packet.shadowOpacity = 1
                        packet.shadowRadius = 4
                        packet.shadowOffset = .zero
                        packet.opacity = 0
                        layer.addSublayer(packet)
                        let move = CAKeyframeAnimation(keyPath: "position")
                        move.path = outward ? forward : reversed
                        move.calculationMode = .paced
                        move.rotationMode = .rotateAuto
                        move.duration = trip
                        let seen = CAKeyframeAnimation(keyPath: "opacity")
                        seen.values = [1, 0, 0]
                        seen.keyTimes = [0, NSNumber(value: trip / cycle), 1]
                        seen.calculationMode = .discrete
                        seen.duration = cycle
                        let group = CAAnimationGroup()
                        group.animations = [move, seen]
                        group.duration = cycle
                        group.repeatCount = .infinity
                        group.isRemovedOnCompletion = false
                        // Each packet is at a different part of its cycle.
                        group.timeOffset = (start + cycle - Double(car) * 0.16).truncatingRemainder(dividingBy: cycle)
                        packet.add(group, forKey: "trip")
                    }
                }
            }
        }
    }
}
