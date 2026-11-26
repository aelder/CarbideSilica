//! What waits for a block (`WaitingRoom`): a time record with the blocks on it, the
//! analysis, the number of each kind, and a row for each transaction.
//!
//! The rows are in order of arrival and do not move when a transaction arrives. A
//! transaction that spends from another waiting transaction is below it, with an indent.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use gtk::prelude::*;

use crate::bridge::api::Route;
use crate::format;
use crate::model::AppModel;
use crate::page::{self, Page};
use crate::theme::{self, font};
use crate::wait_record;
use crate::widgets::display::{self, PlateFill};

use super::feed;
use super::logic::{self, Context, MANY_ROWS};
use super::record::TimeRecord;
use super::rows::{KeyedRows, MempoolRow};
use super::text::span;
use super::words;
use crate::shared::statement_words::Namer;

pub struct WaitingRoom {
    pub root: gtk::Box,
    page: Weak<Page>,
    model: Weak<AppModel>,
    pub record: Rc<TimeRecord>,
    points: gtk::Grid,
    kind_rule: gtk::Box,
    kind_table: gtk::Grid,
    kinds: gtk::FlowBox,
    rows: KeyedRows<MempoolRow>,
    /// The kind that the rows are limited to. None shows each kind.
    kind: RefCell<Option<String>>,
    shown_points: RefCell<Vec<logic::Point>>,
    shown_table: RefCell<Vec<Vec<String>>>,
    shown_kinds: RefCell<Vec<(String, usize, bool)>>,
}

impl WaitingRoom {
    pub fn new(page: &Rc<Page>, model: &Rc<AppModel>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
        // The time record and the next block are one instrument.
        let record = TimeRecord::new(model);
        root.append(&record.root);

        let analysis = gtk::Box::new(gtk::Orientation::Vertical, 14);
        analysis.set_margin_top(4);
        analysis.set_margin_bottom(4);
        let points = gtk::Grid::new();
        points.set_column_spacing(22);
        points.set_row_spacing(10);
        analysis.append(&points);
        // With one kind, the first line says all that the table says.
        let kind_rule = display::rule();
        analysis.append(&kind_rule);
        let kind_table = gtk::Grid::new();
        kind_table.set_column_spacing(26);
        kind_table.set_row_spacing(5);
        kind_table.set_margin_top(2);
        kind_table.set_halign(gtk::Align::Start);
        analysis.append(&kind_table);
        root.append(&analysis);

        let kinds = gtk::FlowBox::new();
        kinds.set_selection_mode(gtk::SelectionMode::None);
        kinds.set_column_spacing(6);
        kinds.set_row_spacing(6);
        kinds.set_homogeneous(false);
        kinds.set_max_children_per_line(40);
        kinds.set_halign(gtk::Align::Start);
        root.append(&kinds);
        let rows = KeyedRows::new();
        root.append(&rows.root);

        let room = Rc::new(Self {
            root,
            page: Rc::downgrade(page),
            model: Rc::downgrade(model),
            record,
            points,
            kind_rule,
            kind_table,
            kinds,
            rows,
            kind: RefCell::new(None),
            shown_points: Default::default(),
            shown_table: Default::default(),
            shown_kinds: Default::default(),
        });
        // The usual wait comes from the record of waits; a new read redraws.
        let weak = Rc::downgrade(&room);
        wait_record::watch(&room.root, move || {
            if let Some(room) = weak.upgrade() {
                room.update();
            }
        });
        room
    }

