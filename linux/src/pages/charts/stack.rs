//! The five measures of a range as short panels on one time axis, with one cursor through
//! all of them (`ChartStack` in ChartStack.swift). `macos/design/charts-research.html` has
//! the reasons for the form.
//!
//! Each panel has a reference for what is usual: process limits for the block rate, the
//! limits of chance for the staked share, and Carbide's trailing MA100 for the two
//! difficulties. The longest waits of the range are marked through all panels, so one
//! event can be seen in each measure.
//!
//! Drawing is in layers: the panels (`base`) redraw only for a new range or width; the
//! cursor (`cursors`) redraws when the pointer moves to another block or the selection
//! changes; the light (`light`) redraws on the frame clock.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{gdk, glib, graphene, gsk};

use crate::bridge::api::{BlockProduction, ChartPage};
use crate::format;
use crate::page::load::clear;
use crate::theme::draw::{self, Anchor};
use crate::theme::{self, font, tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, PlateFill};
use crate::widgets::link;

use super::format as chart_format;
use super::light::ChartLight;
use super::model::{partition_index, FeePoolState, Flag, Panel, Point, Scale, StackModel, BURST_SECONDS, CHANCE_HIGH, CHANCE_LOW};

pub const LEFT: f32 = 66.0;
pub const RIGHT: f32 = 22.0;
const HEADER: f32 = 22.0;
const PANEL: f32 = 84.0;
const GAP: f32 = 10.0;
const PLOT_HEIGHT: f32 = 5.0 * (HEADER + PANEL + GAP) + 18.0;

/// What the stack draws, given by the page.
struct Input {
    page: Rc<ChartPage>,
    span: u64,
    fee_loading: bool,
    scale: Scale,
    coin: String,
    selected: Option<String>,
    key: String,
}

struct Shared {
    input: Option<Input>,
    model: Option<Rc<StackModel>>,
    hovered: Option<usize>,
    /// The place of the pointer over the panels, so that the block under it is found again
    /// when a new range or more of the range comes.
    pointer: Option<f32>,
    /// The width the pictures of the light were made for, and their key.
    pictures: Option<(String, i32)>,
    pictures_pending: bool,
}

pub struct ChartStack {
    /// The display panel; it takes the keyboard focus.
    pub root: gtk::Overlay,
    shared: Rc<RefCell<Shared>>,
    base: Canvas,
    cursors: Canvas,
    light: Rc<ChartLight>,
    readout: gtk::Box,
    waits: WaitsStrip,
    select: Rc<dyn Fn(&str)>,
    open: Rc<dyn Fn(&str)>,
}

fn rect(panel: Panel, width: f32) -> graphene::Rect {
    let top = panel.index() as f32 * (HEADER + PANEL + GAP) + HEADER;
    draw::rect(LEFT, top, width - LEFT - RIGHT, PANEL)
}

fn plot_frame(width: f32) -> (f32, f32) {
    (LEFT, width - RIGHT)
}

/// A line through points, broken where a point is missing.
fn line(points: &[Option<(f32, f32)>]) -> gsk::Path {
    let builder = gsk::PathBuilder::new();
    let mut open = false;
    for point in points {
        match point {
            Some((x, y)) => {
                if open {
                    builder.line_to(*x, *y);
                } else {
                    builder.move_to(*x, *y);
                }
                open = true;
            }
            None => open = false,
        }
    }
    builder.to_path()
}

fn dots(points: impl Iterator<Item = (f32, f32)>, radius: f32) -> gsk::Path {
    let builder = gsk::PathBuilder::new();
    for (x, y) in points {
        builder.add_circle(&draw::point(x, y), radius);
    }
    builder.to_path()
}

/// The colour of a panel's plate: the signal of its measure, or an outline.
fn plate_fill(panel: Panel) -> Option<Color> {
    match panel {
        Panel::Mining => Some(tone::MINED),
        Panel::Staking => Some(tone::STAKED),
        _ => None,
    }
}

