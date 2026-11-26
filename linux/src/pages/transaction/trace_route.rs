//! A traced transfer as a receipt, an axis of blocks and a batch strip (`TraceRoute` and
//! `TraceAxis` in `TraceRoute.swift`).
//!
//! Most transfers end in the same way: a conversion with a verified result. Thus the page
//! shows what differs between them. The receipt reads from the amount sent to the amount
//! received, with the fee and the rate. The axis has one tick for each block, so that the
//! wait for an export is a length, and stages in one block are one mark. The strip shows
//! the transfer as one member of its export batch.

use gtk::prelude::*;

use crate::bridge::api::{self, Confidence, TraceStage};
use crate::format;
use crate::page;
use crate::theme::{self, draw, font, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;

use super::extras::arrow;
use super::words::{self, Namer};

/// The stages of one block. A stage with no known block is a mark of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub stages: Vec<TraceStage>,
    pub height: Option<u64>,
    pub time: Option<u64>,
}

/// Joins the stages that are in one block, in the order of the trail.
pub fn marks(moments: &[api::TraceMoment]) -> Vec<Mark> {
    let mut marks: Vec<Mark> = Vec::new();
    for moment in moments {
        if let Some(height) = moment.height
            && let Some(last) = marks.last_mut()
            && last.height == Some(height)
        {
            if !last.stages.contains(&moment.stage) {
                last.stages.push(moment.stage);
            }
            continue;
        }
        marks.push(Mark { stages: vec![moment.stage], height: moment.height, time: moment.time });
    }
    marks
}

pub fn wait(blocks: u64, seconds: Option<u64>) -> String {
    let mut text = format!("{blocks} {}", if blocks == 1 { "block" } else { "blocks" });
    if let Some(seconds) = seconds {
        text += &format!(" · {}", words::span(seconds as f64));
    }
    text
}

fn stage_name(trace: &api::TransferTrace, stage: TraceStage) -> String {
    match stage {
        TraceStage::Sent => "Sent".into(),
        TraceStage::Exported => "Exported".into(),
        TraceStage::Imported => "Imported".into(),
        TraceStage::Result => trace.legs.last().and_then(|l| l.result.clone()).unwrap_or_else(|| "Result".into()),
    }
}

fn names(trace: &api::TransferTrace, stages: &[TraceStage]) -> String {
    words::list(&stages.iter().map(|s| stage_name(trace, *s)).collect::<Vec<_>>())
}

/// The route of a trace. `live` is true while the search continues.
pub fn route(trace: &api::TransferTrace, live: bool, namer: &Namer) -> gtk::Overlay {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let number = |index: usize| (trace.legs.len() > 1).then_some(index + 1);
    for (index, leg) in trace.legs.iter().enumerate() {
        column.append(&receipt(trace, leg, number(index), live, namer));
    }
    let found = marks(&trace.moments);
    if !found.is_empty() {
        column.append(&display::rule());
        let reached = trace.confidence == Confidence::Verified;
        let owned = trace.clone();
        let spoken = found
            .iter()
            .map(|mark| {
                let names = names(trace, &mark.stages);
                match mark.height {
                    Some(height) => format!("{names} in block {}", format::integer(height)),
                    None => format!("{names}, block not known"),
                }
            })
            .collect::<Vec<_>>()
            .join(". ");
        let axis = Canvas::new(0, 74, move |canvas, snapshot, w, _| draw_axis(canvas, snapshot, w, &found, reached, &owned));
        axis.set_hexpand(true);
        axis.update_property(&[gtk::accessible::Property::Label(&spoken)]);
        column.append(&axis);
    }
    if let Some(batch) = trace.batch {
        column.append(&display::rule());
        column.append(&batch_view(trace, batch));
    }
    display::panel(&column, 16)
}

fn money(amount: &api::CurrencyAmount, namer: &Namer) -> String {
    format!("{} {}", format::amount(amount.amount), namer.currency_name(&amount.currency))
}

fn amounts(list: &[api::CurrencyAmount], namer: &Namer) -> String {
    if list.is_empty() { "not known".into() } else { words::list(&list.iter().map(|a| money(a, namer)).collect::<Vec<_>>()) }
}

