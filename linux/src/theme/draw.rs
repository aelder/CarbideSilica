//! Drawing in a canvas: the forms of `extension GraphicsContext` in `DisplayTheme.swift`.
//!
//! The helpers draw into a `gtk::Snapshot` with GSK paths, so that a shape can have the
//! soft light of the display (`lit`), which is a GSK shadow node. Text is a Pango layout in
//! the type roles of [`super::font`].

use gtk::prelude::*;
use gtk::{gdk, graphene, gsk, pango};

use super::{font, lamp, tone, Color};

/// Where a text or a plate is placed relative to its point, as SwiftUI's `UnitPoint`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    Center,
    Leading,
    Trailing,
    Top,
    Bottom,
    TopLeading,
    TopTrailing,
    BottomLeading,
    BottomTrailing,
}

impl Anchor {
    fn unit(self) -> (f32, f32) {
        match self {
            Anchor::Center => (0.5, 0.5),
            Anchor::Leading => (0.0, 0.5),
            Anchor::Trailing => (1.0, 0.5),
            Anchor::Top => (0.5, 0.0),
            Anchor::Bottom => (0.5, 1.0),
            Anchor::TopLeading => (0.0, 0.0),
            Anchor::TopTrailing => (1.0, 0.0),
            Anchor::BottomLeading => (0.0, 1.0),
            Anchor::BottomTrailing => (1.0, 1.0),
        }
    }
}

pub fn rect(x: f32, y: f32, w: f32, h: f32) -> graphene::Rect {
    graphene::Rect::new(x, y, w, h)
}

pub fn point(x: f32, y: f32) -> graphene::Point {
    graphene::Point::new(x, y)
}

/// A straight line.
pub fn rule(snapshot: &gtk::Snapshot, x0: f32, y0: f32, x1: f32, y1: f32, color: Color, width: f32, dash: &[f32]) {
    let builder = gsk::PathBuilder::new();
    builder.move_to(x0, y0);
    builder.line_to(x1, y1);
    let stroke = gsk::Stroke::new(width);
    stroke.set_line_cap(gsk::LineCap::Butt);
    if !dash.is_empty() {
        stroke.set_dash(dash);
    }
    snapshot.append_stroke(&builder.to_path(), &stroke, &color.rgba());
}

pub fn stroke(snapshot: &gtk::Snapshot, path: &gsk::Path, color: Color, width: f32) {
    let stroke = gsk::Stroke::new(width);
    snapshot.append_stroke(path, &stroke, &color.rgba());
}

pub fn fill(snapshot: &gtk::Snapshot, path: &gsk::Path, color: Color) {
    snapshot.append_fill(path, gsk::FillRule::Winding, &color.rgba());
}

pub fn fill_rect(snapshot: &gtk::Snapshot, x: f32, y: f32, w: f32, h: f32, color: Color) {
    snapshot.append_color(&color.rgba(), &rect(x, y, w, h));
}

pub fn rect_path(x: f32, y: f32, w: f32, h: f32) -> gsk::Path {
    let builder = gsk::PathBuilder::new();
    builder.add_rect(&rect(x, y, w, h));
    builder.to_path()
}

pub fn ellipse_path(x: f32, y: f32, w: f32, h: f32) -> gsk::Path {
    let builder = gsk::PathBuilder::new();
    if (w - h).abs() < 0.01 {
        builder.add_circle(&point(x + w / 2.0, y + h / 2.0), w / 2.0);
    } else {
        // An ellipse as four quarter curves.
        let (cx, cy, rx, ry) = (x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0);
        let k = 0.552_284_8_f32;
        builder.move_to(cx + rx, cy);
        builder.cubic_to(cx + rx, cy + ry * k, cx + rx * k, cy + ry, cx, cy + ry);
        builder.cubic_to(cx - rx * k, cy + ry, cx - rx, cy + ry * k, cx - rx, cy);
        builder.cubic_to(cx - rx, cy - ry * k, cx - rx * k, cy - ry, cx, cy - ry);
        builder.cubic_to(cx + rx * k, cy - ry, cx + rx, cy - ry * k, cx + rx, cy);
        builder.close();
    }
    builder.to_path()
}

/// A Pango layout of `text` in a role. `widget` gives the font map and the scale.
pub fn layout(widget: &impl IsA<gtk::Widget>, role: font::Role, text: &str) -> pango::Layout {
    let layout = widget.as_ref().create_pango_layout(None);
    layout.set_markup(&super::markup(role, text));
    layout
}

