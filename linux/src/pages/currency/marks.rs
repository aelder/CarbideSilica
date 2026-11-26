//! The marks of the currency page that the shared icon set does not have: the Linux forms
//! of the SF Symbols that `CurrencyStateSeek.swift` and `NotarizationLanes.swift` use, and
//! SwiftUI's `Label` (a mark and a text in one colour).

use gtk::gsk;
use gtk::prelude::*;

use crate::page;
use crate::theme::{draw, tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::icons::{self, Icon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// `exclamationmark.triangle`
    Warning,
    /// `exclamationmark.triangle.fill`
    WarningFill,
    /// `arrow.down.circle.fill`
    ArrowDownFill,
    /// `arrow.up.circle`
    ArrowUp,
    /// `chevron.left`
    ChevronLeft,
    /// `chevron.right`
    ChevronRight,
    /// `checkmark.seal.fill`
    Seal,
    /// `circle.dashed`
    Dashed,
    /// `circle.dotted`
    Dotted,
    /// `arrow.triangle.branch`
    Branch,
    /// `checkmark.circle.fill`
    CheckFill,
    /// `checkmark.circle`
    Check,
    /// `exclamationmark.circle`
    Exclamation,
    /// `circle.lefthalf.filled`
    HalfFilled,
    /// `clock`
    Clock,
    /// `slash.circle`
    Slash,
    /// `xmark.circle`
    Cross,
    /// `questionmark.circle`
    Question,
}

/// A mark of `size` in `color`.
pub fn mark(kind: Mark, size: i32, color: Color) -> Canvas {
    if kind == Mark::Warning {
        return icons::icon(Icon::Warning, size, color);
    }
    let canvas = Canvas::new(size, size, move |_, snapshot, w, h| paint(snapshot, kind, w, h, color));
    canvas.set_valign(gtk::Align::Center);
    canvas.set_can_target(false);
    canvas
}

fn circle(cx: f32, cy: f32, r: f32) -> gsk::Path {
    let b = gsk::PathBuilder::new();
    b.add_circle(&draw::point(cx, cy), r);
    b.to_path()
}

/// The parts of a mark that are cut out of a filled circle are drawn in the black of the
/// display.
fn paint(snapshot: &gtk::Snapshot, kind: Mark, w: f32, h: f32, color: Color) {
    let s = w.min(h);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let r = s * 0.43;
    let line = (s / 12.0).max(1.0);
    let ink = tone::SCREEN;
    let ring = || draw::stroke(snapshot, &circle(cx, cy, r), color, line);
    let disc = || draw::fill(snapshot, &circle(cx, cy, r), color);
    let polyline = |points: &[(f32, f32)], c: Color, width: f32| {
        let b = gsk::PathBuilder::new();
        for (i, (x, y)) in points.iter().enumerate() {
            if i == 0 { b.move_to(cx + x * s, cy + y * s) } else { b.line_to(cx + x * s, cy + y * s) }
        }
        let stroke = gsk::Stroke::new(width);
        stroke.set_line_cap(gsk::LineCap::Round);
        stroke.set_line_join(gsk::LineJoin::Round);
        snapshot.append_stroke(&b.to_path(), &stroke, &c.rgba());
    };
    match kind {
        Mark::Warning => {}
        Mark::WarningFill => {
            let b = gsk::PathBuilder::new();
            b.move_to(cx, cy - s * 0.42);
            b.line_to(cx + s * 0.47, cy + s * 0.38);
            b.line_to(cx - s * 0.47, cy + s * 0.38);
            b.close();
            draw::fill(snapshot, &b.to_path(), color);
            polyline(&[(0.0, -0.15), (0.0, 0.1)], ink, line * 1.4);
            draw::fill(snapshot, &circle(cx, cy + s * 0.24, line * 0.8), ink);
        }
        Mark::ArrowDownFill | Mark::ArrowUp => {
            let down = kind == Mark::ArrowDownFill;
            let c = if down {
                disc();
                ink
            } else {
                ring();
                color
            };
            let sign = if down { 1.0 } else { -1.0 };
            polyline(&[(0.0, -0.2 * sign), (0.0, 0.2 * sign)], c, line * 1.3);
            polyline(&[(-0.16, 0.05 * sign), (0.0, 0.2 * sign), (0.16, 0.05 * sign)], c, line * 1.3);
        }
        Mark::ChevronLeft => polyline(&[(0.12, -0.3), (-0.12, 0.0), (0.12, 0.3)], color, line * 1.4),
        Mark::ChevronRight => polyline(&[(-0.12, -0.3), (0.12, 0.0), (-0.12, 0.3)], color, line * 1.4),
        Mark::Seal => {
            // A circle with a waved edge, and a check mark cut out of it.
            let b = gsk::PathBuilder::new();
            let points = 48;
            for i in 0..=points {
                let a = i as f32 / points as f32 * std::f32::consts::TAU;
                let radius = r * (0.92 + 0.08 * (a * 8.0).cos());
                let (x, y) = (cx + radius * a.cos(), cy + radius * a.sin());
                if i == 0 { b.move_to(x, y) } else { b.line_to(x, y) }
            }
            b.close();
            draw::fill(snapshot, &b.to_path(), color);
            polyline(&[(-0.17, 0.0), (-0.05, 0.13), (0.18, -0.13)], ink, line * 1.4);
        }
        Mark::Dashed | Mark::Dotted => {
            let count = if kind == Mark::Dashed { 8 } else { 12 };
            for i in 0..count {
                let a = i as f32 / count as f32 * std::f32::consts::TAU;
                if kind == Mark::Dashed {
                    let span = std::f32::consts::TAU / count as f32 * 0.28;
                    let b = gsk::PathBuilder::new();
                    b.move_to(cx + r * (a - span).cos(), cy + r * (a - span).sin());
                    b.line_to(cx + r * (a + span).cos(), cy + r * (a + span).sin());
                    draw::stroke(snapshot, &b.to_path(), color, line * 1.2);
                } else {
                    draw::fill(snapshot, &circle(cx + r * a.cos(), cy + r * a.sin(), line * 0.75), color);
                }
            }
        }
        Mark::Branch => {
            polyline(&[(0.0, 0.42), (0.0, 0.05), (-0.3, -0.25)], color, line * 1.3);
            polyline(&[(0.0, 0.05), (0.3, -0.25)], color, line * 1.3);
            polyline(&[(-0.3, -0.05), (-0.3, -0.25), (-0.1, -0.25)], color, line * 1.3);
            polyline(&[(0.3, -0.05), (0.3, -0.25), (0.1, -0.25)], color, line * 1.3);
        }
        Mark::CheckFill => {
            disc();
            polyline(&[(-0.17, 0.0), (-0.05, 0.13), (0.18, -0.13)], ink, line * 1.4);
        }
        Mark::Check => {
            ring();
            polyline(&[(-0.17, 0.0), (-0.05, 0.13), (0.18, -0.13)], color, line * 1.3);
        }
        Mark::Exclamation => {
            ring();
            polyline(&[(0.0, -0.2), (0.0, 0.06)], color, line * 1.3);
            draw::fill(snapshot, &circle(cx, cy + s * 0.19, line * 0.8), color);
        }
        Mark::HalfFilled => {
            ring();
            let b = gsk::PathBuilder::new();
            b.move_to(cx, cy - r);
            let steps = 16;
            for i in 0..=steps {
                let a = -std::f32::consts::FRAC_PI_2 - i as f32 / steps as f32 * std::f32::consts::PI;
                b.line_to(cx + r * a.cos(), cy + r * a.sin());
            }
            b.close();
            draw::fill(snapshot, &b.to_path(), color);
        }
        Mark::Clock => {
            ring();
            polyline(&[(0.0, -0.24), (0.0, 0.0), (0.16, 0.1)], color, line * 1.2);
        }
        Mark::Slash => {
            ring();
            polyline(&[(-0.3, 0.3), (0.3, -0.3)], color, line);
        }
        Mark::Cross => {
            ring();
            polyline(&[(-0.14, -0.14), (0.14, 0.14)], color, line * 1.2);
            polyline(&[(-0.14, 0.14), (0.14, -0.14)], color, line * 1.2);
        }
        Mark::Question => {
            ring();
            let b = gsk::PathBuilder::new();
            b.move_to(cx - s * 0.12, cy - s * 0.1);
            b.cubic_to(cx - s * 0.12, cy - s * 0.3, cx + s * 0.14, cy - s * 0.3, cx + s * 0.13, cy - s * 0.1);
            b.cubic_to(cx + s * 0.12, cy + s * 0.0, cx, cy - s * 0.0, cx, cy + s * 0.08);
            let stroke = gsk::Stroke::new(line * 1.2);
            stroke.set_line_cap(gsk::LineCap::Round);
            snapshot.append_stroke(&b.to_path(), &stroke, &color.rgba());
            draw::fill(snapshot, &circle(cx, cy + s * 0.2, line * 0.75), color);
        }
    }
}

/// SwiftUI's `Label`: a mark and a text, both in `color`. `classes` are the text's classes.
pub fn label(kind: Mark, text: &str, color: Color, classes: &[&str]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let size = if classes.contains(&"callout") { 13 } else { 14 };
    row.append(&mark(kind, size, color));
    let words = page::text(text, classes);
    words.set_markup(&page::span(text, color));
    row.append(&words);
    row
}
