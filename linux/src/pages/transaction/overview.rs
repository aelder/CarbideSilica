//! The terminal's header lines (`TransactionHeader`, `ValueLines`, `TotalRow`,
//! `ReserveAmount` and `RegistrationLine` in `TransactionView.swift`): status, block,
//! expiry, size, values, fee and the context lines for notarizations, shielded value and
//! identity registrations.

use std::rc::Rc;

use gtk::prelude::*;

use crate::bridge::api::{self, Confidence, Route};
use crate::display as dates;
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, Page};
use crate::shared::confidence::confidence_mark;
use crate::widgets::display::{self, PlateFill};

use super::Ctx;

/// A text in the callout size: secondary when `secondary`.
pub fn callout(text: &str, secondary: bool) -> gtk::Label {
    let label = page::text(text, if secondary { &["callout", "secondary"] } else { &["callout"] });
    label.set_valign(gtk::Align::Baseline);
    label
}

/// A small name in a column of 170, then the value, as the rows of the overview.
pub fn labelled(label: &str, value: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let name = display::name_text(label);
    name.set_size_request(170, -1);
    name.set_valign(gtk::Align::Baseline);
    row.append(&name);
    value.as_ref().set_valign(gtk::Align::Baseline);
    row.append(value);
    row
}

/// A token amount: the number, then the currency's name as a link (`ReserveAmount`).
pub fn reserve_amount(page: &Page, amount: &api::CurrencyAmount, prefix: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let figure = page::text(&format!("{prefix}{}", format::amount(amount.amount)), &["callout", "tabular"]);
    figure.set_valign(gtk::Align::Baseline);
    row.append(&figure);
    let link = page::route_link(page, &amount.currency, Route::Currency { name: amount.currency.clone() }, true);
    if let Some(label) = crate::widgets::link::label_of(&link) {
        label.add_css_class("callout");
    }
    row.append(&link);
    row
}

/// The card. It follows the display settings and the dates of heights.
pub fn card(ctx: &Rc<Ctx>) -> gtk::Box {
    let Some(page) = ctx.page() else { return gtk::Box::new(gtk::Orientation::Vertical, 0) };
    let card = page.card("Overview", None);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.append(&body);
    let refill = {
        let weak = Rc::downgrade(ctx);
        let body = body.downgrade();
        move |_: &Rc<AppModel>| {
            let (Some(ctx), Some(body)) = (weak.upgrade(), body.upgrade()) else { return };
            clear(&body);
            fill(&ctx, &body);
        }
    };
    refill(&ctx.model);
    ctx.model.watch(&[Topic::Display, Topic::HeightDates], &body, refill);
    card.root
}

fn fill(ctx: &Rc<Ctx>, body: &gtk::Box) {
    let Some(page) = ctx.page() else { return };
    let tx = &ctx.tx;
    let model = &ctx.model;
    let shows_dates = dates::shows_dates();
    let height = |value: u64, known: Option<u64>| dates::height_text(value, known, model);

    let status = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    status.append(&status_mark(tx, shows_dates));
    if tx.in_mempool {
        status.append(&display::plate("Mempool", PlateFill::Mined));
    }
    if tx.from_public_api {
        let plate = display::plate("Public API", PlateFill::None);
        plate.set_tooltip_text(Some("A public API supplied this transaction, not the local node."));
        status.append(&plate);
    }
    body.append(&status);
    for line in &tx.notarization_context {
        body.append(&page::paragraph(line, &["secondary"]));
    }

    match &tx.block_hash {
        Some(hash) => {
            let value = tx.height.map(|h| height(h, tx.block_time)).unwrap_or_else(|| "Height unknown".into());
            body.append(&page::field_row(&page, "Block", &value, Some(Route::Block { id: hash.clone() }), false));
            body.append(&page::field_row(&page, "Block hash", hash, Some(Route::Block { id: hash.clone() }), true));
        }
        None => body.append(&page::field_row(&page, "Block", "Not in a block", None, false)),
    }
    let block_time = tx
        .block_time
        .map(|t| format!("{} ({})", format::date(t), format::age(t, format::now())))
        .unwrap_or_else(|| "Not yet confirmed".into());
    body.append(&page::field_row(&page, "Block time", &block_time, None, false));
    if let Some(confirmations) = tx.confirmations.filter(|c| *c > 0) {
        let value = if shows_dates { dates::confirmations(confirmations, tx.block_time) } else { format::signed(confirmations) };
        body.append(&page::field_row(&page, "Confirmations", &value, None, false));
    }
    let expiry = match tx.expiry_height.filter(|v| *v > 0) {
        Some(value) => height(value, None),
        None => "None".into(),
    };
    body.append(&page::field_row(&page, "Expiry", &expiry, None, false));
    let size = tx.size.map(|s| format!("{} bytes", format::integer(s))).unwrap_or_else(|| "Unknown".into());
    body.append(&page::field_row(&page, "Size", &size, None, false));
    let version = tx.version.map(|v| v.to_string()).unwrap_or_else(|| "Unknown".into());
    body.append(&page::field_row(&page, "Version", &version, None, false));
    // A locktime below 500,000,000 is a height. Above it is a Unix time.
    let locktime = match tx.locktime {
        None => "Unknown".to_string(),
        Some(0) => "0".to_string(),
        Some(value) if value >= 500_000_000 => {
            if shows_dates { format::date(value) } else { format!("{value} (Unix time {})", format::date(value)) }
        }
        Some(value) => {
            if shows_dates { height(value, None) } else { format!("Height {}", format::integer(value)) }
        }
    };
    body.append(&page::field_row(&page, "Locktime", &locktime, None, false));

    value_lines(&page, tx, body);

    for line in &tx.shielded {
        body.append(&page::field_row(&page, "Shielded", line, None, false));
    }
    for line in &tx.registration {
        body.append(&registration_line(&page, line));
    }
    if tx.fee_pool_checkpoint {
        body.append(&page::paragraph(
            "The fee pool output is an unspendable protocol checkpoint. Its transfer trace compares adjacent coinbase states.",
            &["secondary"],
        ));
    }
}

