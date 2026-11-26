//! The marks that mean something, drawn by the app: the Linux forms of the SF Symbols that
//! the Swift app uses. The app draws them so that it does not depend on an icon theme. Each
//! mark is drawn here once; a page names it with [`Icon`].
//!
//! A mark is a square of any size. Its lines are about a twelfth of its size, as the
//! symbols are at the weight of the text beside them. The parts of a mark that are cut out
//! of a filled shape are drawn in the black of the display.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use gtk::gsk;
use gtk::prelude::*;

use super::canvas::Canvas;
use crate::theme::{draw, tone, Color};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// `magnifyingglass`
    Search,
    /// `circle.fill`
    Dot,
    /// `circle`
    Ring,
    /// `circle.dashed`
    RingDashed,
    /// `circle.dotted`
    RingDotted,
    /// `circle.lefthalf.filled`
    HalfFilled,
    /// `clock.arrow.circlepath`: a stored observation.
    Stored,
    /// `clock`
    Clock,
    /// `hourglass`
    Hourglass,
    /// `exclamationmark.triangle`
    Warning,
    /// `exclamationmark.triangle.fill`
    WarningFill,
    /// `exclamationmark.circle`
    ExclamationCircle,
    /// `questionmark.circle`
    QuestionCircle,
    /// `doc.on.doc`
    Copy,
    /// `square.and.arrow.down`
    Save,
    /// `checkmark`
    Check,
    /// `checkmark.circle`
    CheckCircle,
    /// `checkmark.circle.fill`
    CheckCircleFill,
    /// `checkmark.seal.fill`
    Seal,
    /// `xmark`
    Cross,
    /// `xmark.circle`
    CrossCircle,
    /// `xmark.circle.fill`
    CrossCircleFill,
    /// `xmark.octagon.fill`
    CrossOctagonFill,
    /// `minus.circle`
    MinusCircle,
    /// `slash.circle`
    SlashCircle,
    /// `nosign`
    NoSign,
    /// `diamond.fill`
    Diamond,
    /// `chevron.compact.right`
    Chevron,
    /// `chevron.left`
    ChevronLeft,
    /// `chevron.right`
    ChevronRight,
    /// `chevron.up`
    ChevronUp,
    /// `chevron.down`
    Down,
    /// `arrowtriangle.right.fill`
    TriangleRight,
    /// `arrowtriangle.down.fill`
    TriangleDown,
    /// `arrowtriangle.up.fill`
    TriangleUp,
    /// `arrow.left`
    ArrowLeft,
    /// `arrow.right`
    ArrowRight,
    /// `arrow.left.arrow.right`
    ArrowsLeftRight,
    /// `arrow.clockwise`
    Refresh,
    /// `arrow.triangle.2.circlepath`
    Circlepath,
    /// `arrow.up.circle`
    ArrowUpCircle,
    /// `arrow.down.circle.fill`
    ArrowDownCircleFill,
    /// `arrow.triangle.branch`
    Branch,
    /// `chart.xyaxis.line`
    Chart,
    /// `link`
    Link,
    /// `bookmark`
    Bookmark,
    /// `cube`
    Cube,
    /// `person.crop.circle`
    PersonCircle,
    /// `circle.hexagongrid`
    HexagonGrid,
    /// `creditcard`
    CreditCard,
    /// `tray`
    Tray,
    /// `tray.full`
    TrayFull,
    /// `server.rack`
    ServerRack,
    /// `bolt.horizontal.circle`: not connected.
    Disconnected,
    /// The three lines of a menu.
    Menu,
}

/// A mark of `size` in `color`.
pub fn icon(kind: Icon, size: i32, color: Color) -> Canvas {
    let canvas = Canvas::new(size, size, move |_, snapshot, w, h| paint(snapshot, kind, w, h, color));
    canvas.set_valign(gtk::Align::Center);
    canvas.set_can_target(false);
    canvas
}

/// A mark in the colour of the text around it (the CSS colour of its widget), so that it
/// follows the hover and press states of the key or link that holds it.
pub fn text_icon(kind: Icon, size: i32) -> Canvas {
    let canvas = Canvas::new(size, size, move |canvas, snapshot, w, h| {
        let c = canvas.color();
        paint(snapshot, kind, w, h, Color { r: c.red(), g: c.green(), b: c.blue(), a: c.alpha() });
    });
    canvas.set_valign(gtk::Align::Center);
    canvas.set_can_target(false);
    canvas
}

