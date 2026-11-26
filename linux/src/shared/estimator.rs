//! Owner: the Estimate page (`pages/estimate`). The future block estimate from Carbide's
//! calibrated rate, with the calibration shown (`BlockEstimator.swift`). Used by the
//! Estimate page and the Charts page.
//!
//! When a future block will arrive, or what height a future moment will have. Each result
//! shows the calibration it came from. Search can open it with a query, which runs at once.

use std::cell::Cell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api};
use crate::format;
use crate::page::{self, Page};
use crate::theme::draw::{self, Anchor};
use crate::theme::{tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::{display, lamp, loader};

/// The estimator. `query` is a frozen estimator target (a height, or `unix:` and
/// seconds), or None for an empty field.
pub fn block_estimator(page: &Rc<Page>, query: Option<&str>) -> gtk::Box {
    let card = page.card("Future block estimate", None);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::new();
    entry.add_css_class("display-field");
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("Future height or date. Examples: 5000000, in 2 hours, 2028-11-08, unix:1857254400"));
    row.append(&entry);
    let running = Rc::new(Cell::new(false));
    let status = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let button = lamp::button("Estimate", || {});
    row.append(&button);
    card.append(&row);
    card.append(&caption("A bare number is a block height. Use unix: or time: for Unix seconds. The app fixes a relative date when you submit it."));
    card.append(&status);
    status.set_visible(false);

    let run: Rc<dyn Fn()> = {
        let (page, entry, status, running, button) = (Rc::downgrade(page), entry.downgrade(), status.clone(), running.clone(), button.downgrade());
        Rc::new(move || {
            let (Some(page), Some(entry)) = (page.upgrade(), entry.upgrade()) else { return };
            let query = entry.text().trim().to_string();
            if query.is_empty() || running.get() {
                return;
            }
            running.set(true);
            if let Some(button) = button.upgrade() {
                button.set_sensitive(false);
            }
            page::load::clear(&status);
            let progress = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            progress.append(&loader::small());
            progress.append(&page::text("Calibrating from the node's blocks…", &["callout", "secondary"]));
            status.append(&progress);
            status.set_visible(true);
            let explorer = page.model.require_explorer();
            let (page_weak, status, running, button, entry) = (Rc::downgrade(&page), status.clone(), running.clone(), button.clone(), entry.downgrade());
            glib::spawn_future_local(async move {
                let result = match explorer {
                    Ok(explorer) => bridge::run(async move { explorer.forecast(query).await }).await.map_err(|e| bridge::describe(&e)),
                    Err(message) => Err(message),
                };
                running.set(false);
                if let (Some(button), Some(entry)) = (button.upgrade(), entry.upgrade()) {
                    button.set_sensitive(!entry.text().trim().is_empty());
                }
                let Some(page) = page_weak.upgrade() else { return };
                page::load::clear(&status);
                match result {
                    Ok(forecast) => status.append(&forecast_result(&page, &forecast)),
                    Err(message) => status.append(&page::note(&message, true)),
                }
            });
        })
    };
    {
        let click = run.clone();
        button.connect_clicked(move |_| click());
        let submit = run.clone();
        entry.connect_activate(move |_| submit());
        let (running, button) = (running.clone(), button.downgrade());
        entry.connect_changed(move |entry| {
            if let Some(button) = button.upgrade() {
                button.set_sensitive(!running.get() && !entry.text().trim().is_empty());
            }
        });
    }
    button.set_sensitive(false);
    if let Some(query) = query.filter(|q| !q.is_empty()) {
        entry.set_text(query);
        run();
    }
    card.root
}

fn caption(text: &str) -> gtk::Label {
    page::paragraph(text, &["caption-text", "secondary"])
}

/// One estimate: the answer, a timeline of the calibration and the target, and the details.
fn forecast_result(page: &Rc<Page>, forecast: &api::Forecast) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let title = page::paragraph("", &["value"]);
    title.set_markup(&format!("<span font_size=\"{}\">{}</span>", 15 * 1024, glib::markup_escape_text(&headline(forecast, format::now() as i64))));
    column.append(&title);
    let timeline = timeline(forecast);
    timeline.set_margin_start(14);
    timeline.set_margin_end(14);
    timeline.set_margin_top(10);
    timeline.set_margin_bottom(10);
    column.append(&display::panel(&timeline, 0));
    let date = |unix: i64| format::date(unix.max(0) as u64);
    column.append(&page::field_row(page, "Estimated height", &format::integer(forecast.height), None, false));
    column.append(&page::field_row(page, "Estimated time", &format!("{} ({})", date(forecast.time), forecast.time), None, false));
    column.append(&page::field_row(
        page,
        "Average",
        &format!("{:.6} seconds per block", forecast.seconds_per_block),
        None,
        false,
    ));
    column.append(&page::field_row(
        page,
        "Calibration",
        &format!(
            "#{} ({}) to #{} ({})",
            format::integer(forecast.start.height),
            format::date(forecast.start.time),
            format::integer(forecast.tip.height),
            format::date(forecast.tip.time)
        ),
        None,
        false,
    ));
    column.append(&page::field_row(
        page,
        "Observed tip",
        &format!("#{} at {}", format::integer(forecast.tip.height), date(forecast.observed_at)),
        None,
        false,
    ));
    column.append(&page::field_row(page, "Chain", &forecast.system, None, false));
    column.append(&page::field_row(page, "Source", &forecast.source_label, None, false));
    column.append(&caption("This estimate assumes ordinary block production continues. Outages, changes in participation, and protocol updates can change the result."));
    column.append(&caption("The rate comes from two blocks: the calibration start and the verified tip. An average of two endpoints does not detect interruptions between them."));
    column
}

