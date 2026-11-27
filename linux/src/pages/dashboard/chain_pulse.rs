//! The chain pulse: one instrument for the rhythm of the chain (`ChainPulse.swift`).
//!
//! Verus blocks arrive in bursts: several within seconds, then a wait. The upper part shows
//! the last 25 minutes on an even time scale: one mark for each block, staked above the
//! line and mined below it. The lower part sets the present wait among the waits of the
//! last day, on a logarithmic scale. One curve joins the two parts: it is the time of the
//! lower axis on an even scale, and it ends at the present of the upper part.
//!
//! The panel is a display. Colour is a signal only: violet is staked, amber is mined, white
//! is the present, red is a wait longer than each wait in the record.
//! `macos/design/pulse-research.html` has the reasons for the forms.
//!
//! Drawing is in three layers, so that the motion does not redraw the panel:
//! - the plot (`base`) redraws once a second: axes, grid, waits, the selected ring;
//! - the trace (`trace`) draws the marks of the blocks. With each beat a front goes from
//!   the present to the left in one second; the marks get their new places as the front
//!   passes, and the front gives light to the line, the marks, the time curve and the dots,
//!   as the trace of a vector monitor does. It redraws on the frame clock only during the
//!   second of the front;
//! - the mark (`mark`) draws the live wait on both axes from the same frame timestamp.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{glib, gsk};

use crate::bridge::api;
use crate::format;
use crate::model::AppModel;
use crate::theme::draw::{self, Anchor};
use crate::theme::{self, font, lamp, tone, Color};
use crate::wait_record::{self, SPAN};
use crate::widgets::canvas::Canvas;
use crate::widgets::wait_plot::WaitPlot;
use crate::widgets::display::{self, Readout};

/// The target time between blocks.
const TARGET: f64 = 60.0;
const GUTTER: f32 = 78.0;
const RIGHT_INSET: f32 = 22.0;
const CELL: f32 = 9.0;
const CELL_STEP: f32 = 11.0;
/// Room for four stacked cells on each side of the line. A larger burst is rare, and its
/// cells then move closer together to stay inside the lane.
const LANE_CELLS: usize = 4;
const LANE: f32 = 7.0 + LANE_CELLS as f32 * CELL_STEP;
/// The upper part: the top of the lanes, the line, and the bottom of the lanes.
const BLOCKS_TOP: f32 = 20.0;
const AXIS_Y: f32 = BLOCKS_TOP + LANE;
const BLOCKS_BOTTOM: f32 = AXIS_Y + LANE;
/// The lower part: the line of the waits.
const WAITS_BASE: f32 = BLOCKS_BOTTOM + 96.0;
const RULER_TOP: f32 = WAITS_BASE + 26.0;
const PLOT_HEIGHT: f32 = RULER_TOP + 30.0;
/// The time at which the axis of the waits changes from an even scale to a logarithmic one.
const KNEE: f64 = 10.0;
/// The length of the light behind the front.
const TAIL: f32 = 190.0;

// A mark is a pale core, and its colour is in the light around it, as on film.
const STAKED_LIGHT: Color = Color::rgb(0.88, 0.87, 0.9);
const MINED_LIGHT: Color = Color::rgb(0.9, 0.86, 0.68);
const LINE_LIGHT: Color = Color::rgb(0.84, 0.88, 0.8);
const WARM: Color = Color::rgb(0.84, 0.88, 0.8);
const PALE: Color = Color::rgb(0.78, 0.9, 0.88);
const BLUE: Color = Color::rgb(0.3, 0.74, 0.7);

#[derive(Clone)]
struct Placed {
    block: usize,
    x: f32,
    y: f32,
    staked: bool,
}

struct Burst {
    blocks: Vec<usize>,
    x: f32,
}

#[derive(Default)]
struct State {
    /// Newest first.
    blocks: Vec<api::BlockRow>,
    /// True when the node is at the chain tip, so the right edge is the present moment.
    live: bool,
    /// Unconfirmed transactions in the mempool.
    waiting: Option<u64>,
    /// The block selected in the dashboard's list, drawn like a hovered block.
    selected: Option<String>,
    hovered: Option<String>,
    width: f32,
    /// Unix time at the current display frame, or the newest block's time for stored data.
    now: f64,
    beat: i64,
    marks: Vec<Placed>,
    bursts: Vec<Burst>,
    /// The marks of the beat before, for the part ahead of the front.
    before: Vec<Placed>,
    /// When the front started, in microseconds of the frame clock.
    sweep_start: Option<i64>,
}

pub struct ChainPulse {
    pub root: gtk::Overlay,
    state: Rc<RefCell<State>>,
    model: std::rc::Weak<AppModel>,
    base: Canvas,
    trace: Canvas,
    mark: Canvas,
    caption: gtk::Label,
    readouts: Readouts,
}

struct Readouts {
    row: gtk::Box,
    count: Readout,
    bullet: Canvas,
    bullet_values: Rc<Cell<(i64, i64, Option<(i64, i64)>)>>,
    target: Readout,
    usual: Readout,
    staked: Readout,
    mined: Readout,
    unconfirmed: Readout,
    problem: gtk::Label,
}

fn record() -> Option<api::PulseHistory> {
    wait_record::with(|r| r.history.clone())
}

fn longest_wait() -> Option<f64> {
    wait_record::with(|r| r.history.as_ref()?.waits.last().map(|w| *w as f64))
}

