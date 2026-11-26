//! Connected peers, fastest first, and the node's ban list, in one list as in the terminal
//! (`PeersCard`, `PeerScale` and `ResponseScale` in `NodeView.swift`).
//!
//! A node has a small number of peers, and they stay for hours. Thus the list is the one
//! view of them: a sentence, a count by network operator and by country, then one row for
//! each peer with its measures on scales that all rows share. The response time is a dot
//! at the last measure and a line for the range in this session. Colour is not a
//! judgement of speed. The list grows with the page. Up and Down select, Return or a
//! double-click opens a peer.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use super::columns::Columns;
use super::feed::PeerFeed;
use super::is_syncing;
use crate::bridge::api;
use crate::format;
use crate::page::{self, load::clear, Card, Page};
use crate::theme::{draw, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;
use crate::widgets::link;

/// The scales that all rows of the peer list share.
pub struct PeerScale {
    /// The top of the response-time scale, in milliseconds: the slowest peer, rounded up.
    pub slowest: f64,
    rates: HashMap<i64, f64>,
    fastest_rate: f64,
}

impl PeerScale {
    pub fn new(peers: &[api::PeerRow], ranges: &HashMap<i64, (f64, f64)>, now: f64) -> Self {
        let times = peers.iter().filter_map(|p| p.ping_ms).chain(peers.iter().filter_map(|p| ranges.get(&p.id).map(|r| r.1)));
        let slowest = Self::round(times.fold(None, |m: Option<f64>, t| Some(m.map_or(t, |m| m.max(t)))).unwrap_or(100.0));
        let mut rates = HashMap::new();
        for peer in peers {
            let Some(since) = peer.connected_since else { continue };
            let seconds = now - since as f64;
            if seconds >= 1.0 {
                rates.insert(peer.id, peer.bytes_received as f64 / seconds);
            }
        }
        let fastest_rate = rates.values().copied().fold(0.0, f64::max).max(1.0);
        Self { slowest, rates, fastest_rate }
    }

    /// The next step above a time: 100, 200, 500, then 1, 2 and 5 seconds, to 10 seconds.
    pub fn round(milliseconds: f64) -> f64 {
        for step in [100.0, 200.0, 300.0, 400.0, 500.0, 750.0, 1000.0, 2000.0, 5000.0, 10_000.0] {
            if milliseconds <= step {
                return step;
            }
        }
        10_000.0
    }

    pub fn time(milliseconds: f64) -> String {
        if milliseconds < 1000.0 {
            format!("{} ms", milliseconds.round() as i64)
        } else {
            format!("{} s", format::decimal(milliseconds / 1000.0, 1))
        }
    }

    pub fn rate_text(bytes: f64) -> String {
        if bytes >= 10.0 { format::bytes(bytes as u64) } else { format!("{:.1} bytes", bytes) }
    }

    pub fn rate(&self, peer: &api::PeerRow) -> Option<f64> {
        self.rates.get(&peer.id).copied()
    }

    /// The data rate of a peer as a part of the highest rate, from 0 to 1.
    pub fn share(&self, peer: &api::PeerRow) -> f64 {
        (self.rates.get(&peer.id).copied().unwrap_or(0.0) / self.fastest_rate).min(1.0)
    }
}

/// How many blocks the peer is behind the node. None when it is at the tip or not known.
pub fn behind(node: &api::NodeStatus, peer: &api::PeerRow) -> Option<u64> {
    if is_syncing(node) {
        return None;
    }
    let synced = peer.synced_blocks.filter(|s| *s > 0)?;
    let gap = node.blocks as i64 - synced;
    (gap > 1).then_some(gap as u64)
}

/// The located place of a peer.
fn place(feed: &PeerFeed, peer: &api::PeerRow) -> Option<api::PeerLocation> {
    feed.location(&peer.address).filter(|l| l.place == api::PeerPlace::Located)
}

/// The peers in groups by one property, the largest group first.
pub fn groups(names: Vec<String>) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for name in names {
        *counts.entry(name).or_default() += 1;
    }
    let mut list: Vec<(String, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    list
}

fn operator_groups(node: &api::NodeStatus, feed: &PeerFeed) -> Vec<(String, usize)> {
    groups(node.peers.iter().filter_map(|p| place(feed, p).and_then(|l| l.organization)).collect())
}

fn country_groups(node: &api::NodeStatus, feed: &PeerFeed) -> Vec<(String, usize)> {
    groups(node.peers.iter().filter_map(|p| place(feed, p).and_then(|l| l.country)).collect())
}

/// The sentence above the list.
pub fn sentence(node: &api::NodeStatus, operators: &[(String, usize)]) -> String {
    let peers = &node.peers;
    let count = peers.len();
    let incoming = peers.iter().filter(|p| p.inbound).count();
    let mut parts: Vec<String> = Vec::new();
    let noun = if count == 1 { "peer" } else { "peers" };
    if count == 1 {
        parts.push(format!("1 peer, {}.", if incoming == 1 { "incoming" } else { "outgoing" }));
    } else if incoming == 0 {
        parts.push(format!("{count} {noun}, all outgoing."));
    } else if incoming == count {
        parts.push(format!("{count} {noun}, all incoming."));
    } else {
        parts.push(format!("{count} {noun}: {} outgoing and {incoming} incoming.", count - incoming));
    }
    if count > 1
        && let Some((name, largest)) = operators.first()
        && largest * 2 > count
    {
        parts.push(if *largest == count {
            format!("All {count} use one network operator, {name}.")
        } else {
            format!("{largest} of the {count} use one network operator, {name}.")
        });
    }
    if !is_syncing(node) && peers.iter().all(|p| p.synced_blocks.unwrap_or(0) > 0) {
        let late = peers.iter().filter(|p| behind(node, p).is_some()).count();
        let mut tip = if late == 0 {
            if count == 1 { "It is at the tip of the chain".to_string() } else { format!("All {count} are at the tip of the chain") }
        } else {
            format!("{late} of the {count} {} behind the node", if late == 1 { "is" } else { "are" })
        };
        let mut versions: Vec<&String> = peers.iter().filter_map(|p| p.version.as_ref()).collect();
        versions.sort();
        versions.dedup();
        if count > 1 && versions.len() == 1 {
            tip.push_str(if late == 0 { " and run the same version" } else { ". All run the same version" });
        }
        parts.push(format!("{tip}."));
    }
    parts.join(" ")
}

fn ban_text(banned: &api::BannedPeer) -> String {
    match banned.banned_until {
        Some(until) if until > 0 => format!("until {}", format::date(until as u64)),
        _ => banned.reason.clone().unwrap_or_else(|| "banned".into()),
    }
}

/// A line of text in the callout size, in the secondary colour.
fn callout(text: &str) -> gtk::Label {
    page::text(text, &["callout", "secondary"])
}

/// The mark of a ban: a circle with a line through it (`nosign`).
fn ban_mark() -> Canvas {
    let mark = Canvas::new(13, 13, |_, snapshot, w, h| {
        let builder = gtk::gsk::PathBuilder::new();
        let r = w.min(h) * 0.4;
        builder.add_circle(&draw::point(w / 2.0, h / 2.0), r);
        draw::stroke(snapshot, &builder.to_path(), tone::ALERT, 1.3);
        let d = r * std::f32::consts::FRAC_1_SQRT_2;
        draw::rule(snapshot, w / 2.0 - d, h / 2.0 - d, w / 2.0 + d, h / 2.0 + d, tone::ALERT, 1.3, &[]);
    });
    mark.set_valign(gtk::Align::Center);
    mark
}

/// One line of the list: a connected peer or an entry in the ban list.
#[derive(Clone)]
enum Item {
    Peer(api::PeerRow),
    Banned(api::BannedPeer),
}

impl Item {
    fn id(&self) -> String {
        match self {
            Item::Peer(peer) => format!("peer-{}", peer.id),
            Item::Banned(banned) => format!("ban-{}", banned.address),
        }
    }
}

/// The widgets of a peer's row that change.
struct PeerLine {
    place: gtk::Label,
    detail: gtk::Label,
    address: gtk::Label,
    addresses: gtk::Box,
    behind: gtk::Label,
    response: ResponseView,
    data_bar: Canvas,
    share: Rc<std::cell::Cell<f64>>,
    data_text: gtk::Label,
    data: gtk::Box,
    connected: gtk::Label,
}

struct ResponseView {
    root: gtk::Box,
    canvas: Canvas,
    label: gtk::Label,
    absent: gtk::Label,
    values: Rc<RefCell<(Option<f64>, Option<(f64, f64)>, f64)>>,
}

impl ResponseView {
    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 3);
        let values: Rc<RefCell<(Option<f64>, Option<(f64, f64)>, f64)>> = Rc::new(RefCell::new((None, None, 100.0)));
        let v = values.clone();
        let canvas = Canvas::new(0, 12, move |_, snapshot, w, h| {
            let (Some(ms), range, slowest) = *v.borrow() else { return };
            let middle = h / 2.0;
            let x = |ms: f64| 5.0 + (ms / slowest).min(1.0) as f32 * (w - 10.0);
            draw::rule(snapshot, 5.0, middle, w - 5.0, middle, tone::AXIS, 1.0, &[]);
            draw::rule(snapshot, 5.0, middle - 4.0, 5.0, middle + 4.0, tone::AXIS, 1.0, &[]);
            if let Some((low, high)) = range
                && high - low > 0.0
            {
                let from = x(low);
                draw::fill_rect(snapshot, from, middle - 2.0, (x(high) - from).max(2.0), 4.0, tone::RECORD.opacity(0.35));
            }
            let at = x(ms);
            draw::fill(snapshot, &draw::ellipse_path(at - 4.0, middle - 4.0, 8.0, 8.0), tone::RECORD);
        });
        canvas.set_hexpand(true);
        let label = page::text("", &["callout"]);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        root.append(&canvas);
        root.append(&label);
        let absent = page::text("not measured yet", &["secondary"]);
        absent.set_valign(gtk::Align::Start);
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        outer.append(&root);
        outer.append(&absent);
        Self { root: outer, canvas, label, absent, values }
    }

    fn set(&self, ms: Option<f64>, range: Option<(f64, f64)>, slowest: f64) {
        *self.values.borrow_mut() = (ms, range, slowest);
        let shown = ms.is_some();
        self.canvas.set_visible(shown);
        self.label.set_visible(shown);
        self.absent.set_visible(!shown);
        if let Some(ms) = ms {
            let mut text = PeerScale::time(ms);
            if let Some((low, high)) = range
                && high - low >= 5.0
            {
                text.push_str(&format!(" · {} to {}", PeerScale::time(low), PeerScale::time(high)));
            }
            self.label.set_text(&text);
            self.root.update_property(&[gtk::accessible::Property::Label(&format!("Response time {text}"))]);
        }
        self.canvas.queue_draw();
    }
}

