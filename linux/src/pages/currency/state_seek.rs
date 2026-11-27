//! The currency's state at one exact block, found by height, Unix seconds or a date, and
//! the imports and exports around that block (`CurrencyStateSeek.swift`).
//!
//! This follows Carbide's currency viewer: a bare number up to the tip is a height and a
//! larger one is Unix seconds; `height:`, `#` and `unix:` remove doubt. A future state is
//! refused. A time request reports which blocks it checked, because block times are not in
//! order and the nearest checked block is not proof of the nearest block.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;

use super::pool_batch;
use crate::bridge::{self, api, api::Route};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, Card, Page};
use crate::shared::currency_stores::{height_text, known_name, observation_source, other_chain_note, source_word};
use crate::theme::{self, draw, font, tone};
use crate::widgets::icons::{self, Icon};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, PlateFill, Readout};
use crate::widgets::{lamp, loader};

pub const TITLE: &str = "State at a chosen block";

/// The window of the app, for the confirmation of a chain switch.
pub fn window() -> Option<gtk::Window> {
    gtk::gio::Application::default()
        .and_downcast::<gtk::Application>()
        .and_then(|app| app.active_window())
}

/// `OtherChainNote`, or nothing when the app has no window.
pub fn other_chain(model: &Rc<AppModel>, system_id: &str, fallback: Option<&str>) -> Option<gtk::Box> {
    window().map(|window| other_chain_note(model, &window, system_id, fallback))
}

/// A key of the display with a mark before its legend, as a `Label` in a button.
pub fn mark_key(title: &str, mark: Icon, action: impl Fn() + 'static) -> gtk::Button {
    lamp::button_with(title, Some(mark), action)
}

/// A small spinner, as `ProgressView().controlSize(.small)`.
fn spinner() -> gtk::Widget {
    loader::small().upcast()
}

/// The text of the checked blocks of a time request.
pub fn checked_text(coverage: &api::TimeCoverage) -> String {
    format!(
        "{} to {}, {} blocks checked",
        format::integer(coverage.checked_first),
        format::integer(coverage.checked_last),
        coverage.probes
    )
}

/// What the block of a time request is to the requested time.
pub fn coverage_sentence(coverage: &api::TimeCoverage) -> String {
    let seconds = coverage.difference_seconds;
    let direction = if seconds == 0 {
        ""
    } else if seconds > 0 {
        " after the requested time"
    } else {
        " before the requested time"
    };
    let gap = if seconds == 0 {
        "The block time matches the requested time exactly.".to_string()
    } else {
        format!("The block time is {} seconds{direction}.", seconds.unsigned_abs())
    };
    if coverage.exact {
        return gap;
    }
    if coverage.whole_chain {
        return format!("{gap} This is the nearest block.");
    }
    format!("{gap} This is the nearest of the checked blocks. Block times are not in order, so a block outside the checked range can be nearer.")
}

/// The state as a history point, so that the batch statement can draw it.
pub fn point(state: &api::CurrencyStateAt) -> api::CurrencyPoint {
    api::CurrencyPoint {
        height: state.height,
        time: state.block_time,
        supply: state.supply,
        reserves: state
            .reserves
            .iter()
            .map(|r| api::ReservePoint {
                currency_id: r.currency_id.clone(),
                reserves: r.reserves,
                price_in_reserve: r.price_in_reserve,
                last_in: r.reserve_in,
                last_out: r.reserve_out,
                last_fees: r.conversion_fees,
            })
            .collect(),
    }
}

// MARK: The card

#[derive(Default)]
struct Seek {
    loading: bool,
    failure: Option<String>,
    state: Option<api::CurrencyStateAt>,
    show_json: bool,
}

pub struct StateCard {
    pub card: Card,
    model: Weak<AppModel>,
    page: Weak<Page>,
    currency: api::CurrencyDetail,
    seek: RefCell<Seek>,
    entry: gtk::Entry,
    show_button: gtk::Button,
    latest_button: gtk::Button,
    busy: gtk::Box,
    failure_box: gtk::Box,
    detail: gtk::Box,
    activity_holder: gtk::Box,
    activity: RefCell<Option<Rc<Activity>>>,
    empty: gtk::Label,
}