fn wait_quantile(share: f64) -> Option<f64> {
    wait_record::with(|r| {
        let waits = &r.history.as_ref()?.waits;
        let last = waits.len().checked_sub(1)?;
        Some(waits[((waits.len() as f64 * share) as usize).min(last)] as f64)
    })
}

fn is_record(seconds: f64) -> bool {
    longest_wait().is_some_and(|longest| seconds > longest)
}

fn wait_color(seconds: f64) -> Color {
    if is_record(seconds) { tone::ALERT } else { tone::PRESENT }
}

/// True for a wait in the longest tenth of the record.
fn is_long(seconds: f64) -> bool {
    wait_quantile(0.9).is_some_and(|limit| seconds >= limit)
}

fn burst_seconds() -> i64 {
    record().map(|h| h.burst_seconds as i64).unwrap_or(10)
}

/// The longest time on the axis of the waits.
fn waits_end(current: Option<f64>) -> f64 {
    1800f64.max(longest_wait().unwrap_or(0.0) * 1.15).max(current.unwrap_or(0.0) * 1.15)
}

/// Where a wait is on the axis, from 0 at the left to 1 at the right.
fn wait_share(seconds: f64, high: f64) -> f64 {
    (seconds.max(0.0) / KNEE).ln_1p() / (high / KNEE).ln_1p()
}

fn wait_seconds_at(share: f64, high: f64) -> f64 {
    KNEE * (share * (high / KNEE).ln_1p()).exp_m1()
}

fn wait_x(seconds: f64, width: f32, high: f64) -> f32 {
    let right = width - RIGHT_INSET;
    GUTTER + wait_share(seconds, high) as f32 * (right - GUTTER)
}

fn x_of(time: f64, width: f32, now: f64) -> f32 {
    let right = width - RIGHT_INSET;
    right - ((now - time) / SPAN) as f32 * (right - GUTTER)
}

/// "6:08" for a wait of six minutes and eight seconds.
fn clock(seconds: f64) -> String {
    let whole = seconds as i64;
    if whole >= 3600 {
        format!("{}:{:02}:{:02}", whole / 3600, whole / 60 % 60, whole % 60)
    } else {
        format!("{}:{:02}", whole / 60, whole % 60)
    }
}

fn live_clock(seconds: f64) -> String {
    let tenths = (seconds * 10.0) as u64;
    format!("{}.{}", clock((tenths / 10) as f64), tenths % 10)
}

fn duration(seconds: f64) -> String {
    if seconds < 60.0 {
        format!("{}s", seconds as i64)
    } else {
        format!("{}m {:02}s", seconds as i64 / 60, seconds as i64 % 60)
    }
}

fn animations_on() -> bool {
    gtk::Settings::default().map(|s| s.is_gtk_enable_animations()).unwrap_or(true)
}

impl State {
    fn current_wait(&self) -> Option<f64> {
        if !self.live {
            return None;
        }
        self.blocks.first().map(|b| (self.now - b.time as f64).max(0.0))
    }

    /// Groups the blocks into bursts, in chain order, and gives each burst one position.
    /// The bridge uses the same rule for the record: a block no more than the burst time
    /// after the block before is in the same burst.
    fn place(&mut self) {
        let burst = burst_seconds();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for index in (0..self.blocks.len()).rev() {
            let block = &self.blocks[index];
            if let Some(group) = groups.last_mut()
                && let Some(previous) = group.last()
                && block.time as i64 - self.blocks[*previous].time as i64 <= burst
            {
                group.push(index);
                continue;
            }
            groups.push(vec![index]);
        }
        let right = self.width - RIGHT_INSET;
        let mut bursts: Vec<Burst> = Vec::new();
        for group in groups {
            // A timestamp can be a little later than this computer's clock.
            let mut bx = x_of(self.blocks[group[0]].time as f64, self.width, self.now).min(right - CELL);
            if bx < GUTTER {
                continue;
            }
            // Two bursts a few seconds apart must not draw on each other.
            if let Some(before) = bursts.last()
                && bx - before.x < CELL_STEP
            {
                bx = before.x + CELL_STEP;
            }
            bursts.push(Burst { blocks: group, x: bx });
        }
        // One cell for each block: staked cells up from the line, mined cells down.
        let step = |count: usize| -> f32 {
            if count <= LANE_CELLS { CELL_STEP } else { (LANE - 7.0 - CELL) / (count - 1) as f32 }
        };
        let mut marks = Vec::new();
        for burst in &bursts {
            let staked_count = burst.blocks.iter().filter(|i| self.blocks[**i].staked).count();
            let (up_step, down_step) = (step(staked_count), step(burst.blocks.len() - staked_count));
            let (mut up, mut down) = (0, 0);
            for index in &burst.blocks {
                let staked = self.blocks[*index].staked;
                let y = if staked {
                    let y = AXIS_Y - 7.0 - CELL - up as f32 * up_step;
                    up += 1;
                    y
                } else {
                    let y = AXIS_Y + 7.0 + down as f32 * down_step;
                    down += 1;
                    y
                };
                marks.push(Placed { block: *index, x: burst.x - CELL / 2.0, y, staked });
            }
        }
        self.bursts = bursts;
        self.marks = marks;
    }

