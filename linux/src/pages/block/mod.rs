//! The Block page. Port of BlockView.swift in `macos/Sources/CarbideSilica/Views`: a
//! block's header, its place in the chain and its transactions.
//!
//! Names and dates follow the app's display settings.

use std::rc::Rc;

use gtk::prelude::*;

use crate::bridge::api::{self, Confidence, Route};
use crate::display;
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, Adjacent, LoadView, Page};
use crate::shared::{confidence::confidence_mark, inclusion, json_tree};
use crate::theme::tone;
use crate::widgets::icons::Icon;
use crate::widgets::display::PlateFill;
use crate::widgets::{lamp, link};

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>, id: &str) -> Rc<Page> {
    let page = Page::scrolling(model);
    let weak = Rc::downgrade(&page);
    let id_owned = id.to_string();
    let view = LoadView::new(
        model,
        move |explorer| {
            let id = id_owned.clone();
            async move { explorer.block(id).await }
        },
        move |holder, block| {
            if let Some(page) = weak.upgrade() {
                show(&page, holder, block);
            }
        },
    );
    page.append(&view.root);
    // The refresh key holds the view, so it lives as long as the page.
    page.hooks().refresh = Some(Rc::new(move || view.reload()));
    page
}

fn show(page: &Rc<Page>, holder: &gtk::Box, block: api::BlockDetail) {
    let model = page.model.clone();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 20);
    holder.append(&column);

    // The keys of the page: n and p step to the next and previous block in place, and a
    // opens the active block at this height for a block of a side branch.
    {
        let mut hooks = page.hooks();
        let step = |hash: &Option<String>| -> Option<page::Action> {
            let (hash, model) = (hash.clone()?, Rc::downgrade(&model));
            Some(Rc::new(move || {
                if let Some(model) = model.upgrade() {
                    model.step(Route::Block { id: hash.clone() });
                }
            }))
        };
        hooks.adjacent = Some(Adjacent {
            noun: "Block".into(),
            previous: step(&block.previous_hash),
            next: step(&block.next_hash),
        });
        hooks.keys.remove("a");
        if !block.on_active_chain
            && let Some(active) = block.active_hash.clone()
        {
            let model = Rc::downgrade(&model);
            hooks.keys.insert(
                "a".into(),
                (
                    "Open the Active Block at this Height".into(),
                    Rc::new(move || {
                        if let Some(model) = model.upgrade() {
                            model.open(Route::Block { id: active.clone() });
                        }
                    }),
                ),
            );
        }
    }

    let mut tags: Vec<(String, PlateFill)> =
        vec![(if block.row.staked { "Staked" } else { "Mined" }.to_string(), PlateFill::None)];
    tags.extend(block.combined_tags.iter().map(|t| (t.clone(), PlateFill::None)));
    column.append(&page::page_header(&format!("#{}", format::integer(block.row.height)), Some(&block.row.hash), &tags));
    column.append(&page::provenance_note(&block.provenance));

    // The chain status, and the keys of the block.
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    keys.append(&chain_status(&block));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    keys.append(&spacer);
    let charts = {
        let (model, height) = (Rc::downgrade(&model), block.row.height);
        let hash = (!block.on_active_chain).then(|| block.row.hash.clone());
        lamp::button_with("Open in Charts", Some(Icon::Chart), move || {
            if let Some(model) = model.upgrade() {
                model.show_charts(Some(height), hash.clone());
            }
        })
    };
    charts.set_tooltip_text(Some("Open Charts at this block."));
    keys.append(&charts);
    for (glyph, title, hash) in
        [(Icon::ChevronLeft, "Previous", block.previous_hash.clone()), (Icon::ChevronRight, "Next", block.next_hash.clone())]
    {
        let model = Rc::downgrade(&model);
        let enabled = hash.is_some();
        let key = lamp::button_with(title, Some(glyph), move || {
            if let (Some(model), Some(hash)) = (model.upgrade(), hash.clone()) {
                model.step(Route::Block { id: hash });
            }
        });
        key.set_sensitive(enabled);
        keys.append(&key);
    }
    column.append(&keys);

    if !block.on_active_chain && block.confirmations.is_some_and(|c| c < 0) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let note = page::note("This block is not on the active chain.", true);
        note.set_hexpand(false);
        row.append(&note);
        if let Some(active) = block.active_hash.clone() {
            let model = Rc::downgrade(&model);
            row.append(&link::link(
                &format!("Open the active block at height {}", format::integer(block.row.height)),
                false,
                move || {
                    if let Some(model) = model.upgrade() {
                        model.open(Route::Block { id: active.clone() });
                    }
                },
            ));
        }
        column.append(&row);
    }

    // The header. Its rows follow the dates setting.
    let header = page.card("Header", None);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 12);
    header.append(&rows);
    column.append(&header.root);
    let fill_header = {
        let (page, rows, block) = (Rc::downgrade(page), rows.clone(), block.clone());
        move || {
            let Some(page) = page.upgrade() else { return };
            page::load::clear(&rows);
            for row in header_rows(&page, &block) {
                rows.append(&row);
            }
        }
    };
    fill_header();
    let shows_dates = std::cell::Cell::new(display::shows_dates());
    model.watch(&[Topic::Display], &rows, move |_| {
        if shows_dates.replace(display::shows_dates()) != display::shows_dates() {
            fill_header();
        }
    });

    // The transactions.
    let transactions = page.card("Transactions", Some(&block.transactions.len().to_string()));
    if !block.combined_tags.is_empty() {
        let tags: Vec<(String, PlateFill)> = block.combined_tags.iter().map(|t| (t.clone(), PlateFill::None)).collect();
        transactions.append(&page::tag_row(&tags));
        transactions.append(&page::text(
            "The tags of all transactions in the block, without Coinbase and Stake.",
            &["secondary", "caption-small"],
        ));
    }
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (index, transaction) in block.transactions.iter().enumerate() {
        if index > 0 {
            list.append(&page::divider());
        }
        list.append(&transaction_row(page, transaction));
    }
    transactions.append(&list);
    column.append(&transactions.root);

    column.append(&inclusion::block_branches_card(page, &block.row.hash).root);
    column.append(&json_tree::json_section(page, &block.json, &format!("block-{}.json", block.row.height)));
}

