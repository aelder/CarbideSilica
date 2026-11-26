//! Owner: shell (complete). How well a fact is established, as a mark, a colour and a word
//! (`ConfidenceMark` in `TransferTrace.swift`). Used by the block, transaction and identity
//! pages.

use gtk::prelude::*;
use gtk::gsk;

use crate::bridge::api::Confidence;
use crate::theme::{draw, tone, Color};
use crate::widgets::canvas::Canvas;

fn word(level: Confidence) -> &'static str {
    match level {
        Confidence::Verified => "Verified",
        Confidence::Observed => "In progress",
        Confidence::Candidate => "Candidate",
        Confidence::Unknown => "Unresolved",
    }
}

fn color(level: Confidence) -> Color {
    match level {
        Confidence::Verified => tone::GOOD,
        Confidence::Observed => tone::RECORD,
        Confidence::Candidate => tone::LABEL,
        Confidence::Unknown => tone::MINED,
    }
}

/// What the level means, for the tooltip.
pub fn meaning(level: Confidence) -> &'static str {
    match level {
        Confidence::Verified => "Proven by exact match or replay.",
        Confidence::Observed => "A source reported this at a known block. The trace is not complete.",
        Confidence::Candidate => "An estimate. Not confirmed.",
        Confidence::Unknown => "The data is not available.",
    }
}

/// The mark of a level: a sealed check, a half circle, a dashed circle or a question.
fn mark(level: Confidence) -> Canvas {
    let c = color(level);
    let canvas = Canvas::new(13, 13, move |_, snapshot, w, h| {
        let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0 - 1.0);
        let circle = gsk::PathBuilder::new();
        circle.add_circle(&draw::point(cx, cy), r);
        let circle = circle.to_path();
        match level {
            Confidence::Verified => {
                draw::fill(snapshot, &circle, c);
                let check = gsk::PathBuilder::new();
                check.move_to(cx - r * 0.45, cy);
                check.line_to(cx - r * 0.1, cy + r * 0.35);
                check.line_to(cx + r * 0.5, cy - r * 0.35);
                draw::stroke(snapshot, &check.to_path(), tone::SCREEN, 1.6);
            }
            Confidence::Observed => {
                draw::stroke(snapshot, &circle, c, 1.2);
                let half = gsk::PathBuilder::new();
                half.move_to(cx, cy - r);
                for i in 0..=16 {
                    let a = std::f32::consts::PI / 2.0 + std::f32::consts::PI * i as f32 / 16.0;
                    half.line_to(cx + r * a.cos(), cy - r * a.sin());
                }
                half.close();
                draw::fill(snapshot, &half.to_path(), c);
            }
            Confidence::Candidate => {
                let stroke = gsk::Stroke::new(1.2);
                stroke.set_dash(&[2.0, 2.0]);
                snapshot.append_stroke(&circle, &stroke, &c.rgba());
            }
            Confidence::Unknown => {
                draw::stroke(snapshot, &circle, c, 1.2);
                let q = gsk::PathBuilder::new();
                q.move_to(cx - r * 0.35, cy - r * 0.25);
                q.quad_to(cx - r * 0.3, cy - r * 0.6, cx, cy - r * 0.6);
                q.quad_to(cx + r * 0.4, cy - r * 0.55, cx + r * 0.35, cy - r * 0.2);
                q.quad_to(cx + r * 0.3, cy, cx, cy + r * 0.15);
                draw::stroke(snapshot, &q.to_path(), c, 1.2);
                draw::fill_rect(snapshot, cx - 0.7, cy + r * 0.38, 1.4, 1.4, c);
            }
        }
    });
    canvas.set_valign(gtk::Align::Center);
    canvas
}

/// The mark with its word, or with `label` in place of the word.
pub fn confidence_mark(level: Confidence, label: Option<&str>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    row.append(&mark(level));
    let text = gtk::Label::new(Some(label.unwrap_or(word(level))));
    text.add_css_class("callout");
    text.add_css_class("semibold");
    let c = color(level);
    text.set_markup(&crate::page::span(label.unwrap_or(word(level)), c));
    row.append(&text);
    row.set_tooltip_text(Some(meaning(level)));
    row
}
