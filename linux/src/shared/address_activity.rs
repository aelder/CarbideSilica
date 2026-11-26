//! An address's transactions, newest first, a page at a time (`AddressActivityList`,
//! `AddressTileView` and `FlowRow` in `AddressView.swift`). Each tile shows the
//! transaction's inputs and outputs. Used by the Address page and by the Identity page for
//! the identity's address.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Route};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, Page};
use crate::shared::identity_support;
use crate::theme::{self, draw, font, tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::PlateFill;
use crate::widgets::{lamp, loader};

const PAGE_SIZE: u64 = 10;

#[derive(Default)]
struct State {
    stale: bool,
    tiles: Vec<api::AddressTile>,
    missing: Vec<String>,
    total: Option<u64>,
    loading: bool,
    failure: Option<String>,
}

impl State {
    fn loaded_count(&self) -> u64 {
        (self.tiles.len() + self.missing.len()) as u64
    }
}

/// The heading of the list: the count, or the part of it that is loaded.
pub fn heading(total: u64, loaded: u64) -> String {
    if loaded == 0 {
        return format!("{} transactions", format::integer(total));
    }
    format!("Newest {} of {}", format::integer(loaded), format::integer(total))
}

/// The legend of the key that loads the next page.
pub fn older_title(total: u64, loaded: u64) -> Option<String> {
    (loaded < total).then(|| format!("Show {} older", PAGE_SIZE.min(total - loaded)))
}

pub struct ActivityList {
    page: Weak<Page>,
    model: Weak<AppModel>,
    address: String,
    /// Called when a reorganization removed the block of a loaded transaction.
    on_stale: Option<Rc<dyn Fn()>>,
    state: RefCell<State>,
    /// The parts of the list. The list holds them weakly: the watchers of the model hold
    /// the list while its root exists.
    top: glib::WeakRef<gtk::Box>,
    tiles: glib::WeakRef<gtk::Box>,
    missing: glib::WeakRef<gtk::Box>,
    footer: glib::WeakRef<gtk::Box>,
    /// The tip that the last check of the blocks saw.
    checked_tip: Cell<Option<u64>>,
    /// Ends the answers of an earlier load after a reset.
    epoch: Cell<u64>,
}

impl ActivityList {
    /// The list's root widget. The list lives as long as the widget.
    pub fn new(page: &Rc<Page>, address: &str, on_stale: Option<Rc<dyn Fn()>>) -> gtk::Box {
        install_style();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let (top, tiles, missing, footer) = (
            gtk::Box::new(gtk::Orientation::Vertical, 12),
            gtk::Box::new(gtk::Orientation::Vertical, 12),
            gtk::Box::new(gtk::Orientation::Vertical, 12),
            gtk::Box::new(gtk::Orientation::Vertical, 12),
        );
        for part in [&top, &tiles, &missing, &footer] {
            root.append(part);
        }
        let list = Rc::new(Self {
            page: Rc::downgrade(page),
            model: Rc::downgrade(&page.model),
            address: address.to_string(),
            on_stale,
            state: RefCell::default(),
            top: top.downgrade(),
            tiles: tiles.downgrade(),
            missing: missing.downgrade(),
            footer: footer.downgrade(),
            checked_tip: Cell::new(page.model.state().status.as_ref().map(|s| s.blocks)),
            epoch: Cell::new(0),
        });
        // A new connection starts the list again. A new tip checks the loaded blocks.
        let held = list.clone();
        page.model.watch(&[Topic::Connection], &root, move |_| held.reset());
        let held = list.clone();
        page.model.watch(&[Topic::Status], &root, move |model| {
            let list = &held;
            let tip = model.state().status.as_ref().map(|s| s.blocks);
            if list.checked_tip.replace(tip) != tip {
                list.recheck_stale();
            }
        });
        list.reset();
        root
    }

    fn reset(self: &Rc<Self>) {
        self.epoch.set(self.epoch.get() + 1);
        *self.state.borrow_mut() = State::default();
        if let Some(tiles) = self.tiles.upgrade() {
            page::load::clear(&tiles);
        }
        self.load_more();
    }

    /// Shows the parts around the tiles.
    fn render(self: &Rc<Self>) {
        let (Some(top), Some(missing), Some(footer)) = (self.top.upgrade(), self.missing.upgrade(), self.footer.upgrade())
        else {
            return;
        };
        let state = self.state.borrow();
        page::load::clear(&top);
        page::load::clear(&missing);
        page::load::clear(&footer);
        if state.stale {
            top.append(&page::note(
                "This list is out of date. A block of these transactions left the active chain. The list is refreshing.",
                true,
            ));
        }
        if let Some(total) = state.total {
            top.append(&page::text(&heading(total, state.loaded_count()), &["secondary"]));
            if total == 0 && !state.loading {
                top.append(&page::text("No indexed transactions.", &["secondary"]));
            }
        }
        top.set_visible(top.first_child().is_some());
        if let Some(page) = self.page.upgrade() {
            for txid in &state.missing {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.append(&page::route_link(&page, txid, Route::Transaction { txid: txid.clone() }, true));
                row.append(&page::text("details did not load", &["secondary"]));
                missing.append(&row);
            }
        }
        missing.set_visible(missing.first_child().is_some());
        if state.loading {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&loader::small());
            row.append(&page::text("Loading transactions…", &["secondary"]));
            footer.append(&row);
        } else if let Some(failure) = &state.failure {
            footer.append(&page::note(failure, true));
            let weak = Rc::downgrade(self);
            footer.append(&lamp::button("Try Again", move || {
                if let Some(list) = weak.upgrade() {
                    list.load_more();
                }
            }));
        } else if let Some(title) = state.total.and_then(|total| older_title(total, state.loaded_count())) {
            let weak = Rc::downgrade(self);
            footer.append(&lamp::button(&title, move || {
                if let Some(list) = weak.upgrade() {
                    list.load_more();
                }
            }));
        }
        footer.set_visible(footer.first_child().is_some());
    }

    fn load_more(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        {
            let mut state = self.state.borrow_mut();
            if state.loading {
                return;
            }
            state.loading = true;
            state.failure = None;
        }
        self.render();
        let explorer = match model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                let mut state = self.state.borrow_mut();
                state.loading = false;
                state.failure = Some(message);
                drop(state);
                return self.render();
            }
        };
        let (offset, address, epoch) = (self.state.borrow().loaded_count(), self.address.clone(), self.epoch.get());
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.address_activity(address, offset, PAGE_SIZE).await }).await;
            let Some(list) = weak.upgrade() else { return };
            if list.epoch.get() != epoch {
                return;
            }
            let Some(page) = list.page.upgrade() else { return };
            let mut ids: Vec<String> = Vec::new();
            {
                let mut state = list.state.borrow_mut();
                state.loading = false;
                match result {
                    Ok(answer) => {
                        state.total = Some(answer.total);
                        for tile in &answer.tiles {
                            for flow in tile.inputs.iter().chain(&tile.outputs) {
                                ids.extend(flow.addresses.iter().cloned());
                                ids.extend(flow.amounts.iter().map(|a| a.currency.clone()));
                            }
                            if let Some(tiles) = list.tiles.upgrade() {
                                tiles.append(&Tile::build(&page, tile.clone(), &list.address));
                            }
                        }
                        state.tiles.extend(answer.tiles);
                        state.missing.extend(answer.missing);
                    }
                    Err(error) => state.failure = Some(bridge::describe(&error)),
                }
            }
            list.render();
            if let Some(model) = list.model.upgrade() {
                model.resolve_names(ids);
            }
        });
    }

    /// When the tip changes, checks that the blocks of the loaded transactions are still on
    /// the active chain, as the terminal marks an address stale after a reorganization.
    fn recheck_stale(self: &Rc<Self>) {
        let hashes: Vec<String> = {
            let state = self.state.borrow();
            if state.stale {
                return;
            }
            let unique: HashSet<String> = state.tiles.iter().filter_map(|t| t.block_hash.clone()).collect();
            unique.into_iter().collect()
        };
        let Some(explorer) = self.model.upgrade().and_then(|m| m.explorer()) else { return };
        if hashes.is_empty() {
            return;
        }
        let (weak, epoch) = (Rc::downgrade(self), self.epoch.get());
        glib::spawn_future_local(async move {
            let checked = bridge::run(async move { explorer.blocks_active(hashes).await }).await;
            let Some(list) = weak.upgrade() else { return };
            if list.epoch.get() != epoch || !checked.iter().any(|b| b.active == Some(false)) {
                return;
            }
            list.state.borrow_mut().stale = true;
            list.render();
            glib::timeout_future(Duration::from_secs(2)).await;
            let Some(list) = weak.upgrade() else { return };
            if list.epoch.get() != epoch {
                return;
            }
            match list.on_stale.clone() {
                Some(on_stale) => on_stale(),
                None => list.reset(),
            }
        });
    }
}

