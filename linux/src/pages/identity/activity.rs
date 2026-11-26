//! The transactions of the identity's address, newest first, in pages of ten, each as its
//! inputs and outputs: a port of `AddressActivityList`, `AddressTileView` and `FlowRow` in
//! `AddressView.swift`, which the Identity page shows in its "Transactions" card.
//!
//! The Address page owns those Swift views. This copy is here because the Identity page
//! needs the list and `pages/address` is another module; one of the two can call the
//! other's once both are merged.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge;
use crate::bridge::api::{AddressFlow, AddressTile, FlowStatus, Route};
use crate::format;
use crate::model::Topic;
use crate::page::{self, load::clear, Page};
use crate::shared::identity_support::{self as support, small_progress};
use crate::theme::{self, font, tone};
use crate::widgets::display;
use crate::widgets::lamp;

use super::control::height_link;
use super::marks::{self, Mark};

const PAGE_SIZE: u64 = 10;

pub struct ActivityList {
    pub root: gtk::Box,
    page: Weak<Page>,
    address: String,
    stale_note: gtk::Box,
    heading: gtk::Label,
    list: gtk::Box,
    footer: gtk::Box,
    tiles: RefCell<Vec<AddressTile>>,
    missing: RefCell<Vec<String>>,
    total: Cell<Option<u64>>,
    loading: Cell<bool>,
    failure: RefCell<Option<String>>,
    stale: Cell<bool>,
    /// A new value discards the answers of earlier requests.
    reloads: Cell<u64>,
    last_tip: Cell<Option<u64>>,
}