pub struct PeersCard {
    pub card: Card,
    page: std::rc::Weak<Page>,
    feed: Rc<PeerFeed>,
    open: Rc<dyn Fn(i64)>,
    empty_note: gtk::Box,
    sentence: gtk::Label,
    counts: gtk::Box,
    list: gtk::Box,
    response_heading: gtk::Label,
    rows: gtk::Box,
    shown_ids: RefCell<Vec<String>>,
    lines: RefCell<HashMap<String, PeerLine>>,
    ban_error: gtk::Label,
    totals: gtk::Box,
    node: RefCell<Option<api::NodeStatus>>,
}

impl PeersCard {
    pub fn new(page: &Rc<Page>, feed: &Rc<PeerFeed>, open: Rc<dyn Fn(i64)>) -> Rc<Self> {
        let card = page.card("Peers", Some(" "));
        let empty_note = page::note("No peers are connected. The node cannot synchronize without a peer.", true);
        card.append(&empty_note);
        let sentence = page::paragraph("", &["value"]);
        card.append(&sentence);
        let counts = gtk::Box::new(gtk::Orientation::Vertical, 5);
        card.append(&counts);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let response_heading = display::name_text("Response time, 0 to 100 ms");
        let heading = Columns::new(&display::name_text("Peer"), &response_heading, &display::name_text("Data received"), &{
            let name = display::name_text("Connected");
            name.set_xalign(1.0);
            name
        });
        heading.set_margin_start(8);
        heading.set_margin_end(8);
        heading.set_margin_bottom(6);
        list.append(&heading);
        list.append(&page::divider());
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.append(&rows);
        card.append(&list);
        let ban_error = page::text("", &["callout", "secondary"]);
        ban_error.set_wrap(true);
        card.append(&ban_error);
        let totals = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        totals.add_css_class("callout");
        totals.set_tooltip_text(Some("The node's traffic since it started. The app reads it once each time this page opens."));
        card.append(&totals);
        let view = Rc::new(Self {
            card,
            page: Rc::downgrade(page),
            feed: feed.clone(),
            open,
            empty_note,
            sentence,
            counts,
            list,
            response_heading,
            rows,
            shown_ids: RefCell::default(),
            lines: RefCell::default(),
            ban_error,
            totals,
            node: RefCell::new(None),
        });
        // The ages and the rates of the rows move on every five seconds.
        let weak = Rc::downgrade(&view);
        page::every(&view.card.root, Duration::from_secs(5), move || {
            if let Some(view) = weak.upgrade() {
                let node = view.node.borrow().clone();
                if let Some(node) = node {
                    view.update_rows(&node);
                }
            }
        });
        view
    }

