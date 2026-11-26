//! Fee pool controls, the selected checkpoint, the current balance and a table
//! (`FeePoolSection` in FeePoolChart.swift).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::bridge::api::{FeePoolCheckpoint, Route};
use crate::format;
use crate::page::{self, Page};
use crate::theme::tone;
use crate::widgets::{lamp, loader};

use super::format as chart_format;
use super::model::{Currency, Scale};
use super::parts::{self, caption, Holder, Mark};
use super::state::Charts;

/// The names of the two currencies of the fee pool.
fn labels(charts: &Charts) -> (String, String) {
    let state = charts.state.borrow();
    let coin = charts.model.upgrade().map(|m| m.coin()).unwrap_or_else(|| "VRSC".into());
    let current = state.fee.current.as_ref();
    (
        current.map(|c| c.currencies.native_label.clone()).unwrap_or(coin),
        current.map(|c| c.currencies.reserve_label.clone()).unwrap_or_else(|| "VRSC".into()),
    )
}

/// The four cards of the fee pool, added to the page.
pub fn section(page: &Rc<Page>, charts: &Rc<Charts>) {
    // MARK: Fee pool
    let card = page.card("Fee pool", None);
    let controls = Holder::new(0);
    card.append(&controls.root);
    let legend = Holder::new(12);
    card.append(&legend.root);
    let progress = Holder::new(12);
    card.append(&progress.root);
    page.append(&card.root);

    // MARK: Selected checkpoint
    let selected_card = page.card("Selected checkpoint", None);
    let selected = Holder::new(12);
    selected_card.append(&selected.root);
    page.append(&selected_card.root);

    // MARK: Current fee pool
    let current_card = page.card("Current fee pool", None);
    let current = Holder::new(12);
    current_card.append(&current.root);
    let weak = Rc::downgrade(charts);
    let again = lamp::button("Read again", move || {
        if let Some(charts) = weak.upgrade() {
            charts.state.borrow_mut().fee.reset(true);
            charts.changed();
        }
    });
    current_card.append(&again);
    page.append(&current_card.root);

    // MARK: Checkpoints as a table
    let table_card = page.card("Checkpoints as a table", None);
    let shown = Rc::new(RefCell::new(false));
    let table = Holder::new(0);
    let disclosure = lamp::Disclosure::new("Show the loaded checkpoints", false, {
        let (shown, weak) = (shown.clone(), Rc::downgrade(charts));
        move |open| {
            *shown.borrow_mut() = open;
            if let Some(charts) = weak.upgrade() {
                charts.changed();
            }
        }
    });
    disclosure.content.append(&table.root);
    table_card.append(&disclosure.root);
    page.append(&table_card.root);

    let page_weak = Rc::downgrade(page);
    charts.listen(move |charts| {
        let Some(page) = page_weak.upgrade() else { return };
        let (native, reserve) = labels(charts);
        let state = charts.state.borrow();
        let fee = &state.fee;
        let has_reserve = fee.has_reserve();

        // Controls: a chain without a reserve currency has one series, so there is nothing
        // to choose.
        let signature = format!("{has_reserve}|{native}|{reserve}|{:?}|{:?}|{}", fee.currency, fee.scale, fee.detail);
        controls.set(signature, |root| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
            if has_reserve {
                let selected = match fee.currency {
                    Currency::Both => 0,
                    Currency::Native => 1,
                    Currency::Reserve => 2,
                };
                let weak = Rc::downgrade(charts);
                let segments = lamp::Segments::new(&["Both", &native, &reserve], selected, move |index| {
                    if let Some(charts) = weak.upgrade() {
                        charts.state.borrow_mut().fee.currency = [Currency::Both, Currency::Native, Currency::Reserve][index];
                        charts.changed();
                    }
                });
                segments.root.set_tooltip_text(Some("Press v to go to the next currency. Each choice has its own vertical range."));
                row.append(&segments.root);
            }
            let selected = match fee.scale {
                Scale::Zero => 0,
                Scale::Fit => 1,
                Scale::Log => 2,
            };
            let weak = Rc::downgrade(charts);
            let scales = lamp::Segments::new(&["Linear from zero", "Linear, fitted", "Log-like"], selected, move |index| {
                if let Some(charts) = weak.upgrade() {
                    charts.state.borrow_mut().fee.scale = [Scale::Zero, Scale::Fit, Scale::Log][index];
                    charts.changed();
                }
            });
            scales.root.set_tooltip_text(Some("Press z to go to the next scale."));
            row.append(&scales.root);
            let weak = Rc::downgrade(charts);
            let detail = lamp::toggle("Detail", fee.detail, false, move || {
                if let Some(charts) = weak.upgrade() {
                    let mut state = charts.state.borrow_mut();
                    state.fee.detail = !state.fee.detail;
                    drop(state);
                    charts.changed();
                }
            });
            detail.set_tooltip_text(Some("Read up to 2,000 checkpoints, not 100. Press t to change this."));
            row.append(&detail);
            root.append(&row);
        });

        // Legend.
        let signature = format!("{has_reserve}|{native}|{reserve}|{:?}", fee.scale);
        legend.set(signature, |root| {
            let mut items = vec![(parts::Shape::Line, tone::RECORD, format!("{native} in the fee pool"))];
            if has_reserve {
                items.push((parts::Shape::Line, tone::GOOD, format!("{reserve} in the fee pool")));
            }
            root.append(&parts::legend(&items));
            match fee.scale {
                Scale::Log => root.append(&caption("The scale is linear from 0 to 0.0001 and logarithmic above that, so that zero stays on the axis. Labels are whole-currency amounts at powers of ten.")),
                Scale::Fit => root.append(&caption("The vertical axis is fitted to the values and does not start at zero. It shows small changes.")),
                Scale::Zero => {}
            }
        });

        // Progress.
        let loaded = fee.scheduled.iter().filter(|h| fee.checkpoints.get(*h).is_some_and(|c| c.known)).count();
        let read = fee.scheduled.iter().filter(|h| fee.checkpoints.contains_key(*h)).count();
        let blocks = state.page.as_ref().map(|p| p.samples.len()).unwrap_or(0);
        let signature = format!("{}|{loaded}|{read}|{}|{}|{blocks}|{:?}", fee.loading, fee.requested, fee.detail, fee.error);
        progress.set(signature, |root| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            if fee.loading {
                row.append(&loader::small());
            }
            row.append(&page::text(
                &format!(
                    "{loaded} of {} sampled checkpoints have a balance. {read} of {} are read.",
                    fee.requested, fee.requested
                ),
                &["callout", "tabular"],
            ));
            root.append(&row);
            let text = if fee.detail && blocks <= 2_000 {
                "Detail mode includes every block of this range.".to_string()
            } else {
                format!(
                    "The chart samples at most {} of {blocks} blocks. It can miss a peak between two samples. Shorten the span or turn on detail to look closer.",
                    if fee.detail { "2,000" } else { "100" }
                )
            };
            root.append(&caption(&text));
            root.append(&caption("The endpoints load first, then finer passes in random order. A block without a checkpoint, such as before activation, has no balance and breaks the line. This is a balance chart. It does not separate inflow from payout."));
            if let Some(error) = &fee.error {
                root.append(&page::note(error, true));
            }
        });
        drop(state);

        // Selected checkpoint.
        let sample = charts.selected_sample();
        let state = charts.state.borrow();
        let fee = &state.fee;
        let checkpoint = sample.as_ref().and_then(|s| fee.checkpoints.get(&s.hash).cloned());
        let signature = format!(
            "{:?}|{:?}|{has_reserve}|{native}|{reserve}|{}",
            sample.as_ref().map(|s| &s.hash),
            checkpoint.as_ref().map(|c| (c.known, c.native_sats, c.reserve_sats, c.problem.clone())),
            crate::display::uses_utc()
        );
        selected.set(signature, |root| match &sample {
            Some(sample) => {
                root.append(&moves(charts, &sample.hash));
                root.append(&page::field_row(&page, "Height", &format::integer(sample.height), Some(Route::Block { id: sample.hash.clone() }), false));
                root.append(&page::field_row(&page, "Time", &format::date(sample.time), None, false));
                match &checkpoint {
                    Some(checkpoint) => amounts(&page, root, checkpoint, has_reserve, &native, &reserve),
                    None => root.append(&page::text("This block's checkpoint has not loaded.", &["secondary"])),
                }
            }
            None => root.append(&page::text("No checkpoint with a balance has loaded yet.", &["secondary"])),
        });

        // Current fee pool.
        let signature = format!(
            "{:?}|{:?}|{has_reserve}|{native}|{reserve}|{}",
            fee.current.as_ref().map(|c| (&c.checkpoint.hash, c.checkpoint.known, c.checkpoint.native_sats, c.checkpoint.reserve_sats)),
            fee.current_error,
            crate::display::uses_utc()
        );
        current.set(signature, |root| {
            if let Some(current) = &fee.current {
                let checkpoint = &current.checkpoint;
                root.append(&page::field_row(
                    &page,
                    "Latest observed block",
                    &format::integer(checkpoint.height),
                    Some(Route::Block { id: checkpoint.hash.clone() }),
                    false,
                ));
                root.append(&page::field_row(&page, "Time", &format::date(checkpoint.time), None, false));
                amounts(&page, root, checkpoint, has_reserve, &native, &reserve);
                root.append(&caption("This balance is at the latest block when the app last read it, not at the end of the chart range."));
            } else if let Some(error) = &fee.current_error {
                root.append(&page::note(
                    &format!("The current fee pool is not available: {error}. The app does not assume a zero balance."),
                    true,
                ));
            } else {
                root.append(&page::text("Reading the latest block's checkpoint.", &["secondary"]));
            }
        });

        // Checkpoints as a table.
        let samples = state.page.as_ref().map(|p| p.samples.clone()).unwrap_or_default();
        let open = *shown.borrow();
        set_disclosure_title(&disclosure, if open { "Hide" } else { "Show the loaded checkpoints" });
        let signature = if open {
            format!(
                "{}|{}|{}|{has_reserve}|{native}|{reserve}|{}",
                samples.len(),
                samples.last().map(|s| s.hash.as_str()).unwrap_or(""),
                fee.checkpoints.len(),
                crate::display::uses_utc()
            )
        } else {
            "closed".into()
        };
        table.set(signature, |root| {
            if !open {
                return;
            }
            let rows: Vec<parts::Row> = samples
                .iter()
                .rev()
                .filter_map(|sample| {
                    let checkpoint = fee.checkpoints.get(&sample.hash)?;
                    Some(vec![
                        sample.height.to_string(),
                        format::date(sample.time),
                        if checkpoint.known { chart_format::sats(checkpoint.native_sats) } else { "No checkpoint".into() },
                        if checkpoint.known && has_reserve { chart_format::sats(checkpoint.reserve_sats) } else { "–".into() },
                    ])
                })
                .collect();
            root.append(&parts::table(&[("Height", 80), ("Time", 180), (&native, 140), (&reserve, 140)], rows, 280));
        });
    });
}