impl StateCard {
    pub fn new(page: &Rc<Page>, currency: &api::CurrencyDetail) -> Rc<Self> {
        let model = page.model.clone();
        let card = page.card(TITLE, None);
        card.append(&page::paragraph(
            "Enter a block height, Unix seconds or a date and time in UTC. A date without a time means 00:00 UTC. Use # or height: for a height, and unix: for a time.",
            &["callout", "secondary"],
        ));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let entry = gtk::Entry::new();
        entry.add_css_class("display-field");
        entry.set_placeholder_text(Some("3962166, #3962166, 2024-06-01 12:00 or unix:1717243200"));
        entry.update_property(&[gtk::accessible::Property::Label("Block or time")]);
        entry.set_size_request(420, -1);
        row.append(&entry);
        let busy = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let failure_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let activity_holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let empty = page::paragraph(
            "No state is selected. Enter a block or time, or use Show Exact State on the slider above.",
            &["secondary"],
        );
        let card = Rc::new_cyclic(|weak: &Weak<StateCard>| {
            let show_button = {
                let weak = weak.clone();
                lamp::button("Show State", move || {
                    if let Some(card) = weak.upgrade() {
                        card.seek(&card.entry.text());
                    }
                })
            };
            let latest_button = {
                let weak = weak.clone();
                lamp::button("Latest Block", move || {
                    let Some(card) = weak.upgrade() else { return };
                    let tip = card.model.upgrade().and_then(|m| m.state().status.as_ref().map(|s| s.blocks));
                    if let Some(tip) = tip {
                        let query = format!("#{tip}");
                        card.entry.set_text(&query);
                        card.seek(&query);
                    }
                })
            };
            row.append(&show_button);
            row.append(&latest_button);
            row.append(&busy);
            Self {
                card,
                model: Rc::downgrade(&model),
                page: Rc::downgrade(page),
                currency: currency.clone(),
                seek: RefCell::default(),
                entry: entry.clone(),
                show_button,
                latest_button,
                busy: busy.clone(),
                failure_box: failure_box.clone(),
                detail: detail.clone(),
                activity_holder: activity_holder.clone(),
                activity: RefCell::new(None),
                empty: empty.clone(),
            }
        });
        card.card.append(&row);
        card.card.append(&failure_box);
        card.card.append(&detail);
        card.card.append(&activity_holder);
        card.card.append(&empty);
        {
            let weak = Rc::downgrade(&card);
            entry.connect_activate(move |entry| {
                if let Some(card) = weak.upgrade() {
                    card.seek(&entry.text());
                }
            });
            let weak = Rc::downgrade(&card);
            entry.connect_changed(move |_| {
                if let Some(card) = weak.upgrade() {
                    card.update_keys();
                }
            });
        }
        let weak = Rc::downgrade(&card);
        model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &card.card.root, move |_| {
            if let Some(card) = weak.upgrade() {
                card.render_detail();
            }
        });
        let weak = Rc::downgrade(&card);
        model.watch(&[Topic::Status], &card.card.root, move |_| {
            if let Some(card) = weak.upgrade() {
                card.update_keys();
            }
        });
        card.render();
        card
    }

    /// Puts the keyboard in the field (the terminal's g).
    pub fn focus(&self) {
        self.entry.grab_focus();
    }

    /// Asks for the state at a block, as Show Exact State and a request of another page do.
    pub fn choose(self: &Rc<Self>, height: u64) {
        let query = format!("#{height}");
        self.entry.set_text(&query);
        self.seek(&query);
    }

    fn update_keys(&self) {
        let loading = self.seek.borrow().loading;
        let connected = self.model.upgrade().is_some_and(|m| m.state().status.is_some());
        self.show_button.set_sensitive(!loading && !self.entry.text().trim().is_empty());
        self.latest_button.set_sensitive(!loading && connected);
    }

    fn seek(self: &Rc<Self>, text: &str) {
        let text = text.trim().to_string();
        if text.is_empty() || self.seek.borrow().loading {
            return;
        }
        let Some(model) = self.model.upgrade() else { return };
        {
            let mut seek = self.seek.borrow_mut();
            seek.loading = true;
            seek.failure = None;
        }
        self.render();
        let explorer = match model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                let mut seek = self.seek.borrow_mut();
                seek.loading = false;
                seek.failure = Some(message);
                drop(seek);
                self.render();
                return;
            }
        };
        let key = self.currency.currency_id.clone().unwrap_or_else(|| self.currency.name.clone());
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.currency_state_at(key, text).await }).await;
            let Some(card) = weak.upgrade() else { return };
            let new_state = {
                let mut seek = card.seek.borrow_mut();
                seek.loading = false;
                match result {
                    Ok(found) => {
                        seek.state = Some(found);
                        true
                    }
                    // The earlier state stays, with its source and block.
                    Err(error) => {
                        seek.failure = Some(bridge::describe(&error));
                        false
                    }
                }
            };
            if new_state {
                card.replace_activity();
                if let Some(model) = card.model.upgrade()
                    && let Some(state) = card.seek.borrow().state.as_ref()
                {
                    model.resolve_names(state.reserves.iter().map(|r| r.currency_id.clone()).collect::<Vec<_>>());
                }
            }
            card.render();
        });
    }

    /// A new state has a new imports and exports section.
    fn replace_activity(self: &Rc<Self>) {
        clear(&self.activity_holder);
        let (Some(page), Some(state)) = (self.page.upgrade(), self.seek.borrow().state.clone()) else { return };
        let activity = Activity::new(&page, state);
        self.activity_holder.append(&activity.root);
        *self.activity.borrow_mut() = Some(activity);
    }

    fn render(self: &Rc<Self>) {
        let (loading, failure, has_state) = {
            let seek = self.seek.borrow();
            (seek.loading, seek.failure.clone(), seek.state.is_some())
        };
        clear(&self.busy);
        if loading {
            self.busy.append(&spinner());
        }
        clear(&self.failure_box);
        if let Some(failure) = failure.as_ref() {
            self.failure_box.append(&page::note(failure, true));
        }
        self.failure_box.set_visible(failure.is_some());
        self.empty.set_visible(!has_state && !loading && failure.is_none());
        self.update_keys();
        self.render_detail();
    }

    fn reserve_name(&self, model: &AppModel, id: &str) -> String {
        let fallback = self.currency.reserves.iter().find(|r| r.currency_id == id).and_then(|r| r.name.clone());
        known_name(id, model, fallback.as_deref())
    }

    fn render_detail(self: &Rc<Self>) {
        clear(&self.detail);
        let state = self.seek.borrow().state.clone();
        let (Some(state), Some(page), Some(model)) = (state, self.page.upgrade(), self.model.upgrade()) else {
            self.detail.set_visible(false);
            return;
        };
        self.detail.set_visible(true);
        let detail = &self.detail;
        detail.append(&page::provenance_note(&state.provenance));
        detail.append(&page::field_row(&page, "Request", &state.interpreted, None, false));
        detail.append(&page::field_row(
            &page,
            "Block",
            &height_text(state.height, state.block_time, &model),
            Some(Route::Block { id: state.block_hash.clone() }),
            false,
        ));
        detail.append(&page::field_row(&page, "Block hash", &state.block_hash, Some(Route::Block { id: state.block_hash.clone() }), true));
        if let Some(time) = state.block_time {
            detail.append(&page::field_row(&page, "Block time", &format::date(time), None, false));
        }
        if let Some(coverage) = state.coverage.as_ref() {
            detail.append(&page::field_row(&page, "Requested time", &format::date(coverage.target_time.max(0) as u64), None, false));
            detail.append(&page::field_row(&page, "Checked blocks", &checked_text(coverage), None, false));
            detail.append(&page::note(&coverage_sentence(coverage), !coverage.exact && !coverage.whole_chain));
        }
        detail.append(&page::field_row(
            &page,
            "Source",
            &format!("{} · {}", source_word(state.data_source), observation_source(&state.source)),
            None,
            false,
        ));
        if let Some(tip) = state.tip_height {
            detail.append(&page::field_row(&page, "Tip when read", &format::integer(tip), None, false));
        }
        detail.append(&page::field_row(&page, "Launch status", &state.launch_label, None, false));
        if let Some(supply) = state.supply {
            detail.append(&page::field_row(&page, "Supply", &format!("{} {}", format::amount(supply), self.currency.name), None, false));
        }
        if !state.reserves.is_empty() {
            detail.append(&reserve_table(&page, &state));
            let name = |id: &str| self.reserve_name(&model, id);
            detail.append(&pool_batch::pool_batch(&point(&state), None, &name));
        }
        // The state from the node, with its source and block.
        let show_json = self.seek.borrow().show_json;
        let json = state.json.clone();
        let weak = Rc::downgrade(self);
        detail.append(&super::data_disclosure(
            show_json,
            |open| if open { "Hide the state" } else { "Show the state from the node, with its source and block" },
            &format!("source: {}\nblock: {} {}\n{}", state.source, state.height, state.block_hash, state.json),
            move || super::json_text(&json),
            move |open| {
                if let Some(card) = weak.upgrade() {
                    card.seek.borrow_mut().show_json = open;
                }
            },
        ));
    }
}

