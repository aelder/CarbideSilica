//! Synchronization progress and the validation speed (`SyncCard` and `SpeedCard` in
//! `NodeView.swift`).

use std::cell::RefCell;
use std::rc::Rc;

use chrono::{Datelike, TimeZone, Utc};
use gtk::glib;
use gtk::prelude::*;

use super::is_syncing;
use crate::bridge::api;
use crate::format;
use crate::model::SpeedSample;
use crate::page::{self, load::clear, Page};
use crate::theme::draw::{self, Anchor};
use crate::theme::tone;
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, Readout};

/// The first Verus block.
const GENESIS: f64 = 1_526_889_600.0;

/// The part of the chain that the node validated.
pub fn fraction(node: &api::NodeStatus) -> f64 {
    match node.longest_chain {
        Some(longest) if longest > 0 => (node.blocks as f64 / longest as f64).min(1.0),
        _ => 1.0,
    }
}

fn header_fraction(node: &api::NodeStatus) -> Option<f64> {
    let headers = node.synced_headers?;
    let longest = node.longest_chain.filter(|l| *l > 0)?;
    Some((headers as f64 / longest as f64).min(1.0))
}

fn header_detail(node: &api::NodeStatus) -> Option<String> {
    let headers = header_fraction(node)?;
    (headers > fraction(node) + 0.005 && headers < 0.999).then(|| format::percent(headers))
}

/// "approximately 3 h 20 min", from the blocks left and the speed.
pub fn time_left(node: &api::NodeStatus, speed: Option<f64>) -> String {
    let (Some(speed), Some(longest)) = (speed, node.longest_chain) else { return "–".into() };
    if speed <= 0.5 {
        return "–".into();
    }
    let seconds = longest.saturating_sub(node.blocks) as f64 / speed;
    let hours = seconds / 3600.0;
    if hours >= 48.0 {
        return format!("approximately {} days", (hours / 24.0).round() as i64);
    }
    if hours >= 1.0 {
        return format!("approximately {} h {} min", hours as i64, ((hours % 1.0) * 60.0) as i64);
    }
    format!("approximately {} min", ((seconds / 60.0) as i64).max(1))
}

/// The chain's whole history as one bar that fills as blocks are validated.
pub struct SyncCard {
    pub root: gtk::Box,
    page: std::rc::Weak<Page>,
    syncing: RefCell<Option<bool>>,
    title: gtk::Label,
    readouts: gtk::Box,
    note: gtk::Label,
    bar: Canvas,
    node: Rc<RefCell<Option<api::NodeStatus>>>,
}

impl SyncCard {
    pub fn new(page: &Rc<Page>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let title = gtk::Label::new(None);
        title.set_xalign(0.0);
        title.set_wrap(true);
        title.add_css_class("value");
        let readouts = gtk::Box::new(gtk::Orientation::Horizontal, 26);
        readouts.set_margin_start(18);
        readouts.set_margin_end(18);
        readouts.set_margin_top(12);
        readouts.set_margin_bottom(12);
        readouts.set_overflow(gtk::Overflow::Hidden);
        let node: Rc<RefCell<Option<api::NodeStatus>>> = Rc::default();
        let n = node.clone();
        let bar = Canvas::new(0, 62, move |canvas, snapshot, w, _| {
            if let Some(node) = n.borrow().as_ref() {
                draw_bar(canvas, snapshot, w, node);
            }
        });
        bar.set_hexpand(true);
        let note = page::paragraph(
            "The year marks are approximate. They assume a constant block rate. Blocks with more transactions take longer to validate.",
            &["caption-text", "label"],
        );
        Rc::new(Self {
            root,
            page: Rc::downgrade(page),
            syncing: RefCell::new(None),
            title,
            readouts,
            note,
            bar,
            node,
        })
    }

