//! Connected peers on a map of the world, placed by Carbide's location database
//! (`PeerMapCard` in `PeerMap.swift` and `PeerOutlineMap` in `PeerOutlineMap.swift`).
//!
//! The app draws the map itself, from the coast points of the terminal's map, and makes no
//! request to a map service. The map opens on the area that holds all placed peers. Private
//! and unresolved peers have no place on a map. The terminal draws them in empty sea areas.
//! The app lists them below the map, so that each mark is a real position.
//!
//! The map is a canvas that redraws when the data or the pointer changes. The coasts and
//! the grid are drawn once for a size and kept as a render node. The packets on the lines
//! are a small canvas of their own over the map ([`super::pulses`]).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::commands::window_of;
use super::feed::{self, PeerFeed};
use super::geometry::{self, MapFrame, PeerMark, PeerRoute, Pin, Pt};
use super::peers::PeerScale;
use super::pulses::Pulses;
use crate::bridge::{self, api};
use crate::format;
use crate::page::{self, load::clear, Card, Page};
use crate::shell::dialogs;
use crate::theme::draw::{self, Anchor};
use crate::theme::{font, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, Readout};
use crate::widgets::{lamp, link, loader};

const HEIGHT: i32 = 360;

thread_local! {
    /// The coast points of the world, read from the bridge once.
    static WORLD: RefCell<Option<Rc<Vec<api::WorldPoint>>>> = const { RefCell::new(None) };
    static LOADING: Cell<bool> = const { Cell::new(false) };
}

fn world() -> Option<Rc<Vec<api::WorldPoint>>> {
    WORLD.with(|w| w.borrow().clone())
}

/// Reads the coast points off the main thread, then calls `done`.
fn load_world(done: impl Fn() + 'static) {
    if world().is_some() || LOADING.with(|l| l.replace(true)) {
        return;
    }
    glib::spawn_future_local(async move {
        let points = bridge::run_blocking(api::world_outline).await;
        WORLD.with(|w| *w.borrow_mut() = Some(Rc::new(points)));
        done();
    });
}

/// What the map shows, and the geometry for the size it was last drawn at.
#[derive(Default)]
struct MapState {
    pins: Vec<Pin>,
    home: Option<api::PeerLocation>,
    hovered: Option<i64>,
    marks: Vec<PeerMark>,
    /// The lines for the packets: the pulses canvas reads them.
    routes: Rc<RefCell<Vec<PeerRoute>>>,
    /// The coasts and the grid, for a size and a frame.
    layer: Option<((i32, i32, [i64; 4], bool), gtk::gsk::RenderNode)>,
}

/// The drawn map: readouts, then the map with its lines and packets.
pub struct OutlineMap {
    pub root: gtk::Overlay,
    readouts: gtk::Box,
    canvas: Canvas,
    state: Rc<RefCell<MapState>>,
}