    pub fn update(&self, node: &api::NodeStatus) {
        *self.node.borrow_mut() = Some(node.clone());
        let feed = &self.feed;
        let bans = if feed.banned_error.borrow().is_none() { format!("{} banned", feed.banned.borrow().len()) } else { "bans not available".into() };
        self.card.set_trailing(Some(&format!("{} connected · {bans}", node.peers.len())));
        let empty = node.peers.is_empty();
        self.empty_note.set_visible(empty);
        self.sentence.set_visible(!empty);
        let operators = operator_groups(node, feed);
        let countries = country_groups(node, feed);
        self.sentence.set_markup(&format!("<span font_size=\"{}\">{}</span>", 15 * 1024, glib::markup_escape_text(&sentence(node, &operators))));
        clear(&self.counts);
        if !empty {
            // The names of the two lines are one width, so the squares start at one place.
            let names = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
            if !operators.is_empty() {
                self.counts.append(&count_line("Network operator", &operators, node.peers.len(), &names));
            }
            if !countries.is_empty() {
                self.counts.append(&count_line("Country", &countries, node.peers.len(), &names));
            }
        }
        self.counts.set_visible(self.counts.first_child().is_some());
        match feed.banned_error.borrow().as_ref() {
            Some(error) => {
                self.ban_error.set_text(&format!("The ban list is not available: {error}"));
                self.ban_error.set_visible(true);
            }
            None => self.ban_error.set_visible(false),
        }
        self.update_totals();
        self.update_rows(node);
    }

