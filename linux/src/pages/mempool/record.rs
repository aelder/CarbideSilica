//! The time record of the mempool (`MempoolTimeRecord`).
//!
//! The last 15 minutes on one time axis that ends at the present. Each block is a mark: up
//! for a staked block, down for a mined block, as in the chain pulse. Each transaction that
//! the app saw in the mempool is a line from its arrival to the block that took it. A line
//! that reaches the right edge is a transaction that waits now.
//!
//! Drawing is in two layers, so that the motion does not redraw the panel:
//! - the plot (`base`) redraws once a second and when the pointer moves to another target:
//!   plates, ticks, the line of the blocks, the frame of the target and the present;
//! - the trace (`trace`, `LightTrace` in `ChainPulse.swift`) draws the guides, the marks
//!   and the lines. With each beat a front goes from the present to the left in one second;
//!   the shapes get their new places as the front passes, and the front gives light to the
//!   line of the blocks, the marks and the lines. It redraws on the frame clock only during
//!   the second of the front.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{glib, gsk};

use crate::bridge::api::{self, Route};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::theme::draw::{self, Anchor};
use crate::theme::{font, tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, Readout};

use super::feed::{self, Sighting};
use super::logic::BLOCK_LIMIT;
use super::text::{clock, count, fill_percent, span};
use super::words::{self, Names};

pub const SPAN: f64 = 15.0 * 60.0;
pub const GUTTER: f32 = 92.0;
pub const RIGHT_INSET: f32 = 22.0;
pub const LANE_STEP: f32 = 12.0;
pub const MAX_LANES: usize = 8;
/// The part for the transactions has room for this many lines also when it has none.
pub const MIN_LANES: usize = 4;
/// The middle of the first transaction lane.
pub const LANES_TOP: f32 = 30.0;
/// The width of a lane's plate.
pub const PLATE_WIDTH: f32 = 60.0;
/// The length of the light behind the front.
const TAIL: f32 = 190.0;

// A mark is a pale core, and its colour is in the light around it, as on film.
const STAKED_LIGHT: Color = Color::rgb(0.88, 0.87, 0.9);
const MINED_LIGHT: Color = Color::rgb(0.9, 0.86, 0.68);
const WARM: Color = Color::rgb(0.84, 0.88, 0.8);
const PALE: Color = Color::rgb(0.78, 0.9, 0.88);
const BLUE: Color = Color::rgb(0.3, 0.74, 0.7);

// MARK: Geometry

/// The line of the blocks. The staked marks are above it and the mined marks below.
pub fn axis_y(lanes: usize) -> f32 {
    LANES_TOP + lanes as f32 * LANE_STEP + 34.0
}

/// The height of the plot.
pub fn plot_height(lanes: usize) -> f32 {
    axis_y(lanes) + 46.0
}

/// The end of a sighting's line: the present, the arrival of its block, or the refresh that
/// first did not have it.
pub fn end(sighting: &Sighting, now: f64, arrivals: &HashMap<String, f64>) -> f64 {
    let Some(left) = sighting.left else { return now };
    if let Some(arrived) = sighting.block_hash.as_ref().and_then(|hash| arrivals.get(hash)) {
        return arrived.min(left);
    }
    left
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub sighting: Sighting,
    pub lane: usize,
}

/// The sightings in the window, each in the first lane that is free at its arrival.
pub fn lines(sightings: &HashMap<String, Sighting>, now: f64, arrivals: &HashMap<String, f64>) -> Vec<Line> {
    let start = now - SPAN;
    let mut visible: Vec<&Sighting> = sightings.values().filter(|s| end(s, now, arrivals) > start).collect();
    visible.sort_by(|a, b| a.arrival.total_cmp(&b.arrival).then_with(|| a.txid.cmp(&b.txid)));
    let mut free: Vec<f64> = Vec::new();
    let mut result = Vec::new();
    for sighting in visible {
        let lane = free.iter().position(|f| *f <= sighting.arrival).unwrap_or(free.len());
        if lane == free.len() {
            free.push(f64::NEG_INFINITY);
        }
        free[lane] = end(sighting, now, arrivals) + 4.0;
        result.push(Line { sighting: sighting.clone(), lane: lane.min(MAX_LANES - 1) });
    }
    result
}