/// Draws the panels. `marks_only` draws the data of the panels and nothing more: the lines
/// and the dots, with no bands, no grid, no names and no cursor. The light uses that picture,
/// so that light is on the chart elements only.
fn draw_panels(snapshot: &gtk::Snapshot, widget: &impl IsA<gtk::Widget>, width: f32, input: &Input, model: &StackModel, marks_only: bool) {
    let (left, right) = plot_frame(width);
    let x = |time: f64| model.x(time, left, right);
    let bottom = rect(Panel::Pool, width).y() + PANEL;

    if !marks_only {
        // The longest waits of the range, through all panels.
        for (start, end) in &model.long_waits {
            let x0 = x(*start);
            let x1 = x(*end).max(x0 + 1.0);
            draw::fill_rect(snapshot, x0, HEADER, x1 - x0, bottom - HEADER, tone::VALUE.opacity(0.09));
        }
        // Time grid and its labels.
        for tick in &model.time_ticks {
            let tx = x(*tick);
            draw::rule(snapshot, tx, HEADER, tx, bottom, tone::GRID, 1.0, &[]);
            draw::rule(snapshot, tx, bottom, tx, bottom + 3.0, tone::AXIS, 1.0, &[]);
            draw::name(snapshot, widget, &chart_format::axis_time(*tick as u64, input.span), tx, bottom + 11.0, Anchor::Center, tone::LABEL);
        }
    }

    for panel in Panel::ALL {
        let bx = rect(panel, width);
        let (low, high) = model.range(panel);
        let (min_y, max_y) = (bx.y(), bx.y() + bx.height());
        let y = |value: f64| max_y - ((model.scaled(value, panel) - low) / (high - low)) as f32 * bx.height();
        if !marks_only {
            // The name on a plate, and the answer, above the panel.
            let plate_width = draw::text_size(widget, font::PLATE, panel.title()).0 + 14.0;
            draw::plate(snapshot, widget, panel.title(), LEFT, min_y - 10.0, plate_width, plate_fill(panel));
            draw::text(
                snapshot,
                widget,
                font::CAPTION,
                &model.answer(panel),
                LEFT + plate_width + 10.0,
                min_y - 10.0,
                Anchor::Leading,
                if model.signal(panel) { tone::ALERT } else { tone::LABEL },
            );
            draw::stroke(snapshot, &draw::rect_path(bx.x(), bx.y(), bx.width(), bx.height()), tone::RULE, 1.0);
            for tick in model.ticks(panel) {
                let ty = y(tick.value);
                if ty < min_y - 0.5 || ty > max_y + 0.5 {
                    continue;
                }
                draw::rule(snapshot, bx.x(), ty, bx.x() + bx.width(), ty, tone::GRID, 1.0, &[]);
                draw::rule(snapshot, bx.x() - 3.0, ty, bx.x(), ty, tone::AXIS, 1.0, &[]);
                draw::name(snapshot, widget, &tick.label, bx.x() - 7.0, ty, Anchor::Trailing, tone::LABEL);
            }
        }
        snapshot.push_clip(&bx);
        match panel {
            Panel::Rate => draw_rate(snapshot, &bx, model, &x, &y, marks_only),
            Panel::Mining => draw_difficulty(snapshot, &bx, &model.mining, &model.mining_average, tone::MINED, &x, &y),
            Panel::Staking => draw_difficulty(snapshot, &bx, &model.staking, &model.staking_average, tone::STAKED, &x, &y),
            Panel::Share => draw_share(snapshot, &bx, model, &x, &y, marks_only),
            Panel::Pool => draw_pool(snapshot, widget, &bx, input, model, &x, &y, marks_only),
        }
        snapshot.pop();
    }
}

fn draw_rate(snapshot: &gtk::Snapshot, bx: &graphene::Rect, model: &StackModel, x: &dyn Fn(f64) -> f32, y: &dyn Fn(f64) -> f32, marks_only: bool) {
    let (min_x, max_x) = (bx.x(), bx.x() + bx.width());
    if !marks_only {
        if let Some(limits) = model.rate_limits {
            draw::fill_rect(snapshot, min_x, y(limits.high), bx.width(), y(limits.low) - y(limits.high), tone::RECORD.opacity(0.1));
            draw::rule(snapshot, min_x, y(limits.mean), max_x, y(limits.mean), tone::RECORD.opacity(0.6), 1.0, &[]);
        }
        draw::rule(snapshot, min_x, y(model.rate_target), max_x, y(model.rate_target), tone::VALUE.opacity(0.6), 1.0, &[4.0, 3.0]);
    }
    let points: Vec<(f32, f32)> = model.buckets.iter().map(|b| (x((b.start + b.end) / 2.0), y(b.count as f64))).collect();
    let joined: Vec<Option<(f32, f32)>> = points.iter().copied().map(Some).collect();
    draw::stroke(snapshot, &line(&joined), tone::VALUE, 1.0);
    for (bucket, (px, py)) in model.buckets.iter().zip(points) {
        let radius = if bucket.flag == Flag::None { 2.2 } else { 3.5 };
        let color = match bucket.flag {
            Flag::Outside => tone::ALERT,
            Flag::Run => tone::MINED,
            Flag::None => tone::VALUE,
        };
        draw::fill(snapshot, &dots(std::iter::once((px, py)), radius), color);
    }
}

