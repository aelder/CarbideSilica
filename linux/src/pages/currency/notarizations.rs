//! Carbide's notarization browser (`NotarizationLanes.swift`).
//!
//! The first sentence gives the state of the link: the time since the last confirmed
//! notarization against the usual interval. The rhythm strip shows the earlier intervals
//! and the wait now on one scale. The graph shows the entries that the node has: the best
//! chain is the straight line, and a side branch is a second line. Below the graph, each
//! row gives the committed heights by system, the observer's block that includes it, and
//! the check of each foreign proof root on the owning chain's own source.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;

use super::graph::{self, Graph, GraphData};
use super::picture::{self, LinkState, NotarizationMark};
use super::state_seek::other_chain;
use crate::bridge::{self, api, api::Route};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, Card, Page};
use crate::shared::currency_stores::{height_text, known_name, observation_source, take_notarization_request};
use crate::theme::{tone, Color};
use crate::widgets::icons::{self, Icon};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, PlateFill};
use crate::widgets::{lamp, loader};

pub const TITLE: &str = "Notarizations";

#[derive(Default)]
struct Lanes {
    subject: Option<String>,
    perspectives: Vec<api::NotarizationPerspective>,
    active: usize,
    views: HashMap<usize, api::NotarizationBrowser>,
    loading: bool,
    failure: Option<String>,
    saved: Option<String>,
    show_json: bool,
    /// The earlier notarizations of each view, newest first.
    records: HashMap<usize, Vec<api::RecordPoint>>,
    selected: Option<u32>,
}

pub struct NotarizationCard {
    pub card: Card,
    model: Weak<AppModel>,
    page: Weak<Page>,
    currency: api::CurrencyDetail,
    /// The name or ID the currency page was opened with.
    route: String,
    data: RefCell<Lanes>,
    /// The currency, the connection and the chain: a change starts again.
    key: RefCell<Option<(String, u64, String)>>,
    run: Cell<u64>,
    body: gtk::Box,
    rows_holder: RefCell<Option<gtk::Box>>,
    graph: RefCell<Option<Rc<Graph>>>,
}

