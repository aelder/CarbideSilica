//! The history of an identity as lanes: the keys, the revoke authority, the recover
//! authority and the lock, with one step for each version (`VersionStrip` in
//! `IdentityControl.swift`).
//!
//! The updates of an identity frequently come in groups of some days with years between
//! them. Thus the lanes give each version the same width, and a time line above shows the
//! true block of each one. A bar is the versions for which one value was in effect: an
//! authority that did not change is one bar. A red band is a revoked version. Select a
//! step to view the identity as it was after that update.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gsk;
use gtk::prelude::*;

use crate::bridge::api::{IdentityState, Timelock};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::shared::identity_support::{self as support, abbreviated_date};
use crate::theme::{self, draw, font, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, PlateFill};

const LABEL_WIDTH: i32 = 72;
const TIME_HEIGHT: f32 = 34.0;
const LANE_HEIGHT: f32 = 24.0;
pub const LANES: [&str; 4] = ["Keys", "Revoke", "Recover", "Lock"];


// MARK: - Layout (pure)

/// The x of a version's true block on the time line, in a width of `width`.
pub fn time_x(height: u64, first: u64, last: u64, width: f32) -> f32 {
    if last > first {
        4.0 + ((height.saturating_sub(first)) as f64 / (last - first) as f64) as f32 * (width - 8.0)
    } else {
        width / 2.0
    }
}

/// The first and last heights of the time line: the first version, and the tip or the
/// last version, whichever is later.
pub fn time_range(heights: &[u64], tip: Option<u64>) -> (u64, u64) {
    let first = heights.first().copied().unwrap_or(0);
    let last_version = heights.last().copied().unwrap_or(first);
    (first, tip.unwrap_or(last_version).max(last_version))
}

/// One bar of a lane: the versions `start..end` with one value, and the index of its
/// colour (its value's place in the order of first appearance, at most `palette - 1`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bar {
    pub start: usize,
    pub end: usize,
    pub colour: usize,
}

pub fn bars(values: &[String], palette: usize) -> Vec<Bar> {
    let mut result = Vec::new();
    if values.is_empty() {
        return result;
    }
    let mut seen: Vec<&String> = Vec::new();
    let mut start = 0;
    for index in 1..=values.len() {
        if index == values.len() || values[index] != values[start] {
            let value = &values[start];
            if !seen.contains(&value) {
                seen.push(value);
            }
            let colour = seen.iter().position(|v| *v == value).unwrap_or(0).min(palette.saturating_sub(1));
            result.push(Bar { start, end: index, colour });
            start = index;
        }
    }
    result
}

/// The value of the keys lane: what the bar says, then "|" and every key, so that a change
/// of a key that the short text hides is still a change.
pub fn keys_value(version: &IdentityState) -> String {
    let count = version.primary_addresses.len();
    let first = version
        .primary_addresses
        .first()
        .map(|key| format::short(key, 6, 4))
        .unwrap_or_else(|| "no key".into());
    let label = if count <= 1 {
        first
    } else {
        format!("{} of {count}: {first}", version.minimum_signatures.unwrap_or(1))
    };
    format!("{label}|{}", version.primary_addresses.join(","))
}

/// The values of one lane, one for each version.
pub fn lane_values(versions: &[IdentityState], lane: usize, display: &dyn Fn(&str) -> String) -> Vec<String> {
    let authority = |id: &Option<String>, is_self: bool| match id {
        Some(id) if !is_self => display(id),
        _ => "itself".to_string(),
    };
    versions
        .iter()
        .map(|version| match lane {
            0 => keys_value(version),
            1 => authority(
                &version.revocation_authority,
                version.revocation_authority.is_none() || version.revocation_is_self,
            ),
            2 => authority(&version.recovery_authority, version.recovery_authority.is_none() || version.recovery_is_self),
            _ => version.timelock_label.clone(),
        })
        .collect()
}