    /// Redraws from the feed and the model. The page calls it once a second and after each
    /// change.
    pub fn update(self: &Rc<Self>) {
        let (Some(model), Some(page)) = (self.model.upgrade(), self.page.upgrade()) else { return };
        let now = format::now();
        let coin = model.coin();
        let tip = model.state().status.as_ref().map(|s| s.blocks);
        let names = Namer::from_model(&model);
        let waits = wait_record::with(|r| r.history.as_ref().map(|h| h.waits.clone()));
        feed::with(|feed| {
            let context = Context { now, coin: &coin, tip, waits: waits.as_deref() };
            let points = logic::meaning(&feed.entries, feed.total_bytes, &feed.sightings, &feed.details, &context);
            self.show_points(points);

            let counts = logic::kind_counts(&feed.entries);
            self.kind_rule.set_visible(counts.len() > 1);
            self.kind_table.set_visible(counts.len() > 1);
            if counts.len() > 1 {
                let mut table = vec![vec![
                    "Kind".to_string(),
                    "Count".to_string(),
                    "Size".to_string(),
                    format!("Fees, {coin}"),
                    "Oldest".to_string(),
                ]];
                for kind in &counts {
                    table.push(vec![
                        kind.name.clone(),
                        kind.count.to_string(),
                        format::bytes(kind.bytes),
                        format::coins(kind.fees),
                        kind.oldest.map(|t| format!("{} ago", span(now - t as f64))).unwrap_or_else(|| "–".into()),
                    ]);
                }
                self.show_table(table);
            }

            // The kinds first, when there are many rows.
            let many = !feed.entries.is_empty() && feed.entries.len() > MANY_ROWS;
            let chosen = self.kind.borrow().clone();
            let plates: Vec<(String, usize, bool)> = if many {
                logic::kind_plates(&feed.entries)
                    .into_iter()
                    .map(|(name, count)| {
                        let on = chosen.as_deref() == Some(name.as_str());
                        (name, count, on)
                    })
                    .collect()
            } else {
                vec![]
            };
            self.show_kinds(plates);

            // The rows.
            let waiting: HashSet<&str> = feed.entries.iter().map(|e| e.txid.as_str()).collect();
            let shown: Vec<(usize, bool)> = logic::ordered(&feed.entries)
                .into_iter()
                .filter(|(index, _)| chosen.is_none() || logic::kind_of(&feed.entries[*index]) == chosen)
                .collect();
            let keys: Vec<String> = shown.iter().map(|(index, _)| feed.entries[*index].txid.clone()).collect();
            let rows = self.rows.set(&keys, |txid| {
                let row = MempoolRow::new(txid);
                row.root.set_tooltip_text(Some("Open this transaction."));
                let open = {
                    let (model, txid) = (Rc::downgrade(&model), txid.to_string());
                    move || {
                        if let Some(model) = model.upgrade() {
                            model.open(Route::Transaction { txid: txid.clone() });
                        }
                    }
                };
                let click = gtk::GestureClick::new();
                let action = open.clone();
                click.connect_released(move |gesture, presses, _, _| {
                    if presses == 1 {
                        gesture.set_state(gtk::EventSequenceState::Claimed);
                        action();
                    }
                });
                row.root.add_controller(click);
                page.stop(&row.root, Some(txid.to_string()), Some(txid.to_string()), open);
                (row.root.clone().upcast(), row)
            });
            for ((index, child), row) in shown.iter().zip(rows) {
                let entry = &feed.entries[*index];
                let tags = entry.classified.then_some(entry.tags.as_slice());
                let sentence = words::sentence(feed.details.get(&entry.txid), &names);
                let facts = logic::facts(entry, now, &coin, feed.details.get(&entry.txid), tip, &waiting);
                row.set(tags, sentence.as_deref(), &facts);
                row.root.set_margin_start(if *child { 22 } else { 0 });
            }
            self.rows.root.set_visible(!keys.is_empty());
        });
    }

    fn show_points(&self, points: Vec<logic::Point>) {
        if *self.shown_points.borrow() == points {
            return;
        }
        clear_grid(&self.points);
        for (row, point) in points.iter().enumerate() {
            let name = display::name_text(&point.name);
            name.set_valign(gtk::Align::Baseline);
            self.points.attach(&name, 0, row as i32, 1, 1);
            let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
            column.set_valign(gtk::Align::Baseline);
            column.set_baseline_child(0);
            let value = theme::label(font::FIGURE_SMALL, &point.value, "value");
            column.append(&value);
            if let Some(note) = &point.note {
                let label = page::paragraph(note, &["caption-text", "label"]);
                label.set_wrap_mode(gtk::pango::WrapMode::Word);
                column.append(&label);
            }
            column.set_hexpand(true);
            self.points.attach(&column, 1, row as i32, 1, 1);
        }
        *self.shown_points.borrow_mut() = points;
    }