/// Dots for each block of the kind, or the range of each column when the dots would merge,
/// with Carbide's trailing MA100.
fn draw_difficulty(
    snapshot: &gtk::Snapshot,
    bx: &graphene::Rect,
    points: &[Point],
    average: &[Option<Point>],
    color: Color,
    x: &dyn Fn(f64) -> f32,
    y: &dyn Fn(f64) -> f32,
) {
    if points.len() as f32 > bx.width() {
        draw::fill(snapshot, &envelope(points, bx, x, y), color.opacity(0.45));
    } else {
        draw::fill(snapshot, &dots(points.iter().map(|p| (x(p.time), y(p.value))), 1.4), color.opacity(0.8));
    }
    let averages: Vec<Option<(f32, f32)>> = average.iter().map(|p| p.map(|p| (x(p.time), y(p.value)))).collect();
    draw::stroke(snapshot, &line(&averages), tone::VALUE, 1.0);
}

/// The lowest and highest value of each two-point column, so that no peak is lost.
fn envelope(points: &[Point], bx: &graphene::Rect, x: &dyn Fn(f64) -> f32, y: &dyn Fn(f64) -> f32) -> gsk::Path {
    let mut columns: std::collections::BTreeMap<i64, (f64, f64)> = Default::default();
    for point in points {
        let column = ((x(point.time) - bx.x()) / 2.0) as i64;
        let entry = columns.entry(column).or_insert((point.value, point.value));
        entry.0 = entry.0.min(point.value);
        entry.1 = entry.1.max(point.value);
    }
    let builder = gsk::PathBuilder::new();
    for (column, (low, high)) in columns {
        let top = y(high);
        let bottom = y(low).max(top + 1.5);
        builder.add_rect(&draw::rect(bx.x() + column as f32 * 2.0, top, 2.0, bottom - top));
    }
    builder.to_path()
}

fn draw_share(snapshot: &gtk::Snapshot, bx: &graphene::Rect, model: &StackModel, x: &dyn Fn(f64) -> f32, y: &dyn Fn(f64) -> f32, marks_only: bool) {
    if !marks_only {
        draw::fill_rect(snapshot, bx.x(), y(CHANCE_HIGH), bx.width(), y(CHANCE_LOW) - y(CHANCE_HIGH), tone::RECORD.opacity(0.1));
        draw::rule(snapshot, bx.x(), y(0.5), bx.x() + bx.width(), y(0.5), tone::VALUE.opacity(0.6), 1.0, &[4.0, 3.0]);
    }
    let path = |list: &[Option<Point>]| line(&list.iter().map(|p| p.map(|p| (x(p.time), y(p.value)))).collect::<Vec<_>>());
    draw::stroke(snapshot, &path(&model.share1000), tone::RECORD, 1.0);
    draw::stroke(snapshot, &path(&model.share100), tone::STAKED, 1.0);
}