/// "Version 3 of 7: Revoked".
pub fn selected_text(versions: &[IdentityState], selection: usize) -> String {
    match versions.get(selection) {
        None => String::new(),
        Some(version) => format!(
            "Version {} of {}: {}",
            selection + 1,
            versions.len(),
            if version.revoked { "Revoked".to_string() } else { version.changes.first().cloned().unwrap_or_else(|| "Updated".into()) }
        ),
    }
}

fn spoken(version: &IdentityState) -> String {
    if version.revoked { "revoked".into() } else { version.changes.first().cloned().unwrap_or_else(|| "updated".into()) }
}

// MARK: - The widget

pub struct VersionStrip {
    pub root: gtk::Overlay,
    canvas: Canvas,
    selection: Rc<Cell<usize>>,
    versions: Rc<Vec<IdentityState>>,
    first_edge: gtk::Label,
    last_edge: gtk::Label,
    selected: gtk::Label,
    steps: RefCell<Vec<gtk::Box>>,
    model: Rc<AppModel>,
}

impl VersionStrip {
    pub fn new(model: &Rc<AppModel>, versions: Rc<Vec<IdentityState>>, tip: Option<u64>, selection: usize, select: Rc<dyn Fn(usize)>) -> Rc<Self> {
        let selection = Rc::new(Cell::new(selection));
        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);