    fn nearest(&self, x: f32, y: f32) -> Option<usize> {
        let mut best = None;
        let mut distance = 9.0f32;
        for mark in &self.marks {
            let gap = ((mark.x + CELL / 2.0 - x).powi(2) + (mark.y + CELL / 2.0 - y).powi(2)).sqrt();
            if gap < distance {
                distance = gap;
                best = Some(mark.block);
            }
        }
        best
    }

    /// The time that the record of waits covers: "26 hours".
    fn record_span() -> String {
        let Some(history) = record() else { return "day".into() };
        let hours = history.covered_seconds as f64 / 3600.0;
        if hours >= 1.5 {
            format!("{} hours", hours.round() as i64)
        } else {
            format!("{} minutes", (hours * 60.0).round() as i64)
        }
    }

    fn caption(&self, model: &AppModel) -> (String, bool) {
        let Some(index) = self.hovered.as_ref().and_then(|hash| self.blocks.iter().position(|b| &b.hash == hash)) else {
            return (
                format!(
                    "The top chart shows the blocks of the last 25 minutes. The bottom chart shows the time between blocks in the last {}. Taller stacks or bars mean more waits of that length.",
                    Self::record_span()
                ),
                false,
            );
        };
        let block = &self.blocks[index];
        let mut text = format!(
            "Block {} · {} · {} transaction{} · {}",
            format::integer(block.height),
            if block.staked { "staked" } else { "mined" },
            block.transaction_count,
            if block.transaction_count == 1 { "" } else { "s" },
            format::date(block.time)
        );
        if index + 1 < self.blocks.len() {
            let gap = block.time as i64 - self.blocks[index + 1].time as i64;
            if gap >= 0 {
                text += &format!(" · {} after the previous block", duration(gap as f64));
            } else {
                text += &format!(" · timestamp {}s before the previous block", -gap);
            }
        }
        if let Some(seen) = model.state().arrivals.get(&block.hash) {
            text += &format!(" · this app saw it at {}", format::time_of_day(*seen));
        }
        (text, true)
    }
}