/// The fee of the conversion as a part of the amount sent.
pub fn fee_share(leg: &api::TraceLeg) -> Option<String> {
    let fee = leg.conversion_fee.as_ref()?;
    let sent = leg.principal.iter().find(|a| a.currency == fee.currency).filter(|a| a.amount > 0.0)?;
    let percent = fee.amount / sent.amount * 100.0;
    Some(if percent < 0.1 { format!("fee {percent:.3}%") } else { format!("fee {percent:.2}%") })
}

fn fee_line(leg: &api::TraceLeg, namer: &Namer) -> String {
    let mut parts = Vec::new();
    if let Some(fee) = &leg.conversion_fee {
        parts.push(format!("Conversion fee: {}.", money(fee, namer)));
    }
    if let Some(budget) = &leg.transfer_fee_budget {
        parts.push(format!("Transfer fee budget, apart from the amount: {}.", money(budget, namer)));
    }
    parts.join(" ")
}

fn receipt(trace: &api::TransferTrace, leg: &api::TraceLeg, number: Option<usize>, live: bool, namer: &Namer) -> gtk::Box {
    let rate = leg.rates.iter().find(|r| r.purpose == "Principal").or(leg.rates.first());
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if let Some(number) = number {
        column.append(&display::name_text(&format!("Leg {number}")));
    }
    let verified = trace.confidence == Confidence::Verified;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&receipt_box("Sent", &if leg.principal_known { amounts(&leg.principal, namer) } else { "not known".into() }, Strength::Plain, verified));
    if let Some(rate) = rate {
        row.append(&receipt_arrow(fee_share(leg).as_deref()));
        let pool = leg.pool.as_ref().map(|p| namer.currency_name(p)).unwrap_or_else(|| "the pool".into());
        row.append(&receipt_box(&format!("Converted in {pool}"), &money(&rate.net_input, namer), Strength::Plain, verified));
    }
    row.append(&receipt_arrow(None));
    if let Some(result) = &leg.result {
        row.append(&receipt_box(result, &amounts(&leg.result_amounts, namer), Strength::Strong, verified));
    } else if let Some(burned) = &leg.burned {
        row.append(&receipt_box("Burned", &money(burned, namer), Strength::Strong, verified));
    } else {
        let value = if live { "The search continues." } else { "No result output is matched." };
        row.append(&receipt_box(&leg.stage, value, Strength::Open, verified));
    }
    column.append(&row);
    if let Some(rate) = rate
        && let Some(sats) = rate.rate_sats
    {
        let line = page::paragraph(
            &format!(
                "Rate of the batch: {}{} {} for 1 {}.",
                if rate.exact { "" } else { "approximately " },
                format::coins(sats),
                namer.currency_name(&rate.output.currency),
                namer.currency_name(&rate.net_input.currency)
            ),
            &["caption-text", "value"],
        );
        line.set_selectable(true);
        column.append(&line);
    }
    let fees = fee_line(leg, namer);
    if !fees.is_empty() {
        let line = page::paragraph(&fees, &["caption-text", "label"]);
        line.set_selectable(true);
        column.append(&line);
    }
    column
}

#[derive(Clone, Copy, PartialEq)]
enum Strength {
    Plain,
    /// The received box has the colour of the result: green when the trace is verified,
    /// else the present.
    Strong,
    /// A stage with no result: a dashed edge.
    Open,
}

fn receipt_box(title: &str, value: &str, strength: Strength, verified: bool) -> gtk::Box {
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 3);
    frame.add_css_class("tx-box");
    frame.set_hexpand(true);
    let tone_class = match strength {
        Strength::Strong => {
            frame.add_css_class(if verified { "strong-good" } else { "strong-present" });
            if verified { "good" } else { "present" }
        }
        Strength::Open => {
            frame.add_css_class("open");
            "label"
        }
        Strength::Plain => "value",
    };
    let name = display::name_text(title);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.set_max_width_chars(1);
    frame.append(&name);
    let figure = theme::label(font::FIGURE_SMALL, value, tone_class);
    figure.set_wrap(true);
    figure.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    figure.set_lines(2);
    figure.set_ellipsize(gtk::pango::EllipsizeMode::End);
    // A small natural width: the boxes share the width of the row in equal parts.
    figure.set_max_width_chars(1);
    figure.set_width_chars(1);
    figure.set_selectable(true);
    frame.append(&figure);
    frame
}