    fn items(&self, node: &api::NodeStatus) -> Vec<Item> {
        let mut peers = node.peers.clone();
        peers.sort_by(|a, b| a.ping_ms.unwrap_or(f64::INFINITY).total_cmp(&b.ping_ms.unwrap_or(f64::INFINITY)).then(a.id.cmp(&b.id)));
        peers.into_iter().map(Item::Peer).chain(self.feed.banned.borrow().iter().cloned().map(Item::Banned)).collect()
    }

    fn update_rows(&self, node: &api::NodeStatus) {
        let Some(page) = self.page.upgrade() else { return };
        let items = self.items(node);
        self.list.set_visible(!items.is_empty());
        let now = format::now();
        let ranges = self.feed.response_range.borrow().clone();
        let scale = PeerScale::new(&node.peers, &ranges, now);
        theme_name(&self.response_heading, &format!("Response time, 0 to {}", PeerScale::time(scale.slowest)));
        let ids: Vec<String> = items.iter().map(Item::id).collect();
        // The rows are built again only when the list changes. Otherwise the values change.
        if *self.shown_ids.borrow() != ids {
            clear(&self.rows);
            let mut lines = HashMap::new();
            for item in &items {
                let (row, line) = self.row(&page, item);
                if let Some(line) = line {
                    lines.insert(item.id(), line);
                }
                self.rows.append(&row);
                self.rows.append(&page::divider());
            }
            *self.lines.borrow_mut() = lines;
            *self.shown_ids.borrow_mut() = ids;
        }
        let lines = self.lines.borrow();
        for item in &items {
            if let (Item::Peer(peer), Some(line)) = (item, lines.get(&item.id())) {
                self.fill(line, node, peer, &scale, ranges.get(&peer.id).copied(), now);
            }
        }
    }

