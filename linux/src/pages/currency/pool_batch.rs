//! What the most recent conversion batch did to a pool, at the chosen moment, as a
//! statement: one row for each reserve that moved (`PoolBatch.swift`).
//!
//! A hollow bar to the left is a reserve that went out of the pool. A solid bar to the
//! right is a reserve that came in. The bars are the value of each amount in the unit
//! reserve, at the pool's own rate, so that amounts of different currencies share one
//! scale. Conversions are batched and all pass through the pool, so the chain records what
//! went in and out for each reserve, not which reserve was swapped for which.

use gtk::prelude::*;

use super::basket;
use crate::bridge::api::CurrencyPoint;
use crate::format;
use crate::page;
use crate::theme::{draw, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;

/// One reserve that moved.
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    pub id: String,
    pub amount_in: f64,
    pub amount_out: f64,
    /// The value of the larger amount in the unit. None when a price is missing.
    pub value: Option<f64>,
}

impl Flow {
    pub fn incoming(&self) -> bool {
        self.amount_in >= self.amount_out
    }
}

pub fn flows(point: &CurrencyPoint, unit: Option<&str>) -> Vec<Flow> {
    let unit_price = point.reserves.iter().find(|r| Some(r.currency_id.as_str()) == unit).and_then(|r| r.price_in_reserve);
    point
        .reserves
        .iter()
        .filter(|r| r.last_in > 0.0 || r.last_out > 0.0)
        .map(|r| {
            let worth = if Some(r.currency_id.as_str()) == unit { Some(1.0) } else { basket::worth(r.price_in_reserve, unit_price) };
            Flow {
                id: r.currency_id.clone(),
                amount_in: r.last_in,
                amount_out: r.last_out,
                value: worth.map(|w| r.last_in.max(r.last_out) * w),
            }
        })
        .collect()
}

/// "in 12.5 DAI.vETH · 0.68 VRSC"
pub fn line(flow: &Flow, unit: Option<&str>, name: &dyn Fn(&str) -> String) -> String {
    let incoming = flow.incoming();
    let mut text = format!(
        "{} {} {}",
        if incoming { "in" } else { "out" },
        format::amount(if incoming { flow.amount_in } else { flow.amount_out }),
        name(&flow.id)
    );
    if let (Some(value), Some(unit)) = (flow.value, unit)
        && flow.id != unit
    {
        text += &format!(" · {} {}", basket::text(value), name(unit));
    }
    text
}

/// The statement. `name` gives the name of a reserve.
pub fn pool_batch(point: &CurrencyPoint, unit: Option<&str>, name: &dyn Fn(&str) -> String) -> gtk::Box {
    let flows = flows(point, unit);
    let largest = flows.iter().filter_map(|f| f.value).fold(0.0_f64, f64::max).max(1e-12);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    if flows.is_empty() {
        column.append(&display::name_text("Last conversion batch at this date"));
        column.append(&page::text("The last batch before this block had no conversions.", &["callout", "secondary"]));
        return column;
    }
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let title = display::name_text("Last conversion batch at this date");
    title.set_hexpand(true);
    heading.append(&title);
    let legend = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    legend.append(&legend_key(false));
    legend.append(&display::name_text("Out"));
    legend.append(&legend_key(true));
    legend.append(&display::name_text("In"));
    heading.append(&legend);
    inner.append(&heading);
    inner.append(&display::rule());
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (position, flow) in flows.iter().enumerate() {
        if position > 0 {
            let grid = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            grid.add_css_class("rule");
            grid.set_opacity(0.6);
            rows.append(&grid);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        let label = page::text(&name(&flow.id), &["callout", "semibold", "value"]);
        label.set_size_request(130, -1);
        label.set_max_width_chars(1);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&label);
        let (value, incoming) = (flow.value.unwrap_or(0.0), flow.incoming());
        let bar = Canvas::new(0, 11, move |_, snapshot, w, h| {
            let middle = w / 2.0;
            draw::rule(snapshot, middle, 0.0, middle, h, tone::AXIS, 1.0, &[]);
            let length = ((value / largest) as f32 * (middle - 2.0)).max(2.0);
            let x = if incoming { middle } else { middle - length };
            if incoming {
                draw::fill_rect(snapshot, x, 1.0, length, h - 2.0, tone::RECORD);
            } else {
                draw::stroke(snapshot, &draw::rect_path(x + 0.5, 1.5, length - 1.0, h - 3.0), tone::VALUE, 1.0);
            }
        });
        bar.set_hexpand(true);
        bar.set_valign(gtk::Align::Center);
        row.append(&bar);
        let text = page::text(&line(flow, unit, name), &["callout", "tabular", "value"]);
        text.set_xalign(1.0);
        text.set_size_request(330, -1);
        text.set_max_width_chars(1);
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&text);
        rows.append(&row);
    }
    inner.append(&rows);
    inner.append(&display::rule());
    let unit_sentence = unit.map(|u| format!("The bars are the value of each amount in {}, at the pool's own rate. ", name(u))).unwrap_or_default();
    let caption = format!(
        "A solid bar to the right came into the pool. A hollow bar to the left went out. {unit_sentence}The pool does conversions in batches. The chain records the quantity in and out for each reserve. It does not record the pairs."
    );
    inner.append(&page::paragraph(&caption, &["caption-text", "label"]));
    column.append(&display::panel(&inner, 16));
    column
}

/// A small square of the legend: an outline for out, solid for in.
fn legend_key(incoming: bool) -> Canvas {
    let key = Canvas::new(9, 9, move |_, snapshot, w, h| {
        if incoming {
            draw::fill_rect(snapshot, 0.0, 0.0, w, h, tone::RECORD);
        } else {
            draw::stroke(snapshot, &draw::rect_path(0.5, 0.5, w - 1.0, h - 1.0), tone::VALUE, 1.0);
        }
    });
    key.set_valign(gtk::Align::Center);
    key
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::api::ReservePoint;

    fn reserve(id: &str, price: f64, last_in: f64, last_out: f64) -> ReservePoint {
        ReservePoint { currency_id: id.into(), reserves: Some(100.0), price_in_reserve: Some(price), last_in, last_out, last_fees: 0.0 }
    }

    #[test]
    fn flows_are_valued_in_the_unit() {
        let point = CurrencyPoint {
            height: 1,
            time: None,
            supply: Some(10.0),
            reserves: vec![reserve("iUnit", 18.0, 0.0, 9.0), reserve("iDai", 12.0, 6.0, 0.0), reserve("iStill", 3.0, 0.0, 0.0)],
        };
        let flows = flows(&point, Some("iUnit"));
        assert_eq!(flows.len(), 2);
        assert_eq!(flows[0].value, Some(9.0));
        assert!(!flows[0].incoming());
        // One DAI is worth 18 / 12 = 1.5 of the unit.
        assert_eq!(flows[1].value, Some(9.0));
        let name = |id: &str| id.trim_start_matches('i').to_string();
        assert_eq!(line(&flows[1], Some("iUnit"), &name), "in 6 Dai · 9 Unit");
        assert_eq!(line(&flows[0], Some("iUnit"), &name), "out 9 Unit");
        assert_eq!(flows_no_unit(&point), vec![None, None]);
    }

    fn flows_no_unit(point: &CurrencyPoint) -> Vec<Option<f64>> {
        flows(point, None).into_iter().map(|f| f.value).collect()
    }
}