impl ChainPulse {
    pub fn new(model: &Rc<AppModel>) -> Rc<Self> {
        let state = Rc::new(RefCell::new(State::default()));
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.add_css_class("panel");

        let readouts = Readouts::new();
        readouts.row.set_margin_start(18);
        readouts.row.set_margin_end(18);
        readouts.row.set_margin_top(14);
        readouts.row.set_margin_bottom(10);
        column.append(&readouts.row);
        column.append(&display::rule());

        let s = state.clone();
        let base = Canvas::new(0, PLOT_HEIGHT as i32, move |canvas, snapshot, w, _| {
            let state = s.borrow();
            draw_blocks(canvas, snapshot, &state, w);
            draw_waits(canvas, snapshot, &state, w);
        });
        base.set_hexpand(true);
        let s = state.clone();
        let trace = Canvas::new(0, 0, move |canvas, snapshot, w, _| draw_trace(canvas, snapshot, &s.borrow(), w));
        trace.set_can_target(false);
        let s = state.clone();
        let mark = Canvas::new(0, 0, move |canvas, snapshot, w, _| draw_mark(canvas, snapshot, &s.borrow(), w));
        mark.set_can_target(false);
        let plot = gtk::Overlay::new();
        plot.set_child(Some(&base));
        plot.add_overlay(&trace);
        plot.add_overlay(&mark);
        plot.set_tooltip_text(None);
        column.append(&plot);
        column.append(&display::rule());

        let caption = gtk::Label::new(None);
        caption.set_xalign(0.0);
        caption.set_wrap(true);
        caption.set_lines(2);
        caption.set_ellipsize(gtk::pango::EllipsizeMode::End);
        caption.add_css_class("caption-text");
        caption.add_css_class("label");
        caption.set_margin_start(18);
        caption.set_margin_end(18);
        caption.set_margin_top(10);
        caption.set_margin_bottom(10);
        caption.set_size_request(-1, 16);
        column.append(&caption);

        let root = gtk::Overlay::new();
        root.set_child(Some(&column));
        root.add_overlay(&display::corner_marks());

        let pulse = Rc::new(Self {
            root,
            state,
            model: Rc::downgrade(model),
            base,
            trace,
            mark,
            caption,
            readouts,
        });

        // Hover and click on the marks.
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&pulse);
        motion.connect_motion(move |_, x, y| {
            if let Some(pulse) = weak.upgrade() {
                let found = {
                    let state = pulse.state.borrow();
                    state.nearest(x as f32, y as f32).map(|i| state.blocks[i].hash.clone())
                };
                pulse.set_hovered(found);
            }
        });
        let weak = Rc::downgrade(&pulse);
        motion.connect_leave(move |_| {
            if let Some(pulse) = weak.upgrade() {
                pulse.set_hovered(None);
            }
        });
        plot.add_controller(motion);
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&pulse);
        click.connect_released(move |_, _, x, y| {
            let Some(pulse) = weak.upgrade() else { return };
            let hash = {
                let state = pulse.state.borrow();
                state.nearest(x as f32, y as f32).map(|i| state.blocks[i].hash.clone())
            };
            if let (Some(hash), Some(model)) = (hash, pulse.model.upgrade()) {
                crate::theme::sound::link();
                model.open(api::Route::Block { id: hash });
            }
        });
        plot.add_controller(click);

        // GTK's frame clock is monotonic. Convert its timestamp to the same Unix time
        // used by block headers so delayed frames and newly received tips use actual age.
        let weak = Rc::downgrade(&pulse);
        pulse.mark.add_tick_callback(move |canvas, clock| {
            let Some(pulse) = weak.upgrade() else { return glib::ControlFlow::Break };
            let mut state = pulse.state.borrow_mut();
            if state.live {
                let unix_frame = glib::real_time() + (clock.frame_time() - glib::monotonic_time());
                state.now = unix_frame as f64 / 1_000_000.0;
            }
            let new_second = state.live && state.now as i64 != state.beat;
            let resized = state.width != pulse.base.width() as f32;
            let start = new_second.then(|| clock.frame_time() - (state.now.fract() * 1e6) as i64);
            let live = state.live;
            drop(state);
            if new_second || resized {
                pulse.beat(start);
            }
            if live {
                canvas.queue_draw();
                if pulse.state.borrow().sweep_start.is_some() {
                    pulse.trace.queue_draw();
                }
            }
            glib::ControlFlow::Continue
        });

        // The record of waits: a new read redraws the plot.
        let weak = Rc::downgrade(&pulse);
        wait_record::watch(&pulse.root, move || {
            if let Some(pulse) = weak.upgrade() {
                pulse.beat(None);
            }
        });
        pulse
    }

    /// New data from the dashboard.
    pub fn set(&self, blocks: &[api::BlockRow], live: bool, waiting: Option<u64>, selected: Option<String>) {
        {
            let mut state = self.state.borrow_mut();
            state.blocks = blocks.to_vec();
            state.live = live;
            state.now = if live { format::now() } else { blocks.first().map_or(0.0, |b| b.time as f64) };
            state.waiting = waiting;
            state.selected = selected;
        }
        if let Some(model) = self.model.upgrade() {
            // The reader starts again when the node gets to the tip: it reads only then.
            wait_record::keep(&model, live);
        }
        self.beat(None);
    }

    pub fn set_selected(&self, selected: Option<String>) {
        self.state.borrow_mut().selected = selected;
        self.base.queue_draw();
    }

    fn set_hovered(&self, hovered: Option<String>) {
        if self.state.borrow().hovered == hovered {
            return;
        }
        self.state.borrow_mut().hovered = hovered;
        self.update_caption();
        self.base.queue_draw();
    }

    fn update_caption(&self) {
        let Some(model) = self.model.upgrade() else { return };
        let (text, hovering) = self.state.borrow().caption(&model);
        self.caption.set_text(&text);
        if hovering {
            self.caption.remove_css_class("label");
            self.caption.add_css_class("value");
        } else {
            self.caption.remove_css_class("value");
            self.caption.add_css_class("label");
        }
    }

    /// Lay out a new second, new data or a resized plot. Only a new live second
    /// starts a sweep. The frame callback animates the existing layout between beats.
    fn beat(&self, sweep_start: Option<i64>) {
        {
            let mut state = self.state.borrow_mut();
            state.width = self.base.width() as f32;
            let previous = std::mem::take(&mut state.marks);
            state.place();
            state.sweep_start = sweep_start.filter(|_| animations_on() && state.width > 0.0);
            state.before = previous;
            state.beat = state.now as i64;
        }
        self.update_readouts();
        self.update_caption();
        self.base.queue_draw();
        self.trace.queue_draw();
        self.mark.queue_draw();
    }

    /// The figures that answer first: how many blocks, against the target and against what
    /// is usual for this chain.
    fn update_readouts(&self) {
        let state = self.state.borrow();
        let now = state.now;
        let in_window: Vec<&api::BlockRow> = state
            .blocks
            .iter()
            .filter(|b| (b.time as f64) > now - SPAN && (b.time as f64) <= now + 120.0)
            .collect();
        let oldest = state.blocks.last().map(|b| b.time as f64).unwrap_or(now);
        // The list can be shorter than the window. Count against the time it covers.
        let covered = SPAN.min((now - oldest).max(TARGET));
        let minutes = (covered / 60.0).round() as i64;
        let staked = in_window.iter().filter(|b| b.staked).count();
        let goal = (covered / TARGET).round() as i64;
        let counts = record().map(|h| h.window_counts).unwrap_or_default();
        let usual = (counts.len() >= 10 && covered >= SPAN)
            .then(|| (counts[counts.len() / 10] as i64, counts[counts.len() * 9 / 10] as i64));
        let r = &self.readouts;
        r.count.set(&format!("Blocks in {minutes} min"), &in_window.len().to_string());
        r.bullet_values.set((in_window.len() as i64, goal, usual));
        r.bullet.queue_draw();
        r.bullet.set_tooltip_text(Some("The bar is the number of blocks. The tick is the target. The band is the usual range."));
        r.target.set("Target", &goal.to_string());
        match usual {
            Some((low, high)) => {
                r.usual.set("Usual", &format!("{low}–{high}"));
                r.usual.root.set_visible(true);
            }
            None => r.usual.root.set_visible(false),
        }
        r.staked.set("Staked", &staked.to_string());
        r.mined.set("Mined", &(in_window.len() - staked).to_string());
        match (state.live, state.waiting) {
            (true, Some(waiting)) => {
                r.unconfirmed.set("Unconfirmed", &waiting.to_string());
                r.unconfirmed.root.set_visible(true);
            }
            _ => r.unconfirmed.root.set_visible(false),
        }
        let problem = wait_record::with(|r| r.problem.clone());
        match problem {
            Some(problem) => {
                r.problem.set_text(&format!("The app cannot read the record of waits. {problem}"));
                r.problem.set_visible(true);
            }
            None => r.problem.set_visible(false),
        }
        // The row gets less space between its figures when the panel is narrow.
        let available = self.root.width() - 36;
        for spacing in [26, 16, 9] {
            r.row.set_spacing(spacing);
            let (_, natural, _, _) = r.row.measure(gtk::Orientation::Horizontal, -1);
            if available <= 0 || natural <= available {
                break;
            }
        }
    }
}