impl NotarizationCard {
    pub fn new(page: &Rc<Page>, currency: &api::CurrencyDetail, route: &str) -> Rc<Self> {
        let model = page.model.clone();
        let card = page.card(TITLE, None);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        card.append(&body);
        card.root.set_visible(false);
        let lanes = Rc::new(Self {
            card,
            model: Rc::downgrade(&model),
            page: Rc::downgrade(page),
            currency: currency.clone(),
            route: route.to_string(),
            data: RefCell::default(),
            key: RefCell::new(None),
            run: Cell::new(0),
            body,
            rows_holder: RefCell::new(None),
            graph: RefCell::new(None),
        });
        let weak = Rc::downgrade(&lanes);
        model.watch(&[Topic::Status, Topic::Connection], &lanes.card.root, move |_| {
            if let Some(lanes) = weak.upgrade() {
                lanes.start_if_due();
            }
        });
        let weak = Rc::downgrade(&lanes);
        model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &lanes.card.root, move |_| {
            if let Some(lanes) = weak.upgrade() {
                lanes.render();
            }
        });
        lanes.start_if_due();
        lanes
    }

    fn start_if_due(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let chain = model.state().status.as_ref().and_then(|s| s.chain_id.clone()).unwrap_or_default();
        let id = self.currency.currency_id.clone().unwrap_or_else(|| self.currency.name.clone());
        let key = (id, model.generation(), chain);
        if self.key.borrow().as_ref() == Some(&key) {
            return;
        }
        *self.key.borrow_mut() = Some(key);
        self.start();
    }

    /// The subject is the currency for a gateway or chain, and its parent for a currency
    /// such as a bridge whose parent carries the running series. Take the fuller record.
    fn start(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let (Some(explorer), Some(chain)) = (model.explorer(), model.state().status.as_ref().and_then(|s| s.chain_id.clone())) else {
            return;
        };
        let run = self.run.get() + 1;
        self.run.set(run);
        let currency = self.currency.clone();
        let requested = take_notarization_request(&[
            Some(self.route.as_str()),
            currency.currency_id.as_deref(),
            Some(currency.name.as_str()),
            currency.fully_qualified_name.as_deref(),
        ]);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let mut best: Option<(String, usize)> = None;
            for candidate in [currency.currency_id.clone(), currency.parent.clone()].into_iter().flatten() {
                if candidate == chain {
                    continue;
                }
                let (ex, observer, subject) = (explorer.clone(), chain.clone(), candidate.clone());
                let found = bridge::run(async move { ex.notarization_browser(observer, subject, None, false).await }).await;
                let Ok(found) = found else { continue };
                if found.load != api::BrowserLoad::Ready {
                    continue;
                }
                if found.rows.len() > best.as_ref().map(|b| b.1).unwrap_or(0) {
                    best = Some((candidate, found.rows.len()));
                }
            }
            let Some(lanes) = weak.upgrade() else { return };
            if lanes.run.get() != run {
                return;
            }
            // A view of this chain from another chain needs no rows here to be offered.
            let chosen = best.map(|b| b.0).or_else(|| requested.as_ref().and(currency.currency_id.clone()));
            let Some(chosen) = chosen else {
                lanes.data.borrow_mut().subject = None;
                lanes.render();
                return;
            };
            let is_currency = Some(&chosen) == currency.currency_id.as_ref();
            let perspectives = explorer.notarization_perspectives(
                chosen.clone(),
                is_currency.then(|| currency.name.clone()),
                if is_currency { currency.options } else { None },
                if is_currency { currency.system_id.clone() } else { None },
            );
            if let Some(model) = lanes.model.upgrade() {
                model.resolve_names(perspectives.iter().flat_map(|p| [p.observer_id.clone(), p.subject_id.clone()]).collect::<Vec<_>>());
            }
            {
                let mut data = lanes.data.borrow_mut();
                data.subject = Some(chosen);
                data.active = perspectives.iter().position(|p| Some(&p.observer_id) == requested.as_ref()).unwrap_or(0);
                data.perspectives = perspectives;
                data.views.clear();
                data.selected = None;
            }
            let active = lanes.data.borrow().active;
            lanes.load(active, true);
        });
    }

    fn load(self: &Rc<Self>, index: usize, force: bool) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(explorer) = model.explorer() else { return };
        let Some(perspective) = self.data.borrow().perspectives.get(index).cloned() else { return };
        if !force && self.data.borrow().views.contains_key(&index) {
            self.render();
            return;
        }
        {
            let mut data = self.data.borrow_mut();
            data.loading = true;
            data.failure = None;
        }
        self.render();
        let run = self.run.get();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let (ex, p) = (explorer.clone(), perspective.clone());
            let result = bridge::run(async move {
                ex.notarization_browser(p.observer_id, p.subject_id, Some(p.subject_name), true).await
            })
            .await;
            let mut record = Vec::new();
            if let Ok(view) = result.as_ref()
                && perspective.local
                && let Some(txid) = view.rows.iter().find(|r| r.confirmed).and_then(|r| r.txid.clone())
            {
                let (ex, subject) = (explorer.clone(), perspective.subject_id.clone());
                record = bridge::run(async move { ex.notarization_record(subject, txid).await }).await;
            }
            let Some(lanes) = weak.upgrade() else { return };
            if lanes.run.get() != run {
                return;
            }
            let mut systems = Vec::new();
            {
                let mut data = lanes.data.borrow_mut();
                data.loading = false;
                match result {
                    Ok(view) => {
                        systems = view.rows.iter().flat_map(|r| r.roots.iter().map(|root| root.system_id.clone())).collect();
                        data.views.insert(index, view);
                        data.records.insert(index, record);
                    }
                    Err(error) => data.failure = Some(bridge::describe(&error)),
                }
            }
            if let Some(model) = lanes.model.upgrade() {
                model.resolve_names(systems);
            }
            lanes.render();
        });
    }

    fn bookmark(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(explorer) = model.explorer() else { return };
        let Some(perspective) = self.data.borrow().perspectives.get(self.data.borrow().active).cloned() else { return };
        let saved = match explorer.save_bookmark(
            Some(perspective.observer_id.clone()),
            "notarizations".into(),
            perspective.subject_id.clone(),
            String::new(),
        ) {
            Ok(()) => format!(
                "The bookmark is saved with the {} perspective.",
                self.name(&model, &perspective.observer_id, &perspective.observer_name)
            ),
            Err(error) => bridge::describe(&error),
        };
        self.data.borrow_mut().saved = Some(saved);
        model.reload_bookmarks();
        self.render();
    }

    // MARK: Text

    fn name(&self, model: &AppModel, id: &str, fallback: &str) -> String {
        known_name(id, model, Some(fallback))
    }

    fn observer_tip(model: &AppModel, view: &api::NotarizationBrowser) -> Option<u64> {
        if view.perspective.local { model.state().status.as_ref().map(|s| s.blocks) } else { None }
    }

    fn other_name(&self, model: &AppModel, view: &api::NotarizationBrowser) -> String {
        if let Some(root) = view.rows.iter().flat_map(|r| r.roots.iter()).find(|r| r.foreign) {
            return known_name(&root.system_id, model, root.system_name.as_deref());
        }
        self.name(model, &view.perspective.subject_id, &view.perspective.subject_name)
    }

    fn blocks_since_confirmed(model: &AppModel, view: &api::NotarizationBrowser) -> Option<u64> {
        let row = view.rows.iter().find(|r| r.confirmed)?;
        let height = row.included_height.or_else(|| picture::observer_height(row))?;
        let tip = Self::observer_tip(model, view)?;
        (tip > height).then(|| tip - height)
    }

    fn sentence(&self, model: &AppModel, view: &api::NotarizationBrowser, marks: &[NotarizationMark], intervals: &[u64], state: Option<LinkState>) -> String {
        let observer = self.name(model, &view.perspective.observer_id, &view.perspective.observer_name);
        let other = self.other_name(model, view);
        let confirmed = view.rows.iter().find(|r| r.confirmed);
        let Some(height) = confirmed.and_then(|c| c.included_height.or_else(|| picture::observer_height(c))) else {
            return format!("The {observer} node returned no confirmed notarization of {other}. {}", picture::waiting(marks));
        };
        let mut text = state.map(|s| format!("{}. ", s.word())).unwrap_or_default();
        text += &format!("{observer} last confirmed a notarization of {other} at block {}", format::integer(height));
        if let Some(wait) = Self::blocks_since_confirmed(model, view) {
            text += &format!(", {} ago", picture::span(wait));
        }
        text += ".";
        if let (Some(usual), Some(longest)) = (picture::median(intervals), intervals.iter().max()) {
            let verb = if state == Some(LinkState::Current) { "is" } else { "was" };
            text += &format!(
                " The usual interval {verb} {} blocks, and the longest of the last {} {verb} {}.",
                format::integer(usual),
                intervals.len(),
                format::integer(*longest)
            );
        }
        format!("{text} {}", picture::waiting(marks))
    }

    /// The text below a mark of the graph.
    fn note(model: &AppModel, mark: &NotarizationMark, view: &api::NotarizationBrowser) -> String {
        let Some(row) = mark.newest() else { return String::new() };
        if mark.lane > 0
            && let Some(confirmed) = view.rows.iter().find(|r| r.confirmed)
            && picture::other_height(confirmed) == picture::other_height(row)
        {
            return if mark.rows.len() == 1 {
                "the confirmed root again".into()
            } else {
                format!("the confirmed root, {} times", mark.rows.len())
            };
        }
        let mut parts = Vec::new();
        if mark.confirmed() {
            parts.push("confirmed".to_string());
        }
        if mark.rows.len() > 1 {
            parts.push(format!("{} entries", mark.rows.len()));
        }
        match (Self::observer_tip(model, view), picture::observer_height(row)) {
            (Some(tip), Some(height)) if tip >= height => parts.push(format!("{} ago", picture::span(tip - height))),
            _ => {
                if let Some(time) = row.included_time {
                    parts.push(format::age(time, format::now()));
                }
            }
        }
        parts.join(", ")
    }

    fn caption(&self, model: &AppModel, view: &api::NotarizationBrowser) -> String {
        let checked: Vec<&api::ProofRootRow> = view.rows.iter().flat_map(|r| r.roots.iter()).filter(|r| r.foreign && r.check.is_some()).collect();
        let other = self.other_name(model, view);
        let lead = format!(
            "Each mark is one notarization with the two heights that it commits to. A mark with a number is that number of entries for one {other} block."
        );
        if checked.is_empty() {
            return format!("{lead} The app did not check {other} blocks. The rows say why.");
        }
        let active = checked.iter().filter(|r| r.check == Some(api::RootCheck::Active)).count();
        format!("{lead} {active} of {} {other} proof roots are active on the {other} source.", checked.len())
    }

    fn inclusion_text(row: &api::BrowserNotarization) -> String {
        let Some(height) = row.included_height else { return row.inclusion.clone() };
        let mut text = format!("at block {}", format::integer(height));
        if let Some(time) = row.included_time {
            text += &format!(", {} ({})", format::date(time), format::age(time, format::now()));
        }
        text
    }

    // MARK: Drawing

    fn render(self: &Rc<Self>) {
        clear(&self.body);
        *self.rows_holder.borrow_mut() = None;
        *self.graph.borrow_mut() = None;
        let (Some(model), Some(page)) = (self.model.upgrade(), self.page.upgrade()) else { return };
        let data = self.data.borrow();
        self.card.root.set_visible(data.subject.is_some());
        if data.subject.is_none() {
            return;
        }
        let view = data.views.get(&data.active).cloned();
        self.card.set_trailing(
            view.as_ref()
                .filter(|v| v.load == api::BrowserLoad::Ready)
                .map(|v| format!("{} in the current view", v.rows.len()))
                .as_deref(),
        );
        // The header.
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        if data.perspectives.len() > 1 {
            let titles: Vec<String> =
                data.perspectives.iter().map(|p| format!("{} view", self.name(&model, &p.observer_id, &p.observer_name))).collect();
            let titles: Vec<&str> = titles.iter().map(String::as_str).collect();
            let weak = Rc::downgrade(self);
            let segments = lamp::Segments::new(&titles, data.active, move |index| {
                let Some(lanes) = weak.upgrade() else { return };
                {
                    let mut data = lanes.data.borrow_mut();
                    data.active = index;
                    data.selected = None;
                }
                lanes.load(index, false);
            });
            header.append(&segments.root);
        }
        if let Some(perspective) = data.perspectives.get(data.active) {
            let words = page::text(
                &format!(
                    "The {} node asked about {}.",
                    self.name(&model, &perspective.observer_id, &perspective.observer_name),
                    self.name(&model, &perspective.subject_id, &perspective.subject_name)
                ),
                &["secondary"],
            );
            words.set_ellipsize(gtk::pango::EllipsizeMode::End);
            header.append(&words);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        header.append(&spacer);
        if data.loading {
            header.append(&loader::small());
        }
        let weak = Rc::downgrade(self);
        let refresh = lamp::button("Refresh", move || {
            if let Some(lanes) = weak.upgrade() {
                let active = lanes.data.borrow().active;
                lanes.load(active, true);
            }
        });
        refresh.set_sensitive(!data.loading);
        header.append(&refresh);
        let weak = Rc::downgrade(self);
        let save = lamp::button("Bookmark This View", move || {
            if let Some(lanes) = weak.upgrade() {
                lanes.bookmark();
            }
        });
        save.set_sensitive(data.perspectives.get(data.active).is_some());
        save.set_tooltip_text(Some("Saves a notarization bookmark with this perspective"));
        header.append(&save);
        self.body.append(&header);
        if let Some(saved) = data.saved.as_ref() {
            self.body.append(&page::paragraph(saved, &["callout", "secondary"]));
        }
        let records = data.records.get(&data.active).cloned().unwrap_or_default();
        let (loading, failure, show_json, selected) = (data.loading, data.failure.clone(), data.show_json, data.selected);
        drop(data);
        // The content.
        match view.as_ref() {
            Some(view) => self.content(&page, &model, view, &records, show_json, selected),
            None if loading => {
                let progress = loader::progress("Checking notarizations…");
                progress.set_size_request(-1, 80);
                self.body.append(&progress);
            }
            None => {}
        }
        if let Some(failure) = failure {
            self.body.append(&page::note(&failure, true));
        }
    }

    fn content(self: &Rc<Self>, page: &Rc<Page>, model: &Rc<AppModel>, view: &api::NotarizationBrowser, records: &[api::RecordPoint], show_json: bool, selected: Option<u32>) {
        if let Some(provenance) = view.provenance.as_ref() {
            self.body.append(&page::provenance_note(provenance));
            if provenance.retained {
                self.body.append(&page::text("The app does not check the blocks of a stored view.", &["caption-text", "secondary"]));
            }
        }
        match view.load {
            api::BrowserLoad::Unavailable => {
                self.body.append(&page::note(view.message.as_deref().unwrap_or("Notarization data unavailable"), true));
            }
            api::BrowserLoad::NoData => {
                self.body.append(&page::paragraph(view.message.as_deref().unwrap_or("No notarization data returned"), &["secondary"]));
            }
            api::BrowserLoad::Ready => self.ready(page, model, view, records, show_json, selected),
        }
    }

    fn ready(self: &Rc<Self>, page: &Rc<Page>, model: &Rc<AppModel>, view: &api::NotarizationBrowser, records: &[api::RecordPoint], show_json: bool, selected: Option<u32>) {
        let intervals = picture::intervals(records);
        let wait = Self::blocks_since_confirmed(model, view);
        let state = LinkState::new(wait, &intervals);
        let marks = picture::marks(view);
        let sentence = super::title3(&self.sentence(model, view, &marks, &intervals, state));
        sentence.set_selectable(true);
        self.body.append(&sentence);
        if let Some(state) = state {
            let rule = page::paragraph(&state.rule(), &["callout"]);
            rule.set_markup(&page::span(&state.rule(), state.color()));
            self.body.append(&rule);
        } else if view.perspective.local && view.rows.iter().any(|r| r.confirmed) {
            self.body.append(&page::paragraph(
                "The app has no record of earlier notarizations to compare the wait with.",
                &["callout", "secondary"],
            ));
        }
        if let Some(confirmed) = view.rows.iter().find(|r| r.confirmed) {
            self.body.append(&self.confirmed_line(page, model, confirmed, view));
        }
        let observer = self.name(model, &view.perspective.observer_id, &view.perspective.observer_name);
        let other = self.other_name(model, view);
        if !intervals.is_empty() || !view.rows.is_empty() {
            // The rhythm and the graph are one instrument: the scale of the rhythm starts
            // at the left edge of the lanes of the graph.
            let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
            if !intervals.is_empty() {
                let rhythm = graph::rhythm(intervals.clone(), wait, state);
                rhythm.set_margin_top(12);
                column.append(&rhythm);
                column.append(&padded_caption(&format!(
                    "Each dot is the number of {observer} blocks between two earlier notarizations in sequence, for the last {}. Each step to the right is ten times more.",
                    intervals.len()
                )));
            }
            if !intervals.is_empty() && !view.rows.is_empty() {
                column.append(&display::rule());
            }
            if !view.rows.is_empty() {
                let notes = marks.iter().map(|m| Self::note(model, m, view)).collect();
                let weak = Rc::downgrade(self);
                let graph = Rc::new(Graph::new(
                    GraphData { marks: marks.clone(), observer: observer.clone(), other: other.clone(), notes, selected },
                    move |chosen| {
                        if let Some(lanes) = weak.upgrade() {
                            lanes.data.borrow_mut().selected = chosen;
                            lanes.render_rows();
                        }
                    },
                ));
                column.append(&graph.root);
                column.append(&display::rule());
                column.append(&padded_caption(&self.caption(model, view)));
                *self.graph.borrow_mut() = Some(graph);
            }
            self.body.append(&display::panel(&column, 0));
        }
        if view.rows.is_empty() {
            self.body.append(&page::text("No notarizations in current view", &["secondary"]));
            return;
        }
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        self.body.append(&rows);
        *self.rows_holder.borrow_mut() = Some(rows);
        self.render_rows();
        if view.checks_limited {
            self.body.append(&page::note("Some proof roots were not checked. The app checks at most 160 blocks for one view.", true));
        }
        let json = view.json.clone();
        let weak = Rc::downgrade(self);
        self.body.append(&super::data_disclosure(
            show_json,
            |open| if open { "Hide" } else { "Show the notarization data from the node, without the rows" },
            &view.json,
            move || super::json_text(&json),
            move |open| {
                if let Some(lanes) = weak.upgrade() {
                    lanes.data.borrow_mut().show_json = open;
                }
            },
        ));
    }

    /// The rows, in the order of the graph, with the rows of the selected mark lit.
    fn render_rows(self: &Rc<Self>) {
        let Some(holder) = self.rows_holder.borrow().clone() else { return };
        clear(&holder);
        let (Some(model), Some(page)) = (self.model.upgrade(), self.page.upgrade()) else { return };
        let data = self.data.borrow();
        let Some(view) = data.views.get(&data.active).cloned() else { return };
        let selected = data.selected;
        drop(data);
        let marks = picture::marks(&view);
        let lit: HashSet<u32> = marks.iter().find(|m| Some(m.id()) == selected).map(|m| m.rows.iter().map(|r| r.index).collect()).unwrap_or_default();
        let groups = picture::ordered(&marks);
        for (lane, rows) in &groups {
            if groups.len() > 1 {
                let title = if *lane == 0 {
                    "Best chain".to_string()
                } else if groups.len() > 2 {
                    format!("Side branch {lane}")
                } else {
                    "Side branch".to_string()
                };
                let plate = display::plate(&title, if *lane == 0 { PlateFill::None } else { PlateFill::Mined });
                plate.set_margin_top(10);
                plate.set_margin_bottom(4);
                holder.append(&plate);
            }
            for row in rows {
                holder.append(&display::rule());
                let view_row = row_view(&page, &model, row, &view.perspective);
                view_row.set_margin_start(6);
                view_row.set_margin_end(6);
                // A lit row has a faint field of the present behind it.
                let cell = gtk::Grid::new();
                if lit.contains(&row.index) {
                    let field = Canvas::new(0, 0, |_, snapshot, w, h| draw_field(snapshot, w, h, tone::PRESENT.opacity(0.1)));
                    field.set_hexpand(true);
                    field.set_vexpand(true);
                    field.set_can_target(false);
                    cell.attach(&field, 0, 0, 1, 1);
                }
                view_row.set_hexpand(true);
                cell.attach(&view_row, 0, 0, 1, 1);
                holder.append(&cell);
            }
        }
    }

    fn confirmed_line(self: &Rc<Self>, page: &Rc<Page>, model: &Rc<AppModel>, row: &api::BrowserNotarization, view: &api::NotarizationBrowser) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let name = display::name_text("Last confirmed");
        name.set_size_request(170, -1);
        line.append(&name);
        if row.synthetic {
            line.append(&page::text("Synthetic proof root", &[]));
        } else if let Some(txid) = row.txid.as_ref() {
            line.append(&transaction_text(page, txid, view.perspective.local, false));
            line.append(&page::text(&format!("output {}", row.output.map(|o| o.to_string()).unwrap_or_else(|| "–".into())), &["secondary"]));
            line.append(&page::text(&format!("included {}", Self::inclusion_text(row)), &["secondary"]));
            if !view.perspective.local
                && let Some(note) = other_chain(model, &view.perspective.observer_id, Some(&view.perspective.observer_name))
            {
                line.append(&note);
            }
        } else {
            line.append(&page::text("transaction unavailable", &["secondary"]));
        }
        line
    }
}