    fn row(&self, page: &Rc<Page>, item: &Item) -> (gtk::Box, Option<PeerLine>) {
        let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let id = item.id();
        match item {
            Item::Peer(peer) => {
                let peer_column = gtk::Box::new(gtk::Orientation::Vertical, 2);
                let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let (open, peer_id) = (self.open.clone(), peer.id);
                let name = link::link(&format!("Peer {}", peer.id), false, move || open(peer_id));
                top.append(&name);
                let place = page::text("", &[]);
                place.set_ellipsize(gtk::pango::EllipsizeMode::End);
                top.append(&place);
                peer_column.append(&top);
                let addresses = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                let detail = callout("");
                let address = callout(&peer.address);
                address.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                addresses.append(&detail);
                addresses.append(&address);
                peer_column.append(&addresses);
                let behind = page::text("", &["callout", "mined"]);
                peer_column.append(&behind);
                let response = ResponseView::new();
                let data = gtk::Box::new(gtk::Orientation::Vertical, 3);
                let share = Rc::new(std::cell::Cell::new(0.0));
                let s = share.clone();
                let data_bar = Canvas::new(0, 8, move |_, snapshot, w, h| {
                    draw::fill_rect(snapshot, 0.0, 0.0, (w * s.get() as f32).max(1.5), h, tone::RECORD.opacity(0.85));
                });
                data_bar.set_hexpand(true);
                data_bar.update_property(&[gtk::accessible::Property::Label("")]);
                let data_text = callout("");
                data_text.set_ellipsize(gtk::pango::EllipsizeMode::End);
                data.append(&data_bar);
                data.append(&data_text);
                let connected = page::text("", &["tabular"]);
                connected.set_xalign(1.0);
                let columns = Columns::new(&peer_column, &response.root, &data, &connected);
                columns.set_margin_top(6);
                columns.set_margin_bottom(6);
                columns.set_margin_start(8);
                columns.set_margin_end(8);
                outer.append(&columns);
                let (open, peer_id) = (self.open.clone(), peer.id);
                page.selectable_row(&id, &outer, Some(id.clone()), move || open(peer_id));
                let line = PeerLine { place, detail, address, addresses, behind, response, data_bar, share, data_text, data, connected };
                (outer, Some(line))
            }
            Item::Banned(banned) => {
                let peer_column = gtk::Box::new(gtk::Orientation::Vertical, 2);
                let label = gtk::Box::new(gtk::Orientation::Horizontal, 5);
                label.append(&ban_mark());
                label.append(&page::text("Banned", &["alert"]));
                peer_column.append(&label);
                let line = callout(&format!("{} · {}", banned.address, ban_text(banned)));
                line.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                peer_column.append(&line);
                let columns = Columns::new(&peer_column, &gtk::Box::new(gtk::Orientation::Vertical, 0), &gtk::Box::new(gtk::Orientation::Vertical, 0), &gtk::Label::new(None));
                columns.set_margin_top(6);
                columns.set_margin_bottom(6);
                columns.set_margin_start(8);
                columns.set_margin_end(8);
                outer.append(&columns);
                // A ban has no page: Return does nothing, as in the Swift app.
                page.selectable_row(&id, &outer, Some(id.clone()), || {});
                (outer, None)
            }
        }
    }