impl OutlineMap {
    pub fn new(open: Rc<dyn Fn(i64)>) -> Rc<Self> {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("panel");
        let readouts = gtk::Box::new(gtk::Orientation::Horizontal, 26);
        readouts.set_margin_start(18);
        readouts.set_margin_end(18);
        readouts.set_margin_top(12);
        readouts.set_margin_bottom(12);
        readouts.set_overflow(gtk::Overflow::Hidden);
        column.append(&readouts);
        column.append(&display::rule());
        let state: Rc<RefCell<MapState>> = Rc::default();
        let s = state.clone();
        let canvas = Canvas::new(0, HEIGHT, move |canvas, snapshot, w, h| draw_map(canvas, snapshot, w, h, &mut s.borrow_mut()));
        canvas.set_hexpand(true);
        let pulses = Pulses::new(state.borrow().routes.clone());
        let area = gtk::Overlay::new();
        area.set_child(Some(&canvas));
        area.add_overlay(&pulses.canvas);
        area.set_overflow(gtk::Overflow::Hidden);
        area.set_size_request(-1, HEIGHT);
        column.append(&area);
        let root = gtk::Overlay::new();
        root.set_child(Some(&column));
        root.add_overlay(&display::corner_marks());
        let map = Rc::new(Self { root, readouts, canvas, state });

        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&map);
        motion.connect_motion(move |_, x, y| {
            if let Some(map) = weak.upgrade() {
                let found = geometry::mark_at(&map.state.borrow().marks, Pt { x, y }).map(|m| m.id());
                map.set_hovered(found);
            }
        });
        let weak = Rc::downgrade(&map);
        motion.connect_leave(move |_| {
            if let Some(map) = weak.upgrade() {
                map.set_hovered(None);
            }
        });
        map.canvas.add_controller(motion);
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&map);
        click.connect_released(move |_, _, x, y| {
            let Some(map) = weak.upgrade() else { return };
            let peer = {
                let state = map.state.borrow();
                geometry::mark_at(&state.marks, Pt { x, y }).filter(|m| m.peers.len() == 1).map(|m| m.peers[0].0.id)
            };
            if let Some(peer) = peer {
                crate::theme::sound::link();
                open(peer);
            }
        });
        map.canvas.add_controller(click);
        let weak = map.canvas.downgrade();
        load_world(move || {
            if let Some(canvas) = weak.upgrade() {
                canvas.queue_draw();
            }
        });
        map
    }

    fn set_hovered(&self, hovered: Option<i64>) {
        if self.state.borrow().hovered == hovered {
            return;
        }
        self.state.borrow_mut().hovered = hovered;
        self.canvas.queue_draw();
    }

    pub fn update(&self, pins: Vec<Pin>, home: Option<api::PeerLocation>) {
        clear(&self.readouts);
        let times: Vec<f64> = pins.iter().filter_map(|p| p.0.ping_ms).collect();
        let inbound = pins.iter().filter(|p| p.0.inbound).count();
        let add = |readout: Readout| {
            readout.root.set_valign(gtk::Align::Baseline);
            self.readouts.append(&readout.root);
        };
        add(Readout::new("Placed", &pins.len().to_string(), "value", Some("00")));
        add(Readout::new("Outbound", &(pins.len() - inbound).to_string(), "value", Some("00")));
        add(Readout::new("Inbound", &inbound.to_string(), "value", Some("00")));
        if !times.is_empty() {
            let fastest = times.iter().copied().fold(f64::MAX, f64::min);
            let slowest = times.iter().copied().fold(f64::MIN, f64::max);
            add(Readout::new("Fastest", &PeerScale::time(fastest), "record", None));
            add(Readout::new("Slowest", &PeerScale::time(slowest), "value", None));
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        self.readouts.append(&spacer);
        if let Some(home) = &home {
            let place = [home.city.clone(), home.country_code.clone()].into_iter().flatten().collect::<Vec<_>>().join(", ");
            add(Readout::new("This node", &place, "present", None));
        }
        {
            let mut state = self.state.borrow_mut();
            state.pins = pins;
            state.home = home;
        }
        self.canvas.queue_draw();
    }
}

