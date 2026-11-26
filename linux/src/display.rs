//! The display settings, as the terminal's Shift-I and Shift-D toggles (`Display.swift`),
//! and dates for block heights (`HeightDates`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gtk::glib;

use crate::bridge::{self, api};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::settings::{self, key};

pub fn shows_names() -> bool {
    settings::bool(key::SHOWS_NAMES, true)
}

pub fn shows_dates() -> bool {
    settings::bool(key::SHOWS_DATES, false)
}

pub fn uses_utc() -> bool {
    settings::bool(key::USE_UTC, false)
}

/// Dates for block heights, loaded in small batches, as the terminal's `height_display`.
///
/// Past heights use the block's own time. Heights above the tip are estimated and shown
/// with "est.". A height whose date is not available keeps its number. A page that shows
/// heights watches `Topic::HeightDates` and `Topic::Display`.
#[derive(Default)]
pub struct HeightDates {
    dates: HashMap<u64, api::HeightDate>,
    pending: HashSet<u64>,
    requested: HashSet<u64>,
    generation: Option<u64>,
    scheduled: bool,
}

thread_local! {
    static HEIGHT_DATES: RefCell<HeightDates> = RefCell::new(HeightDates::default());
}

/// The text for a height under the current setting. `known` is the block's time when the
/// caller has it. Loads missing dates in the background.
pub fn height_text(height: u64, known: Option<u64>, model: &Rc<AppModel>) -> String {
    if !shows_dates() {
        return format::integer(height);
    }
    if let Some(known) = known.filter(|t| *t > 0) {
        return format::date(known);
    }
    let generation = model.generation();
    let (text, schedule) = HEIGHT_DATES.with(|cell| {
        let mut dates = cell.borrow_mut();
        if dates.generation != Some(generation) {
            dates.generation = Some(generation);
            dates.pending.clear();
            dates.requested.clear();
            dates.dates.clear();
        }
        if let Some(date) = dates.dates.get(&height) {
            return match date.time.filter(|t| *t > 0) {
                Some(time) => (
                    format!("{}{}", if date.estimated { "est. " } else { "" }, format::date(time as u64)),
                    false,
                ),
                None => (format!("{} (date not available)", format::integer(height)), false),
            };
        }
        let mut schedule = false;
        if !dates.requested.contains(&height) && dates.pending.len() < 256 {
            dates.pending.insert(height);
            schedule = !dates.scheduled;
            dates.scheduled = true;
        }
        (format!("{} (loading date)", format::integer(height)), schedule)
    });
    if schedule {
        schedule_dates(model.clone());
    }
    text
}

/// A confirmation count, or the date of the block that holds the transaction when dates
/// are on and the block time is known. The terminal never subtracts from the tip.
pub fn confirmations(count: i64, block_time: Option<u64>) -> String {
    match block_time.filter(|t| *t > 0) {
        Some(time) if shows_dates() => format!("included {}", format::date(time)),
        _ => format::signed(count),
    }
}

/// A Unix timestamp as a date when dates are on, otherwise as the number.
pub fn timestamp(unix: u64) -> String {
    if shows_dates() { format::date(unix) } else { unix.to_string() }
}

fn schedule_dates(model: Rc<AppModel>) {
    glib::spawn_future_local(async move {
        // Collect the heights that one redraw asks for, then ask for them together.
        glib::timeout_future(std::time::Duration::from_millis(150)).await;
        let batch: Vec<u64> = HEIGHT_DATES.with(|cell| {
            let mut dates = cell.borrow_mut();
            dates.scheduled = false;
            let batch: Vec<u64> = dates.pending.iter().copied().take(128).collect();
            for height in &batch {
                dates.pending.remove(height);
                dates.requested.insert(*height);
            }
            batch
        });
        let Some(explorer) = model.explorer() else { return };
        let generation = model.generation();
        let found = bridge::run(async move { explorer.height_dates(batch).await }).await;
        if generation != model.generation() {
            return;
        }
        let more = HEIGHT_DATES.with(|cell| {
            let mut dates = cell.borrow_mut();
            for date in found {
                dates.dates.insert(date.height, date);
            }
            let more = !dates.pending.is_empty() && !dates.scheduled;
            if more {
                dates.scheduled = true;
            }
            more
        });
        model.notify(Topic::HeightDates);
        if more {
            schedule_dates(model);
        }
    });
}
