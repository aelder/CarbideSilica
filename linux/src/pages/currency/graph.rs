//! The two pictures of the notarization browser (`NotarizationPicture.swift`): the rhythm
//! of earlier intervals with the wait now (`NotarizationRhythm`), and the graph of entries
//! (`NotarizationGraph`). Neither moves: each redraws only when its data or a name changes.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gsk, pango};

use super::picture::{self, LinkState, NotarizationMark, RhythmScale};
use crate::format;
use crate::theme::draw::{self, Anchor};
use crate::theme::{font, tone, Color};
use crate::widgets::canvas::Canvas;

/// The intervals between earlier notarizations as dots on one scale, and the wait now as
/// one mark. Each step to the right on the scale is ten times more.
pub fn rhythm(intervals: Vec<u64>, wait: Option<u64>, state: Option<LinkState>) -> Canvas {
    let spoken = rhythm_spoken(&intervals, wait);
    let canvas = Canvas::new(0, 112, move |widget, snapshot, w, h| {
        let scale = RhythmScale::new(w, &intervals, wait);
        let (left, right) = (scale.left, scale.right);
        let base = h - 22.0;
        draw::plate(snapshot, widget, "Intervals", 14.0, base - 12.0, 88.0, None);
        draw::rule(snapshot, left, base, right, base, tone::AXIS, 1.0, &[]);
        for power in 0..=scale.decades {
            let value = 10u64.pow(power);
            let at = scale.x(value);
            draw::rule(snapshot, at, base, at, base + 4.0, tone::AXIS, 1.0, &[]);
            let words = format!("{} {}", format::integer(value), if value == 1 { "block" } else { "blocks" });
            draw::name(snapshot, widget, &words, at, base + 13.0, Anchor::Center, tone::LABEL);
        }
        // The dots, in columns.
        let builder = gsk::PathBuilder::new();
        for (x, y) in scale.dots(&intervals, base) {
            builder.add_circle(&draw::point(x, y), 2.3);
        }
        draw::fill(snapshot, &builder.to_path(), tone::RECORD);
        if let Some(usual) = picture::median(&intervals) {
            let at = scale.x(usual);
            draw::rule(snapshot, at, base - 2.0, at, base + 4.0, tone::LABEL, 1.0, &[]);
            let words = format!("usual {} blocks", format::integer(usual));
            draw::name(snapshot, widget, &words, at.max(left + 60.0), 8.0, Anchor::Center, tone::LABEL);
        }
        if let Some(wait) = wait {
            let at = scale.x(wait);
            let color = state.map(LinkState::color).unwrap_or(tone::PRESENT);
            draw::lit(snapshot, &draw::rect_path(at - 0.75, 18.0, 1.5, base - 18.0), color, 0.7);
            let on_left = at > w / 2.0;
            let side = if on_left { Anchor::Trailing } else { Anchor::Leading };
            let text_x = at + if on_left { -8.0 } else { 8.0 };
            draw::name(snapshot, widget, "now", text_x, 20.0, side, color);
            draw::figure(snapshot, widget, &format!("{} blocks", format::integer(wait)), text_x, 34.0, side, color);
        }
    });
    canvas.set_hexpand(true);
    canvas.update_property(&[gtk::accessible::Property::Label(&spoken)]);
    canvas
}

/// The text that a screen reader says for the rhythm.
pub fn rhythm_spoken(intervals: &[u64], wait: Option<u64>) -> String {
    let mut text = format!("{} intervals between earlier notarizations.", intervals.len());
    if let (Some(usual), Some(longest)) = (picture::median(intervals), intervals.iter().max()) {
        text += &format!(
            " The usual interval is {} blocks. The longest is {}.",
            format::integer(usual),
            format::integer(*longest)
        );
    }
    if let Some(wait) = wait {
        text += &format!(" The wait now is {} blocks.", format::integer(wait));
    }
    text
}

/// What the graph draws. `notes` has the text below each mark, in the order of `marks`.
pub struct GraphData {
    pub marks: Vec<NotarizationMark>,
    pub observer: String,
    pub other: String,
    pub notes: Vec<String>,
    pub selected: Option<u32>,
}

/// The entries of a view as a graph. The best chain is the straight line. A side branch is
/// a second line that leaves the first at one mark. A click on a mark selects it, and
/// `chosen` gets the new selection.
pub struct Graph {
    pub root: gtk::ScrolledWindow,
    pub canvas: Canvas,
    pub data: Rc<RefCell<GraphData>>,
}

const MARK_SIZE: f32 = 20.0;

