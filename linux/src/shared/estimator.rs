//! The future block estimate from Carbide's
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
    title.set_markup(&format!("<span font_size=\"{}\">{}</span>", crate::theme::pango_points(15.0), glib::markup_escape_text(&headline(forecast, format::now() as i64))));
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

/// A date relative to now, as `Date.formatted(.relative(presentation: .named))` prints it in
/// English, in the local time zone: "in 2 hours", "tomorrow", "next week", "3 days ago".
pub fn relative(unix: i64, now: i64) -> String {
    relative_in(unix, now, &glib::TimeZone::local())
}

/// The units of a relative date, from the largest (`sortedAllowedComponents` of Foundation's
/// relative formatter).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Unit {
    Year,
    Month,
    Week,
    Day,
    Hour,
    Minute,
    Second,
}

const UNITS: [Unit; 7] = [Unit::Year, Unit::Month, Unit::Week, Unit::Day, Unit::Hour, Unit::Minute, Unit::Second];

fn add(date: &glib::DateTime, unit: Unit, n: i64) -> glib::DateTime {
    let n32 = n.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    let added = match unit {
        Unit::Year => date.add_years(n32),
        Unit::Month => date.add_months(n32),
        Unit::Week => date.add_weeks(n32),
        Unit::Day => date.add_days(n32),
        Unit::Hour => date.add_hours(n32),
        Unit::Minute => date.add_minutes(n32),
        Unit::Second => date.add_seconds(n as f64),
    };
    added.unwrap_or_else(|_| date.clone())
}

/// The difference from `from` to `to` in `units`, largest first, as Foundation's
/// `Calendar.dateComponents(_:from:to:)`: as many of each unit as fit, then the next unit
/// on what is left. All values have the sign of the difference.
fn components(from: &glib::DateTime, to: &glib::DateTime, units: &[Unit]) -> Vec<(Unit, i64)> {
    let forward = to.to_unix() >= from.to_unix();
    let sign = if forward { 1 } else { -1 };
    let within = |date: &glib::DateTime| if forward { date.to_unix() <= to.to_unix() } else { date.to_unix() >= to.to_unix() };
    let mut cursor = from.clone();
    let mut found = Vec::new();
    for &unit in units {
        let count = match unit {
            Unit::Hour | Unit::Minute | Unit::Second => {
                let size = match unit {
                    Unit::Hour => 3_600,
                    Unit::Minute => 60,
                    _ => 1,
                };
                (to.to_unix() - cursor.to_unix()).abs() / size
            }
            _ => {
                let mut n = 0;
                while n < 100_000 && within(&add(&cursor, unit, sign * (n + 1))) {
                    n += 1;
                }
                n
            }
        };
        cursor = add(&cursor, unit, sign * count);
        found.push((unit, sign * count));
    }
    found
}

fn largest_nonzero(components: &[(Unit, i64)]) -> Option<(Unit, i64)> {
    components.iter().copied().find(|(_, value)| *value != 0)
}

/// `relative` in a given time zone. Port of `_largestNonZeroComponent` of swift-foundation's
/// `Date.RelativeFormatStyle`: hours, minutes and seconds are rounded to the nearest unit,
/// and a day or a longer unit is counted from the start or the end of that unit around
/// now. ICU then names one day, week, month or year before or after now.
fn relative_in(unix: i64, now: i64, zone: &glib::TimeZone) -> String {
    let (Ok(date), Ok(reference)) = (glib::DateTime::from_unix_utc(unix), glib::DateTime::from_unix_utc(now)) else {
        return String::new();
    };
    let (Ok(date), Ok(reference)) = (date.to_timezone(zone), reference.to_timezone(zone)) else { return String::new() };
    let Some((unit, value)) = largest_nonzero(&components(&reference, &date, &UNITS)) else {
        return "now".into();
    };
    let (unit, value) = if unit >= Unit::Hour {
        rounded(&reference, &date, unit)
    } else {
        aligned(&reference, &date, unit).unwrap_or((unit, value))
    };
    words(unit, value)
}