impl Readouts {
    fn new() -> Self {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 26);
        let count = Readout::new("Blocks in 25 min", "0", "value", Some("00"));
        let values = Rc::new(Cell::new((0i64, 0i64, None::<(i64, i64)>)));
        let v = values.clone();
        // Stephen Few's bullet graph in one line: a bar for the count, a tick for the
        // target, and a band for the range that is usual for this chain.
        let bullet = Canvas::new(120, 10, move |_, snapshot, w, h| {
            let (value, target, usual) = v.get();
            let top = (value.max(target).max(usual.map(|u| u.1).unwrap_or(0))) as f64 * 1.2;
            if top <= 0.0 {
                return;
            }
            let pos = |count: i64| (count as f64 / top) as f32 * w;
            let white = Color::rgb(1.0, 1.0, 1.0);
            draw::fill_rect(snapshot, 0.0, 0.0, w, h, white.opacity(0.08));
            if let Some((low, high)) = usual {
                draw::fill_rect(snapshot, pos(low), 0.0, pos(high) - pos(low), h, white.opacity(0.2));
            }
            draw::fill_rect(snapshot, 0.0, h / 2.0 - 1.5, pos(value), 3.0, white.opacity(0.92));
            draw::fill_rect(snapshot, pos(target) - 1.0, -2.0, 2.0, h + 4.0, white);
        });
        bullet.set_valign(gtk::Align::Center);
        bullet.set_margin_top(4);
        let target = Readout::new("Target", "0", "value", Some("00"));
        let usual = Readout::new("Usual", "", "value", Some("00–00"));
        usual.root.set_tooltip_text(Some("Four in five 25-minute periods in the record had this number of blocks."));
        let staked = Readout::new("Staked", "0", "staked", Some("00"));
        let mined = Readout::new("Mined", "0", "mined", Some("00"));
        let unconfirmed = Readout::new("Unconfirmed", "0", "value", Some("00"));
        let problem = gtk::Label::new(None);
        problem.add_css_class("alert");
        problem.add_css_class("caption-text");
        problem.set_ellipsize(gtk::pango::EllipsizeMode::End);
        problem.set_visible(false);
        for widget in [&count.root, &target.root, &usual.root, &staked.root, &mined.root, &unconfirmed.root] {
            widget.set_valign(gtk::Align::Baseline);
        }
        row.append(&count.root);
        row.append(&bullet);
        row.append(&target.root);
        row.append(&usual.root);
        row.append(&staked.root);
        row.append(&mined.root);
        row.append(&unconfirmed.root);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
        row.append(&problem);
        let _ = theme::markup(font::SMALL, "");
        Self { row, count, bullet, bullet_values: values, target, usual, staked, mined, unconfirmed, problem }
    }
}

// MARK: Drawing the blocks

fn draw_blocks(widget: &Canvas, snapshot: &gtk::Snapshot, state: &State, width: f32) {
    let right = width - RIGHT_INSET;
    let top = BLOCKS_TOP - 6.0;
    let bottom = BLOCKS_BOTTOM + 6.0;
    let now = state.now;

    draw::plate(snapshot, widget, "Staked", 18.0, AXIS_Y - LANE / 2.0, 46.0, Some(tone::STAKED));
    draw::plate(snapshot, widget, "Mined", 18.0, AXIS_Y + LANE / 2.0, 46.0, Some(tone::MINED));

    // A tick for each minute, and a line for each five.
    let mut minute = 1;
    while (minute * 60) as f64 <= SPAN {
        let gx = x_of(now - (minute * 60) as f64, width, now);
        if minute % 5 == 0 {
            draw::rule(snapshot, gx, top, gx, bottom, tone::GRID, 1.0, &[]);
            draw::name(snapshot, widget, &format!("{minute} min"), gx, bottom + 11.0, Anchor::Center, tone::LABEL);
        } else {
            draw::rule(snapshot, gx, AXIS_Y - 2.5, gx, AXIS_Y + 2.5, tone::RULE, 1.0, &[]);
        }
        minute += 1;
    }

    // The window can start before the oldest block that the app has loaded. Say so: an
    // empty stretch there is not a stretch without blocks.
    if let Some(oldest) = state.blocks.last() {
        let ox = x_of(oldest.time as f64, width, now);
        if ox > GUTTER + 40.0 {
            let hatch = gsk::PathBuilder::new();
            let mut hx = GUTTER;
            while hx < ox - 8.0 {
                hatch.move_to(hx, bottom);
                hatch.line_to((hx + (bottom - top)).min(ox - 8.0), top);
                hx += 9.0;
            }
            draw::stroke(snapshot, &hatch.to_path(), tone::GRID, 1.0);
            draw::name(snapshot, widget, "not loaded", (GUTTER + ox - 8.0) / 2.0, top + 9.0, Anchor::Center, tone::LABEL);
        }
    }

    // The line. It has no ticks for the target: this chain has no steady rhythm.
    draw::rule(snapshot, GUTTER, AXIS_Y, right, AXIS_Y, tone::AXIS, 1.0, &[]);

    // Waits between bursts that are long for this chain.
    for pair in state.bursts.windows(2) {
        let (before, burst) = (&pair[0], &pair[1]);
        let (Some(last), Some(first)) = (before.blocks.last(), burst.blocks.first()) else { continue };
        let wait = state.blocks[*first].time as f64 - state.blocks[*last].time as f64;
        if !is_long(wait) {
            continue;
        }
        wait_span(widget, snapshot, before.x + 8.0, burst.x - 8.0, &clock(wait), wait_color(wait), false);
    }

    // The marks are drawn by the trace. The plot draws only the ring of the mark that the
    // pointer or the list has.
    for mark in &state.marks {
        let block = &state.blocks[mark.block];
        let chosen = state.hovered.as_deref() == Some(&block.hash) || state.selected.as_deref() == Some(&block.hash);
        if !chosen {
            continue;
        }
        let (x, y, size) = (mark.x - 1.5, mark.y - 1.5, CELL + 3.0);
        let shape = if block.staked { draw::ellipse_path(x, y, size, size) } else { draw::rect_path(x, y, size, size) };
        draw::lit(snapshot, &shape, if block.staked { tone::STAKED } else { tone::MINED }, 0.9);
        draw::stroke(snapshot, &shape, tone::PRESENT, 1.5);
    }

    // The right edge: the present, or the newest block the node has validated.
    let spine = draw::rect_path(right - 0.75, top, 1.5, BLOCKS_BOTTOM - top);
    draw::lit(snapshot, &spine, tone::PRESENT, if state.live { 0.7 } else { 0.0 });
    draw::name(snapshot, widget, if state.live { "now" } else { "newest" }, right, top - 8.0, Anchor::Trailing, tone::PRESENT);
}