/// The rows of the Header card.
fn header_rows(page: &Rc<Page>, block: &api::BlockDetail) -> Vec<gtk::Box> {
    let dates = display::shows_dates();
    let row = &block.row;
    let mut rows = vec![page::field_row(page, "Hash", &row.hash, None, true)];
    if dates {
        rows.push(page::field_row(
            page,
            "Height",
            &format!("{}, recorded {}", format::integer(row.height), format::date(row.time)),
            None,
            false,
        ));
    }
    rows.push(page::field_row(
        page,
        "Time",
        &format!("{} ({})", format::date(row.time), format::age(row.time, format::now())),
        None,
        false,
    ));
    if let Some(confirmations) = block.confirmations {
        let text = if dates && confirmations > 0 {
            display::confirmations(confirmations, Some(row.time))
        } else if confirmations >= 0 {
            format::integer(confirmations as u64)
        } else {
            confirmations.to_string()
        };
        rows.push(page::field_row(page, "Confirmations", &text, None, false));
    }
    rows.push(page::field_row(page, "Type", &production_text(block), None, false));
    let size = row.size.map(|s| format!("{} bytes", format::integer(s))).unwrap_or_else(|| "Unknown".into());
    rows.push(page::field_row(page, "Size", &size, None, false));
    rows.push(page::field_row(page, "Transactions", &row.transaction_count.to_string(), None, false));
    rows.push(page::field_row(page, "Detail", &detail(block), None, false));
    for (label, value) in [("Previous block", &block.previous_hash), ("Next block", &block.next_hash)] {
        if let Some(hash) = value {
            rows.push(page::field_row(page, label, hash, Some(Route::Block { id: hash.clone() }), true));
        }
    }
    for field in &block.fields {
        rows.push(page::field_row(page, &field.label, &field.value, None, true));
    }
    rows
}