impl ActivityList {
    pub fn new(page: &Rc<Page>, address: &str) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let stale_note = page::note(
            "This list is out of date. A block of these transactions left the active chain. The list is refreshing.",
            true,
        );
        stale_note.set_visible(false);
        root.append(&stale_note);
        let heading = page::text("", &["secondary"]);
        heading.set_visible(false);
        root.append(&heading);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.append(&list);
        let footer = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.append(&footer);
        let this = Rc::new(Self {
            root,
            page: Rc::downgrade(page),
            address: address.to_string(),
            stale_note,
            heading,
            list,
            footer,
            tiles: RefCell::default(),
            missing: RefCell::default(),
            total: Cell::new(None),
            loading: Cell::new(false),
            failure: RefCell::default(),
            stale: Cell::new(false),
            reloads: Cell::new(0),
            last_tip: Cell::new(page.model.state().status.as_ref().map(|s| s.blocks)),
        });
        let weak = Rc::downgrade(&this);
        page.model.watch(&[Topic::Connection], &this.root, move |_| {
            if let Some(this) = weak.upgrade() {
                this.restart();
            }
        });
        let weak = Rc::downgrade(&this);
        page.model.watch(&[Topic::Status], &this.root, move |model| {
            let Some(this) = weak.upgrade() else { return };
            let tip = model.state().status.as_ref().map(|s| s.blocks);
            if tip != this.last_tip.get() {
                this.last_tip.set(tip);
                this.recheck_stale();
            }
        });
        let weak = Rc::downgrade(&this);
        page.model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &this.root, move |_| {
            if let Some(this) = weak.upgrade() {
                this.render_tiles();
            }
        });
        this.restart();
        this
    }

    fn loaded_count(&self) -> u64 {
        (self.tiles.borrow().len() + self.missing.borrow().len()) as u64
    }

    fn restart(self: &Rc<Self>) {
        self.reloads.set(self.reloads.get() + 1);
        self.stale.set(false);
        self.tiles.borrow_mut().clear();
        self.missing.borrow_mut().clear();
        self.total.set(None);
        self.loading.set(false);
        self.render_tiles();
        self.load_more();
    }

    /// When the tip changes, checks that the blocks of the loaded transactions are still
    /// on the active chain, as the terminal marks an address stale after a reorg.
    fn recheck_stale(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else { return };
        let hashes: Vec<String> = self
            .tiles
            .borrow()
            .iter()
            .filter_map(|t| t.block_hash.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let Some(explorer) = page.model.explorer() else { return };
        if hashes.is_empty() || self.stale.get() {
            return;
        }
        let reloads = self.reloads.get();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let checked = bridge::run(async move { explorer.blocks_active(hashes).await }).await;
            let Some(this) = weak.upgrade() else { return };
            if this.reloads.get() != reloads || !checked.iter().any(|c| c.active == Some(false)) {
                return;
            }
            this.stale.set(true);
            this.stale_note.set_visible(true);
            glib::timeout_future(Duration::from_secs(2)).await;
            if this.reloads.get() == reloads {
                this.restart();
            }
        });
    }

    fn load_more(self: &Rc<Self>) {
        if self.loading.get() {
            return;
        }
        let Some(page) = self.page.upgrade() else { return };
        let model = page.model.clone();
        let explorer = match model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                self.failure.replace(Some(message));
                self.render_footer();
                return;
            }
        };
        self.loading.set(true);
        self.failure.replace(None);
        self.render_footer();
        let (address, offset, reloads) = (self.address.clone(), self.loaded_count(), self.reloads.get());
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.address_activity(address, offset, PAGE_SIZE).await }).await;
            let Some(this) = weak.upgrade() else { return };
            if this.reloads.get() != reloads {
                return;
            }
            this.loading.set(false);
            match result {
                Ok(found) => {
                    let mut ids = Vec::new();
                    for tile in &found.tiles {
                        for flow in tile.inputs.iter().chain(tile.outputs.iter()) {
                            ids.extend(flow.addresses.iter().cloned());
                            ids.extend(flow.amounts.iter().map(|a| a.currency.clone()));
                        }
                    }
                    this.total.set(Some(found.total));
                    this.tiles.borrow_mut().extend(found.tiles);
                    this.missing.borrow_mut().extend(found.missing);
                    model.resolve_names(ids);
                    this.render_tiles();
                }
                Err(error) => {
                    this.failure.replace(Some(bridge::describe(&error)));
                    this.render_footer();
                }
            }
        });
    }

    fn render_tiles(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else { return };
        self.stale_note.set_visible(self.stale.get());
        let loaded = self.loaded_count();
        match self.total.get() {
            Some(total) => {
                self.heading.set_text(&if loaded > 0 {
                    format!("Newest {} of {}", format::integer(loaded), format::integer(total))
                } else {
                    format!("{} transactions", format::integer(total))
                });
                self.heading.set_visible(true);
            }
            None => self.heading.set_visible(false),
        }
        clear(&self.list);
        if self.total.get() == Some(0) && !self.loading.get() {
            self.list.append(&page::text("No indexed transactions.", &["secondary"]));
        }
        for tile in self.tiles.borrow().iter() {
            self.list.append(&tile_view(&page, tile, &self.address));
        }
        for txid in self.missing.borrow().iter() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&page::route_link(&page, txid, Route::Transaction { txid: txid.clone() }, true));
            row.append(&page::text("details did not load", &["secondary"]));
            self.list.append(&row);
        }
        self.list.set_visible(self.list.first_child().is_some());
        self.render_footer();
    }

    fn render_footer(self: &Rc<Self>) {
        clear(&self.footer);
        if self.loading.get() {
            self.footer.append(&small_progress("Loading transactions…"));
        } else if let Some(failure) = self.failure.borrow().clone() {
            self.footer.append(&page::note(&failure, true));
            let weak = Rc::downgrade(self);
            self.footer.append(&lamp::button("Try Again", move || {
                if let Some(this) = weak.upgrade() {
                    this.load_more();
                }
            }));
        } else if let Some(total) = self.total.get()
            && self.loaded_count() < total
        {
            let weak = Rc::downgrade(self);
            let count = PAGE_SIZE.min(total - self.loaded_count());
            self.footer.append(&lamp::button(&format!("Show {count} older"), move || {
                if let Some(this) = weak.upgrade() {
                    this.load_more();
                }
            }));
        }
        self.footer.set_visible(self.footer.first_child().is_some());
    }
}