/// Changes the colour or the kind of a mark made by [`icon`].
pub fn set_icon(canvas: &Canvas, kind: Icon, color: Color) {
    canvas.set_draw(move |_, snapshot, w, h| paint(snapshot, kind, w, h, color));
}

/// SwiftUI's `Label`: a mark and a text, both in `color`. `classes` are the text's classes
/// of `style.css`; the mark is 13 beside a callout and 14 beside body text.
pub fn label(kind: Icon, text: &str, color: Color, classes: &[&str]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let size = if classes.contains(&"callout") { 13 } else { 14 };
    row.append(&icon(kind, size, color));
    let words = crate::page::text(text, classes);
    words.set_markup(&crate::page::span(text, color));
    row.append(&words);
    row
}

fn circle(cx: f32, cy: f32, r: f32) -> gsk::Path {
    let b = gsk::PathBuilder::new();
    b.add_circle(&draw::point(cx, cy), r);
    b.to_path()
}

/// Lines through points, with round ends and joins.
fn polyline(snapshot: &gtk::Snapshot, points: &[(f32, f32)], color: Color, width: f32) {
    let b = gsk::PathBuilder::new();
    for (i, (x, y)) in points.iter().enumerate() {
        if i == 0 { b.move_to(*x, *y) } else { b.line_to(*x, *y) }
    }
    round_stroke(snapshot, &b.to_path(), color, width);
}

fn round_stroke(snapshot: &gtk::Snapshot, path: &gsk::Path, color: Color, width: f32) {
    let stroke = gsk::Stroke::new(width);
    stroke.set_line_cap(gsk::LineCap::Round);
    stroke.set_line_join(gsk::LineJoin::Round);
    snapshot.append_stroke(path, &stroke, &color.rgba());
}

/// An arc from `start` through `sweep` (radians, clockwise on the screen).
fn arc(b: &gsk::PathBuilder, cx: f32, cy: f32, r: f32, start: f32, sweep: f32) {
    let steps = 24;
    for i in 0..=steps {
        let a = start + sweep * i as f32 / steps as f32;
        let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
        if i == 0 { b.move_to(x, y) } else { b.line_to(x, y) }
    }
}

/// An open arrow head at (x, y) that points in `direction` (radians).
fn arrow_head(snapshot: &gtk::Snapshot, x: f32, y: f32, direction: f32, length: f32, color: Color, width: f32) {
    let back = direction + PI;
    let points = [
        (x + length * (back + 0.6).cos(), y + length * (back + 0.6).sin()),
        (x, y),
        (x + length * (back - 0.6).cos(), y + length * (back - 0.6).sin()),
    ];
    polyline(snapshot, &points, color, width);
}