/// A long wait: a stronger stretch of the line with its duration above it.
fn wait_span(widget: &Canvas, snapshot: &gtk::Snapshot, x0: f32, x1: f32, label: &str, color: Color, dashed: bool) {
    if x1 <= x0 {
        return;
    }
    draw::rule(snapshot, x0, AXIS_Y, x1, AXIS_Y, color, 2.0, if dashed { &[5.0, 4.0] } else { &[] });
    if x1 - x0 <= 56.0 {
        return;
    }
    draw::figure(snapshot, widget, label, (x0 + x1) / 2.0, AXIS_Y - 11.0, Anchor::Center, color);
}

// MARK: Drawing the waits

/// The shapes of the lower part that get light from the front: the time curve, the dots
/// that the present wait has passed, and the dots that it has not.
fn wait_shapes(width: f32, current: Option<f64>) -> (gsk::Path, gsk::Path, gsk::Path, Vec<(f32, f32)>) {
    let right = width - RIGHT_INSET;
    let base = WAITS_BASE;
    let high = waits_end(current);
    let curve = gsk::PathBuilder::new();
    let mut points = Vec::new();
    let mut cx = GUTTER;
    while cx <= right {
        let share = ((cx - GUTTER) / (right - GUTTER)) as f64;
        let seconds = wait_seconds_at(share, high);
        let y = base - (seconds / high) as f32 * (base - BLOCKS_BOTTOM);
        if cx == GUTTER { curve.move_to(cx, y) } else { curve.line_to(cx, y) }
        points.push((cx, y));
        cx += 2.0;
    }
    curve.line_to(right, BLOCKS_BOTTOM);
    points.push((right, BLOCKS_BOTTOM));

    let history = record();
    let waits = history.as_ref().map_or(&[][..], |h| h.waits.as_slice());
    // Reserve the space under the upper time axis for its minute labels.
    let plot = WaitPlot { left: GUTTER, right, top: BLOCKS_BOTTOM + 31.0, bottom: base - 3.0, diameter: 5.0, pitch: 6.5 };
    let (passed, ahead) = plot.paths(waits, current, |seconds| wait_x(seconds, width, high));
    (curve.to_path(), passed, ahead, points)
}

