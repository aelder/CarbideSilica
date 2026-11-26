//! The parts of a display: panel, corner marks, rule, readout, plate and name
//! (`DisplayTheme.swift`).

use gtk::prelude::*;

use super::canvas::Canvas;
use crate::theme::{self, draw, font, tone, Color};

/// Four small corners that frame a panel, 5 inside its edge.
pub fn corner_marks() -> Canvas {
    let marks = Canvas::new(0, 0, |_, snapshot, w, h| {
        let arm = 9.0;
        let inset = 5.5;
        let (x0, y0, x1, y1) = (inset, inset, w - inset, h - inset);
        let builder = gtk::gsk::PathBuilder::new();
        for (cx, cy, dx, dy) in [(x0, y0, 1.0, 1.0), (x1, y0, -1.0, 1.0), (x0, y1, 1.0, -1.0), (x1, y1, -1.0, -1.0)] {
            builder.move_to(cx + arm * dx, cy);
            builder.line_to(cx, cy);
            builder.line_to(cx, cy + arm * dy);
        }
        draw::stroke(snapshot, &builder.to_path(), tone::VALUE.opacity(0.55), 1.0);
    });
    marks.set_can_target(false);
    marks
}

/// A display panel: the frame of each chart and diagram. It has a thin border and four
/// corner marks. `padding` is the room between the frame and the content; a chart that
/// draws to the frame gives 0 and sets its own.
pub fn panel(child: &impl IsA<gtk::Widget>, padding: i32) -> gtk::Overlay {
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("panel");
    frame.set_hexpand(true);
    let child = child.as_ref();
    child.set_margin_top(padding);
    child.set_margin_bottom(padding);
    child.set_margin_start(padding);
    child.set_margin_end(padding);
    frame.append(child);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&frame));
    overlay.add_overlay(&corner_marks());
    overlay
}

/// A child in a thin square outline of `color`, `horizontal` and `vertical` inside it: the
/// Swift app's `.padding().overlay(Rectangle().strokeBorder(color, lineWidth: 1))`.
pub fn outlined(child: &impl IsA<gtk::Widget>, color: Color, horizontal: i32, vertical: i32) -> gtk::Overlay {
    let child = child.as_ref();
    child.set_margin_start(horizontal);
    child.set_margin_end(horizontal);
    child.set_margin_top(vertical);
    child.set_margin_bottom(vertical);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(child));
    let frame = Canvas::new(0, 0, move |_, snapshot, w, h| {
        draw::stroke(snapshot, &draw::rect_path(0.5, 0.5, w - 1.0, h - 1.0), color, 1.0);
    });
    frame.set_can_target(false);
    overlay.add_overlay(&frame);
    overlay
}

/// A thin rule across a panel, between its parts.
pub fn rule() -> gtk::Box {
    let rule = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    rule.add_css_class("rule");
    rule.set_hexpand(true);
    rule
}

/// A thin upright rule.
pub fn vertical_rule() -> gtk::Box {
    let rule = gtk::Box::new(gtk::Orientation::Vertical, 0);
    rule.add_css_class("rule");
    rule.set_vexpand(true);
    rule
}

/// A small name in spaced capitals: the name of a field, an axis or a column.
pub fn name_text(text: &str) -> gtk::Label {
    let label = theme::label(font::SMALL, text, "label");
    label.set_single_line_mode(true);
    label
}

/// Sets a width that holds `reserve`, so that the figures after it do not move when the
/// value gets longer or shorter.
pub fn reserve_width(label: &gtk::Label, role: font::Role, reserve: &str) {
    let (width, _) = draw::text_size(label, role, reserve);
    label.set_size_request(width.ceil() as i32, -1);
}

/// A name and a figure in one line: "TARGET 25".
pub struct Readout {
    pub root: gtk::Box,
    pub value: gtk::Label,
    pub name: gtk::Label,
}

impl Readout {
    pub fn new(label: &str, value: &str, tone_class: &str, reserve: Option<&str>) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        let name = name_text(label);
        name.set_valign(gtk::Align::Baseline);
        let figure = theme::label(font::FIGURE, value, if tone_class.is_empty() { "value" } else { tone_class });
        figure.set_valign(gtk::Align::Baseline);
        if let Some(reserve) = reserve {
            reserve_width(&figure, font::FIGURE, reserve);
        }
        root.append(&name);
        root.append(&figure);
        Self { root, value: figure, name }
    }

    pub fn set(&self, label: &str, value: &str) {
        theme::set_label(&self.name, font::SMALL, label);
        theme::set_label(&self.value, font::FIGURE, value);
    }
}

/// The fill of a plate: a signal colour, or none for an outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlateFill {
    None,
    Staked,
    Mined,
    Record,
    Good,
    Alert,
    /// The present's colour: a chosen kind.
    Present,
}

impl PlateFill {
    pub fn color(self) -> Option<Color> {
        match self {
            PlateFill::None => None,
            PlateFill::Staked => Some(tone::STAKED),
            PlateFill::Mined => Some(tone::MINED),
            PlateFill::Record => Some(tone::RECORD),
            PlateFill::Good => Some(tone::GOOD),
            PlateFill::Alert => Some(tone::ALERT),
            PlateFill::Present => Some(tone::PRESENT),
        }
    }
}

/// A name in capitals on a field of its colour, or in an outline when it has no colour.
pub fn plate(text: &str, fill: PlateFill) -> gtk::Label {
    let label = theme::label(font::PLATE, text, "");
    label.add_css_class("plate");
    label.set_xalign(0.5);
    label.set_halign(gtk::Align::Start);
    label.set_valign(gtk::Align::Center);
    let class = match fill {
        PlateFill::None => "fill-none",
        PlateFill::Staked => "fill-staked",
        PlateFill::Mined => "fill-mined",
        PlateFill::Record => "fill-record",
        PlateFill::Good => "fill-good",
        PlateFill::Alert => "fill-alert",
        PlateFill::Present => "fill-present",
    };
    label.add_css_class(class);
    label
}

/// The signal colour of a transaction tag: violet for staked, amber for mined. Other tags
/// have no colour and show as an outline (`signalTagFill`).
pub fn signal_tag_fill(tag: &str) -> PlateFill {
    match tag.to_lowercase().as_str() {
        "staked" | "stake" => PlateFill::Staked,
        "mined" => PlateFill::Mined,
        _ => PlateFill::None,
    }
}

/// A card heading in the type of a heading: for windows that have sections, such as the
/// settings and the keyboard reference.
pub fn heading(text: &str) -> gtk::Label {
    theme::label(font::HEADING, text, "value")
}
