//! A currency's supply and the value of its reserves across time, with one cursor through
//! all of them (`CurrencyOverTime.swift`).
//!
//! The node gives the price of a basket in each reserve. A line of that price goes up when
//! the reserve loses value, which a person reads the other way. Thus the card shows the
//! basket and each reserve in one unit that the person selects, as lines on one scale that
//! start at 100. Dragging the slider moves every figure together. Each point is the
//! currency's recorded state at a real block.
//!
//! The request for the states can make the node slow (see `currency_history` in the
//! bridge): the card asks for 90 samples over the chosen span, once for each span and
//! connection, and stops the request when the span changes or the page closes.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use chrono::{Local, TimeZone};
use gtk::glib;
use gtk::prelude::*;

use super::{basket, pool_batch};
use crate::bridge::{self, api};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, Card, Page};
use crate::shared::currency_stores::known_name;
use crate::theme::{self, draw, font, tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;
use crate::widgets::icons::{self, Icon};
use crate::widgets::{lamp, loader};

// MARK: Spans and samples

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    Month,
    Quarter,
    Year,
    All,
}

pub const SPANS: [Span; 4] = [Span::Month, Span::Quarter, Span::Year, Span::All];

impl Span {
    pub fn title(self) -> &'static str {
        match self {
            Span::Month => "30 days",
            Span::Quarter => "90 days",
            Span::Year => "1 year",
            Span::All => "All",
        }
    }

    /// Blocks back from the tip, at Verus's one-minute block target.
    pub fn blocks(self) -> Option<u64> {
        match self {
            Span::Month => Some(30 * 1440),
            Span::Quarter => Some(90 * 1440),
            Span::Year => Some(365 * 1440),
            Span::All => None,
        }
    }
}

pub const SAMPLES: u32 = 90;

/// The first height of the range: the span back from the tip, not before the currency's
/// start block.
pub fn from_height(tip: u64, start_block: Option<u64>, span: Span) -> u64 {
    let start = start_block.unwrap_or(1).max(1);
    match span.blocks() {
        Some(blocks) if tip > blocks => (tip - blocks).max(start),
        _ => start,
    }
}

// MARK: The lines from 100

pub struct IndexLine {
    pub id: String,
    pub name: String,
    pub basket: bool,
    pub color: Color,
    /// The value in the unit at each sample, as a part of the first known value, times 100.
    pub values: Vec<Option<f64>>,
}

fn price(point: &api::CurrencyPoint, id: &str) -> Option<f64> {
    basket::positive(point.reserves.iter().find(|r| r.currency_id == id).and_then(|r| r.price_in_reserve))
}

/// Each value as a part of the first known value, times 100.
pub fn rebased(raw: &[Option<f64>]) -> Vec<Option<f64>> {
    match raw.iter().flatten().next().copied() {
        Some(base) if base > 0.0 => raw.iter().map(|v| v.map(|v| v / base * 100.0)).collect(),
        _ => raw.iter().map(|_| None).collect(),
    }
}

/// The reserves of the latest sample.
pub fn reserve_ids(points: &[api::CurrencyPoint]) -> Vec<String> {
    points.last().map(|p| p.reserves.iter().map(|r| r.currency_id.clone()).collect()).unwrap_or_default()
}

/// The basket in the unit, then each other reserve in the unit. The colours of the
/// reserves are the series without its last colour, which is near the colour of the basket
/// line.
pub fn index_lines(points: &[api::CurrencyPoint], unit: &str, basket_name: &str, name: &dyn Fn(&str) -> String) -> Vec<IndexLine> {
    let palette = &tone::SERIES[..tone::SERIES.len() - 1];
    let mut lines = vec![IndexLine {
        id: "basket".into(),
        name: basket_name.to_string(),
        basket: true,
        color: tone::PRESENT,
        values: rebased(&points.iter().map(|p| price(p, unit)).collect::<Vec<_>>()),
    }];
    for (position, id) in reserve_ids(points).iter().enumerate() {
        if id == unit {
            continue;
        }
        let raw: Vec<Option<f64>> = points.iter().map(|p| basket::worth(price(p, id), price(p, unit))).collect();
        lines.push(IndexLine {
            id: id.clone(),
            name: name(id),
            basket: false,
            color: palette[position % palette.len()],
            values: rebased(&raw),
        });
    }
    lines
}