pub fn lane_count(lines: &[Line]) -> usize {
    lines.iter().map(|l| l.lane + 1).max().unwrap_or(0).max(MIN_LANES)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Frame {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn inset(self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w - 2.0 * dx, self.h - 2.0 * dy)
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

/// The transaction or the block under the pointer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Transaction(String),
    Block(String),
}

/// The same places as the drawing, as shapes: for the light of the pulse and for the
/// pointer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shapes {
    /// x0, x1 and y of the line of the blocks.
    pub axis: (f32, f32, f32),
    /// A guide up from each block through the transactions: x, top, bottom.
    pub guides: Vec<(f32, f32, f32)>,
    pub staked: Vec<Frame>,
    pub mined: Vec<Frame>,
    /// Transactions that wait, that a block took, and that left without a block.
    pub waiting: Vec<Frame>,
    pub taken: Vec<Frame>,
    pub gone: Vec<Frame>,
    /// Where each transaction and each block is, for the pointer.
    pub targets: Vec<(Target, Frame)>,
}

/// The x of a time.
pub fn x_of(time: f64, now: f64, width: f32) -> f32 {
    let right = width - RIGHT_INSET;
    let start = now - SPAN;
    GUTTER + (((time - start) / SPAN).clamp(0.0, 1.0)) as f32 * (right - GUTTER)
}

/// When this app saw a block arrive, or its own time.
fn seen(block: &api::BlockRow, arrivals: &HashMap<String, f64>) -> f64 {
    arrivals.get(&block.hash).copied().unwrap_or(block.time as f64)
}

pub fn shapes(
    width: f32,
    now: f64,
    lines: &[Line],
    lanes: usize,
    blocks: &[api::BlockRow],
    arrivals: &HashMap<String, f64>,
) -> Shapes {
    let mut shapes = Shapes::default();
    let start = now - SPAN;
    let right = width - RIGHT_INSET;
    let axis = axis_y(lanes);
    let x = |time: f64| x_of(time, now, width);
    shapes.axis = (GUTTER, right, axis);
    for line in lines {
        let y = LANES_TOP + line.lane as f32 * LANE_STEP;
        let from = x(line.sighting.arrival);
        let to = x(end(&line.sighting, now, arrivals)).max(from + 3.0);
        let frame = Frame::new(from, y - 1.5, to - from, 3.0);
        if line.sighting.left.is_none() {
            shapes.waiting.push(frame);
        } else if line.sighting.gone {
            // A line of dashes.
            let mut dash = from;
            while dash < to {
                shapes.gone.push(Frame::new(dash, y - 1.5, 2.0f32.min(to - dash), 3.0));
                dash += 9.0;
            }
        } else {
            shapes.taken.push(frame);
            // The end of the line: the block that took the transaction.
            shapes.taken.push(Frame::new(to - 1.0, y - 4.0, 2.0, 8.0));
        }
        shapes.targets.push((Target::Transaction(line.sighting.txid.clone()), frame.inset(-3.0, -4.0)));
    }
    for block in blocks {
        let seen = seen(block, arrivals);
        if seen <= start {
            continue;
        }
        let position = x(seen).min(right - 5.0);
        shapes.guides.push((position, LANES_TOP - 10.0, axis));
        let mark = Frame::new(position - 4.5, if block.staked { axis - 16.0 } else { axis + 7.0 }, 9.0, 9.0);
        if block.staked {
            shapes.staked.push(mark);
        } else {
            shapes.mined.push(mark);
        }
        shapes.targets.push((Target::Block(block.hash.clone()), mark.inset(-4.0, -4.0)));
    }
    shapes
}

pub fn target_at(shapes: &Shapes, x: f32, y: f32) -> Option<Target> {
    shapes.targets.iter().find(|(_, frame)| frame.contains(x, y)).map(|(target, _)| target.clone())
}

// MARK: The widget

#[derive(Default)]
struct State {
    now: f64,
    width: f32,
    lines: Vec<Line>,
    lanes: usize,
    blocks: Vec<api::BlockRow>,
    arrivals: HashMap<String, f64>,
    shapes: Shapes,
    /// The shapes of the beat before, for the part ahead of the front.
    before: Shapes,
    pointed: Option<Target>,
    beat: i64,
    /// When the front started, in microseconds of the frame clock.
    sweep_start: Option<i64>,
    /// False when the front does not move: the light is then off.
    light: bool,
    /// True while a frame-clock callback draws the front.
    ticking: bool,
}

impl State {
    /// The area that the front crosses.
    fn area(&self) -> (f32, f32, f32, f32) {
        let top = LANES_TOP - 10.0;
        (GUTTER, top, (self.width - RIGHT_INSET - GUTTER).max(0.0), axis_y(self.lanes) + 22.0 - top)
    }
}