fn receipt_arrow(label: Option<&str>) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 1);
    column.set_valign(gtk::Align::Center);
    let name = display::name_text(label.unwrap_or(" "));
    name.set_xalign(0.5);
    column.append(&name);
    let mark = arrow(false, tone::AXIS);
    mark.set_halign(gtk::Align::Center);
    column.append(&mark);
    column
}

fn draw_axis(canvas: &Canvas, snapshot: &gtk::Snapshot, width: f32, marks: &[Mark], reached: bool, trace: &api::TransferTrace) {
    let placed: Vec<&Mark> = marks.iter().filter(|m| m.height.is_some()).collect();
    let open: Vec<&Mark> = marks.iter().filter(|m| m.height.is_none()).collect();
    let y = 36.0f32;
    let left = 46.0f32;
    // Room at the right for the label of the last mark, and for a dashed end.
    let right = width - if reached { 230.0 } else { 150.0 };
    let (Some(first), Some(last)) = (placed.first().and_then(|m| m.height), placed.last().and_then(|m| m.height)) else {
        let stages: Vec<TraceStage> = marks.iter().flat_map(|m| m.stages.iter().copied()).collect();
        let text = format!("{}: not in a block that this node has", names(trace, &stages));
        draw::text(snapshot, canvas, font::CAPTION, &text, left, y, draw::Anchor::Leading, tone::LABEL);
        return;
    };
    let span = (last.saturating_sub(first) as f64).max(1.0);
    let x = |height: u64| -> f32 {
        if last == first { left } else { left + (height.saturating_sub(first) as f64 / span) as f32 * (right - left) }
    };
    // The line and one tick for each block.
    if last > first {
        // The passed part of the record.
        draw::rule(snapshot, left, y, right, y, tone::RECORD, 1.0, &[]);
        if last - first <= 60 {
            for height in first..=last {
                draw::rule(snapshot, x(height), y - 2.5, x(height), y + 2.5, tone::AXIS, 1.0, &[]);
            }
        }
    }
    if !reached || !open.is_empty() {
        draw::rule(snapshot, x(last), y, width - 8.0, y, tone::AXIS, 1.0, &[5.0, 5.0]);
    }
    for (index, mark) in placed.iter().enumerate() {
        let Some(height) = mark.height else { continue };
        let at = x(height);
        let last_mark = index == placed.len() - 1;
        let side = if last_mark { 11.0 } else { 9.0 };
        let done = last_mark && reached && mark.stages.contains(&TraceStage::Result);
        let shape = draw::rect_path(at - side / 2.0, y - side / 2.0, side, side);
        if last_mark {
            // The result is the live mark: it has light.
            draw::lit(snapshot, &shape, if done { tone::GOOD } else { tone::PRESENT }, 0.7);
        } else {
            draw::fill(snapshot, &shape, tone::RECORD);
        }
        let mut title = names(trace, &mark.stages);
        if mark.stages.len() > 1 {
            title = words::capital(&title.to_lowercase());
        }
        let trailing = last_mark && placed.len() > 1;
        let (tx, anchor) = if trailing { (at + 14.0, draw::Anchor::Leading) } else { (at, draw::Anchor::Center) };
        let color = if last_mark { if done { tone::GOOD } else { tone::PRESENT } } else { tone::VALUE };
        draw::name(snapshot, canvas, &title, tx, y - 20.0, anchor, color);
        let block = format!("block {}{}", format::integer(height), if mark.stages.len() > 1 { ", one block" } else { "" });
        draw::figure(snapshot, canvas, &block, tx, y + 20.0, anchor, tone::LABEL);
        // The wait from the mark before.
        if index > 0
            && let Some(before) = placed[index - 1].height
            && height > before
        {
            let seconds = mark.time.and_then(|end| placed[index - 1].time.map(|start| end.saturating_sub(start)));
            draw::figure(snapshot, canvas, &wait(height - before, seconds), (x(before) + at) / 2.0, y - 14.0, draw::Anchor::Center, tone::VALUE);
        }
    }
    if let Some(pending) = open.first() {
        let text = format!("{}: on a system that this node does not have", names(trace, &pending.stages));
        draw::text(snapshot, canvas, font::CAPTION, &text, width - 8.0, y - 14.0, draw::Anchor::Trailing, tone::LABEL);
    }
}

