//! The few marks that mean something, drawn by the app: the Linux forms of the SF Symbols
//! that the Swift app uses. The app draws them so that it does not depend on an icon theme.

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
    /// `clock.arrow.circlepath`: a stored observation.
    Stored,
    /// `exclamationmark.triangle`
    Warning,
    /// `doc.on.doc`
    Copy,
    /// `checkmark`
    Check,
    /// `chevron.compact.right`
    Chevron,
    /// `bookmark`
    Bookmark,
    /// `bolt.horizontal.circle`: not connected.
    Disconnected,
    /// The three lines of a menu.
    Menu,
    /// `chevron.down`
    Down,
}

/// A mark of `size` in `color`.
pub fn icon(kind: Icon, size: i32, color: Color) -> Canvas {
    let canvas = Canvas::new(size, size, move |_, snapshot, w, h| paint(snapshot, kind, w, h, color));
    canvas.set_valign(gtk::Align::Center);
    canvas.set_can_target(false);
    canvas
}

/// Changes the colour or the kind of a mark made by [`icon`].
pub fn set_icon(canvas: &Canvas, kind: Icon, color: Color) {
    canvas.set_draw(move |_, snapshot, w, h| paint(snapshot, kind, w, h, color));
}

pub fn paint(snapshot: &gtk::Snapshot, kind: Icon, w: f32, h: f32, color: Color) {
    let s = w.min(h);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let line = (s / 11.0).max(1.0);
    let b = gsk::PathBuilder::new();
    match kind {
        Icon::Search => {
            b.add_circle(&draw::point(cx - s * 0.08, cy - s * 0.08), s * 0.3);
            draw::stroke(snapshot, &b.to_path(), color, line * 1.2);
            draw::rule(snapshot, cx + s * 0.14, cy + s * 0.14, cx + s * 0.42, cy + s * 0.42, color, line * 1.4, &[]);
        }
        Icon::Dot => {
            b.add_circle(&draw::point(cx, cy), s * 0.45);
            draw::fill(snapshot, &b.to_path(), color);
        }
        Icon::Ring => {
            b.add_circle(&draw::point(cx, cy), s * 0.4);
            draw::stroke(snapshot, &b.to_path(), color, line.max(1.0));
        }
        Icon::Stored => {
            // A circle that is open at the top left, with an arrow head, and the hands.
            let r = s * 0.38;
            let start = 200f32.to_radians();
            let steps = 24;
            for i in 0..=steps {
                let a = start + (i as f32 / steps as f32) * 300f32.to_radians();
                let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
                if i == 0 { b.move_to(x, y) } else { b.line_to(x, y) }
            }
            draw::stroke(snapshot, &b.to_path(), color, line);
            let (ax, ay) = (cx + r * start.cos(), cy + r * start.sin());
            let head = gsk::PathBuilder::new();
            head.move_to(ax - s * 0.14, ay - s * 0.02);
            head.line_to(ax, ay + s * 0.12);
            head.line_to(ax + s * 0.1, ay - s * 0.06);
            draw::stroke(snapshot, &head.to_path(), color, line);
            draw::rule(snapshot, cx, cy, cx, cy - r * 0.6, color, line, &[]);
            draw::rule(snapshot, cx, cy, cx + r * 0.45, cy + r * 0.3, color, line, &[]);
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
        Icon::Copy => {
            draw::stroke(snapshot, &draw::rect_path(w * 0.32, h * 0.12, w * 0.52, h * 0.6), color, line);
            draw::stroke(snapshot, &draw::rect_path(w * 0.14, h * 0.3, w * 0.52, h * 0.6), color, line);
        }
        Icon::Check => {
            b.move_to(w * 0.15, h * 0.52);
            b.line_to(w * 0.4, h * 0.78);
            b.line_to(w * 0.86, h * 0.2);
            draw::stroke(snapshot, &b.to_path(), color, line * 1.4);
        }
        Icon::Chevron => {
            b.move_to(w * 0.38, h * 0.2);
            b.line_to(w * 0.62, h * 0.5);
            b.line_to(w * 0.38, h * 0.8);
            draw::stroke(snapshot, &b.to_path(), color, line);
        }
        Icon::Down => {
            b.move_to(w * 0.2, h * 0.38);
            b.line_to(w * 0.5, h * 0.66);
            b.line_to(w * 0.8, h * 0.38);
            draw::stroke(snapshot, &b.to_path(), color, line * 1.3);
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
        Icon::Disconnected => {
            b.add_circle(&draw::point(cx, cy), s * 0.44);
            draw::stroke(snapshot, &b.to_path(), color, line);
            let bolt = gsk::PathBuilder::new();
            bolt.move_to(w * 0.2, cy + s * 0.04);
            bolt.line_to(w * 0.48, cy - s * 0.12);
            bolt.line_to(w * 0.48, cy + s * 0.04);
            bolt.line_to(w * 0.8, cy - s * 0.12);
            draw::stroke(snapshot, &bolt.to_path(), color, line);
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