fn status_mark(tx: &api::TransactionDetail, shows_dates: bool) -> gtk::Box {
    match tx.status {
        api::TransactionStatus::Confirmed => match tx.block_time.filter(|_| shows_dates) {
            Some(time) => confidence_mark(Confidence::Verified, Some(&format!("Confirmed since {}", format::date(time)))),
            None => {
                let digits: String = tx.status_text.chars().filter(|c| c.is_ascii_digit()).collect();
                confidence_mark(Confidence::Verified, Some(&format!("Confirmed ({digits})")))
            }
        },
        api::TransactionStatus::Unconfirmed => confidence_mark(Confidence::Observed, Some("Unconfirmed")),
        api::TransactionStatus::ConfirmationsUnverified => {
            let mark = confidence_mark(Confidence::Unknown, Some("Confirmations unverified"));
            mark.set_tooltip_text(Some("The transaction names a block, but the source did not establish its confirmations."));
            mark
        }
        api::TransactionStatus::OrphanedInclusion => {
            let mark = confidence_mark(Confidence::Unknown, Some("Orphaned inclusion"));
            mark.set_tooltip_text(Some(
                "The block that holds this transaction is not on the active chain. The app did not check the mempool.",
            ));
            mark
        }
    }
}

/// Input and output totals, fee and, for protocol transactions, the public native delta.
fn value_lines(page: &Page, tx: &api::TransactionDetail, body: &gtk::Box) {
    let accounting = &tx.accounting;
    body.append(&total_row(page, &format!("Inputs ({})", accounting.input_count), &accounting.inputs));
    body.append(&total_row(page, &format!("Outputs ({})", accounting.output_count), &accounting.outputs));
    let fee: gtk::Label = match accounting.fee_kind {
        api::FeeKind::Known => {
            let label = page::text(&accounting.fee_label, &["tabular"]);
            label.set_selectable(true);
            label
        }
        api::FeeKind::Unknown => page::text("Unknown. A value is missing from the source.", &["secondary"]),
        api::FeeKind::ProtocolAccounting => {
            let label = page::text("Unknown (protocol accounting)", &["secondary"]);
            label.set_tooltip_text(Some("Protocol outputs account for value elsewhere. Carbide does not reconstruct the consensus fee."));
            label
        }
        api::FeeKind::NotApplicable => page::text("Not applicable to a coinbase", &["secondary"]),
    };
    body.append(&labelled("Fee", &fee));
    if accounting.show_public_delta {
        let value = match accounting.public_native_delta {
            Some(delta) => {
                let label = gtk::Label::new(None);
                label.set_markup(&format!(
                    "<span font_features=\"tnum\">{}</span>{}",
                    gtk::glib::markup_escape_text(&format::coins(delta)),
                    page::span(&format!(" {}", tx.coin), crate::theme::tone::SECONDARY)
                ));
                label.set_selectable(true);
                label
            }
            None => page::text("Unknown", &["secondary"]),
        };
        body.append(&labelled("Public native delta", &value));
        body.append(&page::paragraph(
            "Transparent inputs minus transparent outputs, plus the net shielded flow. It is not the fee.",
            &["caption-text", "secondary"],
        ));
    }
}

fn total_row(page: &Page, label: &str, total: &api::ValueTotal) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 3);
    let figure = page::text(&total.label, &["tabular"]);
    figure.set_selectable(true);
    column.append(&figure);
    for reserve in &total.reserves {
        column.append(&reserve_amount(page, reserve, "+ "));
    }
    if total.remaining_unknown {
        column.append(&callout("Remaining values unknown", true));
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let name = display::name_text(label);
    name.set_size_request(170, -1);
    name.set_valign(gtk::Align::Start);
    name.set_margin_top(3);
    row.append(&name);
    row.append(&column);
    row
}

/// An identity registration line. IDs in it are links that show names.
fn registration_line(page: &Page, line: &str) -> gtk::Box {
    if let Some((label, value)) = line.split_once(": ")
        && value.starts_with('i')
        && value.chars().count() == 34
    {
        return page::field_row(page, label, value, Some(Route::Identity { name: value.to_string() }), false);
    }
    let text = page::text(line, &[]);
    text.set_selectable(true);
    labelled("Identity", &text)
}