fn trailing(label: &gtk::Label) {
    label.set_xalign(1.0);
    label.set_halign(gtk::Align::Fill);
}

fn reserve_table(page: &Rc<Page>, state: &api::CurrencyStateAt) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.set_column_spacing(18);
    grid.set_row_spacing(6);
    for (column, title) in ["Reserve", "In reserve", "Price", "Last batch in", "Last batch out"].iter().enumerate() {
        let name = display::name_text(title);
        if column > 0 {
            trailing(&name);
        }
        grid.attach(&name, column as i32, 0, 1, 1);
    }
    grid.attach(&display::rule(), 0, 1, 5, 1);
    for (index, reserve) in state.reserves.iter().enumerate() {
        let row = index as i32 + 2;
        let link = page::route_link(page, &reserve.currency_id, Route::Currency { name: reserve.currency_id.clone() }, true);
        grid.attach(&link, 0, row, 1, 1);
        let values = [
            reserve.reserves.map(format::amount).unwrap_or_else(|| "–".into()),
            reserve.price_in_reserve.map(format::amount).unwrap_or_else(|| "–".into()),
            format::amount(reserve.reserve_in),
            format::amount(reserve.reserve_out),
        ];
        for (offset, value) in values.iter().enumerate() {
            let label = page::text(value, &["tabular"]);
            trailing(&label);
            grid.attach(&label, offset as i32 + 1, row, 1, 1);
        }
    }
    grid
}