/// The largest of hours, minutes and seconds, rounded up when the next smaller unit is at
/// least half of it. A value that rounds to a whole larger unit is said in that unit.
fn rounded(reference: &glib::DateTime, date: &glib::DateTime, largest: Unit) -> (Unit, i64) {
    let units: Vec<Unit> = UNITS.iter().copied().filter(|u| *u >= largest).collect();
    let parts = components(reference, date, &units);
    let mut value = parts[0].1;
    if let Some(&(_, next)) = parts.get(1)
        && next.abs() * 2 >= 60
    {
        value += next.signum();
    }
    let shifted = add(date, largest, -value);
    match largest_nonzero(&components(&shifted, date, &UNITS)) {
        Some((unit, other)) if unit != largest => (unit, other),
        _ => (largest, value),
    }
}

/// The value of a day or longer unit when now is moved to the start of its unit (the date
/// is later) or to the last second of it (the date is earlier).
fn aligned(reference: &glib::DateTime, date: &glib::DateTime, unit: Unit) -> Option<(Unit, i64)> {
    let zone = reference.timezone();
    let (year, month, day) = reference.ymd();
    let start = match unit {
        Unit::Year => glib::DateTime::new(&zone, year, 1, 1, 0, 0, 0.0).ok()?,
        Unit::Month => glib::DateTime::new(&zone, year, month, 1, 0, 0, 0.0).ok()?,
        // Weeks begin on Sunday, as in the English calendar of the macOS app.
        Unit::Week => glib::DateTime::new(&zone, year, month, day, 0, 0, 0.0).ok()?.add_days(-(reference.day_of_week() % 7)).ok()?,
        _ => glib::DateTime::new(&zone, year, month, day, 0, 0, 0.0).ok()?,
    };
    let from = if reference.to_unix() < date.to_unix() { start } else { add(&add(&start, unit, 1), Unit::Second, -1) };
    largest_nonzero(&components(&from, date, &UNITS))
}