fn caption(marks: &[PeerMark], hovered: Option<i64>) -> Option<String> {
    let mark = marks.iter().find(|m| Some(m.id()) == hovered)?;
    Some(
        mark.peers
            .iter()
            .map(|(peer, location)| {
                let mut parts = vec![format!("Peer {}", peer.id), if peer.inbound { "inbound" } else { "outbound" }.to_string(), feed::place_name(location)];
                if let Some(organization) = &location.organization {
                    parts.push(organization.clone());
                }
                if let Some(ms) = peer.ping_ms {
                    parts.push(PeerScale::time(ms));
                }
                parts.join(" · ")
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

fn spoken(marks: &[PeerMark]) -> String {
    format!(
        "A map with {} {}: {}. The peer list has the same peers as text.",
        marks.len(),
        if marks.len() == 1 { "place" } else { "places" },
        marks.iter().map(|m| m.label()).collect::<Vec<_>>().join("; ")
    )
}

fn stroke_path(points: &[Pt]) -> Option<gtk::gsk::Path> {
    let first = points.first()?;
    let builder = gtk::gsk::PathBuilder::new();
    builder.move_to(first.x as f32, first.y as f32);
    for point in &points[1..] {
        builder.line_to(point.x as f32, point.y as f32);
    }
    Some(builder.to_path())
}

fn round_stroke(snapshot: &gtk::Snapshot, path: &gtk::gsk::Path, color: crate::theme::Color, width: f32) {
    let stroke = gtk::gsk::Stroke::new(width);
    stroke.set_line_join(gtk::gsk::LineJoin::Round);
    snapshot.append_stroke(path, &stroke, &color.rgba());
}

fn draw_map(widget: &Canvas, snapshot: &gtk::Snapshot, width: f32, height: f32, state: &mut MapState) {
    let (w, h) = (width as f64, height as f64);
    let mut positions: Vec<(f64, f64)> = state.pins.iter().map(|p| (p.1.longitude.unwrap_or(0.0), p.1.latitude.unwrap_or(0.0))).collect();
    if let Some(home) = &state.home {
        positions.push((home.longitude.unwrap_or(0.0), home.latitude.unwrap_or(0.0)));
    }
    let frame = MapFrame::new(&positions, w / h.max(1.0));
    let marks = geometry::marks(&state.pins, &frame, w, h);
    let origin = state.home.as_ref().map(|home| frame.point(home.longitude.unwrap_or(0.0), home.latitude.unwrap_or(0.0), w, h));
    let routes = origin.map(|o| geometry::routes(o, &marks)).unwrap_or_default();
    if *state.routes.borrow() != routes {
        *state.routes.borrow_mut() = routes.clone();
    }

    // The grid and the coasts: the same for a size and a frame.
    let world = world();
    let key = (width as i32, height as i32, [frame.west, frame.east, frame.south, frame.north].map(|v| (v * 1e6) as i64), world.is_some());
    let cached = state.layer.as_ref().filter(|(k, _)| *k == key).map(|(_, node)| node.clone());
    let layer = match cached {
        Some(node) => Some(node),
        None => {
            let inner = gtk::Snapshot::new();
            draw_base(widget, &inner, w, h, &frame, world.as_deref());
            let node = inner.to_node();
            if let Some(node) = &node {
                state.layer = Some((key, node.clone()));
            }
            node
        }
    };
    if let Some(node) = layer {
        snapshot.append_node(&node);
    }

    // The lines to the peers. The line of the mark under the pointer has light.
    for route in &routes {
        let Some(line) = stroke_path(&geometry::rounded(&route.points, geometry::CORNER)) else { continue };
        if state.hovered == Some(route.mark) {
            draw::push_glow(snapshot, tone::PRESENT, 0.7);
            round_stroke(snapshot, &line, tone::PRESENT, 2.0);
            snapshot.pop();
        } else {
            round_stroke(snapshot, &line, tone::RECORD.opacity(0.55), 1.3);
        }
    }

    // The peers: a square as a station of the line. A hovered mark is the selection.
    for mark in &marks {
        let chosen = state.hovered == Some(mark.id());
        let color = if chosen { tone::PRESENT } else { tone::RECORD };
        let count = mark.peers.len();
        let side = if count > 1 { (count as f32 * 4.0 + 6.0).max(16.0) } else { 9.0 };
        let (cx, cy) = (mark.centre.x as f32, mark.centre.y as f32);
        let (bx, by) = (cx - side / 2.0, cy - side / 2.0);
        draw::fill_rect(snapshot, bx, by, side, side, tone::SCREEN);
        let outline = draw::rect_path(bx, by, side, side);
        if chosen {
            draw::push_glow(snapshot, tone::PRESENT, 0.9);
            draw::stroke(snapshot, &outline, tone::PRESENT, 1.5);
            snapshot.pop();
        } else {
            draw::stroke(snapshot, &outline, color, 1.5);
        }
        if count > 1 {
            draw::name(snapshot, widget, &count.to_string(), cx, cy, Anchor::Center, if chosen { tone::PRESENT } else { tone::VALUE });
        } else {
            draw::fill_rect(snapshot, bx + 2.5, by + 2.5, side - 5.0, side - 5.0, color);
        }
        let right = cx < width - 150.0;
        let x = if right { cx + side / 2.0 + 7.0 } else { cx - side / 2.0 - 7.0 };
        draw::name(snapshot, widget, &mark.label(), x, cy - 9.0, if right { Anchor::Leading } else { Anchor::Trailing }, if chosen { tone::PRESENT } else { tone::VALUE });
    }

    // This node: a lit square in corner marks.
    if let Some(origin) = origin {
        let (ox, oy) = (origin.x as f32, origin.y as f32);
        let (arm, reach) = (5.0f32, 12.0f32);
        let corners = gtk::gsk::PathBuilder::new();
        for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let (x, y) = (ox + sx * reach, oy + sy * reach);
            corners.move_to(x - sx * arm, y);
            corners.line_to(x, y);
            corners.line_to(x, y - sy * arm);
        }
        draw::fill_rect(snapshot, ox - reach, oy - reach, reach * 2.0, reach * 2.0, tone::SCREEN.opacity(0.7));
        draw::stroke(snapshot, &corners.to_path(), tone::PRESENT, 1.2);
        draw::lit(snapshot, &draw::rect_path(ox - 4.0, oy - 4.0, 8.0, 8.0), tone::PRESENT, 0.9);
        draw::plate(snapshot, widget, "This node", ox - 33.0, oy + reach + 12.0, 66.0, Some(tone::PRESENT));
    }

    // The caption of the hovered mark, at the lower left.
    if let Some(text) = caption(&marks, state.hovered) {
        let layout = widget.create_pango_layout(None);
        layout.set_markup(&crate::theme::markup(font::CAPTION, &text));
        let (_, logical) = layout.pixel_extents();
        let (tw, th) = (logical.width() as f32, logical.height() as f32);
        let (bw, bh) = (tw + 16.0, th + 8.0);
        let (bx, by) = (12.0, height - 12.0 - bh);
        draw::fill_rect(snapshot, bx, by, bw, bh, tone::SCREEN.opacity(0.85));
        draw::stroke(snapshot, &draw::rect_path(bx + 0.5, by + 0.5, bw - 1.0, bh - 1.0), tone::RULE, 1.0);
        draw::draw_layout(snapshot, &layout, bx + 8.0, by + 4.0, Anchor::TopLeading, tone::VALUE);
    }

    let label = spoken(&marks);
    state.marks = marks;
    let weak = widget.downgrade();
    // The spoken form changes with the marks; it is set after the frame.
    glib::idle_add_local_once(move || {
        if let Some(widget) = weak.upgrade() {
            widget.update_property(&[gtk::accessible::Property::Label(&label)]);
        }
    });
}

/// A light grid each 30 degrees, with its degrees at the edges, and the coasts.
fn draw_base(widget: &Canvas, snapshot: &gtk::Snapshot, w: f64, h: f64, frame: &MapFrame, world: Option<&Vec<api::WorldPoint>>) {
    let (width, height) = (w as f32, h as f32);
    let mut longitude = -180.0;
    while longitude <= 180.0 {
        if longitude > frame.west && longitude < frame.east {
            let x = frame.point(longitude, 0.0, w, h).x as f32;
            draw::rule(snapshot, x, 0.0, x, height, tone::GRID, 1.0, &[]);
            let word = if longitude == 0.0 { "0".to_string() } else { format!("{}{}", longitude.abs() as i64, if longitude < 0.0 { "W" } else { "E" }) };
            draw::name(snapshot, widget, &word, x + 4.0, height - 9.0, Anchor::Leading, tone::LABEL.opacity(0.7));
        }
        longitude += 30.0;
    }
    let mut latitude = -60.0;
    while latitude <= 60.0 {
        if latitude > frame.south && latitude < frame.north {
            let y = frame.point(0.0, latitude, w, h).y as f32;
            draw::rule(snapshot, 0.0, y, width, y, tone::GRID, 1.0, &[]);
            let word = if latitude == 0.0 { "0".to_string() } else { format!("{}{}", latitude.abs() as i64, if latitude < 0.0 { "S" } else { "N" }) };
            draw::name(snapshot, widget, &word, 6.0, y - 8.0, Anchor::Leading, tone::LABEL.opacity(0.7));
        }
        latitude += 30.0;
    }
    // The coasts, as one path of small squares.
    if let Some(points) = world {
        let builder = gtk::gsk::PathBuilder::new();
        for point in points.iter().filter(|p| frame.contains(p.longitude, p.latitude)) {
            let at = frame.point(point.longitude, point.latitude, w, h);
            builder.add_rect(&draw::rect(at.x as f32 - 0.9, at.y as f32 - 0.9, 1.8, 1.8));
        }
        draw::fill(snapshot, &builder.to_path(), tone::AXIS);
    }
}

/// The card around the map: the state of the location data, the peers with no place, and
/// the attribution.
pub struct MapCard {
    pub card: Card,
    model: std::rc::Weak<crate::model::AppModel>,
    feed: Rc<PeerFeed>,
    open: Rc<dyn Fn(i64)>,
    map: Rc<OutlineMap>,
    map_holder: gtk::Box,
    body: gtk::Box,
    peers: RefCell<Vec<api::PeerRow>>,
}

impl MapCard {
    pub fn new(page: &Rc<Page>, feed: &Rc<PeerFeed>, open: Rc<dyn Fn(i64)>) -> Rc<Self> {
        let card = page.card("Connected node map", Some(" "));
        let map = OutlineMap::new(open.clone());
        let map_holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        map_holder.append(&map.root);
        card.append(&map_holder);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        card.append(&body);
        Rc::new(Self { card, model: Rc::downgrade(&page.model), feed: feed.clone(), open, map, map_holder, body, peers: RefCell::default() })
    }

    fn located(&self, locations: &api::PeerLocations, peers: &[api::PeerRow]) -> Vec<Pin> {
        peers
            .iter()
            .filter_map(|peer| {
                let location = locations.locations.iter().find(|l| l.address == peer.address)?;
                (location.place == api::PeerPlace::Located && location.latitude.is_some() && location.longitude.is_some())
                    .then(|| (peer.clone(), location.clone()))
            })
            .collect()
    }

    pub fn update(self: &Rc<Self>, peers: &[api::PeerRow]) {
        *self.peers.borrow_mut() = peers.to_vec();
        let locations = self.feed.locations.borrow().clone();
        clear(&self.body);
        match &locations {
            Some(locations) => {
                let pins = self.located(locations, peers);
                self.card.set_trailing(Some(&format!("{} of {} placed", pins.len(), peers.len())));
                // The map shows when a peer has a place.
                self.map_holder.set_visible(!pins.is_empty());
                if !pins.is_empty() {
                    self.map.update(pins, self.feed.home.borrow().clone());
                }
                self.body.append(&self.state_note(locations));
                self.unplaced(locations, peers);
            }
            None => {
                self.card.set_trailing(None);
                self.map_holder.set_visible(false);
                self.body.append(&page::text("Reading the location data.", &["secondary"]));
            }
        }
        if let Some(message) = self.feed.install_message.borrow().as_ref() {
            let line = page::paragraph(message, &["callout", "secondary"]);
            self.body.append(&line);
        }
        // When the data is current, its date and the attribution are one line.
        if locations.as_ref().map(|l| l.state) != Some(api::LocationDataState::Current) {
            self.body.append(&attribution());
        }
    }

    fn state_note(self: &Rc<Self>, locations: &api::PeerLocations) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let spacer = || {
            let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            s.set_hexpand(true);
            s
        };
        match locations.state {
            api::LocationDataState::NotInstalled => {
                let text = page::paragraph("The location data is not installed. The app cannot place peers on the map.", &[]);
                row.append(&text);
                row.append(&spacer());
                row.append(&self.install_button("Install Location Data…"));
            }
            api::LocationDataState::UpdateAvailable => {
                let text = page::paragraph(
                    &format!("The location data is from {}. A newer release is available.", locations.release.as_deref().unwrap_or("an earlier month")),
                    &["secondary"],
                );
                row.append(&text);
                row.append(&spacer());
                row.append(&self.install_button("Update Location Data…"));
            }
            api::LocationDataState::Unavailable => {
                let note = page::note(&format!("The location data is not available: {}.", locations.problem.as_deref().unwrap_or("unknown error")), true);
                note.set_hexpand(true);
                row.append(&note);
                row.append(&self.install_button("Install Location Data Again…"));
            }
            api::LocationDataState::Current => {
                row.set_spacing(10);
                let text = page::text(&format!("Location data from {}.", locations.release.as_deref().unwrap_or("this month")), &["caption-text", "label"]);
                text.set_valign(gtk::Align::Baseline);
                row.append(&text);
                let link = attribution();
                link.set_valign(gtk::Align::Baseline);
                row.append(&link);
            }
        }
        row
    }

    fn install_button(self: &Rc<Self>, title: &str) -> gtk::Box {
        let holder = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        holder.set_valign(gtk::Align::Center);
        if self.feed.installing.get() {
            holder.append(&loader::small());
            holder.append(&page::text("Downloading…", &["secondary"]));
        } else {
            let weak = Rc::downgrade(self);
            holder.append(&lamp::button(title, move || {
                if let Some(card) = weak.upgrade() {
                    card.confirm_install();
                }
            }));
        }
        holder
    }

    /// Where the data goes and what is known of its size, before the download starts.
    fn install_text(&self) -> String {
        let locations = self.feed.locations.borrow();
        let locations = locations.as_ref();
        let mut text = format!(
            "The app downloads this month's DB-IP City Lite and ASN Lite databases from download.db-ip.com and keeps them in {}. The terminal app uses the same files. ",
            locations.map(|l| l.directory.as_str()).unwrap_or("the geoip folder beside the cache")
        );
        if let Some(installed) = locations.map(|l| l.installed_bytes).filter(|b| *b > 0) {
            text.push_str(&format!("The data installed now uses {}. The new release replaces it. ", format::bytes(installed)));
        }
        text.push_str("Carbide does not request the size of the download before it starts. The databases are large, and the download can take several minutes. ");
        if locations.map(|l| l.default_folder).unwrap_or(true) {
            text.push_str("This folder is in the default cache folder on the startup disk. To keep the data on another disk, choose a cache folder in Settings first. The location data follows the cache folder.");
        } else {
            text.push_str("This folder is in the cache folder that you chose in Settings.");
        }
        text
    }

    /// The confirmation before the download. Only its Download key installs the data.
    fn confirm_install(self: &Rc<Self>) {
        let Some(window) = window_of(&self.card.root) else { return };
        let weak = Rc::downgrade(self);
        let dialog = dialogs::dialog(&window, "Download the location data?", Some(&self.install_text()));
        dialog.key("Cancel", true, || {});
        dialog.key("Download", false, move || {
            let Some(card) = weak.upgrade() else { return };
            let Some(model) = card.model.upgrade() else { return };
            let addresses: Vec<String> = card.peers.borrow().iter().map(|p| p.address.clone()).collect();
            let Some(install) = card.feed.begin_install(&model, addresses) else { return };
            // The key shows the download while it runs.
            card.update(&card.peers.borrow().clone());
            glib::spawn_future_local(async move {
                install.await;
                card.update(&card.peers.borrow().clone());
            });
        });
        dialog.show();
    }

    /// Peers that have no position: private addresses and addresses the data cannot place.
    fn unplaced(&self, locations: &api::PeerLocations, peers: &[api::PeerRow]) {
        let private: Vec<&api::PeerRow> = peers.iter().filter(|p| feed::place_of(locations, &p.address) == api::PeerPlace::Private).collect();
        let unresolved: Vec<&api::PeerRow> = peers.iter().filter(|p| feed::place_of(locations, &p.address) == api::PeerPlace::Unresolved).collect();
        if !private.is_empty() {
            self.body.append(&self.peer_line("Private or local address", &private));
        }
        if !unresolved.is_empty() {
            let label = if matches!(locations.state, api::LocationDataState::Current | api::LocationDataState::UpdateAvailable) {
                "No location found for these addresses"
            } else {
                "Not placed, because the location data is not available"
            };
            self.body.append(&self.peer_line(label, &unresolved));
        }
    }

    /// Peers without a public location, one per line, each a link to its page.
    fn peer_line(&self, label: &str, list: &[&api::PeerRow]) -> gtk::Box {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 3);
        column.append(&display::name_text(&format!("{label} ({})", list.len())));
        for peer in list {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let (open, id) = (self.open.clone(), peer.id);
            let address = link::link(&peer.address, true, move || open(id));
            if let Some(label) = link::label_of(&address) {
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            row.append(&address);
            row.append(&page::text(&format!("Peer {}", peer.id), &["secondary"]));
            column.append(&row);
        }
        column
    }
}

/// DB-IP's licence (CC BY 4.0) requires this attribution wherever its data appears.
pub fn attribution() -> gtk::Button {
    let button = link::link("IP geolocation by DB-IP (CC BY 4.0)", false, || {});
    if let Some(label) = link::label_of(&button) {
        label.add_css_class("caption-text");
    }
    button.connect_clicked(|button| {
        let window = button.root().and_downcast::<gtk::Window>();
        gtk::UriLauncher::new("https://db-ip.com").launch(window.as_ref(), gtk::gio::Cancellable::NONE, |_| {});
    });
    button.set_tooltip_text(Some("https://db-ip.com"));
    button
}
