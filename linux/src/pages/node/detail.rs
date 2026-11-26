//! One peer: the terminal's groups (connection, traffic and latency, chain progress), its
//! location, the node's log lines for it, and its raw JSON (`PeerDetailView` in
//! `PeerMap.swift`). macOS shows it as a sheet over the page; here it is a modal window over
//! the main window, with the same keys.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::commands::{self, PeerCommands};
use super::feed::{self, PeerFeed};
use crate::bridge::{self, api};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, Card, Page};
use crate::widgets::display::{self, PlateFill};
use crate::widgets::lamp;

/// The value of a field, in the form of its kind. The raw value is kept when it does not
/// parse.
pub fn field_text(field: &api::PeerField) -> String {
    match field.kind {
        api::PeerFieldKind::Time => field.value.parse::<u64>().map(format::date).unwrap_or_else(|_| field.value.clone()),
        api::PeerFieldKind::Bytes => field
            .value
            .parse::<u64>()
            .map(|b| format!("{} ({} bytes)", format::bytes(b), format::integer(b)))
            .unwrap_or_else(|_| field.value.clone()),
        api::PeerFieldKind::Seconds => match field.value.parse::<f64>() {
            Ok(seconds) if seconds < 1.0 => format!("{} ms", (seconds * 1000.0).round() as i64),
            Ok(seconds) => {
                let text = format!("{seconds:.2}");
                let (whole, fraction) = text.split_once('.').unwrap_or((&text, "00"));
                format!("{}.{} s", format::group(whole), fraction)
            }
            Err(_) => field.value.clone(),
        },
        api::PeerFieldKind::Text => field.value.clone(),
    }
}

struct Detail {
    model: std::rc::Weak<AppModel>,
    feed: Rc<PeerFeed>,
    commands: Rc<PeerCommands>,
    window: glib::WeakRef<gtk::Window>,
    peer_id: i64,
    shows_json: Cell<bool>,
    log: RefCell<Vec<String>>,
    log_error: RefCell<Option<String>>,
    page: Rc<Page>,
    actions: gtk::Box,
    gone: gtk::Label,
    /// The peer as last drawn, so that a change of a value redraws and nothing else does.
    shown: RefCell<Option<String>>,
}

/// Opens the page of a peer over `parent`.
pub fn show(model: &Rc<AppModel>, feed: &Rc<PeerFeed>, commands: &Rc<PeerCommands>, peer_id: i64, parent: Option<gtk::Window>) {
    let window = gtk::Window::new();
    window.set_transient_for(parent.as_ref());
    window.set_modal(true);
    window.set_default_size(700, 680);
    window.set_size_request(620, 520);
    window.set_title(Some(&format!("Peer {peer_id}")));
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    bar.set_margin_top(14);
    bar.set_margin_bottom(14);
    bar.set_margin_start(14);
    bar.set_margin_end(14);
    column.append(&bar);
    column.append(&page::divider());
    let page = Page::scrolling(model);
    let gone = page::text("This peer is no longer connected.", &["secondary"]);
    gone.set_halign(gtk::Align::Center);
    gone.set_valign(gtk::Align::Center);
    gone.set_vexpand(true);
    gone.set_visible(false);
    column.append(&gone);
    column.append(&page.root);
    page.content.set_spacing(18);
    for margin in [&page.content] {
        margin.set_margin_top(20);
        margin.set_margin_bottom(20);
        margin.set_margin_start(20);
        margin.set_margin_end(20);
    }
    window.set_child(Some(&column));

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let detail = Rc::new(Detail {
        model: Rc::downgrade(model),
        feed: feed.clone(),
        commands: commands.clone(),
        window: window.downgrade(),
        peer_id,
        shows_json: Cell::new(false),
        log: RefCell::default(),
        log_error: RefCell::default(),
        page,
        actions: actions.clone(),
        gone,
        shown: RefCell::default(),
    });

    let weak = Rc::downgrade(&detail);
    let segments = lamp::Segments::new(&["Overview", "JSON"], 0, move |index| {
        if let Some(detail) = weak.upgrade() {
            detail.shows_json.set(index == 1);
            detail.shown.replace(None);
            detail.update();
        }
    });
    bar.append(&segments.root);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    bar.append(&spacer);
    let weak = Rc::downgrade(&detail);
    actions.append(&lamp::button("Disconnect…", move || {
        if let Some(detail) = weak.upgrade()
            && let Some(peer) = detail.peer()
        {
            if let Some(window) = detail.window.upgrade() { detail.commands.confirm(&window, commands::disconnect_pending(&peer.address)); }
        }
    }));
    let weak = Rc::downgrade(&detail);
    actions.append(&lamp::button("Ban…", move || {
        if let Some(detail) = weak.upgrade()
            && let Some(peer) = detail.peer()
        {
            if let Some(window) = detail.window.upgrade() { detail.commands.confirm(&window, commands::ban_pending(&peer.address)); }
        }
    }));
    bar.append(&actions);
    let w = window.downgrade();
    bar.append(&lamp::button("Done", move || {
        if let Some(window) = w.upgrade() {
            window.close();
        }
    }));
    // Esc is the cancel action of the sheet.
    let keys = gtk::EventControllerKey::new();
    let w = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(window) = w.upgrade() {
                window.close();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);

    let weak = Rc::downgrade(&detail);
    model.watch(&[Topic::Node, Topic::Display], &window, move |_| {
        if let Some(detail) = weak.upgrade() {
            detail.update();
        }
    });
    // The window owns the page of the peer.
    let keep = detail.clone();
    window.connect_close_request(move |_| {
        let _ = &keep;
        glib::Propagation::Proceed
    });
    detail.load_log();
    detail.update();
    window.present();
}

