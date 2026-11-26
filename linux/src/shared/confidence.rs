//! Owner: shell (complete). How well a fact is established, as a mark, a colour and a word
//! (`ConfidenceMark` in `TransferTrace.swift`). Used by the block, transaction and identity
//! pages.

use gtk::prelude::*;

use crate::bridge::api::Confidence;
use crate::theme::{tone, Color};
use crate::widgets::icons::{self, Icon};

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

/// The mark of a level (`ConfidenceMark.symbol`).
fn icon(level: Confidence) -> Icon {
    match level {
        Confidence::Verified => Icon::Seal,
        Confidence::Observed => Icon::HalfFilled,
        Confidence::Candidate => Icon::RingDashed,
        Confidence::Unknown => Icon::QuestionCircle,
    }
}

/// The mark with its word, or with `label` in place of the word.
pub fn confidence_mark(level: Confidence, label: Option<&str>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    row.append(&icons::icon(icon(level), 13, color(level)));
    let text = gtk::Label::new(Some(label.unwrap_or(word(level))));
    text.add_css_class("callout");
    text.add_css_class("semibold");
    let c = color(level);
    text.set_markup(&crate::page::span(label.unwrap_or(word(level)), c));
    row.append(&text);
    row.set_tooltip_text(Some(meaning(level)));
    row
}