/// The scale of the lines from 100: it holds 95 to 105 at least, with 3 more each way.
pub fn index_domain(lines: &[IndexLine]) -> (f64, f64) {
    let known: Vec<f64> = lines.iter().flat_map(|l| l.values.iter().flatten().copied()).collect();
    let low = known.iter().copied().fold(f64::INFINITY, f64::min);
    let high = known.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let low = if known.is_empty() { 90.0 } else { low }.min(95.0);
    let high = if known.is_empty() { 110.0 } else { high }.max(105.0);
    (low - 3.0, high + 3.0)
}

/// The scale of one row: the known values with an eighth more above and below.
pub fn row_domain(values: &[Option<f64>]) -> (f64, f64) {
    let known: Vec<f64> = values.iter().flatten().copied().collect();
    let low = if known.is_empty() { 0.0 } else { known.iter().copied().fold(f64::INFINITY, f64::min) };
    let high = if known.is_empty() { 1.0 } else { known.iter().copied().fold(f64::NEG_INFINITY, f64::max) };
    let pad = if high > low { (high - low) * 0.12 } else { (high.abs() * 0.05).max(0.0001) };
    (low - pad, high + pad)
}

/// A number with one decimal and grouping: "1,234.5".
fn fixed1(value: f64) -> String {
    let text = format!("{value:.1}");
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, "0"));
    format!("{}.{fraction}", format::group(whole))
}

/// The change from 100 at the cursor: "+93.9" or "−20.0".
pub fn change_text(value: f64) -> String {
    let change = value - 100.0;
    format!("{}{}", if change >= 0.0 { "+" } else { "−" }, fixed1(change.abs()))
}

/// The change from the value at the cursor to the latest value: "+12.5% since", or
/// "latest" at the latest sample.
pub fn change_since(value: f64, latest: Option<f64>, is_latest: bool) -> String {
    match latest {
        Some(latest) if !is_latest && value != 0.0 => {
            let percent = (latest - value) / value;
            format!("{}{}% since", if percent >= 0.0 { "+" } else { "" }, fixed1(percent * 100.0))
        }
        _ => "latest".into(),
    }
}

/// A sample counts as activity when the latest batch differs from the previous sample's.
pub fn changed(points: &[api::CurrencyPoint]) -> Vec<bool> {
    let marks: Vec<(Vec<(f64, f64)>, f64)> = points
        .iter()
        .map(|p| (p.reserves.iter().map(|r| (r.last_in, r.last_out)).collect(), p.supply.unwrap_or(0.0)))
        .collect();
    (0..marks.len()).map(|i| i > 0 && marks[i] != marks[i - 1]).collect()
}

/// The time of the first sample with the latest supply, when nothing has changed for the
/// latter part of the range, which a flat line alone leaves ambiguous.
pub fn flat_since(points: &[api::CurrencyPoint]) -> Option<u64> {
    let last_supply = points.last()?.supply?;
    let first_same = points.iter().position(|p| p.supply == Some(last_supply))?;
    if first_same + 3 >= points.len() || first_same == 0 {
        return None;
    }
    points[first_same].time
}

/// "Nov 26, 2026", as `Date.formatted(date: .abbreviated, time: .omitted)`.
pub fn day(unix: u64) -> String {
    Local.timestamp_opt(unix as i64, 0).single().map(|t| t.format("%b %-d, %Y").to_string()).unwrap_or_default()
}

/// Values for the grid of the scale: about five round numbers.
pub fn ticks(low: f64, high: f64) -> Vec<f64> {
    if !(high > low) {
        return vec![];
    }
    let rough = (high - low) / 5.0;
    let magnitude = 10f64.powf(rough.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * magnitude).find(|s| *s >= rough).unwrap_or(10.0 * magnitude);
    let mut value = (low / step).ceil() * step;
    let mut ticks = Vec::new();
    while value <= high + step * 1e-9 {
        ticks.push(value);
        value += step;
    }
    ticks
}

// MARK: The card

struct Loaded {
    points: Vec<api::CurrencyPoint>,
    slow_seconds: Option<f64>,
    complete: bool,
}

pub struct OverTime {
    pub card: Card,
    model: Weak<AppModel>,
    currency: api::CurrencyDetail,
    unit: Rc<dyn Fn() -> Option<String>>,
    on_choose: Rc<dyn Fn(u64)>,
    span: Cell<Span>,
    loaded: RefCell<Loaded>,
    failure: RefCell<Option<String>>,
    loading: Cell<bool>,
    index: Rc<Cell<usize>>,
    /// The span, the connection and whether the tip is known: a change loads again.
    key: RefCell<Option<(Span, u64, bool)>>,
    run: Cell<u64>,
    request: RefCell<Option<tokio::task::AbortHandle>>,
    // The header.
    point_box: gtk::Box,
    date_label: gtk::Label,
    block_label: gtk::Label,
    choose_button: gtk::Button,
    body: gtk::Box,
    /// Called when the cursor moves.
    cursor_moved: RefCell<Vec<Box<dyn Fn(usize)>>>,
}

