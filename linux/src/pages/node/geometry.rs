//! The geometry of the connected node map (`PeerOutlineMap.swift`): the part of the world
//! that the map shows, the marks that join near peers, the octilinear line from this node to
//! each mark, the pipe of each peer beside the others, the round corners of a line, and the
//! timing of the packets on the lines. Everything here is pure, so the tests can check it.

use crate::bridge::api;

/// A point in the view, in logical pixels. Y grows downward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

pub fn pt(x: f64, y: f64) -> Pt {
    Pt { x, y }
}

fn distance(a: Pt, b: Pt) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// The part of the world that a map shows, and where a position is in a view of that part.
///
/// The frame holds all the positions with room around them. It is never smaller than a
/// large region, so that the coast points are near enough to read as land, and it has the
/// proportions of the view. East and west are scaled by the cosine of the middle latitude,
/// which keeps shapes near that latitude correct.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapFrame {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
}

impl MapFrame {
    pub const MINIMUM_WIDTH: f64 = 110.0;
    pub const MINIMUM_HEIGHT: f64 = 40.0;

    /// `positions` are (longitude, latitude).
    pub fn new(positions: &[(f64, f64)], aspect: f64) -> Self {
        let fold = |values: &mut dyn Iterator<Item = f64>, min: bool| -> Option<f64> {
            values.fold(None, |found: Option<f64>, v| match found {
                None => Some(v),
                Some(f) => Some(if min { f.min(v) } else { f.max(v) }),
            })
        };
        let mut west = fold(&mut positions.iter().map(|p| p.0), true).unwrap_or(-180.0);
        let mut east = fold(&mut positions.iter().map(|p| p.0), false).unwrap_or(180.0);
        let mut south = fold(&mut positions.iter().map(|p| p.1), true).unwrap_or(-60.0);
        let mut north = fold(&mut positions.iter().map(|p| p.1), false).unwrap_or(80.0);
        fn grow(low: &mut f64, high: &mut f64, span: f64) {
            let add = (span - (*high - *low)) / 2.0;
            if add > 0.0 {
                *low -= add;
                *high += add;
            }
        }
        let width = east - west;
        grow(&mut west, &mut east, (width * 1.25).max(width + 24.0).max(Self::MINIMUM_WIDTH));
        let height = north - south;
        grow(&mut south, &mut north, (height * 1.25).max(height + 12.0).max(Self::MINIMUM_HEIGHT));
        // The proportions of the view.
        let middle = (south + north) / 2.0;
        let squeeze = (middle * std::f64::consts::PI / 180.0).cos().max(0.3);
        let shown = (east - west) * squeeze / (north - south);
        if shown > aspect {
            let span = (east - west) * squeeze / aspect;
            grow(&mut south, &mut north, span);
        } else {
            let span = (north - south) * aspect / squeeze;
            grow(&mut west, &mut east, span);
        }
        // Stay on the world.
        if north - south > 170.0 {
            south = -85.0;
            north = 85.0;
        } else {
            if north > 85.0 {
                south -= north - 85.0;
                north = 85.0;
            }
            if south < -85.0 {
                north += -85.0 - south;
                south = -85.0;
            }
        }
        if east - west >= 360.0 {
            west = -180.0;
            east = 180.0;
        } else {
            if east > 180.0 {
                west -= east - 180.0;
                east = 180.0;
            }
            if west < -180.0 {
                east += -180.0 - west;
                west = -180.0;
            }
        }
        Self { west, east, south, north }
    }

    pub fn contains(&self, longitude: f64, latitude: f64) -> bool {
        longitude >= self.west && longitude <= self.east && latitude >= self.south && latitude <= self.north
    }

    pub fn point(&self, longitude: f64, latitude: f64, width: f64, height: f64) -> Pt {
        pt(
            (longitude - self.west) / (self.east - self.west) * width,
            (self.north - latitude) / (self.north - self.south) * height,
        )
    }
}

