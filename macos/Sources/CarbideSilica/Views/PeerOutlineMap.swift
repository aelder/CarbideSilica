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

/// A map that the app draws: the coasts as points, a light grid, and one mark for each
/// place that has peers. A mark is an area, because the place of an address is an estimate.
struct PeerOutlineMap: View {
    let pins: [(PeerRow, PeerLocation)]
    let open: (PeerRow) -> Void

    @State private var hovered: Int64?
    private let height: CGFloat = 270
    /// Peers nearer than this on the map are one mark.
    private let joinDistance: CGFloat = 22

    var body: some View {
        DisplayPanel(padding: 0) {
            GeometryReader { geometry in
                let size = geometry.size
                let frame = frame(for: size)
                let marks = marks(frame, size)
                Canvas { context, size in
                    draw(context, size, frame: frame, marks: marks)
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
            .clipShape(RoundedRectangle(cornerRadius: 4))
        }
    }

    private func frame(for size: CGSize) -> MapFrame {
        MapFrame(
            positions: pins.map { ($0.1.longitude ?? 0, $0.1.latitude ?? 0) },
            aspect: Double(size.width / max(size.height, 1)))
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

    private func mark(at point: CGPoint, in marks: [PeerMark]) -> PeerMark? {
        marks.first { hypot($0.centre.x - point.x, $0.centre.y - point.y) <= $0.radius + 6 }
    }

    private func caption(_ marks: [PeerMark]) -> String {
        guard let mark = marks.first(where: { $0.id == hovered }) else { return " " }
        return mark.peers.map { peer, location in
            var parts = ["Peer \(peer.id)", PeerMapCard.placeName(location)]
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

    private func draw(_ context: GraphicsContext, _ size: CGSize, frame: MapFrame, marks: [PeerMark]) {
        // A light grid each 30 degrees.
        for longitude in stride(from: -180.0, through: 180, by: 30) where longitude > frame.west && longitude < frame.east {
            let x = frame.point(longitude: longitude, latitude: 0, in: size).x
            context.rule(x, 0, x, size.height, Tone.grid)
        }
        for latitude in stride(from: -60.0, through: 60, by: 30) where latitude > frame.south && latitude < frame.north {
            let y = frame.point(longitude: 0, latitude: latitude, in: size).y
            context.rule(0, y, size.width, y, Tone.grid)
        }
        // The coasts.
        var coast = Path()
        for point in WorldOutline.points where frame.contains(longitude: point.longitude, latitude: point.latitude) {
            let at = frame.point(longitude: point.longitude, latitude: point.latitude, in: size)
            coast.addEllipse(in: CGRect(x: at.x - 1.1, y: at.y - 1.1, width: 2.2, height: 2.2))
        }
        context.fill(coast, with: .color(Tone.axis))
        // The peers. A hovered mark is the selection: it has light.
        for mark in marks {
            let radius = mark.radius
            let chosen = hovered == mark.id
            let color = chosen ? Tone.present : Tone.record
            let disc = Path(
                ellipseIn: CGRect(x: mark.centre.x - radius, y: mark.centre.y - radius, width: radius * 2, height: radius * 2))
            context.fill(disc, with: .color(Tone.record.opacity(chosen ? 0.28 : 0.16)))
            if chosen {
                context.drawLayer { layer in
                    layer.addFilter(.blur(radius: 3.5))
                    layer.stroke(disc, with: .color(Tone.present.opacity(0.8)), lineWidth: 2)
                }
            }
            context.stroke(disc, with: .color(color), lineWidth: chosen ? 1.5 : 1)
            if mark.peers.count > 1 {
                context.figure("\(mark.peers.count)", mark.centre.x, mark.centre.y, color: chosen ? Tone.present : Tone.value)
            } else {
                context.lit(
                    Path(ellipseIn: CGRect(x: mark.centre.x - 2, y: mark.centre.y - 2, width: 4, height: 4)),
                    color, strength: chosen ? 0.9 : 0)
            }
            // Place names stay in mixed case: a person reads them.
            let right = mark.centre.x < size.width - 150
            context.draw(
                Text(mark.label).font(.system(size: 11.5, weight: .medium))
                    .foregroundStyle(chosen ? Tone.present : Tone.value),
                at: CGPoint(x: mark.centre.x + (right ? radius + 6 : -radius - 6), y: mark.centre.y),
                anchor: right ? .leading : .trailing)
        }
    }
}