/// The block's place in the chain, with shape, colour and a word.
fn chain_status(block: &api::BlockDetail) -> gtk::Box {
    match block.chain_status.as_deref() {
        Some("Chain tip") => confidence_mark(Confidence::Verified, Some("Chain tip")),
        Some("Active") => confidence_mark(Confidence::Verified, Some("Active chain")),
        Some("Orphaned") => confidence_mark(Confidence::Unknown, Some("Orphaned")),
        Some(other) => confidence_mark(Confidence::Unknown, Some(other)),
        None => confidence_mark(Confidence::Unknown, Some("Chain status unknown")),
    }
}

pub fn detail_text(count: usize, loaded: u32) -> String {
    if loaded as usize >= count {
        return format!("All {count} transaction{} loaded", if count == 1 { "" } else { "s" });
    }
    format!("{loaded} of {count} transactions loaded. Tags and totals cover the loaded ones.")
}

fn detail(block: &api::BlockDetail) -> String {
    detail_text(block.transactions.len(), block.bodies_loaded)
}

/// "Staked" or "Mined", the app's words, with the node's own block type when it differs.
pub fn production(staked: bool, reported: &str) -> String {
    let word = if staked { "Staked" } else { "Mined" };
    let reported = reported.trim();
    if reported.is_empty() || reported.to_lowercase() == word.to_lowercase() {
        word.to_string()
    } else {
        format!("{word} (the node reports \"{reported}\")")
    }
}

fn production_text(block: &api::BlockDetail) -> String {
    production(block.row.staked, &block.production)
}

/// A transaction as listed inside a block (`TransactionRowView`).
fn transaction_row(page: &Rc<Page>, transaction: &api::TransactionRow) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_top(7);
    row.set_margin_bottom(7);
    let id = page::route_link(page, &transaction.txid, Route::Transaction { txid: transaction.txid.clone() }, true);
    id.set_valign(gtk::Align::Center);
    row.append(&id);
    let tags: Vec<(String, PlateFill)> = transaction.tags.iter().map(|t| (t.clone(), PlateFill::None)).collect();
    let tag_row = page::tag_row(&tags);
    tag_row.set_valign(gtk::Align::Center);
    row.append(&tag_row);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    spacer.set_size_request(8, -1);
    row.append(&spacer);
    if transaction.input_count + transaction.output_count > 0 {
        let counts = page::text(
            &format!("{} in · {} out", transaction.input_count, transaction.output_count),
            &["callout", "tabular", "label"],
        );
        row.append(&counts);
    }
    if let Some(sats) = transaction.output_sats {
        let amount = gtk::Label::new(None);
        amount.set_markup(&page::coin_markup(sats, &page.model.coin(), tone::VALUE));
        amount.add_css_class("tabular");
        amount.set_xalign(1.0);
        amount.set_size_request(150, -1);
        row.append(&amount);
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_names_the_node_type_when_it_differs() {
        assert_eq!(production(true, "staked"), "Staked");
        assert_eq!(production(false, " Mined "), "Mined");
        assert_eq!(production(false, ""), "Mined");
        assert_eq!(production(true, "pos"), "Staked (the node reports \"pos\")");
    }

    #[test]
    fn detail_counts_the_loaded_bodies() {
        assert_eq!(detail_text(1, 1), "All 1 transaction loaded");
        assert_eq!(detail_text(3, 5), "All 3 transactions loaded");
        assert_eq!(detail_text(40, 25), "25 of 40 transactions loaded. Tags and totals cover the loaded ones.");
    }
}