    fn fill(&self, line: &PeerLine, node: &api::NodeStatus, peer: &api::PeerRow, scale: &PeerScale, range: Option<(f64, f64)>, now: f64) {
        line.place.set_text(&self.place_name(peer));
        let mut parts: Vec<String> = Vec::new();
        if let Some(organization) = place(&self.feed, peer).and_then(|l| l.organization) {
            parts.push(organization);
        }
        parts.push(if peer.inbound { "incoming" } else { "outgoing" }.into());
        line.detail.set_text(&format!("{} · ", parts.join(" · ")));
        line.address.set_text(&peer.address);
        let start = peer.starting_height.map(|h| format::integer(h.max(0) as u64)).unwrap_or_else(|| "unknown".into());
        line.addresses.set_tooltip_text(Some(&format!("{} · height at connection {start}", peer.address)));
        match behind(node, peer) {
            Some(gap) => {
                line.behind.set_text(&format!("{} blocks behind the node", format::integer(gap)));
                line.behind.set_visible(true);
            }
            None => line.behind.set_visible(false),
        }
        line.response.set(peer.ping_ms, range, scale.slowest);
        line.share.set(scale.share(peer));
        line.data_bar.queue_draw();
        line.data_text.set_text(&format!("{} · sent {}", format::bytes(peer.bytes_received), format::bytes(peer.bytes_sent)));
        let help = scale.rate(peer).map(|r| format!("{} for each second of the connection.", PeerScale::rate_text(r))).unwrap_or_default();
        line.data.set_tooltip_text((!help.is_empty()).then_some(help.as_str()));
        line.connected.set_text(&peer.connected_since.map(|since| format::age(since, now).replace(" ago", "")).unwrap_or_default());
    }

    fn place_name(&self, peer: &api::PeerRow) -> String {
        let locations = self.feed.locations.borrow();
        let Some(locations) = locations.as_ref() else { return String::new() };
        let Some(location) = locations.locations.iter().find(|l| l.address == peer.address) else { return String::new() };
        match location.place {
            api::PeerPlace::Located => [location.city.clone(), location.country_code.clone()].into_iter().flatten().collect::<Vec<_>>().join(", "),
            api::PeerPlace::Private => "private or local address".into(),
            api::PeerPlace::Unresolved => {
                if matches!(locations.state, api::LocationDataState::Current | api::LocationDataState::UpdateAvailable) {
                    "no location found".into()
                } else {
                    "no location data".into()
                }
            }
        }
    }

    /// The terminal's traffic tile: totals read once when the page opened.
    pub fn update_totals(&self) {
        clear(&self.totals);
        self.totals.append(&display::name_text("Network totals"));
        if let Some(totals) = self.feed.totals.borrow().as_ref() {
            self.totals.append(&page::text(&format!("Received {}", format::bytes(totals.received)), &["tabular"]));
            self.totals.append(&page::text(&format!("Sent {}", format::bytes(totals.sent)), &["tabular"]));
            self.totals.append(&page::text(&format!("Read at {}", format::date(totals.time_millis / 1000)), &["secondary"]));
        } else if self.feed.totals_error.borrow().is_some() {
            self.totals.append(&page::text("Not available.", &["secondary"]));
        } else {
            self.totals.append(&page::text("Reading…", &["secondary"]));
        }
    }
}

fn theme_name(label: &gtk::Label, text: &str) {
    crate::theme::set_label(label, crate::theme::font::SMALL, text);
}