fn draw_field(snapshot: &gtk::Snapshot, w: f32, h: f32, color: Color) {
    crate::theme::draw::fill_rect(snapshot, 0.0, 0.0, w, h, color);
}

/// A caption in a panel, 18 from its sides.
fn padded_caption(text: &str) -> gtk::Label {
    let label = page::paragraph(text, &["caption-text", "label"]);
    label.set_margin_start(18);
    label.set_margin_end(18);
    label.set_margin_top(10);
    label.set_margin_bottom(10);
    label
}

/// A transaction of the observer: a link on this chain, else a short ID that can be
/// selected, with the whole ID as its help.
fn transaction_text(page: &Rc<Page>, txid: &str, local: bool, callout: bool) -> gtk::Widget {
    if local {
        return page::route_link(page, txid, Route::Transaction { txid: txid.to_string() }, true).upcast();
    }
    let text = page::text(&format::short(txid, 10, 8), if callout { &["mono", "callout"] } else { &["mono"] });
    text.set_selectable(true);
    text.set_tooltip_text(Some(txid));
    text.upcast()
}

/// A name column of 150, as the rows of a notarization have.
fn row_name(text: &str) -> gtk::Label {
    let name = display::name_text(text);
    name.set_size_request(150, -1);
    name
}

/// One notarization: its status, output, committed heights by system, inclusion, and the
/// check of each foreign proof root (`NotarizationRowView`).
fn row_view(page: &Rc<Page>, model: &Rc<AppModel>, row: &api::BrowserNotarization, perspective: &api::NotarizationPerspective) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 5);
    column.set_margin_top(8);
    column.set_margin_bottom(8);
    let chain = model.state().status.as_ref().and_then(|s| s.chain_id.clone());
    // Status, transaction and output.
    let first = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let status = if row.synthetic {
        icons::label(Icon::RingDotted, "Synthetic proof root", tone::LABEL, &[])
    } else if row.confirmed {
        icons::label(Icon::Seal, "Confirmed", tone::GOOD, &[])
    } else {
        icons::label(Icon::RingDashed, "Unconfirmed", tone::MINED, &[])
    };
    status.set_size_request(150, -1);
    first.append(&status);
    if let Some(txid) = row.txid.as_ref() {
        first.append(&transaction_text(page, txid, perspective.local, false));
        first.append(&page::text(&format!("output {}", row.output.map(|o| o.to_string()).unwrap_or_else(|| "–".into())), &["secondary"]));
        if !perspective.local
            && let Some(note) = other_chain(model, &perspective.observer_id, Some(&perspective.observer_name))
        {
            first.append(&note);
        }
    } else {
        first.append(&page::text("–", &["secondary"]));
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    first.append(&spacer);
    if !row.on_best_chain {
        first.append(&icons::label(Icon::Branch, "On a side branch", tone::MINED, &["callout"]));
    }
    column.append(&first);
    // Committed heights by system.
    let heights = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    heights.append(&row_name("Proof roots"));
    let text = page::text(&heights_text(model, row, perspective, chain.as_deref()), &["tabular"]);
    text.set_wrap(true);
    heights.append(&text);
    column.append(&heights);
    // The observer's block that includes it.
    let included = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    included.append(&row_name("Included"));
    let when = |time: u64| page::text(&format!("{} ({})", format::date(time), format::age(time, format::now())), &["secondary"]);
    match (row.included_height, row.block_hash.as_ref()) {
        (Some(height), Some(hash)) if perspective.local => {
            included.append(&page::route_link(page, &height_text(height, row.included_time, model), Route::Block { id: hash.clone() }, false));
            if let Some(time) = row.included_time {
                included.append(&when(time));
            }
        }
        (Some(height), _) => {
            included.append(&page::text(
                &format!(
                    "{} block {}",
                    known_name(&perspective.observer_id, model, Some(&perspective.observer_name)),
                    format::integer(height)
                ),
                &[],
            ));
            if let Some(time) = row.included_time {
                included.append(&when(time));
            }
        }
        _ => included.append(&page::text(&row.inclusion, &["secondary"])),
    }
    column.append(&included);
    for root in row.roots.iter().filter(|r| r.foreign) {
        column.append(&root_line(page, model, root, chain.as_deref()));
    }
    if let Some(previous) = row.previous.as_ref() {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        line.append(&row_name("Previous"));
        line.append(&page::text(
            &format!("on {}", known_name(&previous.output.system_id, model, previous.system_name.as_deref())),
            &[],
        ));
        let id = page::text(&format!("{}:{}", format::short(&previous.output.txid, 10, 8), previous.output.output), &["mono", "callout"]);
        id.set_selectable(true);
        line.append(&id);
        line.append(&icons::label(
            if previous.found { Icon::CheckCircle } else { Icon::QuestionCircle },
            &previous.status,
            if previous.found { tone::GOOD } else { tone::MINED },
            &[],
        ));
        column.append(&line);
    }
    column
}