pub struct TimeRecord {
    pub root: gtk::Overlay,
    model: Weak<AppModel>,
    state: Rc<RefCell<State>>,
    base: Canvas,
    trace: Canvas,
    caption: gtk::Label,
    readouts: Readouts,
    height: std::cell::Cell<i32>,
}

struct Readouts {
    /// The row in a frame that does not take the row's width: what does not fit is cut.
    holder: gtk::Overlay,
    room: gtk::Box,
    row: gtk::Box,
    waiting: Readout,
    size: Readout,
    bar: Canvas,
    share: Rc<std::cell::Cell<f64>>,
    percent: gtk::Label,
    last_block: Readout,
    seen: Readout,
}

fn animations_on() -> bool {
    gtk::Settings::default().map(|s| s.is_gtk_enable_animations()).unwrap_or(true)
}

impl TimeRecord {
    pub fn new(model: &Rc<AppModel>) -> Rc<Self> {
        let state = Rc::new(RefCell::new(State { lanes: MIN_LANES, ..Default::default() }));
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("panel");

        let readouts = Readouts::new();
        column.append(&readouts.holder);
        column.append(&display::rule());

        let s = state.clone();
        let base = Canvas::new(0, plot_height(MIN_LANES) as i32, move |canvas, snapshot, w, _| {
            draw_plot(canvas, snapshot, &s.borrow(), w)
        });
        base.set_hexpand(true);
        let s = state.clone();
        let trace = Canvas::new(0, 0, move |canvas, snapshot, w, _| draw_trace(canvas, snapshot, &s.borrow(), w));
        trace.set_can_target(false);
        let caption = gtk::Label::new(None);
        caption.add_css_class("caption-text");
        caption.add_css_class("value");
        caption.set_single_line_mode(true);
        caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
        caption.set_xalign(0.0);
        caption.set_halign(gtk::Align::Fill);
        caption.set_valign(gtk::Align::Start);
        caption.set_margin_start(GUTTER as i32);
        caption.set_margin_end(80);
        caption.set_margin_top(6);
        caption.set_can_target(false);
        caption.set_visible(false);
        let plot = gtk::Overlay::new();
        plot.set_child(Some(&base));
        plot.add_overlay(&trace);
        plot.add_overlay(&caption);
        plot.set_cursor_from_name(Some("default"));
        column.append(&plot);

        let root = gtk::Overlay::new();
        root.set_child(Some(&column));
        root.add_overlay(&display::corner_marks());

        let record = Rc::new(Self {
            root,
            model: Rc::downgrade(model),
            state,
            base,
            trace,
            caption,
            readouts,
            height: std::cell::Cell::new(plot_height(MIN_LANES) as i32),
        });

        // A line and a mark can be opened: a click on a line opens its transaction, and a
        // click on a mark opens its block.
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&record);
        motion.connect_motion(move |_, x, y| {
            if let Some(record) = weak.upgrade() {
                let found = target_at(&record.state.borrow().shapes, x as f32, y as f32);
                record.set_pointed(found);
            }
        });
        let weak = Rc::downgrade(&record);
        motion.connect_leave(move |_| {
            if let Some(record) = weak.upgrade() {
                record.set_pointed(None);
            }
        });
        plot.add_controller(motion);
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&record);
        click.connect_released(move |_, _, x, y| {
            let Some(record) = weak.upgrade() else { return };
            let found = target_at(&record.state.borrow().shapes, x as f32, y as f32);
            let Some(model) = record.model.upgrade() else { return };
            match found {
                Some(Target::Transaction(txid)) => model.open(Route::Transaction { txid }),
                Some(Target::Block(hash)) => model.open(Route::Block { id: hash }),
                None => {}
            }
        });
        plot.add_controller(click);

        // Once a second: the present moves, and a new front starts.
        let weak = Rc::downgrade(&record);
        crate::page::every(&record.root, Duration::from_secs(1), move || {
            if let Some(record) = weak.upgrade() {
                record.update();
            }
        });
        let weak = Rc::downgrade(&record);
        feed::watch(&record.root, move || {
            if let Some(record) = weak.upgrade()
                && record.root.is_mapped()
            {
                record.update();
            }
        });
        let weak = Rc::downgrade(&record);
        model.watch(&[Topic::Status, Topic::Display, Topic::Names], &record.root, move |_| {
            if let Some(record) = weak.upgrade() {
                record.update();
            }
        });
        // The width is known after the first layout.
        let weak = Rc::downgrade(&record);
        record.base.connect_map(move |_| {
            if let Some(record) = weak.upgrade() {
                let weak = Rc::downgrade(&record);
                glib::idle_add_local_once(move || {
                    if let Some(record) = weak.upgrade() {
                        record.update();
                    }
                });
            }
        });
        record
    }

    fn set_pointed(&self, pointed: Option<Target>) {
        if self.state.borrow().pointed == pointed {
            return;
        }
        self.state.borrow_mut().pointed = pointed;
        self.update_caption();
        self.base.queue_draw();
    }

    /// One update: the present, the lines, the shapes and the readouts. A new second starts
    /// a pass of the front.
    pub fn update(&self) {
        let Some(model) = self.model.upgrade() else { return };
        let width = self.base.width() as f32;
        let now = format::now();
        let (blocks, arrivals) = {
            let state = model.state();
            (state.blocks.clone(), state.arrivals.clone())
        };
        let lines = feed::with(|feed| lines(&feed.sightings, now, &arrivals));
        let lanes = lane_count(&lines);
        let moving;
        {
            let mut state = self.state.borrow_mut();
            let beat = now as i64;
            let new_beat = beat != state.beat;
            let shapes = shapes(width, now, &lines, lanes, &blocks, &arrivals);
            if new_beat {
                // The shapes of the beat before stay ahead of the front.
                state.before = std::mem::take(&mut state.shapes);
            }
            state.shapes = shapes;
            state.now = now;
            state.width = width;
            state.lines = lines;
            state.lanes = lanes;
            state.blocks = blocks;
            state.arrivals = arrivals;
            if new_beat {
                state.beat = beat;
                // A person who asked the system for less movement gets no front.
                let still = !animations_on() || width <= GUTTER + RIGHT_INSET;
                state.light = !still;
                state.sweep_start = if still { None } else { self.trace.frame_clock().map(|clock| clock.frame_time()) };
            }
            moving = state.sweep_start.is_some() && !state.ticking;
            if moving {
                state.ticking = true;
            }
        }
        let height = plot_height(lanes) as i32;
        if self.height.replace(height) != height {
            self.base.set_size(0, height);
        }
        self.update_readouts(&model);
        self.update_caption();
        self.update_spoken(&model);
        self.base.queue_draw();
        self.trace.queue_draw();
        if moving {
            let weak = Rc::downgrade(&self.state);
            self.trace.add_tick_callback(move |canvas, clock| {
                let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
                canvas.queue_draw();
                let mut state = state.borrow_mut();
                match state.sweep_start {
                    Some(start) if clock.frame_time() - start < 1_000_000 => glib::ControlFlow::Continue,
                    // The front is at its end: the shapes are at their new places.
                    _ => {
                        state.sweep_start = None;
                        state.ticking = false;
                        glib::ControlFlow::Break
                    }
                }
            });
        }
    }

    fn update_readouts(&self, model: &Rc<AppModel>) {
        let (waiting, loaded, total) = feed::with(|feed| (feed.entries.len(), feed.loaded, feed.total_bytes));
        let state = self.state.borrow();
        let share = total as f64 / BLOCK_LIMIT as f64;
        let r = &self.readouts;
        r.waiting.set("Waiting", &if loaded { waiting.to_string() } else { "–".into() });
        for (class, on) in [("present", waiting > 0), ("value", waiting == 0)] {
            if on { r.waiting.value.add_css_class(class) } else { r.waiting.value.remove_css_class(class) }
        }
        r.size.set("Size", &if loaded { format::bytes(total) } else { "–".into() });
        if r.share.get() != share {
            r.share.set(share);
            r.bar.queue_draw();
        }
        crate::theme::set_label(&r.percent, font::FIGURE, &fill_percent(waiting, share));
        for (class, on) in [("value", share <= 1.0), ("alert", share > 1.0)] {
            if on { r.percent.add_css_class(class) } else { r.percent.remove_css_class(class) }
        }
        match model.state().blocks.first() {
            Some(block) => {
                r.last_block.set("Last block", &clock((state.now - block.time as f64).max(0.0)));
                r.last_block.root.set_visible(true);
            }
            None => r.last_block.root.set_visible(false),
        }
        r.seen.set("Seen in 15 min", &state.lines.len().to_string());
        // The row gets less space between its figures when the page is narrow, and it
        // never makes the page wider.
        let available = self.root.width();
        for spacing in [26, 16, 9] {
            r.row.set_spacing(spacing);
            let (_, natural, _, _) = r.row.measure(gtk::Orientation::Horizontal, -1);
            if available <= 0 || natural <= available {
                break;
            }
        }
        let (_, tall, _, _) = r.row.measure(gtk::Orientation::Vertical, -1);
        if r.room.height_request() != tall {
            r.room.set_size_request(-1, tall);
        }
    }

    fn update_caption(&self) {
        let Some(model) = self.model.upgrade() else { return };
        let state = self.state.borrow();
        match &state.pointed {
            Some(target) => {
                self.caption.set_text(&caption(target, &state, &model));
                self.caption.set_visible(true);
            }
            None => self.caption.set_visible(false),
        }
    }

    fn update_spoken(&self, model: &Rc<AppModel>) {
        let state = self.state.borrow();
        let start = state.now - SPAN;
        let blocks = model.state().blocks.iter().filter(|b| b.time as f64 >= start).count();
        let waiting = state.lines.iter().filter(|l| l.sighting.left.is_none()).count();
        let text = format!(
            "The last 15 minutes: {}, {} seen, {waiting} waiting now.",
            count(blocks, "block", "blocks"),
            count(state.lines.len(), "transaction", "transactions")
        );
        self.base.update_property(&[gtk::accessible::Property::Label(&text)]);
    }
}