/// "Network operator", then a square for each peer of a group, its name and its count.
fn count_line(title: &str, groups: &[(String, usize)], total: usize, names: &gtk::SizeGroup) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let name = display::name_text(title);
    name.set_size_request(140, -1);
    names.add_widget(&name);
    name.set_valign(gtk::Align::Center);
    row.append(&name);
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_column_spacing(14);
    flow.set_row_spacing(4);
    flow.set_max_children_per_line(64);
    flow.set_hexpand(true);
    flow.set_valign(gtk::Align::Center);
    for (group, count) in groups {
        let most = count * 2 > total;
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let squares = *count;
        let cells = Canvas::new((squares as i32) * 14 - 3, 11, move |_, snapshot, _, _| {
            for index in 0..squares {
                draw::fill_rect(snapshot, index as f32 * 14.0, 0.0, 11.0, 11.0, if most { tone::RECORD } else { tone::AXIS });
            }
        });
        cells.set_valign(gtk::Align::Center);
        item.append(&cells);
        item.append(&page::text(&format!("{group} {count}"), &["callout"]));
        item.update_property(&[gtk::accessible::Property::Label(&format!("{group}, {count}"))]);
        let child = gtk::FlowBoxChild::new();
        child.set_child(Some(&item));
        child.set_focusable(false);
        child.set_can_target(false);
        flow.append(&child);
    }
    row.append(&flow);
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: i64, ping: Option<f64>, inbound: bool, synced: Option<i64>, version: &str) -> api::PeerRow {
        api::PeerRow {
            id,
            address: format!("192.0.2.{id}"),
            inbound,
            ping_ms: ping,
            synced_blocks: synced,
            starting_height: None,
            bytes_sent: 0,
            bytes_received: 1000 * id as u64,
            version: Some(version.into()),
            connected_since: Some(1000),
            json: "{}".into(),
        }
    }

    fn node(peers: Vec<api::PeerRow>) -> api::NodeStatus {
        api::NodeStatus {
            blocks: 100,
            longest_chain: Some(100),
            tip_time: None,
            synced_headers: None,
            mempool_count: None,
            peers,
            peers_error: None,
            source: api::DataSource::LocalDaemon,
            provenance: api::Provenance {
                source_id: String::new(),
                source: String::new(),
                kind: api::SourceKind::LocalNode,
                retained: false,
                observed_at: 0,
                observed_height: None,
                failure: None,
                caution: None,
            },
        }
    }

    #[test]
    fn scale_rounds_up_and_shares_the_rate() {
        assert_eq!(PeerScale::round(80.0), 100.0);
        assert_eq!(PeerScale::round(301.0), 400.0);
        assert_eq!(PeerScale::round(620.0), 750.0);
        assert_eq!(PeerScale::round(25_000.0), 10_000.0);
        assert_eq!(PeerScale::time(42.4), "42 ms");
        assert_eq!(PeerScale::time(1500.0), "1.5 s");
        assert_eq!(PeerScale::time(2000.0), "2 s");
        assert_eq!(PeerScale::rate_text(4.25), "4.2 bytes");
        let peers = vec![peer(1, Some(40.0), false, None, "a"), peer(2, Some(120.0), false, None, "a")];
        let ranges: HashMap<i64, (f64, f64)> = [(1, (20.0, 260.0))].into();
        let scale = PeerScale::new(&peers, &ranges, 1100.0);
        assert_eq!(scale.slowest, 300.0, "the range of a peer counts too");
        assert_eq!(scale.share(&peers[1]), 1.0);
        assert_eq!(scale.share(&peers[0]), 0.5);
        assert_eq!(scale.rate(&peers[0]), Some(10.0));
        assert_eq!(PeerScale::new(&[], &HashMap::new(), 0.0).slowest, 100.0);
    }

    #[test]
    fn sentence_says_direction_operator_and_tip() {
        let one = node(vec![peer(1, None, true, Some(100), "a")]);
        assert_eq!(sentence(&one, &[]), "1 peer, incoming. It is at the tip of the chain.");
        let three = node(vec![peer(1, None, false, Some(100), "a"), peer(2, None, true, Some(90), "a"), peer(3, None, false, Some(99), "a")]);
        let operators = vec![("Hetzner".to_string(), 2usize), ("OVH".to_string(), 1)];
        assert_eq!(
            sentence(&three, &operators),
            "3 peers: 2 outgoing and 1 incoming. 2 of the 3 use one network operator, Hetzner. 1 of the 3 is behind the node. All run the same version."
        );
        let unknown = node(vec![peer(1, None, false, None, "a"), peer(2, None, false, Some(100), "b")]);
        assert_eq!(sentence(&unknown, &[]), "2 peers, all outgoing.");
        assert_eq!(groups(vec!["b".into(), "a".into(), "b".into(), "c".into()]), vec![("b".into(), 2), ("a".into(), 1), ("c".into(), 1)]);
    }
}