/// Futura 10 bold, as the count of a mark.
const COUNT: font::Role = font::Role { family: font::DISPLAY, size: 10.0, weight: 700, tracking: 0.0, uppercase: false, tabular: true };

impl Graph {
    pub fn new(data: GraphData, chosen: impl Fn(Option<u32>) + 'static) -> Self {
        let lanes = data.marks.iter().map(|m| m.lane).max().unwrap_or(0) + 1;
        let columns = data.marks.iter().map(|m| m.column).max().unwrap_or(0) + 1;
        let height = picture::graph_height(lanes);
        let data = Rc::new(RefCell::new(data));
        let shared = data.clone();
        let canvas = Canvas::new(picture::least_width(columns) as i32, height.ceil() as i32, move |widget, snapshot, w, _| {
            paint(widget, snapshot, &shared.borrow(), w, lanes, columns);
        });
        canvas.set_hexpand(true);
        // A click on a mark selects it, or clears the selection when it is selected.
        let click = gtk::GestureClick::new();
        {
            let (data, weak) = (data.clone(), canvas.downgrade());
            let chosen = Rc::new(chosen);
            click.connect_released(move |_, _, x, y| {
                let Some(canvas) = weak.upgrade() else { return };
                let hit = hit(&data.borrow(), canvas.width() as f32, columns, x as f32, y as f32);
                let Some(id) = hit else { return };
                let next = {
                    let mut data = data.borrow_mut();
                    data.selected = if data.selected == Some(id) { None } else { Some(id) };
                    data.selected
                };
                canvas.queue_draw();
                chosen(next);
            });
        }
        canvas.add_controller(click);
        // The help of a mark.
        canvas.set_has_tooltip(true);
        {
            let data = data.clone();
            canvas.connect_query_tooltip(move |canvas, x, y, _, tooltip| {
                let found = hit(&data.borrow(), canvas.width() as f32, columns, x as f32, y as f32);
                match found {
                    Some(_) => {
                        tooltip.set_text(Some("Shows the rows of this mark"));
                        true
                    }
                    None => false,
                }
            });
        }
        let root = gtk::ScrolledWindow::new();
        root.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        root.set_child(Some(&canvas));
        root.set_size_request(-1, height.ceil() as i32);
        root.set_hexpand(true);
        Self { root, canvas, data }
    }

    /// Redraws with new texts, such as after names arrive.
    pub fn update(&self, observer: String, other: String, notes: Vec<String>) {
        {
            let mut data = self.data.borrow_mut();
            data.observer = observer;
            data.other = other;
            data.notes = notes;
        }
        self.canvas.queue_draw();
    }
}

/// The mark under a point, within 6 of its edge.
fn hit(data: &GraphData, width: f32, columns: usize, x: f32, y: f32) -> Option<u32> {
    let step = picture::step(width, columns);
    data.marks
        .iter()
        .find(|mark| {
            let (px, py) = picture::point(mark, step);
            (px - x).hypot(py - y) <= MARK_SIZE / 2.0 + 6.0
        })
        .map(|mark| mark.id())
}

fn paint(widget: &Canvas, snapshot: &gtk::Snapshot, data: &GraphData, width: f32, lanes: usize, columns: usize) {
    let step = picture::step(width, columns);
    for lane in 0..lanes {
        draw::plate(snapshot, widget, &picture::lane_title(lane, lanes), 14.0, picture::lane_y(lane), 88.0, None);
    }
    // The lines from each mark to the mark before it.
    for mark in &data.marks {
        let Some(parent) = data.marks.iter().find(|m| Some(m.id()) == mark.parent) else { continue };
        let (from, to) = (picture::point(parent, step), picture::point(mark, step));
        let builder = gsk::PathBuilder::new();
        builder.move_to(from.0, from.1);
        if from.1 == to.1 {
            builder.line_to(to.0, to.1);
        } else {
            builder.cubic_to(from.0 + 30.0, from.1, to.0 - 40.0, to.1, to.0, to.1);
        }
        let stroke = gsk::Stroke::new(1.0);
        stroke.set_dash(&[5.0, 4.0]);
        let color = if mark.lane == 0 { tone::AXIS } else { tone::MINED };
        snapshot.append_stroke(&builder.to_path(), &stroke, &color.rgba());
    }
    for (position, mark) in data.marks.iter().enumerate() {
        let (x, y) = picture::point(mark, step);
        paint_mark(widget, snapshot, mark, data.selected == Some(mark.id()), x, y);
        let note = data.notes.get(position).map(String::as_str).unwrap_or("");
        let label_y = picture::lane_y(mark.lane) + if mark.lane == 0 { -42.0 } else { 40.0 };
        paint_labels(widget, snapshot, mark, &data.observer, &data.other, note, x, label_y, step - 8.0);
    }
}