/// The recorded waits as dots on a logarithmic time axis. Each dot is an equal share of
/// the waits, so a person can count dots: the dots left of the mark are the share of waits
/// that were shorter than the present one.
fn draw_waits(widget: &Canvas, snapshot: &gtk::Snapshot, state: &State, width: f32) {
    let right = width - RIGHT_INSET;
    let base = WAITS_BASE;
    let current = state.current_wait();
    let high = waits_end(current);
    let position = |seconds: f64| wait_x(seconds, width, high);

    draw::plate(snapshot, widget, "Waits", 18.0, base - 12.0, 46.0, None);
    draw::plate(snapshot, widget, "Time", 18.0, RULER_TOP + 5.0, 46.0, None);

    // The scale is logarithmic. The curve shows it: the height of the curve is the time on
    // this axis, on an even scale. It ends at the present of the upper part, and so the two
    // parts are one instrument.
    let (curve, passed, ahead, points) = wait_shapes(width, current);
    let area = gsk::PathBuilder::new();
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 { area.move_to(*x, *y) } else { area.line_to(*x, *y) }
    }
    area.line_to(right, base);
    area.line_to(GUTTER, base);
    area.close();
    snapshot.push_fill(&area.to_path(), gsk::FillRule::Winding);
    draw::linear_gradient(
        snapshot,
        &draw::rect(GUTTER, BLOCKS_BOTTOM, right - GUTTER, base - BLOCKS_BOTTOM),
        (GUTTER, base),
        (right, base),
        &[(0.0, tone::PRESENT.opacity(0.0)), (1.0, tone::PRESENT.opacity(0.07))],
    );
    snapshot.pop();
    draw::stroke(snapshot, &curve, tone::PRESENT.opacity(0.5), 1.0);

    // The line of the waits and its times.
    draw::rule(snapshot, GUTTER, base, right, base, tone::AXIS, 1.0, &[]);
    let ticks: [(f64, &str); 9] = [
        (0.0, "0"), (10.0, "10 s"), (30.0, "30 s"), (60.0, "1 min"), (120.0, "2 min"), (300.0, "5 min"),
        (600.0, "10 min"), (1200.0, "20 min"), (3600.0, "1 h"),
    ];
    for (seconds, name) in ticks {
        if seconds > high {
            continue;
        }
        let tx = position(seconds);
        draw::rule(snapshot, tx, base, tx, base + 4.0, tone::AXIS, 1.0, &[]);
        draw::name(snapshot, widget, name, tx, base + 13.0, Anchor::Center, tone::LABEL);
    }

    // A ruler of equal steps of time. The steps get closer together to the right, and then
    // the ruler changes to a longer step.
    let steps: [(f64, f64, f64, &str); 3] = [(0.0, 300.0, 10.0, "10 s"), (300.0, 1800.0, 60.0, "1 min"), (1800.0, 36_000.0, 600.0, "10 min")];
    for (from, to, step, name) in steps {
        if from >= high {
            continue;
        }
        let end = to.min(high);
        let marks = gsk::PathBuilder::new();
        let mut seconds = from;
        while seconds <= end {
            let mx = position(seconds);
            marks.move_to(mx, RULER_TOP);
            marks.line_to(mx, RULER_TOP + if seconds == from { 11.0 } else { 6.0 });
            seconds += step;
        }
        draw::stroke(snapshot, &marks.to_path(), tone::AXIS, 1.0);
        if position(end) - position(from) > 40.0 {
            draw::name(snapshot, widget, &format!("step {name}"), position(from) + 5.0, RULER_TOP + 17.0, Anchor::Leading, tone::LABEL);
        }
    }

    // One hundred dots at most. Each dot is the wait at the middle of an equal share of the
    // record. The dots that the present wait has passed have light.
    if record().is_some_and(|h| !h.waits.is_empty()) {
        draw::fill(snapshot, &ahead, tone::AXIS);
        draw::lit(snapshot, &passed, tone::RECORD, 0.4);
    } else if state.live {
        draw::name(snapshot, widget, "reading the record", (GUTTER + right) / 2.0, base - 12.0, Anchor::Center, tone::LABEL);
    }
}

// MARK: The trace

fn marks_path(marks: &[Placed], staked: bool) -> gsk::Path {
    let builder = gsk::PathBuilder::new();
    for mark in marks.iter().filter(|m| m.staked == staked) {
        if staked {
            builder.add_circle(&draw::point(mark.x + CELL / 2.0, mark.y + CELL / 2.0), CELL / 2.0);
        } else {
            builder.add_rect(&draw::rect(mark.x, mark.y, CELL, CELL));
        }
    }
    builder.to_path()
}

/// The marks of the blocks, with soft light of their own colour.
fn draw_marks(snapshot: &gtk::Snapshot, marks: &[Placed]) {
    draw::push_shadow(snapshot, tone::STAKED_GLOW, 0.9, 4.5);
    draw::fill(snapshot, &marks_path(marks, true), tone::STAKED);
    snapshot.pop();
    draw::push_shadow(snapshot, tone::MINED_GLOW, 0.9, 4.5);
    draw::fill(snapshot, &marks_path(marks, false), tone::MINED);
    snapshot.pop();
}