impl Readouts {
    fn new() -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 26);
        row.set_margin_start(18);
        row.set_margin_end(18);
        row.set_margin_top(14);
        row.set_margin_bottom(10);
        let waiting = Readout::new("Waiting", "–", "value", Some("00"));
        let size = Readout::new("Size", "–", "value", Some("Zero KB"));
        let next = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = display::name_text("Next block");
        name.set_valign(gtk::Align::Baseline);
        next.append(&name);
        let share = Rc::new(std::cell::Cell::new(0.0f64));
        let s = share.clone();
        let bar = Canvas::new(110, 8, move |_, snapshot, w, h| {
            let share = s.get();
            draw::fill_rect(snapshot, 0.0, 0.0, w, h, tone::GRID);
            draw::fill_rect(snapshot, 0.0, 0.0, w * share.min(1.0) as f32, h, if share <= 1.0 { tone::RECORD } else { tone::ALERT });
            draw::fill_rect(snapshot, w - 1.0, -2.0, 1.0, h + 4.0, tone::AXIS);
        });
        bar.set_overflow(gtk::Overflow::Visible);
        bar.set_valign(gtk::Align::Center);
        bar.set_margin_top(4);
        next.append(&bar);
        // The widest value, so that the figures after it keep their places.
        let percent = crate::theme::label(font::FIGURE, "0%", "value");
        display::reserve_width(&percent, font::FIGURE, "0.00%");
        percent.set_valign(gtk::Align::Baseline);
        next.append(&percent);
        next.set_tooltip_text(Some(&format!(
            "The part of one block that the waiting transactions fill. A block holds {}.",
            format::bytes(BLOCK_LIMIT)
        )));
        let last_block = Readout::new("Last block", "", "value", Some("00:00"));
        let seen = Readout::new("Seen in 15 min", "0", "value", Some("00"));
        for widget in [&waiting.root, &size.root, &next, &last_block.root, &seen.root] {
            widget.set_valign(gtk::Align::Baseline);
            row.append(widget);
        }
        // The row never makes the page wider: what does not fit is cut at the right.
        let room = gtk::Box::new(gtk::Orientation::Vertical, 0);
        room.set_hexpand(true);
        let holder = gtk::Overlay::new();
        holder.set_child(Some(&room));
        row.set_halign(gtk::Align::Start);
        row.set_valign(gtk::Align::Start);
        holder.add_overlay(&row);
        holder.set_clip_overlay(&row, true);
        holder.set_measure_overlay(&row, false);
        Self { holder, room, row, waiting, size, bar, share, percent, last_block, seen }
    }
}