/// Committed heights by system: the observer, the subject, then any other roots.
fn heights_text(model: &Rc<AppModel>, row: &api::BrowserNotarization, perspective: &api::NotarizationPerspective, chain: Option<&str>) -> String {
    let mut systems = vec![perspective.observer_id.clone()];
    if perspective.subject_id != perspective.observer_id {
        systems.push(perspective.subject_id.clone());
    }
    systems.extend(row.roots.iter().map(|r| r.system_id.clone()));
    let mut seen = HashSet::new();
    systems
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .map(|id| {
            let root = row.roots.iter().find(|r| r.system_id == id);
            // Dates are known for the connected chain's heights only.
            let height = root
                .and_then(|r| r.height)
                .map(|h| if Some(id.as_str()) == chain { height_text(h, None, model) } else { format!("#{}", format::integer(h)) })
                .unwrap_or_else(|| "—".into());
            let fallback = root.and_then(|r| r.system_name.clone()).unwrap_or_else(|| {
                if id == perspective.observer_id { perspective.observer_name.clone() } else { perspective.subject_name.clone() }
            });
            format!("{} {height}", known_name(&id, model, Some(&fallback)))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn check_mark(check: &api::RootCheck) -> Icon {
    use api::RootCheck::*;
    match check {
        Active => Icon::CheckCircleFill,
        KnownOffChain | NotFound => Icon::WarningFill,
        HeightMismatch { .. } => Icon::ExclamationCircle,
        PresentMembershipUnknown => Icon::HalfFilled,
        SourceBehind { .. } => Icon::Clock,
        NoSourceConfigured | NoSourceEnabled => Icon::SlashCircle,
        ChainQueryFailed | BlockLookupFailed | InvalidHeader => Icon::CrossCircle,
        MissingHash | NotChecked => Icon::QuestionCircle,
    }
}

pub fn check_color(check: &api::RootCheck) -> Color {
    use api::RootCheck::*;
    match check {
        Active => tone::GOOD,
        KnownOffChain | NotFound | HeightMismatch { .. } => tone::ALERT,
        PresentMembershipUnknown | SourceBehind { .. } => tone::MINED,
        _ => tone::LABEL,
    }
}

fn root_line(page: &Rc<Page>, model: &Rc<AppModel>, root: &api::ProofRootRow, chain: Option<&str>) -> gtk::Box {
    let system = known_name(&root.system_id, model, root.system_name.as_deref());
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    line.append(&row_name(&format!("{system} root")));
    match (root.check.as_ref(), root.status.as_ref()) {
        (Some(check), Some(text)) => line.append(&icons::label(check_mark(check), text, check_color(check), &[])),
        _ => line.append(&page::text("Not checked. A Verus node cannot check an Ethereum block.", &["secondary"])),
    }
    if let Some(hash) = root.block_hash.as_ref() {
        if root.resolved && Some(root.system_id.as_str()) == chain {
            line.append(&page::route_link(page, hash, Route::Block { id: hash.clone() }, true));
        } else {
            let text = page::text(&format::short(hash, 10, 8), &["mono", "callout", "secondary"]);
            text.set_selectable(true);
            text.set_tooltip_text(Some(hash));
            line.append(&text);
            if root.resolved
                && let Some(note) = other_chain(model, &root.system_id, root.system_name.as_deref())
            {
                line.append(&note);
            }
        }
    }
    if let Some(source) = root.checked_on.as_ref() {
        line.append(&page::text(&format!("checked on {}", observation_source(source)), &["caption-text", "secondary"]));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_have_marks_and_colours() {
        assert_eq!(check_mark(&api::RootCheck::Active), Icon::CheckCircleFill);
        assert_eq!(check_mark(&api::RootCheck::SourceBehind { tip: 3 }), Icon::Clock);
        assert_eq!(check_color(&api::RootCheck::HeightMismatch { found: 1 }), tone::ALERT);
        assert_eq!(check_color(&api::RootCheck::NoSourceEnabled), tone::LABEL);
        assert_eq!(check_color(&api::RootCheck::PresentMembershipUnknown), tone::MINED);
    }
}