// MARK: Imports and exports

/// Imports and exports at the chosen state, in windows of up to 1,000 blocks that end at
/// or before the state's block (`CurrencyActivitySection`).
///
/// Carbide's batches are evidence about a whole export. A single transfer is only found
/// from the export's declared input range, so each record can open that range.
#[derive(Default)]
struct ActivityData {
    page: Option<api::ActivityPage>,
    /// Windows loaded before, kept with their source and block.
    retained: HashMap<String, api::ActivityPage>,
    from_retained: bool,
    loading: bool,
    failure: Option<String>,
    selected: Option<api::ActivityRecord>,
    inputs: Option<api::ExportInputs>,
    inputs_failure: Option<String>,
    inputs_loading: bool,
}

pub struct Activity {
    pub root: gtk::Box,
    body: gtk::Box,
    state: api::CurrencyStateAt,
    model: Weak<AppModel>,
    page: Weak<Page>,
    data: RefCell<ActivityData>,
    run: Cell<u64>,
}

/// An import: filled, above the line.
const IMPORT_TONE: theme::Color = tone::SERIES[0];
/// An export: open, below the line.
const EXPORT_TONE: theme::Color = tone::SERIES[1];

impl Activity {
    fn new(page: &Rc<Page>, state: api::CurrencyStateAt) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.append(&body);
        root.set_margin_top(4);
        let activity = Rc::new(Self {
            root,
            body,
            state,
            model: Rc::downgrade(&page.model),
            page: Rc::downgrade(page),
            data: RefCell::default(),
            run: Cell::new(0),
        });
        let weak = Rc::downgrade(&activity);
        page.model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &activity.root, move |_| {
            if let Some(activity) = weak.upgrade() {
                activity.render();
            }
        });
        activity.render();
        activity
    }

    fn render(self: &Rc<Self>) {
        super::release_focus(&self.body);
        clear(&self.body);
        let (Some(page), Some(model)) = (self.page.upgrade(), self.model.upgrade()) else { return };
        let rule = display::rule();
        rule.set_margin_top(6);
        self.body.append(&rule);
        self.body.append(&theme::label(font::HEADING, "Imports and exports", "value"));
        self.body.append(&page::paragraph(
            "The app asks the node with getimports and getexports. A public API can refuse these requests.",
            &["caption-text", "secondary"],
        ));
        let data = self.data.borrow();
        let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let height = self.state.height;
        let weak = Rc::downgrade(self);
        keys.append(&lamp::button("Check This Block", move || {
            if let Some(activity) = weak.upgrade() {
                activity.load(api::BlockWindow { first: height, last: height });
            }
        }));
        let weak = Rc::downgrade(self);
        keys.append(&lamp::button("Load Preceding 1,000 Blocks", move || {
            if let Some(activity) = weak.upgrade() {
                activity.load(api::activity_window_ending_at(height));
            }
        }));
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        keys.append(&spacer);
        let older = data.page.as_ref().and_then(|p| p.older);
        let newer = data.page.as_ref().and_then(|p| p.newer);
        let weak = Rc::downgrade(self);
        let older_key = mark_key("Older", Icon::ChevronLeft, move || {
            if let (Some(activity), Some(window)) = (weak.upgrade(), older) {
                activity.load(window);
            }
        });
        older_key.set_sensitive(older.is_some() && !data.loading);
        keys.append(&older_key);
        let weak = Rc::downgrade(self);
        let newer_key = mark_key("Newer", Icon::ChevronRight, move || {
            if let (Some(activity), Some(window)) = (weak.upgrade(), newer) {
                activity.load(window);
            }
        });
        newer_key.set_sensitive(newer.is_some() && !data.loading);
        newer_key.set_tooltip_text(Some(&format!("Newer windows stop at the state's block, {}.", format::integer(height))));
        keys.append(&newer_key);
        if data.loading {
            keys.append(&spinner());
        }
        keys.set_sensitive(!data.loading);
        self.body.append(&keys);
        if let Some(failure) = data.failure.as_ref() {
            self.body.append(&page::note(failure, true));
        }
        if let Some(window_page) = data.page.as_ref() {
            self.page_view(&page, &model, window_page, &data);
        }
    }

    fn page_view(self: &Rc<Self>, page: &Rc<Page>, model: &Rc<AppModel>, window: &api::ActivityPage, data: &ActivityData) {
        let imports = window.records.iter().filter(|r| r.kind == api::ActivityKind::Import).count();
        let exports = window.records.len() - imports;
        let mut sentence = format!(
            "Blocks {} to {}: {imports} import{} and {exports} export{}. Source {}, anchored at block {}.",
            format::integer(window.window.first),
            format::integer(window.window.last),
            if imports == 1 { "" } else { "s" },
            if exports == 1 { "" } else { "s" },
            observation_source(&window.source),
            format::integer(window.anchor_height)
        );
        if data.from_retained {
            sentence += " This window is from an earlier request in this session.";
        }
        self.body.append(&page::paragraph(&sentence, &["callout"]));
        if window.truncated {
            self.body.append(&page::note("The window holds more records than Carbide keeps. The app shows the newest 80.", true));
        }
        if window.records.is_empty() {
            self.body.append(&page::paragraph(
                "The app found no imports or exports in this window. This does not prove that none exist outside it.",
                &["secondary"],
            ));
            return;
        }
        let inner = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let readouts = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        readouts.append(&Readout::new("Imports", &imports.to_string(), "record", None).root);
        readouts.append(&Readout::new("Exports", &exports.to_string(), "mined", None).root);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        readouts.append(&spacer);
        readouts.append(&display::plate("Import", PlateFill::Record));
        readouts.append(&display::plate("Export", PlateFill::Mined));
        inner.append(&readouts);
        inner.append(&display::rule());
        inner.append(&self.strip(window, data.selected.clone()));
        inner.append(&display::rule());
        inner.append(&page::paragraph(
            "A filled mark above the line is an import. An open mark below the line is an export. Each mark is at its block. Click a mark to select it.",
            &["caption-text", "label"],
        ));
        self.body.append(&display::panel(&inner, 16));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for record in &window.records {
            list.append(&display::rule());
            list.append(&self.record_row(page, model, record, data));
            if data.selected.as_ref() == Some(record) {
                list.append(&self.inputs_view(page, model, record, data));
            }
        }
        self.body.append(&list);
    }

    /// The window as a line of blocks, with each import above it and each export below it.
    fn strip(self: &Rc<Self>, window: &api::ActivityPage, selected: Option<api::ActivityRecord>) -> Canvas {
        let records = Rc::new(window.records.clone());
        let range = window.window;
        let x = move |height: u64, width: f32| {
            let span = (range.last.saturating_sub(range.first) as f64).max(1.0);
            8.0 + (height.saturating_sub(range.first) as f64 / span) as f32 * (width - 16.0)
        };
        let drawn = records.clone();
        let canvas = Canvas::new(0, 64, move |widget, snapshot, w, h| {
            let middle = h / 2.0 - 6.0;
            draw::rule(snapshot, 8.0, middle, w - 8.0, middle, tone::AXIS, 1.0, &[]);
            for record in drawn.iter() {
                let px = x(record.height, w);
                let is_import = record.kind == api::ActivityKind::Import;
                let y = if is_import { middle - 12.0 } else { middle + 12.0 };
                let color = if is_import { IMPORT_TONE } else { EXPORT_TONE };
                draw::rule(snapshot, px, middle, px, y, color.opacity(0.5), 1.0, &[]);
                let chosen = selected.as_ref() == Some(record);
                if is_import {
                    draw::lit(snapshot, &draw::ellipse_path(px - 4.0, y - 4.0, 8.0, 8.0), color, if chosen { 0.8 } else { 0.0 });
                } else {
                    draw::stroke(snapshot, &draw::ellipse_path(px - 3.5, y - 3.5, 7.0, 7.0), color, 1.0);
                }
                if chosen {
                    draw::push_glow(snapshot, tone::PRESENT, 0.7);
                    draw::stroke(snapshot, &draw::ellipse_path(px - 7.0, y - 7.0, 14.0, 14.0), tone::PRESENT, 1.5);
                    snapshot.pop();
                }
            }
            draw::text(snapshot, widget, crate::theme::font::Role { tabular: true, ..font::SMALL }, &format::integer(range.first), 8.0, h - 4.0, draw::Anchor::BottomLeading, tone::LABEL);
            draw::text(snapshot, widget, crate::theme::font::Role { tabular: true, ..font::SMALL }, &format::integer(range.last), w - 8.0, h - 4.0, draw::Anchor::BottomTrailing, tone::LABEL);
        });
        canvas.set_hexpand(true);
        canvas.update_property(&[gtk::accessible::Property::Label(&format!(
            "{} imports and exports between blocks {} and {}",
            window.records.len(),
            range.first,
            range.last
        ))]);
        let click = gtk::GestureClick::new();
        let (weak, target) = (Rc::downgrade(self), canvas.downgrade());
        click.connect_released(move |_, _, px, py| {
            let (Some(activity), Some(canvas)) = (weak.upgrade(), target.upgrade()) else { return };
            let (w, h) = (canvas.width() as f32, canvas.height() as f32);
            let middle = h / 2.0 - 6.0;
            let mut best = None;
            let mut distance = 14.0_f32;
            for record in records.iter() {
                let y = if record.kind == api::ActivityKind::Import { middle - 12.0 } else { middle + 12.0 };
                let gap = (x(record.height, w) - px as f32).hypot(y - py as f32);
                if gap < distance {
                    distance = gap;
                    best = Some(record.clone());
                }
            }
            if let Some(record) = best {
                activity.select(record);
            }
        });
        canvas.add_controller(click);
        canvas
    }

    fn record_row(self: &Rc<Self>, page: &Rc<Page>, model: &Rc<AppModel>, record: &api::ActivityRecord, data: &ActivityData) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        let is_import = record.kind == api::ActivityKind::Import;
        let kind = icons::label(
            if is_import { Icon::ArrowDownCircleFill } else { Icon::ArrowUpCircle },
            if is_import { "Import" } else { "Export" },
            if is_import { IMPORT_TONE } else { EXPORT_TONE },
            &[],
        );
        kind.set_size_request(90, -1);
        row.append(&kind);
        let block = page::route_link(page, &height_text(record.height, None, model), Route::Block { id: record.height.to_string() }, false);
        let holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        holder.set_size_request(if crate::display::shows_dates() { 190 } else { 90 }, -1);
        holder.append(&block);
        row.append(&holder);
        row.append(&page::route_link(page, &record.output.txid, Route::Transaction { txid: record.output.txid.clone() }, true));
        row.append(&page::text(&format!("output {}", record.output.output), &["secondary"]));
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
        if let Some(export) = record.export.as_ref().filter(|_| is_import) {
            let from = page::text(&format!("from {}", known_name(&export.system_id, model, None)), &["secondary"]);
            from.set_tooltip_text(Some(&format!("Source export {}:{} on {}", export.txid, export.output, export.system_id)));
            row.append(&from);
        }
        let chosen = data.selected.as_ref() == Some(record);
        let (weak, target) = (Rc::downgrade(self), record.clone());
        let key = lamp::button(if chosen { "Hide Inputs" } else { "Inspect Source Export" }, move || {
            let Some(activity) = weak.upgrade() else { return };
            let selected = activity.data.borrow().selected.clone();
            if selected.as_ref() == Some(&target) {
                activity.data.borrow_mut().selected = None;
                activity.render();
            } else {
                activity.select(target.clone());
            }
        });
        key.set_sensitive(record.export.is_some());
        row.append(&key);
        row
    }

    fn inputs_view(self: &Rc<Self>, page: &Rc<Page>, model: &Rc<AppModel>, record: &api::ActivityRecord, data: &ActivityData) -> gtk::Box {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        column.set_margin_start(102);
        column.set_margin_bottom(8);
        let chain = model.state().status.as_ref().and_then(|s| s.chain_id.clone());
        if let Some(export) = record.export.as_ref() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&page::text(&format!("Source export on {}", known_name(&export.system_id, model, None)), &["secondary"]));
            if Some(&export.system_id) == chain.as_ref() {
                row.append(&page::route_link(page, &export.txid, Route::Transaction { txid: export.txid.clone() }, true));
            } else {
                let id = page::text(&format::short(&export.txid, 10, 8), &["mono", "callout"]);
                id.set_selectable(true);
                id.set_tooltip_text(Some(&export.txid));
                row.append(&id);
            }
            row.append(&page::text(&format!("output {}", export.output), &["secondary"]));
            if Some(&export.system_id) != chain.as_ref()
                && let Some(note) = other_chain(model, &export.system_id, None)
            {
                row.append(&note);
            }
            column.append(&row);
        }
        if data.inputs_loading {
            let busy = spinner();
            busy.set_halign(gtk::Align::Start);
            column.append(&busy);
        } else if let Some(failure) = data.inputs_failure.as_ref() {
            column.append(&page::note(failure, true));
        } else if let Some(inputs) = data.inputs.as_ref() {
            let count = inputs.inputs.len();
            column.append(&page::paragraph(
                &format!(
                    "The export spent {count} output{} in its declared input range. Each is a transfer that went into the batch.",
                    if count == 1 { "" } else { "s" }
                ),
                &["callout"],
            ));
            if inputs.truncated {
                column.append(&page::note("The range holds more inputs than Carbide lists. The app shows the first 64.", true));
            }
            for (index, input) in inputs.inputs.iter().enumerate() {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                let name = display::name_text(&format!("Input {}", index + 1));
                name.set_size_request(70, -1);
                row.append(&name);
                let here = Some(&input.system_id) == chain.as_ref();
                if here {
                    row.append(&page::route_link(page, &input.txid, Route::Transaction { txid: input.txid.clone() }, true));
                } else {
                    let id = page::text(&format::short(&input.txid, 10, 8), &["mono", "callout"]);
                    id.set_selectable(true);
                    row.append(&id);
                }
                row.append(&page::text(&format!("output {}", input.output), &["secondary"]));
                let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                spacer.set_hexpand(true);
                row.append(&spacer);
                if here {
                    let (weak, txid, output) = (Rc::downgrade(model), input.txid.clone(), input.output);
                    let follow = lamp::button("Follow Transfer", move || {
                        if let Some(model) = weak.upgrade() {
                            model.open_trace(txid.clone(), output);
                        }
                    });
                    follow.set_tooltip_text(Some(&format!("Opens the transaction and follows the transfer from output {}.", input.output)));
                    row.append(&follow);
                } else if let Some(note) = other_chain(model, &input.system_id, None) {
                    row.append(&note);
                }
                column.append(&row);
            }
            column.append(&page::paragraph(
                "Follow Transfer opens the transaction and starts the transfer trace at that output.",
                &["caption-text", "secondary"],
            ));
        }
        column
    }

    fn select(self: &Rc<Self>, record: api::ActivityRecord) {
        {
            let mut data = self.data.borrow_mut();
            data.selected = Some(record.clone());
            data.inputs = None;
            data.inputs_failure = None;
            data.inputs_loading = record.export.is_some();
        }
        if record.export.is_none() {
            self.render();
            return;
        }
        self.render();
        let Some(model) = self.model.upgrade() else { return };
        let explorer = match model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                let mut data = self.data.borrow_mut();
                data.inputs_failure = Some(message);
                data.inputs_loading = false;
                drop(data);
                self.render();
                return;
            }
        };
        let weak = Rc::downgrade(self);
        let request = record.clone();
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.export_inputs(request).await }).await;
            let Some(activity) = weak.upgrade() else { return };
            {
                let mut data = activity.data.borrow_mut();
                if data.selected.as_ref() != Some(&record) {
                    return;
                }
                match result {
                    Ok(found) => data.inputs = Some(found),
                    Err(error) => data.inputs_failure = Some(bridge::describe(&error)),
                }
                data.inputs_loading = false;
            }
            activity.render();
        });
    }

    fn load(self: &Rc<Self>, window: api::BlockWindow) {
        let key = format!("{}-{}", window.first, window.last);
        {
            let mut data = self.data.borrow_mut();
            data.selected = None;
            if let Some(kept) = data.retained.get(&key).cloned() {
                data.page = Some(kept);
                data.from_retained = true;
                data.failure = None;
                drop(data);
                self.render();
                return;
            }
            data.loading = true;
            data.failure = None;
        }
        self.render();
        let Some(model) = self.model.upgrade() else { return };
        let explorer = match model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                let mut data = self.data.borrow_mut();
                data.failure = Some(message);
                data.loading = false;
                drop(data);
                self.render();
                return;
            }
        };
        let run = self.run.get() + 1;
        self.run.set(run);
        let (currency, height, hash) = (self.state.currency_id.clone(), self.state.height, self.state.block_hash.clone());
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.currency_activity(currency, window.first, window.last, height, hash).await }).await;
            let Some(activity) = weak.upgrade() else { return };
            let mut systems = Vec::new();
            {
                let mut data = activity.data.borrow_mut();
                data.loading = false;
                match result {
                    Ok(found) => {
                        systems = found.records.iter().filter_map(|r| r.export.as_ref().map(|e| e.system_id.clone())).collect();
                        data.retained.insert(key, found.clone());
                        data.page = Some(found);
                        data.from_retained = false;
                    }
                    Err(error) => data.failure = Some(bridge::describe(&error)),
                }
            }
            activity.render();
            if let Some(model) = activity.model.upgrade() {
                model.resolve_names(systems);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage(difference: i64, exact: bool, whole: bool) -> api::TimeCoverage {
        api::TimeCoverage {
            target_time: 1_717_243_200,
            block_time: 1_717_243_200 + difference,
            difference_seconds: difference,
            exact,
            whole_chain: whole,
            checked_first: 3_962_100,
            checked_last: 3_962_200,
            probes: 12,
            description: String::new(),
        }
    }

    #[test]
    fn coverage_in_words() {
        assert_eq!(coverage_sentence(&coverage(0, true, false)), "The block time matches the requested time exactly.");
        assert_eq!(coverage_sentence(&coverage(-30, false, true)), "The block time is 30 seconds before the requested time. This is the nearest block.");
        assert!(coverage_sentence(&coverage(12, false, false)).starts_with("The block time is 12 seconds after the requested time. This is the nearest of the checked blocks."));
        assert_eq!(checked_text(&coverage(0, true, false)), "3,962,100 to 3,962,200, 12 blocks checked");
    }
}