/// What the line or the mark under the pointer is.
fn caption(target: &Target, state: &State, model: &Rc<AppModel>) -> String {
    let coin = model.coin();
    match target {
        Target::Transaction(txid) => {
            let names = Names::of(model);
            feed::with(|feed| {
                let mut parts: Vec<String> = Vec::new();
                if let Some(sentence) = words::sentence(feed.details.get(txid), &names)
                    && !sentence.is_empty()
                {
                    parts.push(sentence);
                }
                if parts.is_empty() {
                    parts.push(format!("Transaction {}.", format::short(txid, 10, 6)));
                }
                if let Some(sighting) = feed.sightings.get(txid) {
                    let mut facts: Vec<String> = Vec::new();
                    if sighting.left.is_none() {
                        facts.push(format!("Waits for {}", span(state.now - sighting.arrival)));
                    } else if sighting.gone {
                        facts.push("Left the mempool and is not in a block".into());
                    } else {
                        facts.push(format!("Waited {}", span(end(sighting, state.now, &state.arrivals) - sighting.arrival)));
                        if let Some(height) = sighting.height {
                            facts.push(format!("block {} took it", format::integer(height)));
                        }
                    }
                    if let Some(fee) = sighting.fee_sats {
                        facts.push(format!("fee {} {coin}", format::coins(fee)));
                    }
                    facts.push(format::bytes(sighting.size));
                    parts.push(facts.join(" · ") + ". Click to open.");
                }
                parts.join(" ")
            })
        }
        Target::Block(hash) => {
            let Some(block) = state.blocks.iter().find(|b| &b.hash == hash) else { return String::new() };
            let mut text = format!(
                "Block {}, {}, with {}.",
                format::integer(block.height),
                if block.staked { "staked" } else { "mined" },
                count(block.transaction_count as usize, "transaction", "transactions")
            );
            feed::with(|feed| {
                if feed.took_blocks.contains(hash) {
                    let took = feed.took.iter().filter(|t| &t.block_hash == hash).count();
                    text += &if took == 0 {
                        " It took no transaction from the mempool.".to_string()
                    } else {
                        format!(" It took {} from the mempool.", count(took, "transaction", "transactions"))
                    };
                }
            });
            text + " Click to open."
        }
    }
}