impl Drop for OverTime {
    fn drop(&mut self) {
        if let Some(request) = self.request.borrow_mut().take() {
            request.abort();
        }
    }
}

impl OverTime {
    pub fn new(
        page: &Rc<Page>,
        currency: &api::CurrencyDetail,
        unit: Rc<dyn Fn() -> Option<String>>,
        on_choose: Rc<dyn Fn(u64)>,
    ) -> Rc<Self> {
        let model = page.model.clone();
        let card = page.card("Over time", Some(&format!("{SAMPLES} samples")));
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let point_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
        point_box.set_halign(gtk::Align::End);
        point_box.set_valign(gtk::Align::Center);
        let date_label = theme::label(font::FIGURE_SMALL, "", "present");
        date_label.set_xalign(1.0);
        let block_label = page::text("", &["caption-text", "tabular", "label"]);
        block_label.set_xalign(1.0);
        point_box.append(&date_label);
        point_box.append(&block_label);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let card_ref = Rc::new_cyclic(|weak: &Weak<OverTime>| {
            let segments = {
                let weak = weak.clone();
                let titles: Vec<&str> = SPANS.iter().map(|s| s.title()).collect();
                lamp::Segments::new(&titles, 2, move |index| {
                    if let Some(card) = weak.upgrade() {
                        card.span.set(SPANS[index]);
                        card.reload_if_due();
                    }
                })
            };
            header.append(&segments.root);
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_hexpand(true);
            header.append(&spacer);
            header.append(&point_box);
            let choose_button = {
                let weak = weak.clone();
                lamp::button("Show Exact State", move || {
                    if let Some(card) = weak.upgrade()
                        && let Some(point) = card.selected()
                    {
                        (card.on_choose)(point.height);
                    }
                })
            };
            header.append(&choose_button);
            Self {
                card,
                model: Rc::downgrade(&model),
                currency: currency.clone(),
                unit,
                on_choose,
                span: Cell::new(Span::Year),
                loaded: RefCell::new(Loaded { points: vec![], slow_seconds: None, complete: true }),
                failure: RefCell::new(None),
                loading: Cell::new(true),
                index: Rc::new(Cell::new(0)),
                key: RefCell::new(None),
                run: Cell::new(0),
                request: RefCell::new(None),
                point_box: point_box.clone(),
                date_label,
                block_label,
                choose_button,
                body: body.clone(),
                cursor_moved: RefCell::new(Vec::new()),
            }
        });
        card_ref.card.append(&header);
        card_ref.card.append(&body);
        card_ref.render();
        card_ref.update_header();
        // Load when the span, the connection or the knowledge of the tip changes.
        let weak = Rc::downgrade(&card_ref);
        model.watch(&[Topic::Status, Topic::Connection], &card_ref.card.root, move |_| {
            if let Some(card) = weak.upgrade() {
                card.reload_if_due();
            }
        });
        let weak = Rc::downgrade(&card_ref);
        model.watch(&[Topic::Names, Topic::Display], &card_ref.card.root, move |_| {
            if let Some(card) = weak.upgrade() {
                card.render();
                card.update_header();
            }
        });
        card_ref.reload_if_due();
        card_ref
    }

    fn selected(&self) -> Option<api::CurrencyPoint> {
        let loaded = self.loaded.borrow();
        if loaded.points.is_empty() {
            return None;
        }
        Some(loaded.points[self.index.get().min(loaded.points.len() - 1)].clone())
    }

    fn reserve_name(&self, model: &AppModel, id: &str) -> String {
        // A currency's name is its identity's name without the trailing "@". With names
        // off in the View menu, the ID.
        if !crate::display::shows_names() {
            return format::short(id, 6, 4);
        }
        self.currency
            .reserves
            .iter()
            .find(|r| r.currency_id == id)
            .and_then(|r| r.name.clone())
            .unwrap_or_else(|| known_name(id, model, None))
    }