    /// Builds the card again when the title changes, so the cursor knows the card by it.
    fn rebuild(&self, syncing: bool) {
        let Some(page) = self.page.upgrade() else { return };
        clear(&self.root);
        for widget in [self.title.upcast_ref::<gtk::Widget>(), self.readouts.upcast_ref(), self.note.upcast_ref(), self.bar.upcast_ref()] {
            if let Some(parent) = widget.parent() {
                if let Some(parent) = parent.downcast_ref::<gtk::Box>() {
                    parent.remove(widget);
                }
            }
        }
        let card = page.card(if syncing { "Synchronization in progress" } else { "Synchronized" }, None);
        card.append(&self.title);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&self.readouts);
        column.append(&display::rule());
        let lower = gtk::Box::new(gtk::Orientation::Vertical, 10);
        lower.set_margin_start(16);
        lower.set_margin_end(16);
        lower.set_margin_top(16);
        lower.set_margin_bottom(16);
        lower.append(&self.bar);
        if syncing {
            lower.append(&display::rule());
            lower.append(&self.note);
        }
        column.append(&lower);
        card.append(&display::panel(&column, 0));
        self.root.append(&card.root);
    }

    pub fn update(&self, node: &api::NodeStatus, speed: Option<f64>) {
        let syncing = is_syncing(node);
        if *self.syncing.borrow() != Some(syncing) {
            self.syncing.replace(Some(syncing));
            self.rebuild(syncing);
        }
        match (syncing, node.tip_time) {
            (true, Some(tip)) => {
                self.title.set_markup(&format!(
                    "<span font_size=\"{}\">Validating blocks from <b>{}</b></span>",
                    crate::theme::pango_points(15.0),
                    glib::markup_escape_text(&format::month_year(tip))
                ));
                self.title.set_visible(true);
            }
            _ => self.title.set_visible(false),
        }
        clear(&self.readouts);
        let fraction = fraction(node);
        let block = if syncing {
            format!("{}{}", format::integer(node.blocks), node.longest_chain.map(|l| format!(" of {}", format::integer(l))).unwrap_or_default())
        } else {
            format::integer(node.blocks)
        };
        let row = &self.readouts;
        let add = |readout: Readout| {
            readout.root.set_valign(gtk::Align::Baseline);
            row.append(&readout.root);
        };
        add(Readout::new("Block", &block, "value", (!syncing).then_some("0,000,000")));
        add(Readout::new("Validated", &format::percent1(fraction), "value", Some("100.0%")));
        if let Some(headers) = header_detail(node) {
            add(Readout::new("Headers", &headers, "value", None));
        }
        if syncing {
            let text = speed.map(|s| format!("{} blocks/s", s.round() as i64)).unwrap_or_else(|| "Measuring…".into());
            add(Readout::new("Speed", &text, "value", None));
            add(Readout::new("Time remaining", &time_left(node, speed), "value", None));
        }
        if let (Some(waiting), false) = (node.mempool_count, syncing) {
            add(Readout::new("Unconfirmed", &waiting.to_string(), "value", Some("00")));
        }
        self.bar.update_property(&[gtk::accessible::Property::Label(&format!(
            "Synchronization progress: {} of the chain validated",
            format::percent(fraction)
        ))]);
        *self.node.borrow_mut() = Some(node.clone());
        self.bar.queue_draw();
    }
}

fn draw_bar(widget: &Canvas, snapshot: &gtk::Snapshot, width: f32, node: &api::NodeStatus) {
    let fraction = fraction(node) as f32;
    let (top, height) = (4.0f32, 30.0f32);
    let bottom = top + height;
    draw::fill_rect(snapshot, 0.0, top, width, height, tone::GRID);
    if let Some(headers) = header_fraction(node) {
        draw::fill_rect(snapshot, 0.0, top, width * headers as f32, height, tone::RECORD.opacity(0.22));
    }
    // The validated part is done: pale blue. Its edge is the present.
    draw::fill_rect(snapshot, 0.0, top, width * fraction, height, tone::RECORD.opacity(0.85));
    draw::stroke(snapshot, &draw::rect_path(0.5, top + 0.5, width - 1.0, height - 1.0), tone::RULE, 1.0);
    let edge = (width * fraction).max(0.75).min(width - 0.75);
    draw::lit(snapshot, &draw::rect_path(edge - 0.75, top - 3.0, 1.5, height + 6.0), tone::PRESENT, if fraction < 1.0 { 0.7 } else { 0.0 });

    // Year marks: height is close to linear in time because blocks target one minute.
    let now = format::now();
    let total = now - GENESIS;
    let this_year = Utc.timestamp_opt(now as i64, 0).single().map(|t| t.year()).unwrap_or(2019).max(2019);
    for year in 2019..=this_year {
        let Some(start) = Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0).single() else { continue };
        let x = width * ((start.timestamp() as f64 - GENESIS) / total) as f32;
        draw::rule(snapshot, x, bottom, x, bottom + 6.0, tone::AXIS, 1.0, &[]);
        draw::name(snapshot, widget, &year.to_string(), x, bottom + 16.0, Anchor::Center, tone::LABEL);
    }
}