// MARK: Drawing

fn draw_plot(widget: &Canvas, snapshot: &gtk::Snapshot, state: &State, width: f32) {
    let now = state.now;
    let right = width - RIGHT_INSET;
    let axis = axis_y(state.lanes);
    let top = LANES_TOP - 10.0;
    let wait_bottom = LANES_TOP + state.lanes as f32 * LANE_STEP;
    let x = |time: f64| x_of(time, now, width);

    // The names of the lanes: the transactions that wait, and the blocks, as on the chain
    // pulse.
    draw::plate(snapshot, widget, "Waiting", 18.0, (top + wait_bottom) / 2.0, PLATE_WIDTH, None);
    draw::plate(snapshot, widget, "Staked", 18.0, axis - 13.0, PLATE_WIDTH, Some(tone::STAKED));
    draw::plate(snapshot, widget, "Mined", 18.0, axis + 13.0, PLATE_WIDTH, Some(tone::MINED));

    // The part of the transactions has a line below it.
    draw::rule(snapshot, GUTTER, wait_bottom + 4.0, right, wait_bottom + 4.0, tone::GRID, 1.0, &[]);

    // A tick for each minute, and a line for each five.
    for minute in 1..=15 {
        let gx = x(now - minute as f64 * 60.0);
        if minute % 5 == 0 {
            draw::rule(snapshot, gx, top, gx, axis + 22.0, tone::GRID, 1.0, &[]);
            draw::name(snapshot, widget, &format!("{minute} min"), gx, axis + 33.0, Anchor::Center, tone::LABEL);
        } else {
            draw::rule(snapshot, gx, axis - 2.5, gx, axis + 2.5, tone::RULE, 1.0, &[]);
        }
    }

    // The line of the blocks.
    draw::rule(snapshot, GUTTER, axis, right, axis, tone::AXIS, 1.0, &[]);

    // The guides and the marks of the blocks and the lines of the transactions are in the
    // trace, which gives them their places at the front.
    if state.lines.is_empty() {
        draw::name(
            snapshot,
            widget,
            "no transaction in the last 15 minutes",
            (GUTTER + right) / 2.0,
            (top + wait_bottom) / 2.0,
            Anchor::Center,
            tone::LABEL,
        );
    }
    // The line or the mark under the pointer.
    match &state.pointed {
        Some(Target::Transaction(txid)) => {
            if let Some(line) = state.lines.iter().find(|l| &l.sighting.txid == txid) {
                let y = LANES_TOP + line.lane as f32 * LANE_STEP;
                let from = x(line.sighting.arrival);
                let to = x(end(&line.sighting, now, &state.arrivals)).max(from + 3.0);
                // A frame round the line: the line itself is in the trace, above this.
                draw::stroke(snapshot, &draw::rect_path(from - 3.0, y - 4.5, to - from + 6.0, 9.0), tone::PRESENT, 1.0);
            }
        }
        Some(Target::Block(hash)) => {
            if let Some(block) = state.blocks.iter().find(|b| &b.hash == hash) {
                let position = x(seen(block, &state.arrivals)).min(right - 5.0);
                let (mx, my) = (position - 6.0, if block.staked { axis - 17.5 } else { axis + 5.5 });
                let shape = if block.staked { draw::ellipse_path(mx, my, 12.0, 12.0) } else { draw::rect_path(mx, my, 12.0, 12.0) };
                draw::stroke(snapshot, &shape, tone::PRESENT, 1.5);
            }
        }
        None => {}
    }

    // The right edge: the present.
    draw::lit(snapshot, &draw::rect_path(right - 0.75, top, 1.5, axis + 22.0 - top), tone::PRESENT, 0.7);
    draw::name(snapshot, widget, "now", right, top - 8.0, Anchor::Trailing, tone::PRESENT);
}