fn install_style() {
    thread_local!(static DONE: Cell<bool> = const { Cell::new(false) });
    if DONE.with(|done| done.replace(true)) {
        return;
    }
    let Some(display) = gtk::gdk::Display::default() else { return };
    let provider = gtk::CssProvider::new();
    // A tile: a thin rule around it, square, as `Rectangle().strokeBorder(Tone.rule)`.
    provider.load_from_string(
        ".address-tile { border: 1px solid @rule; padding: 12px; }\n\
         .address-columns, .address-columns > flowboxchild { padding: 0; margin: 0; background: none; }\n",
    );
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
}

/// One transaction: inputs on the left and outputs on the right, or one above the other
/// when the window is narrow (`AddressTileView`).
struct Tile;

impl Tile {
    fn build(page: &Rc<Page>, tile: api::AddressTile, viewed: &str) -> gtk::Box {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("address-tile");
        let fill = {
            let (page, root, viewed) = (Rc::downgrade(page), root.downgrade(), viewed.to_string());
            move || {
                let (Some(page), Some(root)) = (page.upgrade(), root.upgrade()) else { return };
                page::load::clear(&root);
                Self::fill(&page, &root, &tile, &viewed);
            }
        };
        fill();
        // Heights, dates and names follow the settings, and dates arrive later.
        page.model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &root, move |_| fill());
        root
    }

    fn fill(page: &Rc<Page>, root: &gtk::Box, tile: &api::AddressTile, viewed: &str) {
        let model = &page.model;
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let id = page::route_link(page, &tile.txid, Route::Transaction { txid: tile.txid.clone() }, true);
        id.set_valign(gtk::Align::Center);
        head.append(&id);
        if !tile.tags.is_empty() {
            let tags: Vec<(String, PlateFill)> = tile.tags.iter().map(|t| (t.clone(), PlateFill::None)).collect();
            let row = page::tag_row(&tags);
            row.set_valign(gtk::Align::Center);
            head.append(&row);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        head.append(&spacer);
        if let Some(height) = tile.height {
            let text = model.height_text(height, tile.time);
            let route = Route::Block { id: tile.block_hash.clone().unwrap_or_else(|| height.to_string()) };
            let link = page::route_link(page, &text, route, false);
            link.set_valign(gtk::Align::Center);
            head.append(&link);
        }
        let when = match (tile.time, tile.height) {
            (Some(time), _) => format::date(time),
            (None, None) => "Unconfirmed".into(),
            (None, Some(_)) => "Time not available".into(),
        };
        let when = page::text(&when, &["label", "tabular"]);
        when.set_valign(gtk::Align::Center);
        head.append(&when);
        root.append(&head);

        // Side by side when both columns have 320, else one above the other.
        let columns = gtk::FlowBox::new();
        columns.add_css_class("address-columns");
        columns.set_selection_mode(gtk::SelectionMode::None);
        columns.set_homogeneous(true);
        columns.set_min_children_per_line(1);
        columns.set_max_children_per_line(2);
        columns.set_column_spacing(24);
        columns.set_row_spacing(10);
        columns.set_activate_on_single_click(false);
        for (title, flows, more, outputs) in
            [("Inputs", &tile.inputs, tile.more_inputs, false), ("Outputs", &tile.outputs, tile.more_outputs, true)]
        {
            let column = Self::column(page, title, flows, more, outputs, viewed);
            column.set_size_request(320, -1);
            columns.append(&column);
            if let Some(child) = columns.last_child() {
                child.set_focusable(false);
            }
        }
        root.append(&columns);
    }

    fn column(page: &Rc<Page>, title: &str, flows: &[api::AddressFlow], more: u32, outputs: bool, viewed: &str) -> gtk::Box {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.set_hexpand(true);
        column.append(&theme::label(font::SMALL, title, if outputs { "good" } else { "mined" }));
        if flows.is_empty() {
            column.append(&page::text("none", &["callout", "secondary"]));
        }
        for flow in flows {
            column.append(&flow_row(page, flow, outputs, viewed));
        }
        if more > 0 {
            column.append(&page::text(&format!("… {more} more not shown"), &["callout", "mined"]));
        }
        column
    }
}