    fn date_text(&self, point: &api::CurrencyPoint) -> String {
        let Some(time) = point.time else { return format!("Block {}", format::integer(point.height)) };
        let loaded = self.loaded.borrow();
        let date = day(time);
        if self.index.get() + 1 == loaded.points.len() && loaded.complete { format!("{date} · latest") } else { date }
    }

    /// The date and block at the cursor, and Show Exact State.
    fn update_header(&self) {
        match self.selected() {
            Some(point) => {
                theme::set_label(&self.date_label, font::FIGURE_SMALL, &self.date_text(&point));
                self.block_label.set_text(&format!("block {}", format::integer(point.height)));
                self.choose_button.set_tooltip_text(Some(&format!(
                    "Read the state at block {} and check its imports and exports",
                    format::integer(point.height)
                )));
                self.point_box.set_visible(true);
                self.choose_button.set_visible(true);
            }
            None => {
                self.point_box.set_visible(false);
                self.choose_button.set_visible(false);
            }
        }
    }

    fn reload_if_due(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let has_tip = model.state().status.as_ref().is_some_and(|s| s.blocks > 0);
        let key = (self.span.get(), model.generation(), has_tip);
        if self.key.borrow().as_ref() == Some(&key) {
            return;
        }
        *self.key.borrow_mut() = Some(key);
        self.load();
    }