fn frames_path(frames: &[Frame], round: bool) -> gsk::Path {
    let builder = gsk::PathBuilder::new();
    for f in frames {
        if round {
            builder.add_circle(&draw::point(f.x + f.w / 2.0, f.y + f.h / 2.0), f.w / 2.0);
        } else {
            builder.add_rect(&draw::rect(f.x, f.y, f.w, f.h));
        }
    }
    builder.to_path()
}

fn guides_path(guides: &[(f32, f32, f32)]) -> gsk::Path {
    let builder = gsk::PathBuilder::new();
    for (x, top, bottom) in guides {
        builder.move_to(*x, *top);
        builder.line_to(*x, *bottom);
    }
    builder.to_path()
}

/// A filled shape, with a light of `glow` around it at `opacity` (none at 0).
fn glowing(snapshot: &gtk::Snapshot, path: &gsk::Path, color: Color, glow: Color, opacity: f64, radius: f32) {
    if opacity > 0.0 {
        draw::push_shadow(snapshot, glow, opacity, radius);
        draw::fill(snapshot, path, color);
        snapshot.pop();
    } else {
        draw::fill(snapshot, path, color);
    }
}

/// The shapes themselves, each in its colour with its own soft light.
fn draw_shapes(snapshot: &gtk::Snapshot, shapes: &Shapes) {
    draw::stroke(snapshot, &guides_path(&shapes.guides), tone::GRID, 1.0);
    glowing(snapshot, &frames_path(&shapes.staked, true), tone::STAKED, tone::STAKED_GLOW, 0.6, 4.5);
    glowing(snapshot, &frames_path(&shapes.mined, false), tone::MINED, tone::MINED_GLOW, 0.6, 4.5);
    glowing(snapshot, &frames_path(&shapes.waiting, false), tone::PRESENT, BLUE, 0.7, 4.5);
    glowing(snapshot, &frames_path(&shapes.taken, false), tone::RECORD, BLUE, 0.0, 4.5);
    glowing(snapshot, &frames_path(&shapes.gone, false), tone::ALERT, BLUE, 0.0, 4.5);
}

/// The light of the front on the line of the blocks, the marks and the lines.
fn draw_light(snapshot: &gtk::Snapshot, shapes: &Shapes) {
    let (x0, x1, y) = shapes.axis;
    let line = gsk::PathBuilder::new();
    line.move_to(x0, y);
    line.line_to(x1, y);
    draw::push_shadow(snapshot, WARM, 1.0, 5.0);
    draw::stroke(snapshot, &line.to_path(), WARM, 1.5);
    snapshot.pop();
    // The guides get no light.
    glowing(snapshot, &frames_path(&shapes.staked, true), STAKED_LIGHT, tone::STAKED_GLOW, 1.0, 6.0);
    glowing(snapshot, &frames_path(&shapes.mined, false), MINED_LIGHT, tone::MINED_GLOW, 1.0, 6.0);
    for frames in [&shapes.waiting, &shapes.taken, &shapes.gone] {
        glowing(snapshot, &frames_path(frames, false), PALE, BLUE, 1.0, 6.0);
    }
}