/// One transaction: inputs on the left and outputs on the right, or one above the other
/// when the window is narrow.
fn tile_view(page: &Rc<Page>, tile: &AddressTile, viewed: &str) -> gtk::Overlay {
    let model = page.model.clone();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    heading.append(&page::route_link(page, &tile.txid, Route::Transaction { txid: tile.txid.clone() }, true));
    if !tile.tags.is_empty() {
        let tags: Vec<(String, display::PlateFill)> =
            tile.tags.iter().map(|t| (t.clone(), display::signal_tag_fill(t))).collect();
        heading.append(&page::tag_row(&tags));
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    heading.append(&spacer);
    if let Some(height) = tile.height {
        let id = tile.block_hash.clone().unwrap_or_else(|| height.to_string());
        heading.append(&height_link(page, &support::height_text(&model, height, tile.time), Route::Block { id }));
    }
    let when = match (tile.time, tile.height) {
        (Some(time), _) => format::date(time),
        (None, None) => "Unconfirmed".into(),
        (None, Some(_)) => "Time not available".into(),
    };
    heading.append(&page::text(&when, &["label", "tabular"]));
    column.append(&heading);

    // Two columns of at least 320, or one above the other.
    let flows = gtk::FlowBox::new();
    flows.set_selection_mode(gtk::SelectionMode::None);
    flows.set_homogeneous(true);
    flows.set_max_children_per_line(2);
    flows.set_min_children_per_line(1);
    flows.set_column_spacing(24);
    flows.set_row_spacing(10);
    flows.set_activate_on_single_click(false);
    for (title, list, more, outputs) in [("Inputs", &tile.inputs, tile.more_inputs, false), ("Outputs", &tile.outputs, tile.more_outputs, true)] {
        let side = flow_column(page, title, list, more, outputs, viewed);
        side.set_size_request(320, -1);
        flows.append(&side);
    }
    if let Some(child) = flows.first_child() {
        child.set_can_focus(false);
    }
    column.append(&flows);
    marks::outlined(&column, tone::RULE, 12, 12)
}

fn flow_column(page: &Rc<Page>, title: &str, flows: &[AddressFlow], more: u32, outputs: bool, viewed: &str) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
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

fn flow_row(page: &Rc<Page>, flow: &AddressFlow, output: bool, viewed: &str) -> gtk::Box {
    let model = page.model.clone();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.add_css_class("callout");
    if output {
        let (mark, help) = if flow.private {
            (marks::mark(Mark::Diamond, 10, tone::LABEL), "Private")
        } else {
            match flow.status {
                FlowStatus::Unspent => (crate::widgets::icons::icon(crate::widgets::icons::Icon::Dot, 9, tone::GOOD), "Unspent"),
                FlowStatus::Spent => (marks::mark(Mark::Cross, 10, tone::ALERT), "Spent"),
                FlowStatus::Unknown => (marks::mark(Mark::Question, 11, tone::LABEL), "Not known if spent"),
            }
        };
        mark.set_valign(gtk::Align::Center);
        mark.set_can_target(true);
        mark.set_tooltip_text(Some(help));
        mark.update_property(&[gtk::accessible::Property::Label(help)]);
        row.append(&mark);
    }
    let index = page::text(&format!("#{}", flow.index), &["mono", "label"]);
    index.set_valign(gtk::Align::Baseline);
    row.append(&index);
    if flow.coinbase {
        row.append(&page::text("Coinbase", &["mined", "callout"]));
    } else if flow.private {
        row.append(&page::text(if output { "Private Output" } else { "Private Input" }, &["label", "callout"]));
    } else if flow.addresses.is_empty() {
        row.append(&page::text("address unavailable", &["secondary", "callout"]));
    } else {
        let who = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        for address in &flow.addresses {
            if address == viewed {
                let me = page::text(&support::display_id(&model, address), &["semibold", "callout"]);
                me.set_tooltip_text(Some(&format!("{address} (this page)")));
                who.append(&me);
            } else {
                who.append(&support::id_link(page, address, crate::route::holder(address), true));
            }
        }
        row.append(&who);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    spacer.set_size_request(8, -1);
    row.append(&spacer);
    if flow.private {
        row.append(&page::text("amount hidden", &["secondary", "callout"]));
    } else if flow.amounts.is_empty() {
        row.append(&page::text("value unavailable", &["secondary", "callout"]));
    } else {
        let text: Vec<String> = flow.amounts.iter().map(|a| support::amount_text(&model, a.sats, &a.currency)).collect();
        row.append(&page::text(&text.join(", "), &["tabular", "callout"]));
    }
    row
}