#[allow(clippy::too_many_arguments)]
fn draw_pool(
    snapshot: &gtk::Snapshot,
    widget: &impl IsA<gtk::Widget>,
    bx: &graphene::Rect,
    input: &Input,
    model: &StackModel,
    x: &dyn Fn(f64) -> f32,
    y: &dyn Fn(f64) -> f32,
    marks_only: bool,
) {
    let max_y = bx.y() + bx.height();
    if model.pool.is_empty() {
        if marks_only {
            return;
        }
        let text = if input.fee_loading { "Reading fee pool checkpoints" } else { "No fee pool checkpoint is loaded for this range." };
        draw::text(snapshot, widget, font::CAPTION, text, bx.x() + bx.width() / 2.0, bx.y() + bx.height() / 2.0, Anchor::Center, tone::LABEL);
        return;
    }
    let points: Vec<Option<(f32, f32)>> = model.pool.iter().map(|p| Some((x(p.time), y(p.value)))).collect();
    if !marks_only
        && input.scale != Scale::Fit
        && let (Some(Some(first)), Some(Some(last))) = (points.first(), points.last())
    {
        let builder = gsk::PathBuilder::new();
        builder.move_to(first.0, first.1);
        for (px, py) in points.iter().flatten().skip(1) {
            builder.line_to(*px, *py);
        }
        builder.line_to(last.0, max_y);
        builder.line_to(first.0, max_y);
        builder.close();
        draw::fill(snapshot, &builder.to_path(), tone::RECORD.opacity(0.08));
    }
    draw::stroke(snapshot, &line(&points), tone::RECORD, 1.0);
    // A tick for each block where the pool rose. Known only between neighbouring blocks.
    for time in &model.pool_rises {
        let tx = x(*time);
        draw::rule(snapshot, tx, bx.y(), tx, bx.y() + 6.0, tone::VALUE.opacity(0.75), 1.0, &[]);
    }
    // The second currency of a PBaaS chain, on its own range.
    let top = model.reserve.iter().map(|p| p.value).fold(f64::NEG_INFINITY, f64::max);
    if !model.reserve.is_empty() && top > 0.0 {
        let reserve: Vec<Option<(f32, f32)>> =
            model.reserve.iter().map(|p| Some((x(p.time), max_y - (p.value / (top * 1.08)) as f32 * bx.height()))).collect();
        draw::stroke(snapshot, &line(&reserve), tone::GOOD, 1.0);
    }
}

/// The picture of the data with more light, made once for each range and width: the lines
/// and the dots only, brighter, with the light of a phosphor around them. The strong picture
/// has two lights: one near the line and one wide.
fn picture(widget: &impl IsA<gtk::Widget>, width: f32, input: &Input, model: &StackModel, strong: bool) -> Option<gdk::Texture> {
    let renderer = widget.as_ref().native()?.renderer()?;
    let scale = widget.as_ref().scale_factor().max(1) as f32;
    let snapshot = gtk::Snapshot::new();
    snapshot.scale(scale, scale);
    let shadows = if strong {
        vec![
            gsk::Shadow::new(tone::PRESENT.opacity(0.95).rgba(), 0.0, 0.0, 3.0),
            gsk::Shadow::new(tone::PRESENT.opacity(0.6).rgba(), 0.0, 0.0, 8.0),
        ]
    } else {
        vec![gsk::Shadow::new(tone::PRESENT.opacity(0.55).rgba(), 0.0, 0.0, 2.5)]
    };
    snapshot.push_shadow(&shadows);
    let brightness = if strong { 0.6 } else { 0.22 };
    snapshot.push_color_matrix(&graphene::Matrix::new_identity(), &graphene::Vec4::new(brightness, brightness, brightness, 0.0));
    draw_panels(&snapshot, widget, width, input, model, true);
    snapshot.pop();
    snapshot.pop();
    let node = snapshot.to_node()?;
    let viewport = draw::rect(0.0, 0.0, width * scale, PLOT_HEIGHT * scale);
    Some(renderer.render_texture(node, Some(&viewport)))
}

impl ChartStack {
    pub fn new(select: impl Fn(&str) + 'static, open: impl Fn(&str) + 'static) -> Rc<Self> {
        let shared = Rc::new(RefCell::new(Shared { input: None, model: None, hovered: None, pointer: None, pictures: None, pictures_pending: false }));

        let s = shared.clone();
        let base = Canvas::new(0, PLOT_HEIGHT as i32, move |canvas, snapshot, w, _h| {
            let shared = s.borrow();
            if let (Some(input), Some(model)) = (&shared.input, &shared.model) {
                draw_panels(snapshot, canvas, w, input, model, false);
            }
        });
        base.set_hexpand(true);

        let s = shared.clone();
        let cursors = Canvas::new(0, 0, move |_, snapshot, w, _h| {
            let shared = s.borrow();
            let (Some(input), Some(model)) = (&shared.input, &shared.model) else { return };
            let count = input.page.samples.len();
            let selected_index = input.selected.as_ref().and_then(|h| model.index.get(h).copied());
            let (left, right) = plot_frame(w);
            let bottom = rect(Panel::Pool, w).y() + PANEL;
            let at = |index: usize| model.x(input.page.samples[index].time as f64, left, right);
            // The selected block is a line of the present with a little light. The block
            // under the pointer is a quieter line.
            if let Some(hovered) = shared.hovered
                && Some(hovered) != selected_index
                && hovered < count
            {
                let cx = at(hovered);
                draw::rule(snapshot, cx, HEADER, cx, bottom, tone::PRESENT.opacity(0.45), 1.0, &[]);
            }
            let strong = selected_index.or(if shared.hovered.is_none() && count > 0 { Some(count - 1) } else { None });
            if let Some(index) = strong.filter(|i| *i < count) {
                let cx = at(index);
                draw::lit(snapshot, &draw::rect_path(cx - 0.75, HEADER, 1.5, bottom - HEADER), tone::PRESENT, 0.6);
            }
        });
        cursors.set_can_target(false);

        let light = ChartLight::new();

        let plot = gtk::Overlay::new();
        plot.set_child(Some(&base));
        plot.add_overlay(&light.canvas);
        plot.add_overlay(&cursors);
        plot.set_margin_top(8);
        plot.set_margin_bottom(4);
        plot.update_property(&[gtk::accessible::Property::Label(
            "Block rate, mining difficulty, staking difficulty, staked share and fee pool on one time axis",
        )]);

        let readout = gtk::Box::new(gtk::Orientation::Vertical, 7);
        readout.set_margin_start(18);
        readout.set_margin_end(18);
        readout.set_margin_top(14);
        readout.set_margin_bottom(10);
        readout.set_size_request(-1, 40);

        let waits = WaitsStrip::new();

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&readout);
        column.append(&display::rule());
        column.append(&plot);
        column.append(&waits.root);
        let root = display::panel(&column, 0);
        root.set_focusable(true);