impl Detail {
    fn peer(&self) -> Option<api::PeerRow> {
        self.model.upgrade()?.state().node.as_ref()?.peers.iter().find(|p| p.id == self.peer_id).cloned()
    }

    fn load_log(self: &Rc<Self>) {
        let (Some(peer), Some(model)) = (self.peer(), self.model.upgrade()) else { return };
        let Some(explorer) = model.explorer() else { return };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run_blocking(move || explorer.peer_log(peer.address)).await;
            let Some(detail) = weak.upgrade() else { return };
            match result {
                Ok(lines) => {
                    *detail.log.borrow_mut() = lines;
                    *detail.log_error.borrow_mut() = None;
                }
                Err(error) => {
                    detail.log.borrow_mut().clear();
                    *detail.log_error.borrow_mut() = Some(bridge::describe(&error));
                }
            }
            detail.shown.replace(None);
            detail.update();
        });
    }

    fn update(self: &Rc<Self>) {
        let peer = self.peer();
        self.actions.set_visible(peer.is_some());
        self.gone.set_visible(peer.is_none());
        self.page.root.set_visible(peer.is_some());
        let Some(peer) = peer else { return };
        // Values change every two seconds. The page is drawn again only when one changed,
        // so the scroll position and a selection stay.
        if self.shown.borrow().as_deref() == Some(peer.json.as_str()) {
            return;
        }
        self.shown.replace(Some(peer.json.clone()));
        let content = &self.page.content;
        clear(content);
        content.append(&self.header(&peer));
        if self.shows_json.get() {
            let text = page::text(&peer.json, &["mono", "callout"]);
            text.set_selectable(true);
            text.set_wrap(true);
            text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            content.append(&text);
            let copy = page::copy_button(&peer.json);
            copy.set_halign(gtk::Align::Start);
            content.append(&copy);
        } else {
            if let Some(card) = self.location(&peer) {
                content.append(&card.root);
            }
            for group in api::peer_field_groups(peer.json.clone()) {
                let card = self.page.card(&group.title, None);
                for field in &group.fields {
                    card.append(&page::field_row(&self.page, &field.label, &field_text(field), None, false));
                }
                content.append(&card.root);
            }
            content.append(&self.log_section().root);
        }
    }

    fn header(&self, peer: &api::PeerRow) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let title = gtk::Label::new(None);
        title.set_markup(&format!("<span font_size=\"{}\" weight=\"600\">Peer {}</span>", 17 * 1024, peer.id));
        title.add_css_class("value");
        title.set_valign(gtk::Align::Baseline);
        row.append(&title);
        let address = gtk::Label::new(None);
        address.set_markup(&format!("<span font_family=\"IBM Plex Mono\" font_size=\"{}\">{}</span>", 15 * 1024, glib::markup_escape_text(&peer.address)));
        address.add_css_class("value");
        address.set_selectable(true);
        address.set_valign(gtk::Align::Baseline);
        row.append(&address);
        let plate = display::plate(if peer.inbound { "Inbound" } else { "Outbound" }, if peer.inbound { PlateFill::Record } else { PlateFill::None });
        plate.set_valign(gtk::Align::Center);
        row.append(&plate);
        row
    }

    fn location(&self, peer: &api::PeerRow) -> Option<Card> {
        let location = self.feed.location(&peer.address)?;
        let card = self.page.card("Location", None);
        match location.place {
            api::PeerPlace::Located => {
                card.append(&page::field_row(&self.page, "Location", &feed::place_name(&location), None, false));
                card.append(&page::field_row(&self.page, "ASN", &location.asn.map(|a| format!("AS{a}")).unwrap_or_else(|| "unknown".into()), None, false));
                card.append(&page::field_row(&self.page, "Network operator", location.organization.as_deref().unwrap_or("unknown"), None, false));
                card.append(&page::paragraph("IP geolocation by DB-IP (CC BY 4.0). The location is approximate.", &["caption-text", "secondary"]));
            }
            api::PeerPlace::Private => card.append(&page::paragraph("A private or local address has no public location.", &["secondary"])),
            api::PeerPlace::Unresolved => card.append(&page::paragraph(
                "The location data has no location for this address, or the data is not installed.",
                &["secondary"],
            )),
        }
        Some(card)
    }

    fn log_section(self: &Rc<Self>) -> Card {
        let card = self.page.card("Node log", Some("lines with this peer's IP address"));
        if let Some(error) = self.log_error.borrow().as_ref() {
            card.append(&page::note(error, true));
            card.append(&page::paragraph("The app reads the debug.log file beside the node configuration file.", &["caption-text", "secondary"]));
        } else if self.log.borrow().is_empty() {
            card.append(&page::paragraph("No recent line of the node log contains this peer's IP address.", &["secondary"]));
        } else {
            let text = page::text(&self.log.borrow().join("\n"), &["mono", "caption-text"]);
            text.set_selectable(true);
            text.set_wrap(true);
            text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            text.set_valign(gtk::Align::Start);
            let scroller = gtk::ScrolledWindow::new();
            scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
            scroller.set_child(Some(&text));
            scroller.set_propagate_natural_height(true);
            scroller.set_max_content_height(220);
            card.append(&scroller);
        }
        let weak = Rc::downgrade(self);
        card.append(&lamp::button("Read Again", move || {
            if let Some(detail) = weak.upgrade() {
                detail.load_log();
            }
        }));
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(value: &str, kind: api::PeerFieldKind) -> api::PeerField {
        api::PeerField { key: "k".into(), label: "K".into(), value: value.into(), kind }
    }

    #[test]
    fn fields_in_their_forms() {
        assert_eq!(field_text(&field("2048", api::PeerFieldKind::Bytes)), "2 KB (2,048 bytes)");
        assert_eq!(field_text(&field("0.0421", api::PeerFieldKind::Seconds)), "42 ms");
        assert_eq!(field_text(&field("1.5", api::PeerFieldKind::Seconds)), "1.50 s");
        assert_eq!(field_text(&field("x", api::PeerFieldKind::Time)), "x");
        assert_eq!(field_text(&field("/Verus:1.2/", api::PeerFieldKind::Text)), "/Verus:1.2/");
    }
}