/// Draws a layout with its anchor at (x, y).
pub fn draw_layout(snapshot: &gtk::Snapshot, layout: &pango::Layout, x: f32, y: f32, anchor: Anchor, color: Color) {
    let (_, logical) = layout.pixel_extents();
    let (ux, uy) = anchor.unit();
    let left = x - logical.width() as f32 * ux;
    let top = y - logical.height() as f32 * uy;
    snapshot.save();
    snapshot.translate(&point(left.round(), top.round()));
    snapshot.append_layout(layout, &color.rgba());
    snapshot.restore();
}

/// The size of a text in a role.
pub fn text_size(widget: &impl IsA<gtk::Widget>, role: font::Role, text: &str) -> (f32, f32) {
    let (_, logical) = layout(widget, role, text).pixel_extents();
    (logical.width() as f32, logical.height() as f32)
}

/// A small name in spaced capitals.
pub fn name(snapshot: &gtk::Snapshot, widget: &impl IsA<gtk::Widget>, text: &str, x: f32, y: f32, anchor: Anchor, color: Color) {
    draw_layout(snapshot, &layout(widget, font::SMALL, text), x, y, anchor, color);
}

/// A figure, such as a time or an amount on a chart.
pub fn figure(snapshot: &gtk::Snapshot, widget: &impl IsA<gtk::Widget>, text: &str, x: f32, y: f32, anchor: Anchor, color: Color) {
    draw_layout(snapshot, &layout(widget, font::FIGURE_SMALL, text), x, y, anchor, color);
}

/// Text in a role.
pub fn text(snapshot: &gtk::Snapshot, widget: &impl IsA<gtk::Widget>, role: font::Role, text: &str, x: f32, y: f32, anchor: Anchor, color: Color) {
    draw_layout(snapshot, &layout(widget, role, text), x, y, anchor, color);
}

/// A name on a field of its colour, or in an outline. The plate is `width` wide (46 by
/// default) and 16 high, and its left middle is at (x, y).
pub fn plate(snapshot: &gtk::Snapshot, widget: &impl IsA<gtk::Widget>, text: &str, x: f32, y: f32, width: f32, fill_color: Option<Color>) {
    let (bx, by, bw, bh) = (x, y - 8.0, width, 16.0);
    match fill_color {
        Some(color) => fill_rect(snapshot, bx, by, bw, bh, color),
        None => stroke(snapshot, &rect_path(bx + 0.5, by + 0.5, bw - 1.0, bh - 1.0), tone::LABEL, 1.0),
    }
    let color = if fill_color.is_some() { tone::SCREEN } else { tone::LABEL };
    draw_layout(snapshot, &layout(widget, font::PLATE, text), bx + bw / 2.0, by + bh / 2.0, Anchor::Center, color);
}

/// Begins content with a soft light around it: the halation of film (a wide red-orange
/// edge) and a narrower light of `color`. Draw the content, then call `snapshot.pop()`.
/// `strength` 0 gives no light.
pub fn push_glow(snapshot: &gtk::Snapshot, color: Color, strength: f64) {
    let shadows = [
        gsk::Shadow::new(lamp::HALATION.opacity(strength * 0.55).rgba(), 0.0, 0.0, 7.0),
        gsk::Shadow::new(color.opacity(strength * 0.8).rgba(), 0.0, 0.0, 3.0),
    ];
    snapshot.push_shadow(&shadows);
}

/// A shape with a soft light around it: for the things that are live or selected.
pub fn lit(snapshot: &gtk::Snapshot, path: &gsk::Path, color: Color, strength: f64) {
    if strength > 0.0 {
        push_glow(snapshot, color, strength);
        fill(snapshot, path, color);
        snapshot.pop();
    } else {
        fill(snapshot, path, color);
    }
}

/// A shadow of one colour, as a CALayer shadow: `opacity` of `color` with a blur `radius`.
pub fn push_shadow(snapshot: &gtk::Snapshot, color: Color, opacity: f64, radius: f32) {
    snapshot.push_shadow(&[gsk::Shadow::new(color.opacity(opacity).rgba(), 0.0, 0.0, radius)]);
}

/// A linear gradient across a rectangle.
pub fn linear_gradient(snapshot: &gtk::Snapshot, bounds: &graphene::Rect, start: (f32, f32), end: (f32, f32), stops: &[(f32, Color)]) {
    let stops: Vec<gsk::ColorStop> = stops.iter().map(|(at, c)| gsk::ColorStop::new(*at, c.rgba())).collect();
    snapshot.append_linear_gradient(bounds, &point(start.0, start.1), &point(end.0, end.1), &stops);
}

pub fn rgba(color: Color) -> gdk::RGBA {
    color.rgba()
}