/// Validation speed as a live trace (`SpeedCard`).
pub struct SpeedCard {
    pub root: gtk::Box,
    holder: gtk::Box,
    chart: RefCell<Option<Canvas>>,
    samples: Rc<RefCell<Vec<SpeedSample>>>,
}

impl SpeedCard {
    pub fn new(page: &Rc<Page>) -> Rc<Self> {
        let card = page.card("Validation speed", Some("blocks per second"));
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.append(&holder);
        Rc::new(Self { root: card.root, holder, chart: RefCell::new(None), samples: Rc::default() })
    }

    pub fn update(&self, samples: &[SpeedSample]) {
        *self.samples.borrow_mut() = samples.to_vec();
        let has_chart = samples.len() >= 2;
        let shows_chart = self.chart.borrow().is_some();
        if has_chart != shows_chart || self.holder.first_child().is_none() {
            clear(&self.holder);
            if has_chart {
                let s = self.samples.clone();
                let chart = Canvas::new(0, 140, move |canvas, snapshot, w, h| draw_speed(canvas, snapshot, w, h, &s.borrow()));
                chart.set_hexpand(true);
                self.holder.append(&display::panel(&chart, 12));
                *self.chart.borrow_mut() = Some(chart);
            } else {
                *self.chart.borrow_mut() = None;
                let text = page::text("Measurement in progress.", &["secondary"]);
                text.set_size_request(-1, 120);
                text.set_valign(gtk::Align::Center);
                self.holder.append(&text);
            }
        }
        if let Some(chart) = self.chart.borrow().as_ref() {
            chart.queue_draw();
        }
    }
}

/// Steps of a scale: 1, 2 or 5 times a power of ten, about `count` across `span`.
pub fn nice_step(span: f64, count: f64) -> f64 {
    if span <= 0.0 || !span.is_finite() {
        return 1.0;
    }
    let raw = span / count;
    let power = 10f64.powf(raw.log10().floor());
    let unit = raw / power;
    let nice = if unit <= 1.0 { 1.0 } else if unit <= 2.0 { 2.0 } else if unit <= 5.0 { 5.0 } else { 10.0 };
    nice * power
}

fn draw_speed(widget: &Canvas, snapshot: &gtk::Snapshot, width: f32, height: f32, samples: &[SpeedSample]) {
    if samples.len() < 2 {
        return;
    }
    // The axis of values at the right, as Swift Charts puts it; the times below.
    let label_width = 40.0f32;
    let bottom_room = 20.0f32;
    let plot_w = (width - label_width).max(1.0);
    let plot_h = (height - bottom_room).max(1.0);
    let (first, last) = (samples[0].time, samples[samples.len() - 1].time);
    let span = (last - first).max(1.0);
    let high = samples.iter().map(|s| s.blocks_per_second).fold(0.0f64, f64::max);
    let step = nice_step(high.max(1.0), 4.0);
    let top = (high / step).ceil().max(1.0) * step;
    let x = |time: f64| ((time - first) / span) as f32 * plot_w;
    let y = |value: f64| plot_h - (value / top) as f32 * plot_h;
    // Grid and values.
    let mut value = 0.0;
    while value <= top + 1e-9 {
        let at = y(value);
        draw::rule(snapshot, 0.0, at, plot_w, at, tone::GRID, 1.0, &[]);
        draw::name(snapshot, widget, &format::decimal(value, 2), plot_w + 6.0, at, Anchor::Leading, tone::LABEL);
        value += step;
    }
    // Times: one mark each minute or more.
    let time_step = [30.0, 60.0, 120.0, 300.0, 600.0, 900.0].into_iter().find(|s| span / s <= 6.0).unwrap_or(1800.0);
    let mut tick = (first / time_step).ceil() * time_step;
    while tick <= last {
        let at = x(tick);
        draw::rule(snapshot, at, 0.0, at, plot_h, tone::GRID, 1.0, &[]);
        draw::rule(snapshot, at, plot_h, at, plot_h + 4.0, tone::AXIS, 1.0, &[]);
        let text = chrono::Local
            .timestamp_opt(tick as i64, 0)
            .single()
            .map(|t| t.format("%-I:%M").to_string())
            .unwrap_or_default();
        draw::name(snapshot, widget, &text, at, plot_h + 12.0, Anchor::Center, tone::LABEL);
        tick += time_step;
    }
    // The area and the line, through the points with a monotone curve.
    let points: Vec<(f32, f32)> = samples.iter().map(|s| (x(s.time), y(s.blocks_per_second))).collect();
    let line = monotone(&points);
    let area = gtk::gsk::PathBuilder::new();
    area.move_to(points[0].0, plot_h);
    area.line_to(points[0].0, points[0].1);
    append_monotone(&area, &points);
    area.line_to(points[points.len() - 1].0, plot_h);
    area.close();
    draw::fill(snapshot, &area.to_path(), tone::RECORD.opacity(0.08));
    draw::stroke(snapshot, &line, tone::RECORD, 1.0);
}

