//! The marks of the bookmark list, drawn: the Linux forms of the SF Symbols that the Swift
//! page uses for the kind of a bookmark (cube, arrow.left.arrow.right, person.crop.circle,
//! circle.hexagongrid, creditcard, link, bookmark) and for its cache (tray.full, tray).

use gtk::gsk;
use gtk::prelude::*;

use crate::theme::{draw, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::icons::{self, Icon};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Cube,
    Arrows,
    Person,
    Hexagons,
    Card,
    Link,
    Bookmark,
    TrayFull,
    Tray,
}

pub fn glyph(kind: Glyph, size: i32, color: Color) -> Canvas {
    let canvas = Canvas::new(size, size, move |_, snapshot, w, h| paint(snapshot, kind, w, h, color));
    canvas.set_valign(gtk::Align::Start);
    canvas.set_can_target(false);
    canvas
}

fn paint(snapshot: &gtk::Snapshot, kind: Glyph, w: f32, h: f32, color: Color) {
    let s = w.min(h);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let line = (s / 12.0).max(1.0);
    let b = gsk::PathBuilder::new();
    match kind {
        Glyph::Cube => {
            let (r, t) = (s * 0.4, s * 0.22);
            b.move_to(cx, cy - r);
            b.line_to(cx + r, cy - r + t);
            b.line_to(cx + r, cy + r - t);
            b.line_to(cx, cy + r);
            b.line_to(cx - r, cy + r - t);
            b.line_to(cx - r, cy - r + t);
            b.close();
            b.move_to(cx - r, cy - r + t);
            b.line_to(cx, cy - r + 2.0 * t);
            b.line_to(cx + r, cy - r + t);
            b.move_to(cx, cy - r + 2.0 * t);
            b.line_to(cx, cy + r);
            draw::stroke(snapshot, &b.to_path(), color, line);
        }
        Glyph::Arrows => {
            for (y, left) in [(cy - s * 0.17, true), (cy + s * 0.17, false)] {
                let (from, to) = if left { (w * 0.85, w * 0.15) } else { (w * 0.15, w * 0.85) };
                draw::rule(snapshot, from, y, to, y, color, line, &[]);
                let d = if left { 1.0 } else { -1.0 };
                draw::rule(snapshot, to, y, to + d * s * 0.16, y - s * 0.12, color, line, &[]);
                draw::rule(snapshot, to, y, to + d * s * 0.16, y + s * 0.12, color, line, &[]);
            }
        }
        Glyph::Person => {
            b.add_circle(&draw::point(cx, cy), s * 0.44);
            draw::stroke(snapshot, &b.to_path(), color, line);
            let head = gsk::PathBuilder::new();
            head.add_circle(&draw::point(cx, cy - s * 0.1), s * 0.14);
            draw::stroke(snapshot, &head.to_path(), color, line);
            let body = gsk::PathBuilder::new();
            body.move_to(cx - s * 0.26, cy + s * 0.32);
            body.quad_to(cx, cy + s * 0.02, cx + s * 0.26, cy + s * 0.32);
            draw::stroke(snapshot, &body.to_path(), color, line);
        }
        Glyph::Hexagons => {
            b.add_circle(&draw::point(cx, cy), s * 0.44);
            draw::stroke(snapshot, &b.to_path(), color, line);
            for (dx, dy) in [(0.0, 0.0), (-0.2, 0.0), (0.2, 0.0), (-0.1, -0.18), (0.1, -0.18), (-0.1, 0.18), (0.1, 0.18)] {
                draw::fill(snapshot, &draw::ellipse_path(cx + dx * s - s * 0.05, cy + dy * s - s * 0.05, s * 0.1, s * 0.1), color);
            }
        }
        Glyph::Card => {
            draw::stroke(snapshot, &draw::rect_path(w * 0.1, h * 0.22, w * 0.8, h * 0.56), color, line);
            draw::fill_rect(snapshot, w * 0.1, h * 0.34, w * 0.8, h * 0.1, color);
            draw::rule(snapshot, w * 0.2, h * 0.62, w * 0.42, h * 0.62, color, line, &[]);
        }
        Glyph::Link => {
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
        Glyph::Bookmark => icons::paint(snapshot, Icon::Bookmark, w, h, color),
        Glyph::TrayFull | Glyph::Tray => {
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
            if kind == Glyph::TrayFull {
                draw::rule(snapshot, w * 0.3, h * 0.36, w * 0.7, h * 0.36, color, line, &[]);
                draw::rule(snapshot, w * 0.26, h * 0.46, w * 0.74, h * 0.46, color, line, &[]);
            }
        }
    }
}