/// The front of the trace: ahead of it (to its left) the shapes are where they were,
/// behind it they are at their new places, and at it they have light that goes down behind
/// it as the light of a phosphor does. With no front, the shapes are at their new places.
fn draw_trace(widget: &Canvas, snapshot: &gtk::Snapshot, state: &State, _width: f32) {
    let (ax, ay, aw, ah) = state.area();
    let (start, end) = (ax + aw + 12.0, ax - 12.0);
    let progress = match (state.sweep_start, widget.frame_clock()) {
        (Some(begin), Some(clock)) => ((clock.frame_time() - begin) as f64 / 1e6).clamp(0.0, 1.0),
        _ => 1.0,
    };
    let front = start - (start - end) * progress as f32;
    let (wide, tall, top) = (aw + 60.0, ah + 24.0, ay - 12.0);
    snapshot.push_clip(&draw::rect(front - wide, top, wide, tall));
    draw_shapes(snapshot, &state.before);
    snapshot.pop();
    snapshot.push_clip(&draw::rect(front, top, wide, tall));
    draw_shapes(snapshot, &state.shapes);
    snapshot.pop();
    if !state.light {
        return;
    }
    // The light is full at the front and goes down to the right, behind the front.
    let band = draw::rect(front - TAIL * 0.08, top, TAIL, tall);
    snapshot.push_mask(gsk::MaskMode::Alpha);
    let black = Color::rgb(0.0, 0.0, 0.0);
    draw::linear_gradient(
        snapshot,
        &band,
        (band.x(), 0.0),
        (band.x() + TAIL, 0.0),
        &[(0.0, black.opacity(0.0)), (0.08, black), (0.4, black.opacity(0.35)), (1.0, black.opacity(0.0))],
    );
    snapshot.pop();
    draw_light(snapshot, &state.shapes);
    snapshot.pop();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sighting(txid: &str, arrival: f64, left: Option<f64>) -> Sighting {
        Sighting {
            txid: txid.into(),
            arrival,
            size: 250,
            fee_sats: Some(10_000),
            parents: vec![],
            left,
            height: None,
            block_hash: None,
            gone: false,
            checks: 0,
        }
    }

    fn block(hash: &str, time: u64, staked: bool) -> api::BlockRow {
        api::BlockRow {
            height: 1,
            hash: hash.into(),
            time,
            transaction_count: 2,
            size: None,
            production: String::new(),
            staked,
        }
    }

    #[test]
    fn lanes_are_reused() {
        let now = 10_000.0;
        let mut sightings = HashMap::new();
        for s in [
            sighting("a", now - 600.0, Some(now - 500.0)),
            sighting("b", now - 550.0, None),
            sighting("c", now - 400.0, None),
            sighting("old", now - 2000.0, Some(now - 1000.0)),
        ] {
            sightings.insert(s.txid.clone(), s);
        }
        let lines = lines(&sightings, now, &HashMap::new());
        let lanes: Vec<(&str, usize)> = lines.iter().map(|l| (l.sighting.txid.as_str(), l.lane)).collect();
        // a ends at now-500, free from now-496; c arrives at now-400 and takes lane 0 again.
        assert_eq!(lanes, [("a", 0), ("b", 1), ("c", 0)]);
        assert_eq!(lane_count(&lines), MIN_LANES);
    }

    #[test]
    fn the_end_of_a_line_is_the_block() {
        let mut s = sighting("a", 0.0, Some(50.0));
        assert_eq!(end(&s, 100.0, &HashMap::new()), 50.0);
        s.block_hash = Some("h".into());
        let arrivals: HashMap<String, f64> = [("h".to_string(), 40.0)].into();
        assert_eq!(end(&s, 100.0, &arrivals), 40.0);
        s.left = None;
        assert_eq!(end(&s, 100.0, &arrivals), 100.0);
    }

    #[test]
    fn shapes_and_targets() {
        let now = 10_000.0;
        let width = 1000.0;
        let mut gone = sighting("g", now - 300.0, Some(now - 260.0));
        gone.gone = true;
        let mut sightings = HashMap::new();
        for s in [sighting("w", now - 450.0, None), sighting("t", now - 600.0, Some(now - 120.0)), gone] {
            sightings.insert(s.txid.clone(), s);
        }
        let lines = lines(&sightings, now, &HashMap::new());
        let blocks = [block("s", now as u64 - 120, true), block("m", now as u64 - 60, false), block("old", now as u64 - 2000, false)];
        let shapes = shapes(width, now, &lines, 4, &blocks, &HashMap::new());
        assert_eq!(shapes.waiting.len(), 1);
        assert_eq!(shapes.taken.len(), 2);
        assert!(shapes.gone.len() >= 4);
        assert_eq!(shapes.staked.len(), 1);
        assert_eq!(shapes.mined.len(), 1);
        assert_eq!(shapes.guides.len(), 2);
        // The waiting line reaches the present.
        let w = shapes.waiting[0];
        assert!((w.x + w.w - (width - RIGHT_INSET)).abs() < 0.01);
        // Half the window is the middle of the time axis.
        assert!((x_of(now - SPAN / 2.0, now, width) - (GUTTER + (width - RIGHT_INSET - GUTTER) / 2.0)).abs() < 0.01);
        let staked = shapes.staked[0];
        assert_eq!(target_at(&shapes, staked.x + 4.0, staked.y + 4.0), Some(Target::Block("s".into())));
        assert_eq!(target_at(&shapes, w.x + 10.0, w.y + 1.0), Some(Target::Transaction("w".into())));
        assert_eq!(target_at(&shapes, 5.0, 5.0), None);
        assert_eq!(axis_y(4), 30.0 + 48.0 + 34.0);
    }
}