        let top = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 0);
        labels.set_size_request(LABEL_WIDTH, -1);
        let time = display::name_text("Time");
        time.set_xalign(0.0);
        time.set_valign(gtk::Align::Start);
        time.set_size_request(-1, TIME_HEIGHT as i32);
        labels.append(&time);
        for lane in LANES {
            let holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            holder.set_size_request(-1, LANE_HEIGHT as i32);
            let plate = display::plate(lane, PlateFill::None);
            plate.set_valign(gtk::Align::Center);
            holder.append(&plate);
            labels.append(&holder);
        }
        top.append(&labels);

        let height = TIME_HEIGHT + LANE_HEIGHT * LANES.len() as f32;
        let canvas = {
            let (model, versions, selection) = (Rc::downgrade(model), versions.clone(), selection.clone());
            Canvas::new(0, height as i32, move |canvas, snapshot, w, h| {
                let Some(model) = model.upgrade() else { return };
                draw_lanes(canvas, snapshot, w, h, &model, &versions, tip, selection.get());
            })
        };
        canvas.set_hexpand(true);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&canvas));
        // One target for each version: the selection, the help text and the accessible name.
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.set_homogeneous(true);
        row.set_margin_top(TIME_HEIGHT as i32);
        let mut steps = Vec::new();
        for index in 0..versions.len() {
            let step = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            step.set_hexpand(true);
            step.set_vexpand(true);
            let click = gtk::GestureClick::new();
            let select = select.clone();
            click.connect_released(move |_, _, _, _| select(index));
            step.add_controller(click);
            step.set_accessible_role(gtk::AccessibleRole::Button);
            row.append(&step);
            steps.push(step);
        }
        overlay.add_overlay(&row);
        overlay.set_hexpand(true);
        top.append(&overlay);
        column.append(&top);
        column.append(&display::rule());

        let edges = gtk::CenterBox::new();
        let first_edge = theme::label(font::FIGURE_SMALL, "", "label");
        let last_edge = theme::label(font::FIGURE_SMALL, "", "label");
        // `Tone.caption.weight(.semibold)`: 11.5.
        let selected = super::px_label("", 11.5, 600, &["present"]);
        edges.set_start_widget(Some(&first_edge));
        edges.set_center_widget(Some(&selected));
        edges.set_end_widget(Some(&last_edge));
        column.append(&edges);
        let caption = theme::label(
            font::CAPTION,
            "Each step is one version, and the line at the top shows its true block. A change of colour in a lane \
             is a change of holder. In the lock lane, a solid bar is a lock with a delay and a striped bar is a \
             lock to a set block. A red band is a revoked version. Select a step to view that version.",
            "label",
        );
        caption.set_wrap(true);
        caption.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        column.append(&caption);

        let root = display::panel(&column, 10);
        let strip = Rc::new(Self {
            root,
            canvas,
            selection,
            versions,
            first_edge,
            last_edge,
            selected,
            steps: RefCell::new(steps),
            model: model.clone(),
        });
        strip.update_texts();
        let weak = Rc::downgrade(&strip);
        model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &strip.root, move |_| {
            if let Some(strip) = weak.upgrade() {
                strip.update_texts();
                strip.canvas.queue_draw();
            }
        });
        strip
    }

    pub fn set_selection(&self, selection: usize) {
        self.selection.set(selection);
        self.update_texts();
        self.canvas.queue_draw();
    }

    fn edge(&self, version: Option<&IdentityState>) -> String {
        let Some(version) = version else { return String::new() };
        match version.time {
            Some(time) if !crate::display::shows_dates() => abbreviated_date(time),
            _ => support::height_text(&self.model, version.height, version.time),
        }
    }

    fn update_texts(&self) {
        theme::set_label(&self.first_edge, font::FIGURE_SMALL, &self.edge(self.versions.first()));
        theme::set_label(&self.last_edge, font::FIGURE_SMALL, &self.edge(self.versions.last()));
        let text = selected_text(&self.versions, self.selection.get());
        self.selected.set_text(&text);
        for (index, step) in self.steps.borrow().iter().enumerate() {
            let version = &self.versions[index];
            step.set_tooltip_text(Some(&format!(
                "Version {}: {}. {}",
                index + 1,
                support::height_and_time(version.height, version.time, &self.model),
                version.changes.join(". ")
            )));
            step.update_property(&[gtk::accessible::Property::Label(&format!(
                "Version {}, {}",
                index + 1,
                spoken(version)
            ))]);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_lanes(
    canvas: &Canvas,
    snapshot: &gtk::Snapshot,
    width: f32,
    _height: f32,
    model: &Rc<AppModel>,
    versions: &[IdentityState],
    tip: Option<u64>,
    selection: usize,
) {
    let count = versions.len();
    if count == 0 {
        return;
    }
    let step = width / count as f32;
    // The time line: a tick at the true block of each version, and a line to its step.
    let heights: Vec<u64> = versions.iter().map(|v| v.height).collect();
    let (first, last) = time_range(&heights, tip);
    draw::rule(snapshot, 4.0, 8.0, width - 4.0, 8.0, tone::AXIS, 1.0, &[]);
    let leaders = gsk::PathBuilder::new();
    for (index, version) in versions.iter().enumerate() {
        let x = time_x(version.height, first, last, width);
        draw::fill_rect(snapshot, x - 0.5, 3.0, 1.0, 10.0, tone::VALUE);
        leaders.move_to(x, 13.0);
        leaders.line_to((index as f32 + 0.5) * step, TIME_HEIGHT - 3.0);
    }
    draw::stroke(snapshot, &leaders.to_path(), tone::AXIS, 0.6);

    let display = |id: &str| support::display_id(model, id);
    for lane in 0..LANES.len() {
        let y = TIME_HEIGHT + lane as f32 * LANE_HEIGHT;
        let lock = lane == 3;
        draw::fill_rect(snapshot, 0.0, y + 3.0, width, LANE_HEIGHT - 6.0, tone::GRID);
        let values = lane_values(versions, lane, &display);
        for bar in bars(&values, tone::SERIES.len()) {
            let (bx, by) = (bar.start as f32 * step, y + 3.0);
            let (bw, bh) = (((bar.end - bar.start) as f32 * step - 1.0).max(1.5), LANE_HEIGHT - 6.0);
            if lock {
                match versions[bar.start].timelock {
                    Timelock::Unlocked => {}
                    Timelock::Delay { .. } => draw::fill_rect(snapshot, bx, by, bw, bh, tone::RECORD.opacity(0.9)),
                    Timelock::UnlocksAt { .. } => {
                        draw::fill_rect(snapshot, bx, by, bw, bh, tone::RECORD.opacity(0.25));
                        let stripes = gsk::PathBuilder::new();
                        let mut x = bx - bh;
                        while x < bx + bw {
                            stripes.move_to(x, by + bh);
                            stripes.line_to(x + bh, by);
                            x += 6.0;
                        }
                        snapshot.push_clip(&draw::rect(bx, by, bw, bh));
                        draw::stroke(snapshot, &stripes.to_path(), tone::RECORD, 2.0);
                        snapshot.pop();
                    }
                }
            } else {
                let color = tone::SERIES[bar.colour];
                draw::fill_rect(snapshot, bx, by, bw, bh, color.opacity(0.85));
                let label = values[bar.start].split('|').next().unwrap_or("").to_string();
                // `.caption.weight(.semibold)`: 10.
                let layout = super::px_layout(canvas, &label, 10.0, 600);
                let (_, extents) = layout.pixel_extents();
                if extents.width() as f32 + 10.0 < bw {
                    draw::draw_layout(snapshot, &layout, bx + 5.0, by + bh / 2.0, draw::Anchor::Leading, tone::SCREEN);
                }
            }
        }
    }
    let lanes_height = LANE_HEIGHT * LANES.len() as f32;
    for (index, version) in versions.iter().enumerate() {
        if version.revoked {
            draw::fill_rect(snapshot, index as f32 * step, TIME_HEIGHT, step, lanes_height, tone::ALERT.opacity(0.3));
        }
    }
    // Thin gaps between the steps, and the selected step.
    if count <= 80 {
        for index in 1..count.max(1) {
            draw::fill_rect(snapshot, index as f32 * step - 0.5, TIME_HEIGHT, 1.0, lanes_height, tone::SCREEN.opacity(0.7));
        }
    }
    if selection < count {
        // The selected version is the present: a lit outline, 2 wide, on the column inset
        // by 1 at each side.
        let (x, y, w, h) = (selection as f32 * step + 1.0, TIME_HEIGHT + 1.0, step - 2.0, lanes_height - 2.0);
        let outline = gsk::PathBuilder::new();
        outline.add_rect(&draw::rect(x - 1.0, y - 1.0, w + 2.0, 2.0));
        outline.add_rect(&draw::rect(x - 1.0, y + h - 1.0, w + 2.0, 2.0));
        outline.add_rect(&draw::rect(x - 1.0, y + 1.0, 2.0, h - 2.0));
        outline.add_rect(&draw::rect(x + w - 1.0, y + 1.0, 2.0, h - 2.0));
        draw::lit(snapshot, &outline.to_path(), tone::PRESENT, 0.7);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bars_join_equal_neighbours_and_colour_by_first_appearance() {
        let values: Vec<String> = ["a", "a", "b", "a", "c", "c"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            bars(&values, 6),
            vec![
                Bar { start: 0, end: 2, colour: 0 },
                Bar { start: 2, end: 3, colour: 1 },
                Bar { start: 3, end: 4, colour: 0 },
                Bar { start: 4, end: 6, colour: 2 },
            ]
        );
        // More holders than colours: the last colour repeats.
        let many: Vec<String> = (0..8).map(|i| i.to_string()).collect();
        assert_eq!(bars(&many, 6).iter().map(|b| b.colour).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4, 5, 5, 5]);
        assert!(bars(&[], 6).is_empty());
    }

    #[test]
    fn time_line_places_heights_between_the_ends() {
        assert_eq!(time_range(&[100, 200, 300], Some(500)), (100, 500));
        assert_eq!(time_range(&[100, 200, 300], None), (100, 300));
        assert_eq!(time_range(&[100, 300], Some(200)), (100, 300));
        assert_eq!(time_x(100, 100, 500, 408.0), 4.0);
        assert_eq!(time_x(500, 100, 500, 408.0), 404.0);
        assert_eq!(time_x(300, 100, 500, 408.0), 204.0);
        // One height: the middle.
        assert_eq!(time_x(7, 7, 7, 100.0), 50.0);
    }
}