/// One input or output (`FlowRow`).
fn flow_row(page: &Rc<Page>, flow: &api::AddressFlow, output: bool, viewed: &str) -> gtk::Box {
    let model = &page.model;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("callout");
    if output {
        let (mark, help) = status_mark(flow);
        mark.set_tooltip_text(Some(help));
        mark.set_can_target(true);
        row.append(&mark);
    }
    let index = page::text(&format!("#{}", flow.index), &["mono", "label"]);
    index.set_valign(gtk::Align::Baseline);
    row.append(&index);
    if flow.coinbase {
        row.append(&baseline(page::text("Coinbase", &["callout", "mined"])));
    } else if flow.private {
        row.append(&baseline(page::text(if output { "Private Output" } else { "Private Input" }, &["callout", "label"])));
    } else if flow.addresses.is_empty() {
        row.append(&baseline(page::text("address unavailable", &["callout", "secondary"])));
    } else {
        let who = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        who.set_valign(gtk::Align::Baseline);
        for address in &flow.addresses {
            if address == viewed {
                let own = page::text(&model.display_id(address), &["callout", "semibold"]);
                own.set_tooltip_text(Some(&format!("{address} (this page)")));
                who.append(&own);
            } else {
                let link = identity_support::id_link(page, address, crate::route::holder(address), true);
                if let Some(label) = crate::widgets::link::label_of(&link) {
                    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                }
                who.append(&link);
            }
        }
        row.append(&who);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    spacer.set_size_request(8, -1);
    row.append(&spacer);
    let amount = if flow.private {
        page::text("amount hidden", &["callout", "secondary"])
    } else if flow.amounts.is_empty() {
        page::text("value unavailable", &["callout", "secondary"])
    } else {
        let text: Vec<String> = flow.amounts.iter().map(|a| model.amount_text(a.sats, &a.currency)).collect();
        page::text(&text.join(", "), &["callout", "tabular"])
    };
    row.append(&baseline(amount));
    row
}

fn baseline(label: gtk::Label) -> gtk::Label {
    label.set_valign(gtk::Align::Baseline);
    label
}

/// The mark of an output: unspent, spent, not known, or private.
fn status_mark(flow: &api::AddressFlow) -> (Canvas, &'static str) {
    let (kind, help) = if flow.private {
        (Status::Private, "Private")
    } else {
        match flow.status {
            api::FlowStatus::Unspent => (Status::Unspent, "Unspent"),
            api::FlowStatus::Spent => (Status::Spent, "Spent"),
            api::FlowStatus::Unknown => (Status::Unknown, "Not known if spent"),
        }
    };
    let mark = Canvas::new(11, 11, move |_, snapshot, w, h| paint_status(snapshot, kind, w, h));
    mark.set_valign(gtk::Align::Center);
    (mark, help)
}

#[derive(Clone, Copy)]
enum Status {
    /// `diamond.fill`
    Private,
    /// `circle.fill`
    Unspent,
    /// `xmark`
    Spent,
    /// `questionmark.circle`
    Unknown,
}

fn paint_status(snapshot: &gtk::Snapshot, kind: Status, w: f32, h: f32) {
    let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0 - 0.5);
    let path = gtk::gsk::PathBuilder::new();
    match kind {
        Status::Private => {
            path.move_to(cx, cy - r);
            path.line_to(cx + r * 0.8, cy);
            path.line_to(cx, cy + r);
            path.line_to(cx - r * 0.8, cy);
            path.close();
            draw::fill(snapshot, &path.to_path(), tone::LABEL);
        }
        Status::Unspent => {
            path.add_circle(&draw::point(cx, cy), r * 0.85);
            draw::fill(snapshot, &path.to_path(), tone::GOOD);
        }
        Status::Spent => {
            let d = r * 0.62;
            draw::rule(snapshot, cx - d, cy - d, cx + d, cy + d, tone::ALERT, 1.5, &[]);
            draw::rule(snapshot, cx - d, cy + d, cx + d, cy - d, tone::ALERT, 1.5, &[]);
        }
        Status::Unknown => {
            let c: Color = tone::LABEL;
            path.add_circle(&draw::point(cx, cy), r * 0.9);
            draw::stroke(snapshot, &path.to_path(), c, 1.1);
            let q = gtk::gsk::PathBuilder::new();
            q.move_to(cx - r * 0.32, cy - r * 0.22);
            q.quad_to(cx - r * 0.28, cy - r * 0.55, cx, cy - r * 0.55);
            q.quad_to(cx + r * 0.36, cy - r * 0.5, cx + r * 0.32, cy - r * 0.18);
            q.quad_to(cx + r * 0.28, cy, cx, cy + r * 0.14);
            draw::stroke(snapshot, &q.to_path(), c, 1.1);
            draw::fill_rect(snapshot, cx - 0.65, cy + r * 0.36, 1.3, 1.3, c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_and_paging() {
        assert_eq!(heading(1234, 0), "1,234 transactions");
        assert_eq!(heading(1234, 20), "Newest 20 of 1,234");
        assert_eq!(older_title(1234, 20).as_deref(), Some("Show 10 older"));
        assert_eq!(older_title(23, 20).as_deref(), Some("Show 3 older"));
        assert_eq!(older_title(20, 20), None);
    }
}