/// Changes the name of a disclosure: "Hide" when it is open.
pub fn set_disclosure_title(disclosure: &lamp::Disclosure, title: &str) {
    if let Some(label) = disclosure.header.last_child().and_downcast::<gtk::Label>() {
        crate::theme::set_label(&label, crate::theme::font::SMALL, title);
    }
}

/// Older, Newer and Open block, for the selected block.
pub fn moves(charts: &Rc<Charts>, hash: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let weak = Rc::downgrade(charts);
    row.append(&parts::key("Older", Mark::Left, move || {
        if let Some(charts) = weak.upgrade() {
            charts.move_by(-1);
        }
    }));
    let weak = Rc::downgrade(charts);
    row.append(&parts::key("Newer", Mark::Right, move || {
        if let Some(charts) = weak.upgrade() {
            charts.move_by(1);
        }
    }));
    let (weak, hash) = (Rc::downgrade(charts), hash.to_string());
    row.append(&lamp::button("Open block", move || {
        if let Some(charts) = weak.upgrade() {
            charts.open(&hash);
        }
    }));
    row
}

fn amounts(page: &Rc<Page>, root: &gtk::Box, checkpoint: &FeePoolCheckpoint, has_reserve: bool, native: &str, reserve: &str) {
    if checkpoint.known {
        root.append(&page::field_row(page, native, &format!("{} {native}", chart_format::sats(checkpoint.native_sats)), None, false));
        if has_reserve {
            root.append(&page::field_row(page, reserve, &format!("{} {reserve}", chart_format::sats(checkpoint.reserve_sats)), None, false));
        }
    } else if let Some(problem) = &checkpoint.problem {
        root.append(&page::note(
            &format!("The app did not read the checkpoint: {problem}. The app does not assume a zero balance."),
            true,
        ));
    } else {
        root.append(&page::text(
            "This block's coinbase has no fee pool checkpoint. Blocks before activation have none. The app does not assume a zero balance.",
            &["secondary"],
        ));
    }
}
