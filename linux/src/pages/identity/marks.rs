//! The SF Symbols of the identity page that `widgets::icons` does not have: the chevrons of
//! the version keys, the arrows of the referral chain and the token trail, the status
//! marks and the mark of a defined currency. They are drawn as the shared icons are.

use gtk::gsk;
use gtk::prelude::*;

use crate::theme::{draw, Color};
use crate::widgets::canvas::Canvas;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// `chevron.left`
    ChevronLeft,
    /// `chevron.right`
    ChevronRight,
    /// `arrow.left`
    ArrowLeft,
    /// `arrow.right`
    ArrowRight,
    /// `checkmark.circle`
    CheckCircle,
    /// `circle.hexagongrid`
    HexagonGrid,
    /// `xmark`
    Cross,
    /// `questionmark.circle`
    Question,
    /// `diamond.fill`
    Diamond,
}

/// A mark in the colour of the text around it (the CSS colour of its widget), so that it
/// follows the hover and press states of the key or link that holds it.
pub fn text_mark(kind: Mark, size: i32) -> Canvas {
    let canvas = Canvas::new(size, size, move |canvas, snapshot, w, h| {
        let rgba = canvas.color();
        let color = Color { r: rgba.red(), g: rgba.green(), b: rgba.blue(), a: rgba.alpha() };
        paint(snapshot, kind, w, h, color);
    });
    canvas.set_valign(gtk::Align::Center);
    canvas.set_can_target(false);
    canvas
}

pub fn mark(kind: Mark, size: i32, color: Color) -> Canvas {
    let canvas = Canvas::new(size, size, move |_, snapshot, w, h| paint(snapshot, kind, w, h, color));
    canvas.set_valign(gtk::Align::Center);
    canvas.set_can_target(false);
    canvas
}

fn line(snapshot: &gtk::Snapshot, points: &[(f32, f32)], color: Color, width: f32) {
    let builder = gsk::PathBuilder::new();
    for (index, (x, y)) in points.iter().enumerate() {
        if index == 0 { builder.move_to(*x, *y) } else { builder.line_to(*x, *y) }
    }
    let stroke = gsk::Stroke::new(width);
    stroke.set_line_cap(gsk::LineCap::Round);
    stroke.set_line_join(gsk::LineJoin::Round);
    snapshot.append_stroke(&builder.to_path(), &stroke, &color.rgba());
}

fn paint(snapshot: &gtk::Snapshot, kind: Mark, w: f32, h: f32, color: Color) {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let s = w.min(h);
    match kind {
        Mark::ChevronLeft => line(snapshot, &[(cx + s * 0.18, cy - s * 0.36), (cx - s * 0.18, cy), (cx + s * 0.18, cy + s * 0.36)], color, 1.4),
        Mark::ChevronRight => line(snapshot, &[(cx - s * 0.18, cy - s * 0.36), (cx + s * 0.18, cy), (cx - s * 0.18, cy + s * 0.36)], color, 1.4),
        Mark::ArrowLeft => {
            line(snapshot, &[(cx + s * 0.4, cy), (cx - s * 0.4, cy)], color, 1.3);
            line(snapshot, &[(cx - s * 0.08, cy - s * 0.32), (cx - s * 0.4, cy), (cx - s * 0.08, cy + s * 0.32)], color, 1.3);
        }
        Mark::ArrowRight => {
            line(snapshot, &[(cx - s * 0.4, cy), (cx + s * 0.4, cy)], color, 1.3);
            line(snapshot, &[(cx + s * 0.08, cy - s * 0.32), (cx + s * 0.4, cy), (cx + s * 0.08, cy + s * 0.32)], color, 1.3);
        }
        Mark::CheckCircle => {
            let builder = gsk::PathBuilder::new();
            builder.add_circle(&draw::point(cx, cy), s * 0.42);
            draw::stroke(snapshot, &builder.to_path(), color, 1.2);
            line(snapshot, &[(cx - s * 0.2, cy + s * 0.02), (cx - s * 0.05, cy + s * 0.17), (cx + s * 0.2, cy - s * 0.15)], color, 1.3);
        }
        Mark::HexagonGrid => {
            // A ring of six small hexagons around one, as `circle.hexagongrid`.
            let r = s * 0.11;
            let hexagon = |x: f32, y: f32| {
                let builder = gsk::PathBuilder::new();
                for i in 0..6 {
                    let a = std::f32::consts::PI / 3.0 * i as f32 + std::f32::consts::PI / 6.0;
                    let (px, py) = (x + r * a.cos(), y + r * a.sin());
                    if i == 0 { builder.move_to(px, py) } else { builder.line_to(px, py) }
                }
                builder.close();
                draw::fill(snapshot, &builder.to_path(), color);
            };
            hexagon(cx, cy);
            for i in 0..6 {
                let a = std::f32::consts::PI / 3.0 * i as f32;
                hexagon(cx + s * 0.27 * a.cos(), cy + s * 0.27 * a.sin());
            }
            let builder = gsk::PathBuilder::new();
            builder.add_circle(&draw::point(cx, cy), s * 0.46);
            draw::stroke(snapshot, &builder.to_path(), color, 1.0);
        }
        Mark::Cross => {
            line(snapshot, &[(cx - s * 0.28, cy - s * 0.28), (cx + s * 0.28, cy + s * 0.28)], color, 1.5);
            line(snapshot, &[(cx + s * 0.28, cy - s * 0.28), (cx - s * 0.28, cy + s * 0.28)], color, 1.5);
        }
        Mark::Question => {
            let builder = gsk::PathBuilder::new();
            builder.add_circle(&draw::point(cx, cy), s * 0.42);
            draw::stroke(snapshot, &builder.to_path(), color, 1.1);
            let curve = gsk::PathBuilder::new();
            curve.move_to(cx - s * 0.14, cy - s * 0.1);
            curve.cubic_to(cx - s * 0.14, cy - s * 0.3, cx + s * 0.16, cy - s * 0.3, cx + s * 0.15, cy - s * 0.1);
            curve.cubic_to(cx + s * 0.14, cy + s * 0.02, cx, cy + s * 0.02, cx, cy + s * 0.12);
            let stroke = gsk::Stroke::new(1.2);
            stroke.set_line_cap(gsk::LineCap::Round);
            snapshot.append_stroke(&curve.to_path(), &stroke, &color.rgba());
            draw::fill_rect(snapshot, cx - 0.7, cy + s * 0.22, 1.4, 1.4, color);
        }
        Mark::Diamond => {
            let builder = gsk::PathBuilder::new();
            builder.move_to(cx, cy - s * 0.4);
            builder.line_to(cx + s * 0.32, cy);
            builder.line_to(cx, cy + s * 0.4);
            builder.line_to(cx - s * 0.32, cy);
            builder.close();
            draw::fill(snapshot, &builder.to_path(), color);
        }
    }
}

/// A child in a thin square outline of `color`, `padding` inside it: the Swift app's
/// `.overlay(Rectangle().strokeBorder(color, lineWidth: 1))` after a padding.
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