/// Confirmed is solid green. A waiting mark is an outline: white on the best chain, amber
/// on a side branch.
fn paint_mark(widget: &Canvas, snapshot: &gtk::Snapshot, mark: &NotarizationMark, chosen: bool, x: f32, y: f32) {
    let color = if mark.confirmed() {
        tone::GOOD
    } else if mark.lane == 0 {
        tone::VALUE
    } else {
        tone::MINED
    };
    let radius = MARK_SIZE / 2.0;
    let disc = |r: f32| {
        let b = gsk::PathBuilder::new();
        b.add_circle(&draw::point(x, y), r);
        b.to_path()
    };
    if chosen {
        draw::push_shadow(snapshot, color, 0.9, 4.0);
    }
    draw::fill(snapshot, &disc(radius), if mark.confirmed() { color } else { tone::SCREEN });
    draw::stroke(snapshot, &disc(radius - 0.5), color, 1.0);
    if chosen {
        draw::stroke(snapshot, &disc(radius + 4.0 - 0.75), tone::PRESENT, 1.5);
    }
    if mark.rows.len() > 1 {
        let text_color = if mark.confirmed() { tone::SCREEN } else { color };
        draw::text(snapshot, widget, COUNT, &mark.rows.len().to_string(), x, y, Anchor::Center, text_color);
    }
    if chosen {
        snapshot.pop();
    }
}

/// One line of labels: parts side by side on one baseline, 5 apart.
struct Line {
    parts: Vec<(pango::Layout, Color)>,
}

impl Line {
    fn width(&self) -> f32 {
        let widths: f32 = self.parts.iter().map(|(l, _)| l.pixel_extents().1.width() as f32).sum();
        widths + 5.0 * self.parts.len().saturating_sub(1) as f32
    }

    fn ascent(&self) -> f32 {
        self.parts.iter().map(|(l, _)| l.baseline() as f32 / pango::SCALE as f32).fold(0.0, f32::max)
    }

    fn height(&self) -> f32 {
        let descent = self
            .parts
            .iter()
            .map(|(l, _)| l.pixel_extents().1.height() as f32 - l.baseline() as f32 / pango::SCALE as f32)
            .fold(0.0, f32::max);
        self.ascent() + descent
    }
}

/// The labels of a mark: the observer's height, the other system's height and the note,
/// centred on `x` and `y` in a frame of `room`. A line that is too wide is made smaller, to
/// four fifths at most.
#[allow(clippy::too_many_arguments)]
fn paint_labels(widget: &Canvas, snapshot: &gtk::Snapshot, mark: &NotarizationMark, observer: &str, other: &str, note: &str, x: f32, y: f32, room: f32) {
    let mut lines = Vec::new();
    if let Some(row) = mark.newest() {
        let observer_height = picture::observer_height(row).map(format::integer).unwrap_or_else(|| "–".into());
        let other_height = picture::other_height(row).map(format::integer).unwrap_or_else(|| "–".into());
        lines.push(Line {
            parts: vec![
                (draw::layout(widget, font::SMALL, observer), tone::LABEL),
                (draw::layout(widget, font::FIGURE_SMALL, &observer_height), tone::VALUE),
            ],
        });
        lines.push(Line {
            parts: vec![
                (draw::layout(widget, font::SMALL, other), tone::LABEL),
                (draw::layout(widget, font::FIGURE_SMALL, &other_height), tone::LABEL),
            ],
        });
    }
    lines.push(Line { parts: vec![(draw::layout(widget, font::CAPTION, note), tone::LABEL)] });
    let total: f32 = lines.iter().map(Line::height).sum::<f32>() + 2.0 * lines.len().saturating_sub(1) as f32;
    let mut top = y - total / 2.0;
    for line in &lines {
        let width = line.width();
        let scale = if width > room && width > 0.0 { (room / width).max(0.8) } else { 1.0 };
        let height = line.height();
        let baseline = top + line.ascent();
        snapshot.save();
        snapshot.translate(&draw::point(x, baseline));
        snapshot.scale(scale, scale);
        let mut left = -width / 2.0;
        for (layout, color) in &line.parts {
            let ascent = layout.baseline() as f32 / pango::SCALE as f32;
            snapshot.save();
            snapshot.translate(&draw::point(left.round(), -ascent));
            snapshot.append_layout(layout, &color.rgba());
            snapshot.restore();
            left += layout.pixel_extents().1.width() as f32 + 5.0;
        }
        snapshot.restore();
        top += height + 2.0;
    }
}