        let stack = Rc::new(Self {
            root,
            shared,
            base,
            cursors,
            light,
            readout,
            waits,
            select: Rc::new(select),
            open: Rc::new(open),
        });

        // The pointer: the block under it, and the light it leaves.
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&stack);
        motion.connect_motion(move |_, x, _| {
            if let Some(stack) = weak.upgrade() {
                stack.hover_at(Some(x as f32));
            }
        });
        let weak = Rc::downgrade(&stack);
        motion.connect_leave(move |_| {
            if let Some(stack) = weak.upgrade() {
                stack.hover_at(None);
            }
        });
        stack.base.add_controller(motion);

        // A click selects the nearest block. A wave of light goes out from it.
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&stack);
        click.connect_released(move |_, _, x, _| {
            let Some(stack) = weak.upgrade() else { return };
            stack.root.grab_focus();
            let width = stack.base.width() as f32;
            let found = {
                let shared = stack.shared.borrow();
                let (Some(input), Some(model)) = (&shared.input, &shared.model) else { return };
                let (left, right) = plot_frame(width);
                model
                    .nearest(x as f32, left, right)
                    .filter(|i| *i < input.page.samples.len())
                    .map(|i| (input.page.samples[i].hash.clone(), model.x(input.page.samples[i].time as f64, left, right)))
            };
            if let Some((hash, cx)) = found {
                (stack.select)(&hash);
                stack.light.wave(cx);
            }
        });
        stack.base.add_controller(click);

        // A new width needs new pictures of the light.
        let weak = Rc::downgrade(&stack);
        stack.base.add_tick_callback(move |canvas, _| {
            let Some(stack) = weak.upgrade() else { return glib::ControlFlow::Break };
            let width = canvas.width();
            let stale = {
                let shared = stack.shared.borrow();
                let key = shared.input.as_ref().map(|i| i.key.clone());
                key.is_some() && !shared.pictures_pending && shared.pictures.as_ref().map(|(k, w)| (Some(k.clone()), *w)) != Some((key, width))
            };
            if stale && width > 0 {
                stack.schedule_pictures();
            }
            glib::ControlFlow::Continue
        });
        stack
    }

    /// New data from the page. The model is made again only when the range, the span or the
    /// fee pool's points changed.
    pub fn set(&self, page: &Rc<ChartPage>, span: u64, fee: &FeePoolState, coin: &str, selected: Option<String>) {
        let samples = &page.samples;
        let key = format!(
            "{}|{}|{}|{}|{}|{:?}|{:?}|{}",
            samples.len(),
            samples.first().map(|s| s.hash.as_str()).unwrap_or(""),
            samples.last().map(|s| s.hash.as_str()).unwrap_or(""),
            span,
            fee.checkpoints.len(),
            fee.scale,
            fee.currency,
            coin
        );
        let rebuilt = {
            let mut shared = self.shared.borrow_mut();
            let old_key = shared.input.as_ref().map(|i| i.key.clone());
            let fee_loading_changed = shared.input.as_ref().is_some_and(|i| i.fee_loading != fee.loading);
            let rebuilt = old_key.as_deref() != Some(key.as_str());
            if rebuilt {
                shared.model = Some(Rc::new(StackModel::new(samples, span, fee, coin)));
                shared.hovered = None;
            }
            shared.input = Some(Input {
                page: page.clone(),
                span,
                fee_loading: fee.loading,
                scale: fee.scale,
                coin: coin.to_string(),
                selected,
                key,
            });
            rebuilt || fee_loading_changed
        };
        if rebuilt {
            self.base.queue_draw();
            // The pointer has not moved: the block under it is the hovered block again.
            let pointer = self.shared.borrow().pointer;
            if pointer.is_some() {
                self.hover_at(pointer);
            }
        }
        self.cursors.queue_draw();
        self.update_readout();
    }

    /// Draws the panels again, for a change of a display setting such as UTC.
    pub fn redraw(&self) {
        self.base.queue_draw();
        self.update_readout();
    }

    /// Makes the pictures of the light a moment after the range or the width changed, so
    /// that a series of changes makes them once.
    fn schedule_pictures(self: &Rc<Self>) {
        self.shared.borrow_mut().pictures_pending = true;
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            let Some(stack) = weak.upgrade() else { return };
            stack.shared.borrow_mut().pictures_pending = false;
            let width = stack.base.width();
            let made = {
                let shared = stack.shared.borrow();
                let (Some(input), Some(model)) = (&shared.input, &shared.model) else { return };
                if width <= 0 {
                    return;
                }
                let w = width as f32;
                let lit = picture(&stack.base, w, input, model, false);
                let hot = picture(&stack.base, w, input, model, true);
                (lit, hot, input.key.clone())
            };
            let (lit, hot, key) = made;
            let (left, right) = plot_frame(width as f32);
            stack.light.set_pictures(lit, hot, (width as f32, PLOT_HEIGHT), left, right);
            stack.shared.borrow_mut().pictures = Some((key, width));
        });
    }

    fn hover_at(&self, x: Option<f32>) {
        self.shared.borrow_mut().pointer = x;
        let width = self.base.width() as f32;
        let (hovered, place) = {
            let shared = self.shared.borrow();
            match (x, &shared.model, &shared.input) {
                (Some(x), Some(model), Some(input)) => {
                    let (left, right) = plot_frame(width);
                    let index = model.nearest(x, left, right).filter(|i| *i < input.page.samples.len());
                    (index, index.map(|i| model.x(input.page.samples[i].time as f64, left, right)))
                }
                _ => (None, None),
            }
        };
        self.light.pointer(place);
        if self.shared.borrow().hovered == hovered {
            return;
        }
        self.shared.borrow_mut().hovered = hovered;
        self.cursors.queue_draw();
        self.update_readout();
    }

    /// The block under the pointer, or the selected block.
    fn shown_index(shared: &Shared) -> Option<usize> {
        if let Some(hovered) = shared.hovered {
            return Some(hovered);
        }
        let input = shared.input.as_ref()?;
        let last = if input.page.samples.is_empty() { None } else { Some(input.page.samples.len() - 1) };
        let Some(selected) = &input.selected else { return last };
        shared.model.as_ref().and_then(|m| m.index.get(selected).copied()).or(last)
    }

    /// Each value at the block under the cursor, above the panels and not below the page.
    fn update_readout(&self) {
        clear(&self.readout);
        let shared = self.shared.borrow();
        let index = Self::shown_index(&shared);
        let (Some(input), Some(model), Some(index)) = (&shared.input, &shared.model, index) else {
            self.readout.append(&theme::label(font::CAPTION, " ", ""));
            self.readout.append(&theme::label(font::CAPTION, " ", ""));
            self.waits.set(None, None);
            return;
        };
        if index >= input.page.samples.len() || !model.has(index) {
            self.readout.append(&theme::label(font::CAPTION, " ", ""));
            self.readout.append(&theme::label(font::CAPTION, " ", ""));
            self.waits.set(Some(model.clone()), None);
            return;
        }
        let sample = &input.page.samples[index];
        let first = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let open = self.open.clone();
        let hash = sample.hash.clone();
        let block = link::link_in(font::FIGURE_SMALL, &format!("Block {}", format::integer(sample.height)), move || open(&hash));
        block.set_tooltip_text(Some("Open this block"));
        first.append(&block);
        let fill = match sample.production {
            BlockProduction::Staked => PlateFill::Staked,
            BlockProduction::Mined => PlateFill::Mined,
            BlockProduction::Unknown => PlateFill::None,
        };
        let plate = display::plate(chart_format::production(sample.production), fill);
        plate.set_valign(gtk::Align::Center);
        first.append(&plate);
        let date = theme::label(font::CAPTION, &format::date(sample.time), "value");
        date.set_valign(gtk::Align::Baseline);
        first.append(&date);
        if let Some(interval) = sample.interval {
            let after = theme::label(font::CAPTION, &format!("· {} after the block before", chart_format::signed_seconds(interval)), "label");
            after.set_valign(gtk::Align::Baseline);
            first.append(&after);
        }
        if shared.hovered.is_some() {
            let hint = theme::label(font::CAPTION, "· click to select", "label");
            hint.set_opacity(0.7);
            hint.set_valign(gtk::Align::Baseline);
            first.append(&hint);
        }
        self.readout.append(&first);

        let second = gtk::Box::new(gtk::Orientation::Horizontal, 20);
        second.append(&figure("Rate", &model.rate_text(index), "value"));
        second.append(&figure("Mining", &model.mining_at[index].map(format::difficulty).unwrap_or_else(|| "–".into()), "mined"));
        second.append(&figure("Staking", &model.staking_at[index].map(format::difficulty).unwrap_or_else(|| "–".into()), "staked"));
        second.append(&figure("Staked share", &sample.share100.map(format::percent).unwrap_or_else(|| "–".into()), "value"));
        second.append(&figure("Fee pool", &model.pool_text(index), "value"));
        self.readout.append(&second);

        // The wait that ended with the shown block, when that block started a burst.
        let wait = sample.interval.filter(|i| *i > BURST_SECONDS).map(|i| i as f64);
        self.waits.set(Some(model.clone()), wait);
    }
}