fn batch_view(trace: &api::TransferTrace, batch: api::TraceBatch) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 5);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let name = display::name_text("Export batch");
    name.set_valign(gtk::Align::Center);
    row.append(&name);
    let count = batch.size.min(40) as usize;
    let place = batch.place as usize;
    let strip_width = (count as f32 * 14.0 - 3.0).max(0.0) + 8.0;
    let strip = Canvas::new(strip_width.ceil() as i32, 19, move |_, snapshot, _, _| {
        for index in 0..count {
            let x = 4.0 + index as f32 * 14.0;
            let shape = draw::rect_path(x, 4.0, 11.0, 11.0);
            if index == place {
                draw::push_shadow(snapshot, tone::PRESENT, 0.7, 3.0);
                draw::fill(snapshot, &shape, tone::PRESENT);
                snapshot.pop();
            } else {
                draw::fill(snapshot, &shape, tone::AXIS);
            }
        }
    });
    strip.set_valign(gtk::Align::Center);
    row.append(&strip);
    let text = if batch.size == 1 {
        "This transfer was alone in its export.".to_string()
    } else {
        format!("Transfer {} of {} in the export.", batch.place + 1, batch.size)
    };
    let label = page::text(&text, &["caption-text", "value"]);
    label.set_valign(gtk::Align::Center);
    row.append(&label);
    column.append(&row);
    if trace.confidence == Confidence::Verified && trace.legs.iter().any(|l| !l.rates.is_empty()) {
        column.append(&page::paragraph(&candidate_line(trace.candidates), &["caption-text", "label"]));
    }
    column
}

fn candidate_line(candidates: u32) -> String {
    let same = "All conversions of the batch got the same rate.";
    match candidates {
        0 => format!("{same} No other output of the import went to the same address."),
        1 => format!("{same} 1 other output of the import went to the same address. The replay finds this one."),
        n => format!("{same} {n} other outputs of the import went to the same address. The replay finds this one."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moment(stage: TraceStage, height: Option<u64>, time: Option<u64>) -> api::TraceMoment {
        api::TraceMoment { stage, system_id: "i".into(), txid: "t".into(), output: 0, height, time }
    }

    #[test]
    fn stages_in_one_block_are_one_mark() {
        let found = marks(&[
            moment(TraceStage::Sent, Some(10), Some(100)),
            moment(TraceStage::Exported, Some(10), Some(100)),
            moment(TraceStage::Imported, Some(14), Some(340)),
            moment(TraceStage::Result, Some(14), Some(340)),
            moment(TraceStage::Result, None, None),
        ]);
        assert_eq!(found.len(), 3);
        assert_eq!(found[0].stages, vec![TraceStage::Sent, TraceStage::Exported]);
        assert_eq!(found[1].stages, vec![TraceStage::Imported, TraceStage::Result]);
        assert_eq!(found[2].height, None);
        assert_eq!(wait(4, Some(240)), "4 blocks · 4 min");
        assert_eq!(wait(2, Some(90)), "2 blocks · 90 s");
        assert_eq!(wait(1, None), "1 block");
        assert_eq!(candidate_line(1), "All conversions of the batch got the same rate. 1 other output of the import went to the same address. The replay finds this one.");
    }

    #[test]
    fn the_fee_share() {
        let amount = |currency: &str, amount: f64| api::CurrencyAmount { currency: currency.into(), amount };
        let leg = api::TraceLeg {
            source_system: String::new(),
            source_txid: String::new(),
            source_output: 0,
            stage: String::new(),
            confidence: Confidence::Verified,
            principal: vec![amount("a", 100.0)],
            principal_known: true,
            pool: None,
            conversion_fee: Some(amount("a", 0.025)),
            transfer_fee_budget: None,
            forwarding_budget: None,
            forwarding_conversion_fee: None,
            result: None,
            result_amounts: vec![],
            result_system: None,
            result_txid: None,
            result_output: None,
            next_fee_budget: None,
            burned: None,
            batch_burned: None,
            proceeds: vec![],
            proceeds_output: None,
            import_height: None,
            execution_system: None,
            import_output: None,
            rates: vec![],
            currencies: vec![],
            pool_inspection: None,
        };
        assert_eq!(fee_share(&leg).as_deref(), Some("fee 0.025%"));
        let mut more = leg.clone();
        more.conversion_fee = Some(amount("a", 0.25));
        assert_eq!(fee_share(&more).as_deref(), Some("fee 0.25%"));
        more.conversion_fee = Some(amount("b", 0.25));
        assert_eq!(fee_share(&more), None);
    }
}