    fn show_table(&self, table: Vec<Vec<String>>) {
        if *self.shown_table.borrow() == table {
            return;
        }
        clear_grid(&self.kind_table);
        for (row, cells) in table.iter().enumerate() {
            for (column, cell) in cells.iter().enumerate() {
                // Count, size and fees are at the right of their column.
                let trailing = (1..=3).contains(&column);
                let label = if row == 0 {
                    display::name_text(cell)
                } else {
                    let classes: &[&str] = match column {
                        0 => &["value"],
                        4 => &["label"],
                        _ => &["value", "tabular"],
                    };
                    page::text(cell, classes)
                };
                label.set_xalign(if trailing { 1.0 } else { 0.0 });
                label.set_halign(if trailing { gtk::Align::End } else { gtk::Align::Start });
                label.set_valign(gtk::Align::Baseline);
                self.kind_table.attach(&label, column as i32, row as i32, 1, 1);
            }
        }
        *self.shown_table.borrow_mut() = table;
    }

    fn show_kinds(self: &Rc<Self>, plates: Vec<(String, usize, bool)>) {
        if *self.shown_kinds.borrow() == plates {
            return;
        }
        self.kinds.remove_all();
        for (name, count, on) in &plates {
            let plate = display::plate(&format!("{name} {count}"), PlateFill::None);
            if *on {
                present_fill(&plate);
            }
            let button = gtk::Button::new();
            button.add_css_class("flat");
            button.add_css_class("mempool-plain");
            button.set_child(Some(&plate));
            button.set_tooltip_text(Some(if *on { "Show each kind." } else { "Show only this kind." }));
            let (weak, name) = (Rc::downgrade(self), name.clone());
            button.connect_clicked(move |_| {
                let Some(room) = weak.upgrade() else { return };
                let next = if room.kind.borrow().as_deref() == Some(name.as_str()) { None } else { Some(name.clone()) };
                *room.kind.borrow_mut() = next;
                room.update();
            });
            let child = gtk::FlowBoxChild::new();
            child.set_focusable(false);
            child.set_child(Some(&button));
            self.kinds.append(&child);
        }
        self.kinds.set_visible(!plates.is_empty());
        *self.shown_kinds.borrow_mut() = plates;
    }
}

fn clear_grid(grid: &gtk::Grid) {
    while let Some(child) = grid.first_child() {
        grid.remove(&child);
    }
}

/// A plate on a field of the present's colour (`Plate(fill: Tone.present)`). The shared
/// plate has no such fill, so the page gives it one with its own class.
fn present_fill(plate: &gtk::Label) {
    for class in ["fill-none", "fill-staked", "fill-mined", "fill-record", "fill-good", "fill-alert"] {
        plate.remove_css_class(class);
    }
    plate.add_css_class("mempool-fill-present");
}

/// The page's few styles that `style.css` does not have: the plate of the chosen kind, and
/// a key with no frame of its own (SwiftUI's `.buttonStyle(.plain)`).
pub fn install_style() {
    thread_local! {
        static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if DONE.with(|done| done.replace(true)) {
        return;
    }
    let Some(display) = gtk::gdk::Display::default() else { return };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&format!(
        ".plate.mempool-fill-present {{ background-color: {present}; border-color: {present}; color: {screen}; }}\n\
         button.mempool-plain {{ background: none; border: none; box-shadow: none; padding: 0; min-height: 0; min-width: 0; outline: none; }}\n",
        present = crate::theme::tone::PRESENT.css(),
        screen = crate::theme::tone::SCREEN.css(),
    ));
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
}