    fn load(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(tip) = model.state().status.as_ref().map(|s| s.blocks).filter(|b| *b > 0) else { return };
        // A new request stops the one before it.
        if let Some(request) = self.request.borrow_mut().take() {
            request.abort();
        }
        let run = self.run.get() + 1;
        self.run.set(run);
        self.loading.set(true);
        *self.failure.borrow_mut() = None;
        self.render();
        let from = from_height(tip, self.currency.start_block, self.span.get());
        let name = self.currency.currency_id.clone().unwrap_or_else(|| self.currency.name.clone());
        let explorer = match model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                self.finish(Err(message));
                return;
            }
        };
        let handle = bridge::runtime().spawn(async move { explorer.currency_history(name, from, tip, SAMPLES).await });
        *self.request.borrow_mut() = Some(handle.abort_handle());
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = handle.await;
            let Some(card) = weak.upgrade() else { return };
            if card.run.get() != run {
                return;
            }
            card.request.borrow_mut().take();
            match result {
                Ok(Ok(history)) => card.finish(Ok(history)),
                Ok(Err(error)) => card.finish(Err(bridge::describe(&error))),
                // A stopped request leaves the card as it is, as a cancelled task does.
                Err(error) if error.is_cancelled() => {}
                Err(error) => std::panic::resume_unwind(error.into_panic()),
            }
        });
    }

    fn finish(self: &Rc<Self>, result: Result<api::CurrencyHistory, String>) {
        match result {
            Ok(history) => {
                self.index.set(history.points.len().saturating_sub(1));
                *self.loaded.borrow_mut() = Loaded { points: history.points, slow_seconds: history.slow_seconds, complete: history.complete };
                if let Some(model) = self.model.upgrade() {
                    model.resolve_names(reserve_ids(&self.loaded.borrow().points));
                }
            }
            Err(message) => {
                *self.loaded.borrow_mut() = Loaded { points: vec![], slow_seconds: None, complete: true };
                *self.failure.borrow_mut() = Some(message);
            }
        }
        self.loading.set(false);
        self.render();
        self.update_header();
    }

    /// The note when the node was slow.
    fn limit_note(&self) -> Option<gtk::Box> {
        let loaded = self.loaded.borrow();
        if let Some(seconds) = loaded.slow_seconds {
            return Some(page::note(
                &format!(
                    "The node took approximately {} seconds for one state of {}, and it answers no other request during that time. The app did not ask for the other {} states. \"State at a chosen block\" gives one state.",
                    seconds.round() as i64,
                    self.currency.name,
                    SAMPLES as i64 - loaded.points.len() as i64
                ),
                true,
            ));
        }
        if !loaded.complete && let Some(last) = loaded.points.last() {
            return Some(page::note(
                &format!(
                    "The node was slow. The values stop at block {}, before the end of the range.",
                    format::integer(last.height)
                ),
                true,
            ));
        }
        None
    }

    /// Builds the body again from the state: after a load, a new unit or new names.
    pub fn render(self: &Rc<Self>) {
        clear(&self.body);
        self.cursor_moved.borrow_mut().clear();
        let Some(model) = self.model.upgrade() else { return };
        if self.loading.get() {
            let progress = loader::progress("");
            progress.set_size_request(-1, 120);
            progress.set_hexpand(true);
            self.body.append(&progress);
            return;
        }
        if let Some(failure) = self.failure.borrow().clone() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&icons::icon(Icon::Warning, 14, tone::SECONDARY));
            row.append(&page::paragraph(&failure, &["secondary"]));
            self.body.append(&row);
            return;
        }
        let (count, slow) = {
            let loaded = self.loaded.borrow();
            (loaded.points.len(), loaded.slow_seconds.is_some())
        };
        if count < 2 || slow {
            if let Some(note) = self.limit_note() {
                self.body.append(&note);
            }
            if !slow {
                self.body.append(&page::text("The range contains no recorded data.", &["secondary"]));
            }
            return;
        }
        if let Some(note) = self.limit_note() {
            self.body.append(&note);
        }
        let points = Rc::new(self.loaded.borrow().points.clone());
        let weak = Rc::downgrade(self);
        let slider = lamp::Slider::new(self.index.get() as f64, 0.0, (count - 1) as f64, move |value| {
            if let Some(card) = weak.upgrade() {
                card.move_cursor(value.round().max(0.0) as usize);
            }
        });
        slider.canvas.update_property(&[gtk::accessible::Property::Label("Date")]);
        self.body.append(&slider.canvas);

        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let supply: Rc<Vec<Option<f64>>> = Rc::new(points.iter().map(|p| p.supply).collect());
        rows.append(&self.value_row("Supply", &self.currency.name, supply, 2));
        let unit = (self.unit)();
        if let Some(unit) = unit.as_ref().filter(|u| reserve_ids(&points).contains(u)) {
            rows.append(&display::rule());
            let name = |id: &str| self.reserve_name(&model, id);
            let lines = Rc::new(index_lines(&points, unit, &self.currency.name, &name));
            rows.append(&self.indexed(&lines, &self.reserve_name(&model, unit)));
        }
        rows.append(&display::rule());
        rows.append(&self.activity_row(&points));
        if let Some(time) = flat_since(&points) {
            rows.append(&display::rule());
            let text = page::paragraph(
                &format!(
                    "The supply has not changed since approximately {}. The sampled blocks after that date show no conversions.",
                    day(time)
                ),
                &["caption-text", "label"],
            );
            text.set_margin_top(8);
            rows.append(&text);
        }
        self.body.append(&display::panel(&rows, 16));

        // The last conversion batch at the cursor.
        let batch = gtk::Box::new(gtk::Orientation::Vertical, 0);
        self.body.append(&batch);
        let fill_batch = {
            let (weak, batch, points) = (Rc::downgrade(self), batch.clone(), points.clone());
            move |index: usize| {
                let (Some(card), Some(model)) = (weak.upgrade(), weak.upgrade().and_then(|c| c.model.upgrade())) else { return };
                clear(&batch);
                let point = &points[index.min(points.len() - 1)];
                if point.reserves.is_empty() {
                    return;
                }
                let unit = (card.unit)();
                let name = |id: &str| card.reserve_name(&model, id);
                batch.append(&pool_batch::pool_batch(point, unit.as_deref(), &name));
            }
        };
        fill_batch(self.index.get());
        self.cursor_moved.borrow_mut().push(Box::new(fill_batch));
    }

    fn move_cursor(&self, index: usize) {
        if index == self.index.get() {
            return;
        }
        self.index.set(index);
        for moved in self.cursor_moved.borrow().iter() {
            moved(index);
        }
        self.update_header();
    }

    /// A column of 130 at the left of a row: a name and a line below it.
    fn left_column(lines: &[gtk::Widget]) -> gtk::Box {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 3);
        column.set_size_request(130, -1);
        column.set_hexpand(false);
        column.set_valign(gtk::Align::Start);
        for line in lines {
            column.append(line);
        }
        column
    }

    /// One row: the name, a line of the values with the cursor, and the value at the cursor.
    fn value_row(&self, title: &str, subtitle: &str, values: Rc<Vec<Option<f64>>>, digits: usize) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        row.set_margin_top(7);
        row.set_margin_bottom(7);
        let name = display::name_text(title);
        let sub = page::text(subtitle, &["caption-text", "label"]);
        sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
        sub.set_max_width_chars(1);
        sub.set_hexpand(true);
        let left = Self::left_column(&[name.upcast(), sub.upcast()]);
        left.set_valign(gtk::Align::Center);
        row.append(&left);
        let (low, high) = row_domain(&values);
        let index = self.index.clone();
        let line_values = values.clone();
        let chart = Canvas::new(0, 42, move |_, snapshot, w, h| {
            let count = line_values.len();
            let x = |i: usize| w * i as f32 / (count.max(2) - 1) as f32;
            let y = |v: f64| h - ((v - low) / (high - low)) as f32 * h;
            let builder = gtk::gsk::PathBuilder::new();
            let mut first = true;
            for (i, value) in line_values.iter().enumerate() {
                let Some(value) = value else { continue };
                if first { builder.move_to(x(i), y(*value)) } else { builder.line_to(x(i), y(*value)) }
                first = false;
            }
            if !first {
                draw::stroke(snapshot, &builder.to_path(), tone::RECORD, 1.2);
            }
            let at = x(index.get().min(count.saturating_sub(1)));
            draw::rule(snapshot, at, 0.0, at, h, tone::PRESENT, 1.0, &[]);
        });
        chart.set_hexpand(true);
        row.append(&chart);
        let right = gtk::Box::new(gtk::Orientation::Vertical, 1);
        right.set_size_request(130, -1);
        right.set_hexpand(false);
        right.set_valign(gtk::Align::Center);
        let figure = theme::label(font::FIGURE_SMALL, "", "value");
        figure.set_xalign(1.0);
        let change = page::text("", &["caption-text", "tabular", "label"]);
        change.set_xalign(1.0);
        right.append(&figure);
        right.append(&change);
        row.append(&right);
        let show = {
            let (chart, figure, change, values) = (chart.downgrade(), figure.clone(), change.clone(), values.clone());
            move |index: usize| {
                let position = index.min(values.len().saturating_sub(1));
                match values.get(position).copied().flatten() {
                    Some(value) => {
                        theme::set_label(&figure, font::FIGURE_SMALL, &format::decimal(value, digits));
                        figure.remove_css_class("label");
                        let latest = values.last().copied().flatten();
                        change.set_text(&change_since(value, latest, position + 1 == values.len()));
                        change.set_visible(true);
                    }
                    None => {
                        figure.set_text("–");
                        figure.add_css_class("label");
                        change.set_visible(false);
                    }
                }
                if let Some(chart) = chart.upgrade() {
                    chart.queue_draw();
                }
            }
        };
        show(self.index.get());
        self.cursor_moved.borrow_mut().push(Box::new(show));
        row
    }

    /// The basket and each reserve in the unit, from 100 at the start of the range.
    fn indexed(&self, lines: &Rc<Vec<IndexLine>>, unit_name: &str) -> gtk::Box {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        column.set_margin_top(7);
        column.set_margin_bottom(7);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let in_unit = page::text(&format!("in {unit_name}"), &["caption-text", "label"]);
        in_unit.set_ellipsize(gtk::pango::EllipsizeMode::End);
        in_unit.set_max_width_chars(1);
        in_unit.set_hexpand(true);
        let left = Self::left_column(&[
            display::name_text("Value").upcast(),
            in_unit.upcast(),
            page::text("start of range = 100", &["caption-text", "label"]).upcast(),
        ]);
        row.append(&left);
        let (low, high) = index_domain(lines);
        let (index, chart_lines) = (self.index.clone(), lines.clone());
        let count = lines.first().map(|l| l.values.len()).unwrap_or(0);
        let chart = Canvas::new(0, 220, move |widget, snapshot, w, h| {
            let x = |i: usize| w * i as f32 / (count.max(2) - 1) as f32;
            let y = |v: f64| h - ((v - low) / (high - low)) as f32 * h;
            // The grid and its labels are inside the plot, so that the plot has the width
            // of the lines above and below it and one cursor goes through all of them.
            for tick in ticks(low, high) {
                let at = y(tick);
                draw::rule(snapshot, 0.0, at, w, at, tone::GRID, 1.0, &[]);
                draw::text(snapshot, widget, font::SMALL, &format::decimal(tick, 2), 4.0, at - 2.0, draw::Anchor::BottomLeading, tone::LABEL);
            }
            draw::rule(snapshot, 0.0, y(100.0), w, y(100.0), tone::AXIS, 1.0, &[3.0, 3.0]);
            for line in chart_lines.iter() {
                let builder = gtk::gsk::PathBuilder::new();
                let mut first = true;
                for (i, value) in line.values.iter().enumerate() {
                    let Some(value) = value else { continue };
                    if first { builder.move_to(x(i), y(*value)) } else { builder.line_to(x(i), y(*value)) }
                    first = false;
                }
                if !first {
                    let stroke = gtk::gsk::Stroke::new(if line.basket { 2.0 } else { 1.2 });
                    stroke.set_line_join(gtk::gsk::LineJoin::Round);
                    snapshot.append_stroke(&builder.to_path(), &stroke, &line.color.rgba());
                }
            }
            let at = x(index.get().min(count.saturating_sub(1)));
            draw::rule(snapshot, at, 0.0, at, h, tone::PRESENT, 1.0, &[]);
        });
        chart.set_hexpand(true);
        chart.update_property(&[gtk::accessible::Property::Label(&format!(
            "The value of {} and of each reserve in {unit_name}, from 100 at the start",
            self.currency.name
        ))]);
        row.append(&chart);
        let legend = gtk::Box::new(gtk::Orientation::Vertical, 5);
        legend.set_size_request(130, -1);
        legend.set_hexpand(false);
        legend.set_valign(gtk::Align::Start);
        let mut figures = Vec::new();
        for line in lines.iter() {
            let entry = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let size = if line.basket { 9 } else { 7 };
            let color = line.color;
            let square = Canvas::new(size, size, move |_, snapshot, w, h| draw::fill_rect(snapshot, 0.0, 0.0, w, h, color));
            square.set_valign(gtk::Align::Center);
            entry.append(&square);
            let name = display::name_text(&line.name);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_hexpand(true);
            name.set_max_width_chars(1);
            entry.append(&name);
            let figure = page::text("", &["caption-text", "tabular", if line.basket { "present" } else { "value" }]);
            figure.set_xalign(1.0);
            figure.set_margin_start(4);
            entry.append(&figure);
            figures.push(figure);
            legend.append(&entry);
        }
        legend.append(&display::name_text("at the cursor"));
        row.append(&legend);
        column.append(&row);
        column.append(&page::paragraph(
            &format!(
                "A line at 80 means that the currency lost 20 of each 100 of its value in {unit_name} since the start of the range. The heavy line is {}. The values are the pool's own rates.",
                self.currency.name
            ),
            &["caption-text", "label"],
        ));
        let show = {
            let (chart, lines) = (chart.downgrade(), lines.clone());
            move |index: usize| {
                for (line, figure) in lines.iter().zip(figures.iter()) {
                    let position = index.min(line.values.len().saturating_sub(1));
                    figure.set_text(&line.values.get(position).copied().flatten().map(change_text).unwrap_or_else(|| "–".into()));
                }
                if let Some(chart) = chart.upgrade() {
                    chart.queue_draw();
                }
            }
        };
        show(self.index.get());
        self.cursor_moved.borrow_mut().push(Box::new(show));
        column
    }

    /// Marks where conversions happened between samples, on the same axis as the traces.
    fn activity_row(&self, points: &[api::CurrencyPoint]) -> gtk::Box {
        let flags = Rc::new(changed(points));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        row.set_margin_top(7);
        row.set_margin_bottom(7);
        let left = Self::left_column(&[
            display::name_text("Activity").upcast(),
            page::text("conversions found", &["caption-text", "label"]).upcast(),
        ]);
        left.set_valign(gtk::Align::Center);
        row.append(&left);
        let (index, marks) = (self.index.clone(), flags.clone());
        let canvas = Canvas::new(0, 30, move |_, snapshot, w, h| {
            let count = marks.len().saturating_sub(1).max(1) as f32;
            draw::rule(snapshot, 0.0, h / 2.0, w, h / 2.0, tone::GRID, 1.0, &[]);
            for (offset, active) in marks.iter().enumerate() {
                if *active {
                    let x = w * offset as f32 / count;
                    draw::fill_rect(snapshot, x - 1.5, 6.0, 3.0, h - 12.0, tone::RECORD);
                }
            }
            let position = index.get().min(marks.len().saturating_sub(1));
            let cursor = w * position as f32 / count;
            draw::lit(snapshot, &draw::rect_path(cursor - 0.75, 0.0, 1.5, h), tone::PRESENT, 0.6);
        });
        canvas.set_hexpand(true);
        row.append(&canvas);
        let found = flags.iter().filter(|f| **f).count();
        let total = flags.len().saturating_sub(1);
        let figure = theme::label(font::FIGURE_SMALL, &format!("{found} of {total} intervals"), "value");
        figure.set_xalign(1.0);
        figure.set_size_request(130, -1);
        figure.set_valign(gtk::Align::Center);
        row.append(&figure);
        let weak = canvas.downgrade();
        self.cursor_moved.borrow_mut().push(Box::new(move |_| {
            if let Some(canvas) = weak.upgrade() {
                canvas.queue_draw();
            }
        }));
        row
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(height: u64, supply: f64, prices: &[(&str, f64)], last: f64) -> api::CurrencyPoint {
        api::CurrencyPoint {
            height,
            time: Some(1_700_000_000 + height),
            supply: Some(supply),
            reserves: prices
                .iter()
                .map(|(id, price)| api::ReservePoint {
                    currency_id: id.to_string(),
                    reserves: None,
                    price_in_reserve: Some(*price),
                    last_in: last,
                    last_out: 0.0,
                    last_fees: 0.0,
                })
                .collect(),
        }
    }

    #[test]
    fn ranges_start_at_the_span_or_the_start_block() {
        assert_eq!(from_height(1_000_000, Some(2_758_800), Span::Year), 2_758_800);
        assert_eq!(from_height(4_339_579, Some(2_758_800), Span::Year), 4_339_579 - 525_600);
        assert_eq!(from_height(4_339_579, Some(4_300_000), Span::Year), 4_300_000);
        assert_eq!(from_height(4_339_579, None, Span::All), 1);
        assert_eq!(from_height(10, Some(0), Span::Month), 1);
        assert_eq!(Span::Quarter.blocks(), Some(129_600));
    }

    #[test]
    fn lines_start_at_100_in_the_unit() {
        // The basket is 10 units at first, then 12: it gained a fifth in the unit.
        // DAI's price goes from 5 to 4: one DAI is worth 2 units, then 3.
        let points = vec![point(1, 100.0, &[("iUnit", 10.0), ("iDai", 5.0)], 0.0), point(2, 100.0, &[("iUnit", 12.0), ("iDai", 4.0)], 1.0)];
        let lines = index_lines(&points, "iUnit", "Bridge", &|id| id.to_string());
        assert_eq!(lines.len(), 2);
        assert!(lines[0].basket);
        assert_eq!(lines[0].values, vec![Some(100.0), Some(120.0)]);
        assert_eq!(lines[1].values, vec![Some(100.0), Some(150.0)]);
        assert_eq!(lines[1].color, tone::SERIES[1]);
        assert_eq!(rebased(&[None, Some(4.0), Some(2.0)]), vec![None, Some(100.0), Some(50.0)]);
        assert_eq!(rebased(&[Some(0.0), Some(2.0)]), vec![None, None]);
        assert_eq!(index_domain(&lines), (92.0, 153.0));
        assert_eq!(index_domain(&[]), (87.0, 113.0));
    }

    #[test]
    fn changes_in_words() {
        assert_eq!(change_text(193.94), "+93.9");
        assert_eq!(change_text(80.0), "−20.0");
        assert_eq!(change_text(1334.5), "+1,234.5");
        assert_eq!(change_since(100.0, Some(112.5), false), "+12.5% since");
        assert_eq!(change_since(100.0, Some(90.0), false), "-10.0% since");
        assert_eq!(change_since(100.0, Some(90.0), true), "latest");
        assert_eq!(change_since(0.0, Some(90.0), false), "latest");
    }

    #[test]
    fn activity_and_a_flat_end() {
        let points: Vec<_> = [(100.0, 0.0), (101.0, 1.0), (101.0, 1.0), (102.0, 2.0), (102.0, 2.0), (102.0, 2.0), (102.0, 2.0)]
            .iter()
            .enumerate()
            .map(|(i, (supply, last))| point(i as u64, *supply, &[("iUnit", 1.0)], *last))
            .collect();
        assert_eq!(changed(&points), vec![false, true, false, true, false, false, false]);
        assert_eq!(flat_since(&points), Some(1_700_000_003));
        assert_eq!(flat_since(&points[..5]), None);
        let (low, high) = row_domain(&[Some(10.0), Some(20.0)]);
        assert!((low - 8.8).abs() < 1e-9 && (high - 21.2).abs() < 1e-9);
        assert_eq!(row_domain(&[Some(10.0)]), (9.5, 10.5));
    }

    #[test]
    fn ticks_are_round() {
        assert_eq!(ticks(94.0, 273.0), vec![100.0, 150.0, 200.0, 250.0]);
        assert_eq!(ticks(92.0, 108.0), vec![95.0, 100.0, 105.0]);
        assert!(ticks(5.0, 5.0).is_empty());
    }
}