/// A peer with a place on the map.
pub type Pin = (api::PeerRow, api::PeerLocation);

/// Peers that are one mark on the map, because their positions are near.
#[derive(Clone, Debug)]
pub struct PeerMark {
    pub peers: Vec<Pin>,
    pub centre: Pt,
}

impl PeerMark {
    pub fn id(&self) -> i64 {
        self.peers.first().map(|p| p.0.id).unwrap_or(0)
    }

    pub fn radius(&self) -> f64 {
        (9.0 + (self.peers.len() as f64 - 1.0) * 2.5).min(18.0)
    }

    /// "Walla Walla, US" for one peer. "3 peers · Germany" for more.
    pub fn label(&self) -> String {
        if self.peers.len() == 1 {
            let location = &self.peers[0].1;
            return [location.city.clone(), location.country_code.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ");
        }
        let mut countries: Vec<String> = self.peers.iter().filter_map(|p| p.1.country.clone()).collect();
        countries.sort();
        countries.dedup();
        let place = if countries.len() == 1 { countries[0].clone() } else { format!("{} countries", countries.len()) };
        format!("{} peers · {place}", self.peers.len())
    }
}

/// Peers nearer than this on the map are one mark.
pub const JOIN_DISTANCE: f64 = 22.0;
/// The room of one peer's pipe beside the next, at this node and along the whole line.
pub const PIPE: f64 = 4.0;
/// How many times slower than the response time the point on a line goes.
pub const SLOWER: f64 = 12.0;
/// The radius of the round corners of a line.
pub const CORNER: f64 = 7.0;

/// Joins the peers whose positions on the map are near, from west to east.
pub fn marks(pins: &[Pin], frame: &MapFrame, width: f64, height: f64) -> Vec<PeerMark> {
    let mut placed: Vec<(Pin, Pt)> = pins
        .iter()
        .map(|(peer, location)| {
            let at = frame.point(location.longitude.unwrap_or(0.0), location.latitude.unwrap_or(0.0), width, height);
            ((peer.clone(), location.clone()), at)
        })
        .collect();
    placed.sort_by(|a, b| a.1.x.total_cmp(&b.1.x).then(a.0.0.id.cmp(&b.0.0.id)));
    let mut marks: Vec<PeerMark> = Vec::new();
    for (pin, point) in placed {
        if let Some(mark) = marks.iter_mut().find(|m| distance(m.centre, point) < JOIN_DISTANCE) {
            let count = mark.peers.len() as f64;
            mark.centre = pt((mark.centre.x * count + point.x) / (count + 1.0), (mark.centre.y * count + point.y) / (count + 1.0));
            mark.peers.push(pin);
        } else {
            marks.push(PeerMark { peers: vec![pin], centre: point });
        }
    }
    marks
}

/// The mark under a point.
pub fn mark_at(marks: &[PeerMark], point: Pt) -> Option<&PeerMark> {
    marks.iter().find(|m| distance(m.centre, point) <= m.radius() + 6.0)
}

/// The line from this node to one mark, along a grid as on a transit map.
#[derive(Clone, Debug, PartialEq)]
pub struct PeerRoute {
    /// The peer of the line.
    pub id: i64,
    /// The mark that the line goes to. A mark with three peers has three lines.
    pub mark: i64,
    /// The corners of the line, from this node to the mark.
    pub points: Vec<Pt>,
    /// The time of one trip out and back on the map, in seconds.
    pub round: f64,
    /// True when the peer made the connection.
    pub inbound: bool,
}

/// The time of one trip out and back for a response time: slower for a longer ping.
pub fn round_trip(ping_ms: Option<f64>) -> f64 {
    let ms = ping_ms.unwrap_or(200.0);
    (ms / 1000.0 * SLOWER).clamp(0.5, 8.0)
}

/// One line from this node to each mark. A line has three parts, as on a transit map:
/// straight along the longer direction, then at 45 degrees, then straight to the mark.
/// Lines that go the same way start side by side and turn at different places.
pub fn routes(origin: Pt, marks: &[PeerMark]) -> Vec<PeerRoute> {
    // The lines by the side of this node that they leave from.
    let mut sides: [Vec<&PeerMark>; 4] = Default::default();
    for mark in marks {
        let (dx, dy) = (mark.centre.x - origin.x, mark.centre.y - origin.y);
        if dx.hypot(dy) <= 14.0 {
            continue;
        }
        let side = if dx.abs() >= dy.abs() {
            if dx >= 0.0 { 0 } else { 1 }
        } else if dy >= 0.0 {
            2
        } else {
            3
        };
        sides[side].push(mark);
    }
    let mut routes = Vec::new();
    for (side, group) in sides.iter().enumerate() {
        if group.is_empty() {
            continue;
        }
        let level = side < 2;
        // In the order across the direction of the line, so that lines do not cross at the start.
        let mut ordered = group.clone();
        ordered.sort_by(|a, b| if level { a.centre.y.total_cmp(&b.centre.y) } else { a.centre.x.total_cmp(&b.centre.x) });
        // Each peer has a pipe. The pipes of all the marks of a side are side by side at
        // this node, with a little more room between two marks.
        let total: f64 = ordered.iter().map(|m| m.peers.len() as f64 * PIPE).sum::<f64>() + (ordered.len() as f64 - 1.0) * 3.0;
        let mut across = -total / 2.0;
        for (rank, mark) in ordered.iter().enumerate() {
            let count = mark.peers.len();
            let lane = across + count as f64 * PIPE / 2.0;
            across += count as f64 * PIPE + 3.0;
            let start = pt(origin.x + if level { 0.0 } else { lane }, origin.y + if level { lane } else { 0.0 });
            let (dx, dy) = (mark.centre.x - start.x, mark.centre.y - start.y);
            let slant = dx.abs().min(dy.abs());
            let straight = dx.abs().max(dy.abs()) - slant;
            // Lines nearer the middle of the group turn later.
            let middle = (rank as f64 - (ordered.len() as f64 - 1.0) / 2.0).abs();
            let part = (0.2 + 0.22 * (ordered.len() as f64 / 2.0 - middle)).min(0.8);
            let first = straight * part;
            let sx = if dx >= 0.0 { 1.0 } else { -1.0 };
            let sy = if dy >= 0.0 { 1.0 } else { -1.0 };
            let turn = pt(start.x + if level { sx * first } else { 0.0 }, start.y + if level { 0.0 } else { sy * first });
            let after = pt(turn.x + sx * slant, turn.y + sy * slant);
            let all = [start, turn, after, mark.centre];
            // A part with no length is not a corner.
            let points: Vec<Pt> = all
                .iter()
                .enumerate()
                .filter(|(index, point)| *index == 0 || distance(**point, all[index - 1]) > 0.5)
                .map(|(_, point)| *point)
                .collect();
            // One pipe for each peer of the mark, side by side along the full line.
            let mut peers: Vec<&Pin> = mark.peers.iter().collect();
            peers.sort_by_key(|p| p.0.id);
            for (index, (peer, _)) in peers.iter().enumerate() {
                let shift = (index as f64 - (count as f64 - 1.0) / 2.0) * PIPE;
                routes.push(PeerRoute {
                    id: peer.id,
                    mark: mark.id(),
                    points: beside(&points, shift),
                    round: round_trip(peer.ping_ms),
                    inbound: peer.inbound,
                });
            }
        }
    }
    routes.sort_by_key(|r| r.id);
    routes
}

/// The line that is a constant distance to one side of a line with corners.
pub fn beside(points: &[Pt], distance: f64) -> Vec<Pt> {
    if points.len() < 2 || distance == 0.0 {
        return points.to_vec();
    }
    // The direction to the side of each part.
    let sides: Vec<Pt> = (0..points.len() - 1)
        .map(|index| {
            let (dx, dy) = (points[index + 1].x - points[index].x, points[index + 1].y - points[index].y);
            let length = dx.hypot(dy).max(0.001);
            pt(-dy / length, dx / length)
        })
        .collect();
    (0..points.len())
        .map(|index| {
            let before = sides[index.saturating_sub(1)];
            let after = sides[index.min(sides.len() - 1)];
            // At a corner the two parts meet: the point goes out along the mean direction.
            let sum = pt(before.x + after.x, before.y + after.y);
            let scale = distance / (sum.x * before.x + sum.y * before.y).max(0.3);
            pt(points[index].x + sum.x * scale, points[index].y + sum.y * scale)
        })
        .collect()
}

/// The line with round corners, as a polyline fine enough to draw and to move along. Each
/// corner is an arc of `radius` that touches both parts, as `CGPath.addArc(tangent1End:
/// tangent2End:radius:)`.
pub fn rounded(points: &[Pt], radius: f64) -> Vec<Pt> {
    let Some(first) = points.first() else { return vec![] };
    let mut out = vec![*first];
    let mut current = *first;
    for index in 1..points.len() {
        if index < points.len() - 1 {
            let (corner, next) = (points[index], points[index + 1]);
            let (ax, ay) = (current.x - corner.x, current.y - corner.y);
            let (bx, by) = (next.x - corner.x, next.y - corner.y);
            let (la, lb) = (ax.hypot(ay), bx.hypot(by));
            if la < 1e-6 || lb < 1e-6 {
                out.push(corner);
                current = corner;
                continue;
            }
            let (ua, ub) = ((ax / la, ay / la), (bx / lb, by / lb));
            let cos = (ua.0 * ub.0 + ua.1 * ub.1).clamp(-1.0, 1.0);
            let angle = cos.acos();
            // Parts in one line: no arc.
            if angle > std::f64::consts::PI - 1e-6 || angle < 1e-6 {
                out.push(corner);
                current = corner;
                continue;
            }
            let reach = radius / (angle / 2.0).tan();
            let from = pt(corner.x + ua.0 * reach, corner.y + ua.1 * reach);
            let to = pt(corner.x + ub.0 * reach, corner.y + ub.1 * reach);
            // The centre is on the bisector, at radius / sin(angle / 2) from the corner.
            let (bisx, bisy) = (ua.0 + ub.0, ua.1 + ub.1);
            let bl = bisx.hypot(bisy).max(1e-9);
            let reach_centre = radius / (angle / 2.0).sin();
            let centre = pt(corner.x + bisx / bl * reach_centre, corner.y + bisy / bl * reach_centre);
            out.push(from);
            let start = (from.y - centre.y).atan2(from.x - centre.x);
            let end = (to.y - centre.y).atan2(to.x - centre.x);
            let mut sweep = end - start;
            while sweep > std::f64::consts::PI {
                sweep -= 2.0 * std::f64::consts::PI;
            }
            while sweep < -std::f64::consts::PI {
                sweep += 2.0 * std::f64::consts::PI;
            }
            let steps = ((sweep.abs() * radius / 1.5).ceil() as usize).max(2);
            for step in 1..=steps {
                let a = start + sweep * step as f64 / steps as f64;
                out.push(pt(centre.x + radius * a.cos(), centre.y + radius * a.sin()));
            }
            current = to;
        } else {
            out.push(points[index]);
            current = points[index];
        }
    }
    out
}

/// A polyline with its lengths, to find the point at a part of its length.
#[derive(Clone, Debug)]
pub struct Track {
    points: Vec<Pt>,
    /// The length from the start to each point.
    lengths: Vec<f64>,
}

impl Track {
    pub fn new(points: Vec<Pt>) -> Self {
        let mut lengths = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                total += distance(points[index - 1], *point);
            }
            lengths.push(total);
        }
        Self { points, lengths }
    }

    pub fn length(&self) -> f64 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    pub fn points(&self) -> &[Pt] {
        &self.points
    }

    /// The point at `share` of the length (paced), and the angle of the line there.
    pub fn at(&self, share: f64) -> Option<(Pt, f64)> {
        if self.points.len() < 2 {
            return None;
        }
        let goal = share.clamp(0.0, 1.0) * self.length();
        let index = self.lengths.partition_point(|l| *l < goal).clamp(1, self.points.len() - 1);
        let (a, b) = (self.points[index - 1], self.points[index]);
        let span = (self.lengths[index] - self.lengths[index - 1]).max(1e-9);
        let t = ((goal - self.lengths[index - 1]) / span).clamp(0.0, 1.0);
        let at = pt(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
        Some((at, (b.y - a.y).atan2(b.x - a.x)))
    }
}