fn headline(forecast: &api::Forecast, now: i64) -> String {
    let relative = relative(forecast.time, now);
    let date = format::date(forecast.time.max(0) as u64);
    if forecast.target_is_height {
        return format!("Block {} is estimated at {date} ({relative}).", format::integer(forecast.height));
    }
    format!("At {date} ({relative}) the estimated height is {}.", format::integer(forecast.height))
}

/// A date relative to now, as `.relative(presentation: .named)` in English: "in 2 hours",
/// "3 days ago", "tomorrow", "next year", "now".
pub fn relative(unix: i64, now: i64) -> String {
    let delta = unix - now;
    let distance = delta.unsigned_abs() as f64;
    if distance < 1.0 {
        return "now".into();
    }
    const UNITS: [(&str, f64); 7] = [
        ("year", 365.2425 * 86_400.0),
        ("month", 30.436875 * 86_400.0),
        ("week", 7.0 * 86_400.0),
        ("day", 86_400.0),
        ("hour", 3_600.0),
        ("minute", 60.0),
        ("second", 1.0),
    ];
    let (unit, size) = UNITS.iter().find(|(_, size)| distance >= *size).copied().unwrap_or(("second", 1.0));
    let count = (distance / size).floor() as u64;
    if count == 1 {
        let named = match (unit, delta > 0) {
            ("day", true) => Some("tomorrow".to_string()),
            ("day", false) => Some("yesterday".to_string()),
            ("week" | "month" | "year", true) => Some(format!("next {unit}")),
            ("week" | "month" | "year", false) => Some(format!("last {unit}")),
            _ => None,
        };
        if let Some(named) = named {
            return named;
        }
    }
    let unit = if count == 1 { unit.to_string() } else { format!("{unit}s") };
    if delta > 0 { format!("in {count} {unit}") } else { format!("{count} {unit} ago") }
}

/// The calibration span and the target on one time axis: the measured stretch as a solid
/// line, the projection to the target as a dashed line. The tip is the present.
fn timeline(forecast: &api::Forecast) -> Canvas {
    let (start, tip, target) = (forecast.start.time as f64, forecast.tip.time as f64, forecast.time as f64);
    let labels = [
        format!("Calibration start #{}", format::integer(forecast.start.height)),
        format!("Tip #{}", format::integer(forecast.tip.height)),
        format!("Estimate #{}", format::integer(forecast.height)),
    ];
    let canvas = Canvas::new(0, 64, move |canvas, snapshot, w, _h| {
        let low = start.min(target);
        let high = tip.max(target).max(low + 1.0);
        let x = |time: f64| 8.0 + ((time - low) / (high - low)) as f32 * (w - 16.0);
        let y = 22.0;
        draw::rule(snapshot, x(start), y, x(tip), y, tone::RECORD, 2.0, &[]);
        draw::rule(snapshot, x(tip), y, x(target), y, tone::VALUE.opacity(0.7), 1.0, &[4.0, 3.0]);
        let marks: [(f64, &str, Color, Anchor); 3] = [
            (start, &labels[0], tone::RECORD, Anchor::TopLeading),
            (tip, &labels[1], tone::PRESENT, Anchor::Top),
            (target, &labels[2], tone::VALUE, Anchor::TopTrailing),
        ];
        for (time, label, color, anchor) in marks {
            let px = x(time);
            let dot = draw::ellipse_path(px - 3.5, y - 3.5, 7.0, 7.0);
            // The tip is the present: it alone has light.
            draw::lit(snapshot, &dot, color, if anchor == Anchor::Top { 0.6 } else { 0.0 });
            // The tip's label goes above the line, so that it does not meet the target's label.
            if anchor == Anchor::Top {
                draw::name(snapshot, canvas, label, px, y - 8.0, Anchor::Bottom, tone::PRESENT);
            } else {
                draw::name(snapshot, canvas, label, px, y + 10.0, anchor, tone::LABEL);
            }
        }
    });
    canvas.set_hexpand(true);
    canvas.update_property(&[gtk::accessible::Property::Label(&format!(
        "Calibration from block {} to the tip, then the estimate for block {}",
        forecast.start.height, forecast.height
    ))]);
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_dates() {
        assert_eq!(relative(100, 100), "now");
        assert_eq!(relative(100 + 7_200, 100), "in 2 hours");
        assert_eq!(relative(100 + 86_400 + 5, 100), "tomorrow");
        assert_eq!(relative(100 - 3 * 86_400, 100), "3 days ago");
        assert_eq!(relative(100 + 2 * 365 * 86_400 + 86_400, 100), "in 2 years");
        assert_eq!(relative(100 + 30, 100), "in 30 seconds");
        assert_eq!(relative(100 + 400 * 86_400, 100), "next year");
    }

    #[test]
    fn headlines() {
        let block = |height, time| api::ChartBlock { height, hash: String::new(), time };
        let forecast = api::Forecast {
            target_is_height: true,
            height: 5_000_000,
            time: 1_000 + 7_200,
            seconds_per_block: 60.0,
            start: block(1, 0),
            tip: block(2, 1_000),
            observed_at: 1_000,
            system: "VRSC".into(),
            source_label: "local".into(),
        };
        let text = headline(&forecast, 1_000);
        assert!(text.starts_with("Block 5,000,000 is estimated at "));
        assert!(text.ends_with(" (in 2 hours)."));
    }
}