/// A name and its figure at the shown block: "MINING 1.23T".
fn figure(label: &str, value: &str, class: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = display::name_text(label);
    name.set_valign(gtk::Align::Baseline);
    let figure = theme::label(font::FIGURE_SMALL, value, class);
    figure.set_valign(gtk::Align::Baseline);
    row.append(&name);
    row.append(&figure);
    row
}

// MARK: - Waits

/// The waits between bursts in the range, in the same form as the dashboard's chain pulse.
/// Each dot is an equal share of the waits. The marker is the wait before the shown block.
struct WaitsStrip {
    root: gtk::Box,
    canvas: Canvas,
    caption: gtk::Label,
    shown: Rc<RefCell<(Option<Rc<StackModel>>, Option<f64>)>>,
    last: Cell<(usize, Option<i64>)>,
}

impl WaitsStrip {
    fn new() -> Self {
        let shown: Rc<RefCell<(Option<Rc<StackModel>>, Option<f64>)>> = Rc::default();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.append(&display::rule());
        let plate = display::plate("Waits between bursts in this range", PlateFill::None);
        plate.set_margin_start(LEFT as i32);
        plate.set_margin_top(12);
        root.append(&plate);
        let s = shown.clone();
        // The canvas has the full width, so that its axis has the insets of the panels above.
        let canvas = Canvas::new(0, 62, move |canvas, snapshot, w, h| {
            let shown = s.borrow();
            if let (Some(model), wait) = (&shown.0, shown.1) {
                draw_waits(snapshot, canvas, w, h, model, wait);
            }
        });
        canvas.set_hexpand(true);
        canvas.update_property(&[gtk::accessible::Property::Label("Dot plot of the waits between bursts in this range")]);
        root.append(&canvas);
        let caption = theme::label(font::CAPTION, "", "label");
        caption.set_wrap(true);
        caption.set_margin_start(18);
        caption.set_margin_end(18);
        caption.set_margin_top(4);
        caption.set_margin_bottom(10);
        root.append(&caption);
        root.set_visible(false);
        Self { root, canvas, caption, shown, last: Cell::new((usize::MAX, None)) }
    }