pub fn paint(snapshot: &gtk::Snapshot, kind: Icon, w: f32, h: f32, color: Color) {
    let s = w.min(h);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let line = (s / 11.0).max(1.0);
    let r = s * 0.42;
    let ink = tone::SCREEN;
    // Points relative to the centre, in parts of the size.
    let at = |x: f32, y: f32| (cx + x * s, cy + y * s);
    let ring = || draw::stroke(snapshot, &circle(cx, cy, r), color, line);
    let disc = || draw::fill(snapshot, &circle(cx, cy, r), color);
    let check = |c: Color, width: f32| polyline(snapshot, &[at(-0.19, 0.0), at(-0.05, 0.14), at(0.2, -0.14)], c, width);
    let question = |c: Color| {
        let b = gsk::PathBuilder::new();
        b.move_to(cx - s * 0.13, cy - s * 0.1);
        b.cubic_to(cx - s * 0.13, cy - s * 0.3, cx + s * 0.15, cy - s * 0.3, cx + s * 0.14, cy - s * 0.1);
        b.cubic_to(cx + s * 0.13, cy + s * 0.01, cx, cy + s * 0.01, cx, cy + s * 0.1);
        round_stroke(snapshot, &b.to_path(), c, line * 1.1);
        draw::fill(snapshot, &circle(cx, cy + s * 0.21, line * 0.75), c);
    };
    let b = gsk::PathBuilder::new();
    match kind {
        Icon::Search => {
            b.add_circle(&draw::point(cx - s * 0.08, cy - s * 0.08), s * 0.3);
            draw::stroke(snapshot, &b.to_path(), color, line * 1.2);
            draw::rule(snapshot, cx + s * 0.14, cy + s * 0.14, cx + s * 0.42, cy + s * 0.42, color, line * 1.4, &[]);
        }
        Icon::Dot => draw::fill(snapshot, &circle(cx, cy, s * 0.45), color),
        Icon::Ring => draw::stroke(snapshot, &circle(cx, cy, s * 0.4), color, line),
        Icon::RingDashed | Icon::RingDotted => {
            let dashed = kind == Icon::RingDashed;
            let count = if dashed { 8 } else { 12 };
            for i in 0..count {
                let a = i as f32 / count as f32 * TAU;
                if dashed {
                    let span = TAU / count as f32 * 0.28;
                    let dash = gsk::PathBuilder::new();
                    dash.move_to(cx + r * (a - span).cos(), cy + r * (a - span).sin());
                    dash.line_to(cx + r * (a + span).cos(), cy + r * (a + span).sin());
                    draw::stroke(snapshot, &dash.to_path(), color, line * 1.2);
                } else {
                    draw::fill(snapshot, &circle(cx + r * a.cos(), cy + r * a.sin(), line * 0.75), color);
                }
            }
        }
        Icon::HalfFilled => {
            ring();
            arc(&b, cx, cy, r, -FRAC_PI_2, -PI);
            b.close();
            draw::fill(snapshot, &b.to_path(), color);
        }
        Icon::Stored => {
            // A circle that is open at the top left, with an arrow head, and the hands.
            let rs = s * 0.38;
            let start = 200f32.to_radians();
            arc(&b, cx, cy, rs, start, 300f32.to_radians());
            draw::stroke(snapshot, &b.to_path(), color, line);
            let (ax, ay) = (cx + rs * start.cos(), cy + rs * start.sin());
            let head = gsk::PathBuilder::new();
            head.move_to(ax - s * 0.14, ay - s * 0.02);
            head.line_to(ax, ay + s * 0.12);
            head.line_to(ax + s * 0.1, ay - s * 0.06);
            draw::stroke(snapshot, &head.to_path(), color, line);
            draw::rule(snapshot, cx, cy, cx, cy - rs * 0.6, color, line, &[]);
            draw::rule(snapshot, cx, cy, cx + rs * 0.45, cy + rs * 0.3, color, line, &[]);
        }
        Icon::Clock => {
            ring();
            polyline(snapshot, &[at(0.0, -0.24), at(0.0, 0.0), at(0.16, 0.1)], color, line * 1.2);
        }
        Icon::Hourglass => {
            let (left, right, top, bottom) = (cx - s * 0.3, cx + s * 0.3, cy - s * 0.44, cy + s * 0.44);
            draw::rule(snapshot, left - s * 0.06, top, right + s * 0.06, top, color, line, &[]);
            draw::rule(snapshot, left - s * 0.06, bottom, right + s * 0.06, bottom, color, line, &[]);
            b.move_to(left, top);
            b.line_to(right, top);
            b.line_to(cx + s * 0.05, cy);
            b.line_to(right, bottom);
            b.line_to(left, bottom);
            b.line_to(cx - s * 0.05, cy);
            b.close();
            draw::stroke(snapshot, &b.to_path(), color, line);
            let sand = gsk::PathBuilder::new();
            sand.move_to(left + s * 0.1, bottom - s * 0.04);
            sand.line_to(right - s * 0.1, bottom - s * 0.04);
            sand.line_to(cx, cy + s * 0.14);
            sand.close();
            draw::fill(snapshot, &sand.to_path(), color);
        }
        Icon::Warning => {
            b.move_to(cx, h * 0.08);
            b.line_to(w * 0.96, h * 0.9);
            b.line_to(w * 0.04, h * 0.9);
            b.close();
            let stroke = gsk::Stroke::new(line * 1.1);
            stroke.set_line_join(gsk::LineJoin::Round);
            snapshot.append_stroke(&b.to_path(), &stroke, &color.rgba());
            draw::rule(snapshot, cx, h * 0.36, cx, h * 0.64, color, line * 1.2, &[]);
            draw::fill_rect(snapshot, cx - line * 0.6, h * 0.71, line * 1.2, line * 1.2, color);
        }
        Icon::WarningFill => {
            b.move_to(cx, cy - s * 0.42);
            b.line_to(cx + s * 0.47, cy + s * 0.38);
            b.line_to(cx - s * 0.47, cy + s * 0.38);
            b.close();
            draw::fill(snapshot, &b.to_path(), color);
            polyline(snapshot, &[at(0.0, -0.15), at(0.0, 0.1)], ink, line * 1.4);
            draw::fill(snapshot, &circle(cx, cy + s * 0.24, line * 0.8), ink);
        }
        Icon::ExclamationCircle => {
            ring();
            polyline(snapshot, &[at(0.0, -0.2), at(0.0, 0.06)], color, line * 1.3);
            draw::fill(snapshot, &circle(cx, cy + s * 0.19, line * 0.8), color);
        }
        Icon::QuestionCircle => {
            ring();
            question(color);
        }
        Icon::Copy => {
            draw::stroke(snapshot, &draw::rect_path(w * 0.32, h * 0.12, w * 0.52, h * 0.6), color, line);
            draw::stroke(snapshot, &draw::rect_path(w * 0.14, h * 0.3, w * 0.52, h * 0.6), color, line);
        }
        Icon::Save => {
            b.move_to(w * 0.3, h * 0.38);
            b.line_to(w * 0.14, h * 0.38);
            b.line_to(w * 0.14, h * 0.92);
            b.line_to(w * 0.86, h * 0.92);
            b.line_to(w * 0.86, h * 0.38);
            b.line_to(w * 0.7, h * 0.38);
            draw::stroke(snapshot, &b.to_path(), color, line);
            draw::rule(snapshot, w * 0.5, h * 0.06, w * 0.5, h * 0.68, color, line, &[]);
            polyline(snapshot, &[(w * 0.32, h * 0.5), (w * 0.5, h * 0.68), (w * 0.68, h * 0.5)], color, line);
        }
        Icon::Check => polyline(snapshot, &[(w * 0.15, h * 0.52), (w * 0.4, h * 0.78), (w * 0.86, h * 0.2)], color, line * 1.4),
        Icon::CheckCircle => {
            ring();
            check(color, line * 1.3);
        }
        Icon::CheckCircleFill => {
            disc();
            check(ink, line * 1.5);
        }
        Icon::Seal => {
            // A circle with a waved edge, and a check mark cut out of it.
            let points = 48;
            for i in 0..=points {
                let a = i as f32 / points as f32 * TAU;
                let radius = r * (0.92 + 0.08 * (a * 8.0).cos());
                let (x, y) = (cx + radius * a.cos(), cy + radius * a.sin());
                if i == 0 { b.move_to(x, y) } else { b.line_to(x, y) }
            }
            b.close();
            draw::fill(snapshot, &b.to_path(), color);
            check(ink, line * 1.4);
        }
        Icon::Cross => {
            polyline(snapshot, &[at(-0.28, -0.28), at(0.28, 0.28)], color, line * 1.5);
            polyline(snapshot, &[at(0.28, -0.28), at(-0.28, 0.28)], color, line * 1.5);
        }
        Icon::CrossCircle | Icon::CrossCircleFill => {
            let c = if kind == Icon::CrossCircleFill {
                disc();
                ink
            } else {
                ring();
                color
            };
            polyline(snapshot, &[at(-0.15, -0.15), at(0.15, 0.15)], c, line * 1.3);
            polyline(snapshot, &[at(-0.15, 0.15), at(0.15, -0.15)], c, line * 1.3);
        }
        Icon::CrossOctagonFill => {
            for i in 0..8 {
                let a = (22.5f32 + i as f32 * 45.0).to_radians();
                let (x, y) = (cx + r * 1.05 * a.cos(), cy + r * 1.05 * a.sin());
                if i == 0 { b.move_to(x, y) } else { b.line_to(x, y) }
            }
            b.close();
            draw::fill(snapshot, &b.to_path(), color);
            polyline(snapshot, &[at(-0.16, -0.16), at(0.16, 0.16)], ink, line * 1.4);
            polyline(snapshot, &[at(-0.16, 0.16), at(0.16, -0.16)], ink, line * 1.4);
        }
        Icon::MinusCircle => {
            ring();
            polyline(snapshot, &[at(-0.2, 0.0), at(0.2, 0.0)], color, line * 1.3);
        }
        Icon::SlashCircle => {
            ring();
            polyline(snapshot, &[at(-0.3, 0.3), at(0.3, -0.3)], color, line);
        }
        Icon::NoSign => {
            ring();
            let d = r * std::f32::consts::FRAC_1_SQRT_2;
            draw::rule(snapshot, cx - d, cy - d, cx + d, cy + d, color, line * 1.2, &[]);
        }
        Icon::Diamond => {
            b.move_to(cx, cy - s * 0.4);
            b.line_to(cx + s * 0.32, cy);
            b.line_to(cx, cy + s * 0.4);
            b.line_to(cx - s * 0.32, cy);
            b.close();
            draw::fill(snapshot, &b.to_path(), color);
        }
        Icon::Chevron => polyline(snapshot, &[(w * 0.38, h * 0.2), (w * 0.62, h * 0.5), (w * 0.38, h * 0.8)], color, line),
        Icon::ChevronLeft => polyline(snapshot, &[at(0.15, -0.36), at(-0.17, 0.0), at(0.15, 0.36)], color, line * 1.3),
        Icon::ChevronRight => polyline(snapshot, &[at(-0.15, -0.36), at(0.17, 0.0), at(-0.15, 0.36)], color, line * 1.3),
        Icon::ChevronUp => polyline(snapshot, &[(w * 0.2, h * 0.62), (w * 0.5, h * 0.34), (w * 0.8, h * 0.62)], color, line * 1.3),
        Icon::Down => polyline(snapshot, &[(w * 0.2, h * 0.38), (w * 0.5, h * 0.66), (w * 0.8, h * 0.38)], color, line * 1.3),
        Icon::TriangleRight | Icon::TriangleDown | Icon::TriangleUp => {
            // A solid triangle that fills the width of its square.
            match kind {
                Icon::TriangleRight => {
                    b.move_to(w * 0.14, 0.0);
                    b.line_to(w, h / 2.0);
                    b.line_to(w * 0.14, h);
                }
                Icon::TriangleDown => {
                    b.move_to(0.0, h * 0.14);
                    b.line_to(w, h * 0.14);
                    b.line_to(w / 2.0, h);
                }
                _ => {
                    b.move_to(0.0, h * 0.86);
                    b.line_to(w, h * 0.86);
                    b.line_to(w / 2.0, 0.0);
                }
            }
            b.close();
            draw::fill(snapshot, &b.to_path(), color);
        }
        Icon::ArrowLeft | Icon::ArrowRight => {
            let sign = if kind == Icon::ArrowLeft { -1.0 } else { 1.0 };
            polyline(snapshot, &[at(-0.42 * sign, 0.0), at(0.42 * sign, 0.0)], color, line * 1.2);
            arrow_head(snapshot, cx + 0.42 * sign * s, cy, if sign < 0.0 { PI } else { 0.0 }, s * 0.36, color, line * 1.2);
        }
        Icon::ArrowsLeftRight => {
            for (y, left) in [(cy - s * 0.17, true), (cy + s * 0.17, false)] {
                let (from, to) = if left { (w * 0.85, w * 0.15) } else { (w * 0.15, w * 0.85) };
                polyline(snapshot, &[(from, y), (to, y)], color, line);
                arrow_head(snapshot, to, y, if left { PI } else { 0.0 }, s * 0.2, color, line);
            }
        }
        Icon::Refresh => {
            // A circle open at the upper right, with an arrow head at the end of its
            // clockwise turn.
            let (start, sweep) = (-30f32.to_radians(), 300f32.to_radians());
            arc(&b, cx, cy + s * 0.03, s * 0.38, start, sweep);
            round_stroke(snapshot, &b.to_path(), color, line * 1.2);
            let end = start + sweep;
            let (ax, ay) = (cx + s * 0.38 * end.cos(), cy + s * 0.03 + s * 0.38 * end.sin());
            arrow_head(snapshot, ax, ay, end + FRAC_PI_2, s * 0.3, color, line * 1.2);
        }
        Icon::Circlepath => {
            for (start, end) in [(200.0f32, 340.0f32), (20.0, 160.0)] {
                let (a0, a1) = (start.to_radians(), end.to_radians());
                let part = gsk::PathBuilder::new();
                arc(&part, cx, cy, r, a0, a1 - a0);
                round_stroke(snapshot, &part.to_path(), color, line * 1.1);
                let (x, y) = (cx + r * a1.cos(), cy + r * a1.sin());
                let (tx, ty) = (-a1.sin(), a1.cos());
                let head = gsk::PathBuilder::new();
                head.move_to(x + tx * s * 0.19 + a1.cos() * s * 0.15, y + ty * s * 0.19 + a1.sin() * s * 0.15);
                head.line_to(x + tx * s * 0.19 - a1.cos() * s * 0.15, y + ty * s * 0.19 - a1.sin() * s * 0.15);
                head.line_to(x - tx * s * 0.08, y - ty * s * 0.08);
                head.close();
                draw::fill(snapshot, &head.to_path(), color);
            }
        }
        Icon::ArrowUpCircle | Icon::ArrowDownCircleFill => {
            let down = kind == Icon::ArrowDownCircleFill;
            let c = if down {
                disc();
                ink
            } else {
                ring();
                color
            };
            let sign = if down { 1.0 } else { -1.0 };
            polyline(snapshot, &[at(0.0, -0.2 * sign), at(0.0, 0.2 * sign)], c, line * 1.3);
            polyline(snapshot, &[at(-0.16, 0.05 * sign), at(0.0, 0.2 * sign), at(0.16, 0.05 * sign)], c, line * 1.3);
        }
        Icon::Branch => {
            // A stem that divides into two arrows.
            polyline(snapshot, &[at(0.0, 0.44), at(0.0, 0.06), at(-0.32, -0.3)], color, line * 1.2);
            polyline(snapshot, &[at(0.0, 0.06), at(0.32, -0.3)], color, line * 1.2);
            polyline(snapshot, &[at(-0.32, -0.04), at(-0.32, -0.3), at(-0.06, -0.3)], color, line * 1.2);
            polyline(snapshot, &[at(0.32, -0.04), at(0.32, -0.3), at(0.06, -0.3)], color, line * 1.2);
        }
        Icon::Chart => {
            // Two axes and a line that rises.
            polyline(snapshot, &[(w * 0.08, h * 0.04), (w * 0.08, h * 0.92), (w * 0.96, h * 0.92)], color, line);
            polyline(snapshot, &[(w * 0.22, h * 0.72), (w * 0.44, h * 0.42), (w * 0.64, h * 0.6), (w * 0.92, h * 0.18)], color, line);
        }
        Icon::Link => {
            // Two links of a chain, at an angle.
            for (x, y) in [(cx - s * 0.13, cy + s * 0.13), (cx + s * 0.13, cy - s * 0.13)] {
                snapshot.save();
                snapshot.translate(&draw::point(x, y));
                snapshot.rotate(-45.0);
                let rounded = gsk::RoundedRect::from_rect(draw::rect(-s * 0.24, -s * 0.12, s * 0.48, s * 0.24), s * 0.12);
                let link = gsk::PathBuilder::new();
                link.add_rounded_rect(&rounded);
                draw::stroke(snapshot, &link.to_path(), color, line);
                snapshot.restore();
            }
        }
        Icon::Bookmark => {
            b.move_to(w * 0.25, h * 0.1);
            b.line_to(w * 0.75, h * 0.1);
            b.line_to(w * 0.75, h * 0.9);
            b.line_to(w * 0.5, h * 0.68);
            b.line_to(w * 0.25, h * 0.9);
            b.close();
            draw::stroke(snapshot, &b.to_path(), color, line);
        }
        Icon::Cube => {
            let (rc, t) = (s * 0.4, s * 0.22);
            b.move_to(cx, cy - rc);
            b.line_to(cx + rc, cy - rc + t);
            b.line_to(cx + rc, cy + rc - t);
            b.line_to(cx, cy + rc);
            b.line_to(cx - rc, cy + rc - t);
            b.line_to(cx - rc, cy - rc + t);
            b.close();
            b.move_to(cx - rc, cy - rc + t);
            b.line_to(cx, cy - rc + 2.0 * t);
            b.line_to(cx + rc, cy - rc + t);
            b.move_to(cx, cy - rc + 2.0 * t);
            b.line_to(cx, cy + rc);
            draw::stroke(snapshot, &b.to_path(), color, line);
        }
        Icon::PersonCircle => {
            draw::stroke(snapshot, &circle(cx, cy, s * 0.44), color, line);
            draw::stroke(snapshot, &circle(cx, cy - s * 0.1, s * 0.14), color, line);
            b.move_to(cx - s * 0.26, cy + s * 0.32);
            b.quad_to(cx, cy + s * 0.02, cx + s * 0.26, cy + s * 0.32);
            draw::stroke(snapshot, &b.to_path(), color, line);
        }
        Icon::HexagonGrid => {
            // A ring of six small hexagons around one, in a circle.
            let rh = s * 0.11;
            let hexagon = |x: f32, y: f32| {
                let hb = gsk::PathBuilder::new();
                for i in 0..6 {
                    let a = PI / 3.0 * i as f32 + PI / 6.0;
                    let (px, py) = (x + rh * a.cos(), y + rh * a.sin());
                    if i == 0 { hb.move_to(px, py) } else { hb.line_to(px, py) }
                }
                hb.close();
                draw::fill(snapshot, &hb.to_path(), color);
            };
            hexagon(cx, cy);
            for i in 0..6 {
                let a = PI / 3.0 * i as f32;
                hexagon(cx + s * 0.27 * a.cos(), cy + s * 0.27 * a.sin());
            }
            draw::stroke(snapshot, &circle(cx, cy, s * 0.46), color, line * 0.9);
        }
        Icon::CreditCard => {
            draw::stroke(snapshot, &draw::rect_path(w * 0.1, h * 0.22, w * 0.8, h * 0.56), color, line);
            draw::fill_rect(snapshot, w * 0.1, h * 0.34, w * 0.8, h * 0.1, color);
            draw::rule(snapshot, w * 0.2, h * 0.62, w * 0.42, h * 0.62, color, line, &[]);
        }
        Icon::Tray | Icon::TrayFull => {
            b.move_to(w * 0.1, h * 0.55);
            b.line_to(w * 0.22, h * 0.25);
            b.line_to(w * 0.78, h * 0.25);
            b.line_to(w * 0.9, h * 0.55);
            b.line_to(w * 0.9, h * 0.82);
            b.line_to(w * 0.1, h * 0.82);
            b.close();
            b.move_to(w * 0.1, h * 0.55);
            b.line_to(w * 0.36, h * 0.55);
            b.line_to(w * 0.42, h * 0.65);
            b.line_to(w * 0.58, h * 0.65);
            b.line_to(w * 0.64, h * 0.55);
            b.line_to(w * 0.9, h * 0.55);
            draw::stroke(snapshot, &b.to_path(), color, line);
            if kind == Icon::TrayFull {
                draw::rule(snapshot, w * 0.3, h * 0.36, w * 0.7, h * 0.36, color, line, &[]);
                draw::rule(snapshot, w * 0.26, h * 0.46, w * 0.74, h * 0.46, color, line, &[]);
            }
        }
        Icon::ServerRack => {
            for row in 0..3 {
                let y = h * 0.14 + row as f32 * h * 0.26;
                draw::stroke(snapshot, &draw::rect_path(w * 0.12, y, w * 0.76, h * 0.2), color, line * 0.45);
                draw::fill(snapshot, &draw::ellipse_path(w * 0.22, y + h * 0.07, h * 0.06, h * 0.06), color);
                draw::rule(snapshot, w * 0.5, y + h * 0.1, w * 0.78, y + h * 0.1, color, line * 0.4, &[]);
            }
        }
        Icon::Disconnected => {
            draw::stroke(snapshot, &circle(cx, cy, s * 0.44), color, line);
            polyline(
                snapshot,
                &[(w * 0.2, cy + s * 0.04), (w * 0.48, cy - s * 0.12), (w * 0.48, cy + s * 0.04), (w * 0.8, cy - s * 0.12)],
                color,
                line,
            );
        }
        Icon::Menu => {
            for y in [0.28f32, 0.5, 0.72] {
                draw::rule(snapshot, w * 0.18, h * y, w * 0.82, h * y, color, line, &[]);
            }
        }
    }
}

pub fn label_color() -> Color {
    tone::LABEL
}