/// The words of ICU's relative format, long style, with names for one day, week, month and
/// year before or after now.
fn words(unit: Unit, value: i64) -> String {
    let named = match (unit, value) {
        (Unit::Second, 0) => Some("now"),
        (Unit::Day, 1) => Some("tomorrow"),
        (Unit::Day, -1) => Some("yesterday"),
        (Unit::Week, 1) => Some("next week"),
        (Unit::Week, -1) => Some("last week"),
        (Unit::Month, 1) => Some("next month"),
        (Unit::Month, -1) => Some("last month"),
        (Unit::Year, 1) => Some("next year"),
        (Unit::Year, -1) => Some("last year"),
        _ => None,
    };
    if let Some(named) = named {
        return named.into();
    }
    let name = match unit {
        Unit::Year => "year",
        Unit::Month => "month",
        Unit::Week => "week",
        Unit::Day => "day",
        Unit::Hour => "hour",
        Unit::Minute => "minute",
        Unit::Second => "second",
    };
    let count = value.abs();
    let name = if count == 1 { name.to_string() } else { format!("{name}s") };
    if value > 0 { format!("in {count} {name}") } else { format!("{count} {name} ago") }
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
                // The label stays inside the panel when the tip is near one end of the line.
                let width = draw::text_size(canvas, crate::theme::font::SMALL, label).0;
                draw::name(snapshot, canvas, label, centre_inside(px, width, w), y - 8.0, Anchor::Bottom, tone::PRESENT);
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

/// The centre of a label of `width` at `x`, moved so that the label is inside 0 to `total`.
fn centre_inside(x: f32, width: f32, total: f32) -> f32 {
    if width >= total {
        return total / 2.0;
    }
    x.clamp(width / 2.0, total - width / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tip_label_stays_inside() {
        // Near the right end the label moves left so that its end is at the edge.
        assert_eq!(centre_inside(1030.0, 100.0, 1040.0), 990.0);
        // Near the left end it moves right.
        assert_eq!(centre_inside(4.0, 100.0, 1040.0), 50.0);
        // In the middle it stays above its dot.
        assert_eq!(centre_inside(500.0, 100.0, 1040.0), 500.0);
        // A label wider than the panel is centred.
        assert_eq!(centre_inside(10.0, 200.0, 100.0), 50.0);
    }

    #[test]
    fn relative_dates() {
        // (now, date, text): what `Date.formatted(.relative(presentation: .named))` printed
        // on macOS in UTC for each pair.
        let printed: [(i64, i64, &str); 296] = [
            (1795665054, 1795665054, "now"),
            (1795665054, 1795665054, "now"),
            (1795665053, 1795665054, "in 1 second"),
            (1795665055, 1795665054, "1 second ago"),
            (1795664995, 1795665054, "in 59 seconds"),
            (1795665113, 1795665054, "59 seconds ago"),
            (1795664994, 1795665054, "in 1 minute"),
            (1795665114, 1795665054, "1 minute ago"),
            (1795664965, 1795665054, "in 1 minute"),
            (1795665143, 1795665054, "1 minute ago"),
            (1795664964, 1795665054, "in 2 minutes"),
            (1795665144, 1795665054, "2 minutes ago"),
            (1795661485, 1795665054, "in 59 minutes"),
            (1795668623, 1795665054, "59 minutes ago"),
            (1795661484, 1795665054, "in 1 hour"),
            (1795668624, 1795665054, "1 hour ago"),
            (1795659655, 1795665054, "in 1 hour"),
            (1795670453, 1795665054, "1 hour ago"),
            (1795659654, 1795665054, "in 2 hours"),
            (1795670454, 1795665054, "2 hours ago"),
            (1795623654, 1795665054, "in 12 hours"),
            (1795706454, 1795665054, "12 hours ago"),
            (1795621854, 1795665054, "in 12 hours"),
            (1795708254, 1795665054, "12 hours ago"),
            (1795582254, 1795665054, "in 23 hours"),
            (1795747854, 1795665054, "23 hours ago"),
            (1795580455, 1795665054, "in 23 hours"),
            (1795749653, 1795665054, "23 hours ago"),
            (1795580454, 1795665054, "tomorrow"),
            (1795749654, 1795665054, "yesterday"),
            (1795587294, 1795665054, "in 22 hours"),
            (1795742814, 1795665054, "22 hours ago"),
            (1795561374, 1795665054, "in 2 days"),
            (1795768734, 1795665054, "yesterday"),
            (1795526814, 1795665054, "in 2 days"),
            (1795803294, 1795665054, "yesterday"),
            (1795440414, 1795665054, "in 3 days"),
            (1795889694, 1795665054, "2 days ago"),
            (1795112094, 1795665054, "next week"),
            (1796218014, 1795665054, "6 days ago"),
            (1795094814, 1795665054, "next week"),
            (1796235294, 1795665054, "6 days ago"),
            (1794801054, 1795665054, "next week"),
            (1796529054, 1795665054, "2 weeks ago"),
            (1794541854, 1795665054, "in 2 weeks"),
            (1796788254, 1795665054, "2 weeks ago"),
            (1793850654, 1795665054, "in 3 weeks"),
            (1797479454, 1795665054, "3 weeks ago"),
            (1793591454, 1795665054, "in 3 weeks"),
            (1797738654, 1795665054, "last month"),
            (1793332254, 1795665054, "next month"),
            (1797997854, 1795665054, "last month"),
            (1792986654, 1795665054, "next month"),
            (1798343454, 1795665054, "last month"),
            (1791777054, 1795665054, "next month"),
            (1799553054, 1795665054, "2 months ago"),
            (1789185054, 1795665054, "in 2 months"),
            (1802145054, 1795665054, "3 months ago"),
            (1779853854, 1795665054, "in 6 months"),
            (1811476254, 1795665054, "6 months ago"),
            (1767153054, 1795665054, "in 11 months"),
            (1824177054, 1795665054, "11 months ago"),
            (1765857054, 1795665054, "in 11 months"),
            (1825473054, 1795665054, "last year"),
            (1764129054, 1795665054, "next year"),
            (1827201054, 1795665054, "last year"),
            (1761105054, 1795665054, "next year"),
            (1830225054, 1795665054, "last year"),
            (1748231454, 1795665054, "next year"),
            (1843098654, 1795665054, "2 years ago"),
            (1732593054, 1795665054, "in 2 years"),
            (1858737054, 1795665054, "2 years ago"),
            (1666065054, 1795665054, "in 4 years"),
            (1925265054, 1795665054, "5 years ago"),
            (1795600000, 1795600000, "now"),
            (1795600000, 1795600000, "now"),
            (1795599999, 1795600000, "in 1 second"),
            (1795600001, 1795600000, "1 second ago"),
            (1795599941, 1795600000, "in 59 seconds"),
            (1795600059, 1795600000, "59 seconds ago"),
            (1795599940, 1795600000, "in 1 minute"),
            (1795600060, 1795600000, "1 minute ago"),
            (1795599911, 1795600000, "in 1 minute"),
            (1795600089, 1795600000, "1 minute ago"),
            (1795599910, 1795600000, "in 2 minutes"),
            (1795600090, 1795600000, "2 minutes ago"),
            (1795596431, 1795600000, "in 59 minutes"),
            (1795603569, 1795600000, "59 minutes ago"),
            (1795596430, 1795600000, "in 1 hour"),
            (1795603570, 1795600000, "1 hour ago"),
            (1795594601, 1795600000, "in 1 hour"),
            (1795605399, 1795600000, "1 hour ago"),
            (1795594600, 1795600000, "in 2 hours"),
            (1795605400, 1795600000, "2 hours ago"),
            (1795558600, 1795600000, "in 12 hours"),
            (1795641400, 1795600000, "12 hours ago"),
            (1795556800, 1795600000, "in 12 hours"),
            (1795643200, 1795600000, "12 hours ago"),
            (1795517200, 1795600000, "in 23 hours"),
            (1795682800, 1795600000, "23 hours ago"),
            (1795515401, 1795600000, "in 23 hours"),
            (1795684599, 1795600000, "23 hours ago"),
            (1795515400, 1795600000, "tomorrow"),
            (1795684600, 1795600000, "yesterday"),
            (1795522240, 1795600000, "in 22 hours"),
            (1795677760, 1795600000, "22 hours ago"),
            (1795496320, 1795600000, "tomorrow"),
            (1795703680, 1795600000, "yesterday"),
            (1795461760, 1795600000, "in 2 days"),
            (1795738240, 1795600000, "2 days ago"),
            (1795375360, 1795600000, "in 3 days"),
            (1795824640, 1795600000, "3 days ago"),
            (1795047040, 1795600000, "in 6 days"),
            (1796152960, 1795600000, "6 days ago"),
            (1795029760, 1795600000, "next week"),
            (1796170240, 1795600000, "last week"),
            (1794736000, 1795600000, "next week"),
            (1796464000, 1795600000, "last week"),
            (1794476800, 1795600000, "in 2 weeks"),
            (1796723200, 1795600000, "2 weeks ago"),
            (1793785600, 1795600000, "in 3 weeks"),
            (1797414400, 1795600000, "3 weeks ago"),
            (1793526400, 1795600000, "in 3 weeks"),
            (1797673600, 1795600000, "3 weeks ago"),
            (1793267200, 1795600000, "next month"),
            (1797932800, 1795600000, "last month"),
            (1792921600, 1795600000, "next month"),
            (1798278400, 1795600000, "last month"),
            (1791712000, 1795600000, "next month"),
            (1799488000, 1795600000, "2 months ago"),
            (1789120000, 1795600000, "in 2 months"),
            (1802080000, 1795600000, "3 months ago"),
            (1779788800, 1795600000, "in 6 months"),
            (1811411200, 1795600000, "6 months ago"),
            (1767088000, 1795600000, "in 11 months"),
            (1824112000, 1795600000, "11 months ago"),
            (1765792000, 1795600000, "in 11 months"),
            (1825408000, 1795600000, "last year"),
            (1764064000, 1795600000, "next year"),
            (1827136000, 1795600000, "last year"),
            (1761040000, 1795600000, "next year"),
            (1830160000, 1795600000, "last year"),
            (1748166400, 1795600000, "next year"),
            (1843033600, 1795600000, "2 years ago"),
            (1732528000, 1795600000, "in 2 years"),
            (1858672000, 1795600000, "2 years ago"),
            (1666000000, 1795600000, "in 4 years"),
            (1925200000, 1795600000, "5 years ago"),
            (1795690000, 1795690000, "now"),
            (1795690000, 1795690000, "now"),
            (1795689999, 1795690000, "in 1 second"),
            (1795690001, 1795690000, "1 second ago"),
            (1795689941, 1795690000, "in 59 seconds"),
            (1795690059, 1795690000, "59 seconds ago"),
            (1795689940, 1795690000, "in 1 minute"),
            (1795690060, 1795690000, "1 minute ago"),
            (1795689911, 1795690000, "in 1 minute"),
            (1795690089, 1795690000, "1 minute ago"),
            (1795689910, 1795690000, "in 2 minutes"),
            (1795690090, 1795690000, "2 minutes ago"),
            (1795686431, 1795690000, "in 59 minutes"),
            (1795693569, 1795690000, "59 minutes ago"),
            (1795686430, 1795690000, "in 1 hour"),
            (1795693570, 1795690000, "1 hour ago"),
            (1795684601, 1795690000, "in 1 hour"),
            (1795695399, 1795690000, "1 hour ago"),
            (1795684600, 1795690000, "in 2 hours"),
            (1795695400, 1795690000, "2 hours ago"),
            (1795648600, 1795690000, "in 12 hours"),
            (1795731400, 1795690000, "12 hours ago"),
            (1795646800, 1795690000, "in 12 hours"),
            (1795733200, 1795690000, "12 hours ago"),
            (1795607200, 1795690000, "in 23 hours"),
            (1795772800, 1795690000, "23 hours ago"),
            (1795605401, 1795690000, "in 23 hours"),
            (1795774599, 1795690000, "23 hours ago"),
            (1795605400, 1795690000, "tomorrow"),
            (1795774600, 1795690000, "yesterday"),
            (1795612240, 1795690000, "in 22 hours"),
            (1795767760, 1795690000, "22 hours ago"),
            (1795586320, 1795690000, "tomorrow"),
            (1795793680, 1795690000, "yesterday"),
            (1795551760, 1795690000, "in 2 days"),
            (1795828240, 1795690000, "2 days ago"),
            (1795465360, 1795690000, "in 3 days"),
            (1795914640, 1795690000, "3 days ago"),
            (1795137040, 1795690000, "in 6 days"),
            (1796242960, 1795690000, "6 days ago"),
            (1795119760, 1795690000, "next week"),
            (1796260240, 1795690000, "last week"),
            (1794826000, 1795690000, "next week"),
            (1796554000, 1795690000, "2 weeks ago"),
            (1794566800, 1795690000, "in 2 weeks"),
            (1796813200, 1795690000, "2 weeks ago"),
            (1793875600, 1795690000, "in 3 weeks"),
            (1797504400, 1795690000, "3 weeks ago"),
            (1793616400, 1795690000, "in 3 weeks"),
            (1797763600, 1795690000, "last month"),
            (1793357200, 1795690000, "next month"),
            (1798022800, 1795690000, "last month"),
            (1793011600, 1795690000, "next month"),
            (1798368400, 1795690000, "last month"),
            (1791802000, 1795690000, "next month"),
            (1799578000, 1795690000, "2 months ago"),
            (1789210000, 1795690000, "in 2 months"),
            (1802170000, 1795690000, "3 months ago"),
            (1779878800, 1795690000, "in 6 months"),
            (1811501200, 1795690000, "6 months ago"),
            (1767178000, 1795690000, "in 11 months"),
            (1824202000, 1795690000, "11 months ago"),
            (1765882000, 1795690000, "in 11 months"),
            (1825498000, 1795690000, "last year"),
            (1764154000, 1795690000, "next year"),
            (1827226000, 1795690000, "last year"),
            (1761130000, 1795690000, "next year"),
            (1830250000, 1795690000, "last year"),
            (1748256400, 1795690000, "next year"),
            (1843123600, 1795690000, "2 years ago"),
            (1732618000, 1795690000, "in 2 years"),
            (1858762000, 1795690000, "2 years ago"),
            (1666090000, 1795690000, "in 4 years"),
            (1925290000, 1795690000, "5 years ago"),
            (1798700000, 1798700000, "now"),
            (1798700000, 1798700000, "now"),
            (1798699999, 1798700000, "in 1 second"),
            (1798700001, 1798700000, "1 second ago"),
            (1798699941, 1798700000, "in 59 seconds"),
            (1798700059, 1798700000, "59 seconds ago"),
            (1798699940, 1798700000, "in 1 minute"),
            (1798700060, 1798700000, "1 minute ago"),
            (1798699911, 1798700000, "in 1 minute"),
            (1798700089, 1798700000, "1 minute ago"),
            (1798699910, 1798700000, "in 2 minutes"),
            (1798700090, 1798700000, "2 minutes ago"),
            (1798696431, 1798700000, "in 59 minutes"),
            (1798703569, 1798700000, "59 minutes ago"),
            (1798696430, 1798700000, "in 1 hour"),
            (1798703570, 1798700000, "1 hour ago"),
            (1798694601, 1798700000, "in 1 hour"),
            (1798705399, 1798700000, "1 hour ago"),
            (1798694600, 1798700000, "in 2 hours"),
            (1798705400, 1798700000, "2 hours ago"),
            (1798658600, 1798700000, "in 12 hours"),
            (1798741400, 1798700000, "12 hours ago"),
            (1798656800, 1798700000, "in 12 hours"),
            (1798743200, 1798700000, "12 hours ago"),
            (1798617200, 1798700000, "in 23 hours"),
            (1798782800, 1798700000, "23 hours ago"),
            (1798615401, 1798700000, "in 23 hours"),
            (1798784599, 1798700000, "23 hours ago"),
            (1798615400, 1798700000, "tomorrow"),
            (1798784600, 1798700000, "yesterday"),
            (1798622240, 1798700000, "in 22 hours"),
            (1798777760, 1798700000, "22 hours ago"),
            (1798596320, 1798700000, "tomorrow"),
            (1798803680, 1798700000, "yesterday"),
            (1798561760, 1798700000, "in 2 days"),
            (1798838240, 1798700000, "yesterday"),
            (1798475360, 1798700000, "in 3 days"),
            (1798924640, 1798700000, "2 days ago"),
            (1798147040, 1798700000, "next week"),
            (1799252960, 1798700000, "6 days ago"),
            (1798129760, 1798700000, "next week"),
            (1799270240, 1798700000, "6 days ago"),
            (1797836000, 1798700000, "next week"),
            (1799564000, 1798700000, "2 weeks ago"),
            (1797576800, 1798700000, "in 2 weeks"),
            (1799823200, 1798700000, "2 weeks ago"),
            (1796885600, 1798700000, "in 3 weeks"),
            (1800514400, 1798700000, "3 weeks ago"),
            (1796626400, 1798700000, "in 3 weeks"),
            (1800773600, 1798700000, "4 weeks ago"),
            (1796367200, 1798700000, "next month"),
            (1801032800, 1798700000, "4 weeks ago"),
            (1796021600, 1798700000, "next month"),
            (1801378400, 1798700000, "last month"),
            (1794812000, 1798700000, "next month"),
            (1802588000, 1798700000, "last month"),
            (1792220000, 1798700000, "in 2 months"),
            (1805180000, 1798700000, "3 months ago"),
            (1782888800, 1798700000, "in 5 months"),
            (1814511200, 1798700000, "7 months ago"),
            (1770188000, 1798700000, "in 10 months"),
            (1827212000, 1798700000, "10 months ago"),
            (1768892000, 1798700000, "in 11 months"),
            (1828508000, 1798700000, "last year"),
            (1767164000, 1798700000, "next year"),
            (1830236000, 1798700000, "last year"),
            (1764140000, 1798700000, "next year"),
            (1833260000, 1798700000, "2 years ago"),
            (1751266400, 1798700000, "next year"),
            (1846133600, 1798700000, "2 years ago"),
            (1735628000, 1798700000, "in 2 years"),
            (1861772000, 1798700000, "2 years ago"),
            (1669100000, 1798700000, "in 4 years"),
            (1928300000, 1798700000, "5 years ago"),
        ];
        let utc = glib::TimeZone::utc();
        for (now, date, text) in printed {
            assert_eq!(relative_in(date, now, &utc), text, "now {now}, date {date}");
        }
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
        assert!(text.ends_with(" (in 2 hours)."), "{text}");
    }
}