    fn set(&self, model: Option<Rc<StackModel>>, wait: Option<f64>) {
        let Some(model) = model.filter(|m| m.waits.len() >= 20) else {
            self.root.set_visible(false);
            *self.shown.borrow_mut() = (None, None);
            self.last.set((usize::MAX, None));
            return;
        };
        let identity = Rc::as_ptr(&model) as usize;
        let signature = (identity, wait.map(|w| w as i64));
        self.root.set_visible(true);
        if self.last.get() == signature {
            return;
        }
        self.last.set(signature);
        let count = model.waits.len();
        let mut text = if count <= 100 {
            format!("Each dot is one of the {count} waits. ")
        } else {
            format!("Each dot is 1 in 100 of the {} waits. ", format::integer(count as u64))
        };
        text += &format!("Blocks within {BURST_SECONDS} seconds of each other are one burst. ");
        text += &match wait {
            Some(wait) => format!(
                "The line is the wait before the shown block: {}, longer than {} of 100.",
                chart_format::duration(wait as i64),
                share_shorter(&model.waits, wait)
            ),
            None => "The shown block followed its parent within the burst time.".into(),
        };
        theme::set_label(&self.caption, font::CAPTION, &text);
        *self.shown.borrow_mut() = (Some(model), wait);
        self.canvas.queue_draw();
    }
}