/// Monotone cubic interpolation (Fritsch and Carlson), as `.interpolationMethod(.monotone)`.
fn tangents(points: &[(f32, f32)]) -> Vec<f32> {
    let n = points.len();
    let slopes: Vec<f32> = (0..n - 1)
        .map(|i| {
            let dx = points[i + 1].0 - points[i].0;
            if dx.abs() < 1e-6 { 0.0 } else { (points[i + 1].1 - points[i].1) / dx }
        })
        .collect();
    let mut m = vec![0.0f32; n];
    m[0] = slopes[0];
    m[n - 1] = slopes[n - 2];
    for i in 1..n - 1 {
        m[i] = if slopes[i - 1] * slopes[i] <= 0.0 { 0.0 } else { 3.0 * (slopes[i - 1] + slopes[i]) / ((2.0 * slopes[i] + slopes[i - 1]) / slopes[i - 1] + (slopes[i] + 2.0 * slopes[i - 1]) / slopes[i]) };
    }
    m
}

fn append_monotone(builder: &gtk::gsk::PathBuilder, points: &[(f32, f32)]) {
    let m = tangents(points);
    for i in 0..points.len() - 1 {
        let (x0, y0) = points[i];
        let (x1, y1) = points[i + 1];
        let dx = (x1 - x0) / 3.0;
        builder.cubic_to(x0 + dx, y0 + m[i] * dx, x1 - dx, y1 - m[i + 1] * dx, x1, y1);
    }
}

fn monotone(points: &[(f32, f32)]) -> gtk::gsk::Path {
    let builder = gtk::gsk::PathBuilder::new();
    builder.move_to(points[0].0, points[0].1);
    append_monotone(&builder, points);
    builder.to_path()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(blocks: u64, longest: Option<u64>) -> api::NodeStatus {
        api::NodeStatus {
            blocks,
            longest_chain: longest,
            tip_time: None,
            synced_headers: None,
            mempool_count: None,
            peers: vec![],
            peers_error: None,
            source: api::DataSource::LocalDaemon,
            provenance: api::Provenance {
                source: String::new(),
                source_id: String::new(),
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
    fn time_left_follows_the_swift_forms() {
        assert_eq!(time_left(&node(0, Some(1_000_000)), Some(2.0)), "approximately 6 days");
        assert_eq!(time_left(&node(0, Some(9_000)), Some(2.0)), "approximately 1 h 15 min");
        assert_eq!(time_left(&node(0, Some(30)), Some(2.0)), "approximately 1 min");
        assert_eq!(time_left(&node(0, Some(30)), Some(0.4)), "–");
        assert_eq!(time_left(&node(0, None), Some(4.0)), "–");
        assert_eq!(fraction(&node(50, Some(100))), 0.5);
        assert_eq!(fraction(&node(50, None)), 1.0);
    }

    #[test]
    fn scale_steps() {
        assert_eq!(nice_step(100.0, 4.0), 50.0);
        assert_eq!(nice_step(7.0, 4.0), 2.0);
        assert_eq!(nice_step(0.3, 4.0), 0.1);
    }
}