/// A generator with a fixed start, the same as the Swift app's.
struct Steps {
    state: u64,
}

impl Steps {
    fn unit(&mut self) -> f64 {
        self.state = self.state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.state >> 40) as f64 / (1u64 << 24) as f64
    }

    fn between(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }
}

/// One packet on a line: the direction, the time of a trip, the length of its cycle (a trip
/// and a rest) and where in its cycle it is at the start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Packet {
    pub outward: bool,
    pub trip: f64,
    pub cycle: f64,
    pub offset: f64,
}

impl Packet {
    /// The part of the line that the packet has gone at `elapsed` seconds, or None while it
    /// rests.
    pub fn share(&self, elapsed: f64) -> Option<f64> {
        let local = (elapsed + self.offset).rem_euclid(self.cycle);
        (local < self.trip).then(|| local / self.trip)
    }
}

/// The packets of one line, out and back at the same time.
///
/// Each line has some packets that go out and some that come in. One trip takes half the
/// time of a round trip on the map. Each packet has its own speed, its own rest after a
/// trip, and its own start, and some go in pairs or threes, so that the traffic does not
/// repeat as a pattern. The numbers come from the ID of the line, so that the traffic of a
/// line is the same each time the map is drawn.
pub fn packets(route_id: i64, round: f64) -> Vec<Packet> {
    let mut random = Steps { state: (route_id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(0x5EED) };
    let leg = round / 2.0;
    let mut out = Vec::new();
    for outward in [true, false] {
        for _ in 0..3 {
            // A packet is a little faster or slower than the mean.
            let trip = leg * random.between(0.82, 1.25);
            // Most rests are short. Some are long: a line is quiet for a time.
            let rest = if random.unit() < 0.3 { random.between(3.5, 8.0) } else { random.between(0.3, 2.6) };
            let cycle = trip + rest;
            let start = random.between(0.0, cycle);
            // Some packets go as two or three, one after the other.
            let train = if random.unit() < 0.4 { if random.unit() < 0.4 { 3 } else { 2 } } else { 1 };
            for car in 0..train {
                let offset = (start + cycle - car as f64 * 0.16).rem_euclid(cycle);
                out.push(Packet { outward, trip, cycle, offset });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: i64, ping: Option<f64>, inbound: bool) -> api::PeerRow {
        api::PeerRow {
            id,
            address: format!("192.0.2.{id}:27485"),
            inbound,
            ping_ms: ping,
            synced_blocks: None,
            starting_height: None,
            bytes_sent: 0,
            bytes_received: 0,
            version: None,
            connected_since: None,
            json: "{}".into(),
        }
    }

    fn place(longitude: f64, latitude: f64, country: &str) -> api::PeerLocation {
        api::PeerLocation {
            address: String::new(),
            place: api::PeerPlace::Located,
            latitude: Some(latitude),
            longitude: Some(longitude),
            city: Some("Town".into()),
            country: Some(country.into()),
            country_code: Some("XX".into()),
            asn: None,
            organization: None,
        }
    }

    #[test]
    fn frame_holds_the_positions_with_the_view_proportions() {
        let frame = MapFrame::new(&[(8.0, 50.0), (13.0, 52.0)], 3.0);
        assert!(frame.contains(8.0, 50.0) && frame.contains(13.0, 52.0));
        assert!(frame.east - frame.west >= MapFrame::MINIMUM_WIDTH - 1e-9);
        let middle = (frame.south + frame.north) / 2.0;
        let squeeze = (middle.to_radians()).cos().max(0.3);
        let shown = (frame.east - frame.west) * squeeze / (frame.north - frame.south);
        assert!((shown - 3.0).abs() < 1e-6, "proportion {shown}");
        // No position: the whole world, within the limits.
        let world = MapFrame::new(&[], 2.0);
        assert!(world.west >= -180.0 && world.east <= 180.0 && world.south >= -85.0 && world.north <= 85.0);
    }

    #[test]
    fn near_peers_are_one_mark() {
        let frame = MapFrame::new(&[(8.0, 50.0), (8.1, 50.1), (100.0, 10.0)], 2.5);
        let pins = vec![(peer(2, Some(40.0), false), place(8.0, 50.0, "Germany")), (peer(1, Some(30.0), true), place(8.1, 50.1, "Germany")), (peer(3, None, false), place(100.0, 10.0, "Thailand"))];
        let marks = marks(&pins, &frame, 1000.0, 360.0);
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].peers.len(), 2);
        assert_eq!(marks[0].id(), 2, "the west-most peer names the mark");
        assert_eq!(marks[0].label(), "2 peers · Germany");
        assert_eq!(marks[1].label(), "Town, XX");
        assert!((marks[0].radius() - 11.5).abs() < 1e-9);
    }

    #[test]
    fn routes_are_octilinear_and_each_peer_keeps_its_pipe() {
        let origin = pt(100.0, 200.0);
        let mark = PeerMark {
            peers: vec![(peer(5, Some(50.0), false), place(0.0, 0.0, "A")), (peer(4, Some(90.0), true), place(0.0, 0.0, "A")), (peer(6, None, false), place(0.0, 0.0, "A"))],
            centre: pt(400.0, 100.0),
        };
        let routes = routes(origin, std::slice::from_ref(&mark));
        assert_eq!(routes.iter().map(|r| r.id).collect::<Vec<_>>(), vec![4, 5, 6]);
        // The middle pipe runs on the line itself: straight, then 45 degrees, then straight.
        let middle = &routes[1].points;
        for pair in middle.windows(2) {
            let (dx, dy) = ((pair[1].x - pair[0].x).abs(), (pair[1].y - pair[0].y).abs());
            assert!(dx < 1e-9 || dy < 1e-9 || (dx - dy).abs() < 1e-9, "a part at an angle that is not a multiple of 45: {dx} {dy}");
        }
        assert_eq!(middle.first(), Some(&origin));
        assert_eq!(middle.last(), Some(&mark.centre));
        // The pipes are 4 apart along the whole line: at the start (horizontal part) in y.
        assert!((routes[0].points[0].y - routes[1].points[0].y).abs() - PIPE < 1e-9);
        assert!((routes[2].points[0].y - routes[1].points[0].y).abs() - PIPE < 1e-9);
        // And on the 45-degree part, the perpendicular distance is the pipe.
        let (a, b) = (routes[0].points[1], routes[0].points[2]);
        let (dir_x, dir_y) = ((b.x - a.x) / distance(a, b), (b.y - a.y) / distance(a, b));
        let m = routes[1].points[1];
        let across = ((m.x - a.x) * -dir_y + (m.y - a.y) * dir_x).abs();
        assert!((across - PIPE).abs() < 1e-6, "pipe on the slant {across}");
    }

    #[test]
    fn marks_of_one_side_start_side_by_side() {
        let origin = pt(0.0, 0.0);
        let a = PeerMark { peers: vec![(peer(1, None, false), place(0.0, 0.0, "A"))], centre: pt(300.0, -50.0) };
        let b = PeerMark { peers: vec![(peer(2, None, false), place(0.0, 0.0, "A")), (peer(3, None, false), place(0.0, 0.0, "A"))], centre: pt(300.0, 60.0) };
        let near = PeerMark { peers: vec![(peer(9, None, false), place(0.0, 0.0, "A"))], centre: pt(5.0, 5.0) };
        let routes = routes(origin, &[a, b, near]);
        assert_eq!(routes.len(), 3, "a mark at this node has no line");
        let starts: Vec<f64> = routes.iter().map(|r| r.points[0].y).collect();
        // Total width: 3 pipes and one gap of 3: from -7.5 to 7.5.
        assert!((starts[0] - -5.5).abs() < 1e-9, "{starts:?}");
        assert!((starts[1] - 1.5).abs() < 1e-9, "{starts:?}");
        assert!((starts[2] - 5.5).abs() < 1e-9, "{starts:?}");
    }

    #[test]
    fn beside_keeps_a_constant_distance() {
        let line = [pt(0.0, 0.0), pt(100.0, 0.0), pt(150.0, 50.0)];
        let shifted = beside(&line, 4.0);
        assert!((shifted[0].y - 4.0).abs() < 1e-9);
        assert!((shifted[2].x - (150.0 - 4.0 / 2f64.sqrt())).abs() < 1e-9);
        assert_eq!(beside(&line, 0.0), line.to_vec());
    }

    #[test]
    fn corners_are_round_and_ends_stay() {
        let line = [pt(0.0, 0.0), pt(100.0, 0.0), pt(150.0, 50.0), pt(250.0, 50.0)];
        let round = rounded(&line, CORNER);
        assert_eq!(round.first(), line.first());
        assert_eq!(round.last(), line.last());
        // The curve cuts the corner: no point of the curve is the sharp corner.
        assert!(!round.iter().any(|p| distance(*p, line[1]) < 1e-6));
        let nearest = round.iter().map(|p| distance(*p, line[1])).fold(f64::MAX, f64::min);
        assert!(nearest > 0.5 && nearest < CORNER, "{nearest}");
        let track = Track::new(round);
        let (mid, _) = track.at(0.5).unwrap();
        assert!(mid.x > 100.0 && mid.x < 150.0);
        assert!(track.length() < 100.0 + 50.0 * 2f64.sqrt() + 100.0);
    }

    #[test]
    fn packets_are_seeded_by_the_line_and_slower_for_a_longer_ping() {
        assert_eq!(packets(7, 2.0), packets(7, 2.0));
        assert_ne!(packets(7, 2.0), packets(8, 2.0));
        let quick = packets(7, round_trip(Some(40.0)));
        let slow = packets(7, round_trip(Some(400.0)));
        for (a, b) in quick.iter().zip(&slow) {
            assert!(b.trip > a.trip);
        }
        let all = packets(42, 3.0);
        assert!(all.iter().filter(|p| p.outward).count() >= 3);
        assert!(all.iter().filter(|p| !p.outward).count() >= 3);
        for packet in &all {
            assert!(packet.trip >= 1.5 * 0.82 - 1e-9 && packet.trip <= 1.5 * 1.25 + 1e-9);
            assert!(packet.cycle > packet.trip + 0.29);
            assert!(packet.offset >= 0.0 && packet.offset < packet.cycle);
            // On the line during the trip, at rest after it.
            let begin = packet.cycle - packet.offset;
            assert!(packet.share(begin + 0.001).is_some_and(|s| s < 0.01));
            assert!(packet.share(begin + packet.trip + 0.01).is_none());
        }
        assert!((round_trip(None) - 2.4).abs() < 1e-9);
        assert_eq!(round_trip(Some(1.0)), 0.5);
        assert_eq!(round_trip(Some(5000.0)), 8.0);
    }
}