/// How many of 100 waits in the range are shorter than this one.
fn share_shorter(waits: &[f64], seconds: f64) -> i64 {
    let shorter = partition_index(waits, |w| w >= seconds);
    (shorter as f64 / waits.len().max(1) as f64 * 100.0).round() as i64
}

fn draw_waits(snapshot: &gtk::Snapshot, widget: &impl IsA<gtk::Widget>, width: f32, height: f32, model: &StackModel, wait: Option<f64>) {
    let right = width - RIGHT;
    let base = height - 20.0;
    let dot = 6.0f32;
    let low = 10.0f64;
    let high = 1_800f64.max(model.waits.last().copied().unwrap_or(0.0) * 1.15);
    let position = |seconds: f64| LEFT + ((seconds.max(low).ln() - low.ln()) / (high.ln() - low.ln())) as f32 * (right - LEFT);
    draw::rule(snapshot, LEFT, base, right, base, tone::AXIS, 1.0, &[]);
    let ticks = [
        (10.0, "10 s"),
        (30.0, "30 s"),
        (60.0, "1 min"),
        (120.0, "2 min"),
        (300.0, "5 min"),
        (600.0, "10 min"),
        (1200.0, "20 min"),
        (3600.0, "1 h"),
    ];
    for (seconds, label) in ticks {
        if seconds > high {
            continue;
        }
        let tx = position(seconds);
        draw::rule(snapshot, tx, base, tx, base + 4.0, tone::AXIS, 1.0, &[]);
        draw::name(snapshot, widget, label, tx, base + 12.0, Anchor::Center, tone::LABEL);
    }
    let count = model.waits.len().min(100);
    let mut stacks: std::collections::HashMap<i64, i32> = Default::default();
    // The waits shorter than the shown wait have light, as in the chain pulse.
    let shorter_dots = gsk::PathBuilder::new();
    let longer_dots = gsk::PathBuilder::new();
    for index in 0..count {
        let rank = ((index as f64 + 0.5) / count as f64 * model.waits.len() as f64) as usize;
        let value = model.waits[rank.min(model.waits.len() - 1)];
        let bin = ((position(value) - LEFT) / (dot + 1.0)) as i64;
        let level = *stacks.get(&bin).unwrap_or(&0);
        stacks.insert(bin, level + 1);
        let cx = LEFT + (bin as f32 + 0.5) * (dot + 1.0);
        let cy = base - 2.0 - dot / 2.0 - level as f32 * (dot + 1.0);
        let shorter = wait.is_none_or(|w| value < w);
        let builder = if shorter { &shorter_dots } else { &longer_dots };
        builder.add_circle(&draw::point(cx, cy), dot / 2.0);
    }
    draw::fill(snapshot, &longer_dots.to_path(), tone::AXIS);
    draw::lit(snapshot, &shorter_dots.to_path(), tone::RECORD, 0.4);
    if let Some(wait) = wait {
        let cx = position(wait).min(right);
        draw::lit(snapshot, &draw::rect_path(cx - 0.75, 2.0, 1.5, base - 2.0), tone::PRESENT, 0.6);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry() {
        assert_eq!(PLOT_HEIGHT, 598.0);
        let r = rect(Panel::Pool, 1000.0);
        assert_eq!((r.x(), r.y(), r.width(), r.height()), (66.0, 486.0, 912.0, 84.0));
        assert_eq!(share_shorter(&[10.0, 20.0, 30.0, 40.0], 30.0), 50);
    }
}