/// The marks, and during a second of the front: the marks of the beat before ahead of the
/// front, the new marks behind it, and the light at the front.
fn draw_trace(_widget: &Canvas, snapshot: &gtk::Snapshot, state: &State, width: f32) {
    let area_x = GUTTER;
    let area_w = (width - RIGHT_INSET - GUTTER).max(0.0);
    let (start, end) = (area_x + area_w + 12.0, area_x - 12.0);
    let progress = match (state.sweep_start, _widget.frame_clock()) {
        (Some(begin), Some(clock)) => Some(((clock.frame_time() - begin) as f64 / 1e6).clamp(0.0, 1.0)),
        _ => None,
    };
    let Some(progress) = progress else {
        draw_marks(snapshot, &state.marks);
        return;
    };
    let front = start - (start - end) * progress as f32;
    let top = BLOCKS_TOP - 12.0;
    let tall = PLOT_HEIGHT;
    // Ahead of the front (to its left) the marks are where they were.
    snapshot.push_clip(&draw::rect(-20.0, top, front + 20.0, tall));
    draw_marks(snapshot, &state.before);
    snapshot.pop();
    // Behind the front the marks are at their new places.
    snapshot.push_clip(&draw::rect(front, top, width - front + 20.0, tall));
    draw_marks(snapshot, &state.marks);
    snapshot.pop();

    // The light at the front: full at the front, and down behind it, as the light of a
    // phosphor.
    let band = draw::rect(front - TAIL * 0.08, 0.0, TAIL, tall);
    let push_band = || {
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
    };
    // The upper part: the line and the marks, lighter, with a wider light.
    push_band();
    snapshot.push_clip(&draw::rect(area_x - 12.0, BLOCKS_TOP - 12.0, area_w + 60.0, LANE * 2.0 + 24.0));
    let line = gsk::PathBuilder::new();
    line.move_to(area_x, AXIS_Y);
    line.line_to(area_x + area_w, AXIS_Y);
    draw::push_shadow(snapshot, LINE_LIGHT, 0.9, 5.0);
    draw::stroke(snapshot, &line.to_path(), LINE_LIGHT, 1.5);
    snapshot.pop();
    draw::push_shadow(snapshot, tone::STAKED_GLOW, 1.0, 7.0);
    draw::fill(snapshot, &marks_path(&state.marks, true), STAKED_LIGHT);
    snapshot.pop();
    draw::push_shadow(snapshot, tone::MINED_GLOW, 1.0, 7.0);
    draw::fill(snapshot, &marks_path(&state.marks, false), MINED_LIGHT);
    snapshot.pop();
    snapshot.pop();
    snapshot.pop();

    // The same front crosses the lower part at the same time. It gives light to the time
    // curve and to the dots, at half the light. It moves nothing there.
    let (curve, passed, ahead, _) = wait_shapes(width, state.current_wait());
    push_band();
    snapshot.push_clip(&draw::rect(area_x - 12.0, BLOCKS_BOTTOM - 12.0, area_w + 60.0, WAITS_BASE - BLOCKS_BOTTOM + 26.0));
    snapshot.push_opacity(0.5);
    draw::push_shadow(snapshot, WARM, 1.0, 5.0);
    draw::stroke(snapshot, &curve, WARM, 1.5);
    snapshot.pop();
    draw::push_shadow(snapshot, WARM, 1.0, 6.0);
    draw::fill(snapshot, &ahead, WARM);
    snapshot.pop();
    draw::push_shadow(snapshot, BLUE, 1.0, 6.0);
    draw::fill(snapshot, &passed, PALE);
    snapshot.pop();
    snapshot.pop();
    snapshot.pop();
    snapshot.pop();
}

/// Where the mark of a wait is: on the axis of the waits, at most at its right end.
fn mark_x(wait: f64, width: f32) -> f32 {
    wait_x(wait, width, waits_end(Some(wait))).min(width - RIGHT_INSET)
}

/// The present wait as a mark that moves along the axis without steps. With a new block it
/// starts again at the left. When the front of the trace is at it, it has more light for a
/// moment, which then goes down as the light of a phosphor does.
fn draw_mark(widget: &Canvas, snapshot: &gtk::Snapshot, state: &State, width: f32) {
    let Some(wait) = state.current_wait() else { return };
    let x = mark_x(wait, width);
    let color = wait_color(wait);
    let label = live_clock(wait);
    if let Some(newest) = state.bursts.last() && is_long(wait) {
        wait_span(widget, snapshot, newest.x + 8.0, width - RIGHT_INSET, &label, color, true);
    }
    // The flare: the front crosses from the right to the left in one second. It reaches
    // the mark at a part of the second, and the flare lasts 0.4 seconds from then.
    let (start, end) = (width - RIGHT_INSET + 12.0, GUTTER - 12.0);
    let mut glow = 5.0f32;
    let mut wide = 1.0f32;
    if let (Some(begin), Some(clock)) = (state.sweep_start, widget.frame_clock())
        && start > end
    {
        let reach = ((start - x) / (start - end)).clamp(0.0, 1.0) as f64;
        let t = (clock.frame_time() - begin) as f64 / 1e6 - reach;
        if (0.0..0.4).contains(&t) {
            let k = t / 0.4;
            let amount = if k < 0.1 { k / 0.1 } else { 1.0 - (k - 0.1) / 0.9 } as f32;
            glow = 5.0 + 4.0 * amount;
            wide = 1.0 + 0.6 * amount;
        }
    }
    let (top, bottom) = (WAITS_BASE - 38.0, WAITS_BASE);
    // Place the stroke and its label under one fractional translation, including
    // when the marker moves less than a pixel in a frame.
    snapshot.save();
    snapshot.translate(&draw::point(x, 0.0));
    draw::push_shadow(snapshot, color, 0.9, glow);
    draw::rule(snapshot, 0.0, top, 0.0, bottom, color, 1.5 * wide, &[]);
    snapshot.pop();
    let mark_font = font::Role { size: 13.0, ..font::FIGURE };
    draw::push_shadow(snapshot, color, 0.9, glow);
    draw::text(snapshot, widget, mark_font, &label, 0.0, top - 11.0, Anchor::Center, color);
    snapshot.pop();
    snapshot.restore();
    let _ = lamp::HALATION;
}

#[cfg(test)]
mod tests {
    use super::live_clock;

    #[test]
    fn live_wait_rolls_over_at_the_same_tenth_as_the_marker() {
        assert_eq!(live_clock(0.0), "0:00.0");
        assert_eq!(live_clock(59.999), "0:59.9");
        assert_eq!(live_clock(60.0), "1:00.0");
        assert_eq!(live_clock(3600.125), "1:00:00.1");
    }
}
