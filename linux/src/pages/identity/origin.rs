//! Where an identity came from: who possibly paid for its registration, and the payments
//! the registration made to other identities (`IdentityOriginCard` in
//! `IdentityOrigin.swift`).
//!
//! The payers are always candidates: they funded the inputs. The payments are drawn solid
//! when Carbide verified them against the issuer's rules, and dashed when they are only
//! inferred from the transaction.

use std::rc::Rc;

use gtk::gsk;
use gtk::prelude::*;

use crate::bridge::api::{Confidence, PaymentsStatus, RegistrationPayments, Route, TransactionDetail};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, Page};
use crate::shared::confidence::confidence_mark;
use crate::shared::identity_support::{self as support, abbreviated_date, SectionLoader};
use crate::theme::{self, draw, font, tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payment {
    pub recipient: String,
    pub sats: i64,
    /// Empty for an inferred payment: the native coin.
    pub currency: String,
    /// Carbide's annotation, when the payment is verified.
    pub annotation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub payers: Vec<String>,
    pub payments: Vec<Payment>,
    pub verified: bool,
    pub spent: Option<i64>,
}

fn is_identity(address: &str) -> bool {
    address.starts_with('i') && address.chars().count() == 34
}

/// The origin of a registration from its transaction and Carbide's check of its payments.
pub fn origin(transaction: &TransactionDetail, payments: Option<&RegistrationPayments>, identity: Option<&str>) -> Origin {
    let mut payers: Vec<String> = Vec::new();
    for address in transaction.inputs.iter().flat_map(|input| input.addresses.iter()) {
        if !payers.contains(address) {
            payers.push(address.clone());
        }
    }
    let known: Vec<i64> = transaction.inputs.iter().filter_map(|input| input.value_sats).collect();
    let change: i64 = transaction
        .outputs
        .iter()
        .filter(|output| output.addresses.first().is_some_and(|a| payers.contains(a)))
        .filter_map(|output| output.value_sats)
        .sum();
    let spent = (known.len() == transaction.inputs.len() && !known.is_empty()).then(|| known.iter().sum::<i64>() - change);

    if let Some(payments) = payments
        && payments.status == PaymentsStatus::Verified
    {
        let mut rows: Vec<Payment> = payments
            .referrals
            .iter()
            .map(|r| Payment { recipient: r.recipient.clone(), sats: r.amount_sats, currency: r.currency.clone(), annotation: Some(r.annotation.clone()) })
            .collect();
        if let Some(charge) = &payments.charge {
            rows.insert(
                0,
                Payment { recipient: charge.recipient.clone(), sats: charge.amount_sats, currency: charge.currency.clone(), annotation: Some(charge.annotation.clone()) },
            );
        }
        payers.truncate(4);
        rows.truncate(5);
        return Origin { payers, payments: rows, verified: true, spent };
    }

    let mut inferred: Vec<Payment> = Vec::new();
    for output in &transaction.outputs {
        let Some(address) = output.addresses.first() else { continue };
        let Some(sats) = output.value_sats else { continue };
        if !is_identity(address) || Some(address.as_str()) == identity || payers.contains(address) || sats <= 0 {
            continue;
        }
        match inferred.iter_mut().find(|p| &p.recipient == address) {
            Some(existing) => {
                existing.sats += sats;
                existing.annotation = None;
            }
            None => inferred.push(Payment { recipient: address.clone(), sats, currency: String::new(), annotation: None }),
        }
    }
    payers.truncate(4);
    inferred.truncate(4);
    Origin { payers, payments: inferred, verified: false, spent }
}

fn amount(model: &AppModel, payment: &Payment) -> String {
    if payment.currency.is_empty() {
        format!("{} {}", format::coins(payment.sats), model.coin())
    } else {
        support::amount_text(model, payment.sats, &payment.currency)
    }
}

/// The sentence below the picture. `display` names an ID, `amount` writes a payment.
pub fn sentence(origin: &Origin, when: &str, coin: &str, display: &dyn Fn(&str) -> String, amount: &dyn Fn(&Payment) -> String) -> String {
    let mut text = format!("Registered on {when}");
    if let Some(payer) = origin.payers.first() {
        text.push_str(&format!(", possibly paid for by {}", display(payer)));
        if origin.payers.len() > 1 {
            text.push_str(&format!(" and {} more", origin.payers.len() - 1));
        }
    }
    if let Some(spent) = origin.spent
        && spent > 0
    {
        text.push_str(&format!(", at a cost of {} {coin}", format::coins(spent)));
    }
    text.push('.');
    if !origin.payments.is_empty() {
        let list: Vec<String> = origin
            .payments
            .iter()
            .map(|payment| {
                let role = payment.annotation.as_ref().map(|a| format!(" ({a})")).unwrap_or_default();
                format!("{} to {}{role}", amount(payment), display(&payment.recipient))
            })
            .collect();
        text.push_str(&format!(" The registration paid {}.", list.join(", ")));
        if !origin.verified {
            text.push_str(" Such a payment is usually a referral or a charge from the issuer. Carbide did not verify it.");
        }
    }
    text
}

/// The Origin card. `identity_address` is left out of the inferred payments. The caller
/// keeps the loader as long as the card.
pub fn origin_card(page: &Rc<Page>, txid: &str, height: u64, time: Option<u64>, identity_address: Option<&str>) -> (gtk::Box, Rc<SectionLoader>) {
    let card = page.card("Origin", None);
    let model = page.model.clone();
    let (load_txid, identity) = (txid.to_string(), identity_address.map(str::to_string));
    let weak_page = Rc::downgrade(page);
    let shown_txid = txid.to_string();
    let loader = SectionLoader::new(
        &model,
        "Loading the registration…",
        move |explorer| {
            let (txid, identity) = (load_txid.clone(), identity.clone());
            async move {
                let transaction = explorer.transaction(txid.clone()).await?;
                let payments = explorer.registration_payments(txid).await.ok();
                Ok(origin(&transaction, payments.as_ref(), identity.as_deref()))
            }
        },
        move |holder, origin| {
            let Some(page) = weak_page.upgrade() else { return };
            let origin = Rc::new(origin);
            let mut ids = origin.payers.clone();
            for payment in &origin.payments {
                ids.push(payment.recipient.clone());
                ids.push(payment.currency.clone());
            }
            page.model.resolve_names(ids);
            fill(&page, holder, &shown_txid, height, time, origin);
        },
    );
    card.append(&loader.root);
    (card.root, loader)
}

fn when_text(model: &Rc<AppModel>, height: u64, time: Option<u64>) -> String {
    match time {
        Some(time) => abbreviated_date(time),
        None => support::height_text(model, height, None),
    }
}

fn fill(page: &Rc<Page>, holder: &gtk::Box, txid: &str, height: u64, time: Option<u64>, origin: Rc<Origin>) {
    let model = page.model.clone();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let rows = origin.payers.len().max(origin.payments.len()).max(1) as i32;
    let canvas = {
        let (model, origin) = (Rc::downgrade(&model), origin.clone());
        Canvas::new(0, rows * 52 + 26, move |canvas, snapshot, w, h| {
            if let Some(model) = model.upgrade() {
                draw_origin(canvas, snapshot, w, h, &model, &origin, &when_text(&model, height, time));
            }
        })
    };
    canvas.set_hexpand(true);
    column.append(&canvas);
    column.append(&display::rule());
    let caption = theme::label(
        font::CAPTION,
        "Dashed boxes and lines are inferences. Solid lines are payments that Carbide verified against the issuer's rules.",
        "label",
    );
    caption.set_wrap(true);
    column.append(&caption);
    holder.append(&display::panel(&column, 12));

    let sentence_label = page::paragraph("", &["callout"]);
    holder.append(&sentence_label);
    let marks = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    marks.append(&confidence_mark(Confidence::Verified, Some("Registration")));
    marks.append(&confidence_mark(Confidence::Candidate, Some("Payers are candidates")));
    marks.append(&confidence_mark(
        if origin.verified { Confidence::Verified } else { Confidence::Candidate },
        Some(if origin.verified { "Payments verified" } else { "Payments not verified" }),
    ));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    marks.append(&spacer);
    marks.append(&page::route_link(page, txid, Route::Transaction { txid: txid.to_string() }, true));
    holder.append(&marks);

    let update = {
        let (sentence_label, canvas, origin) = (sentence_label.clone(), canvas.clone(), origin.clone());
        move |model: &Rc<AppModel>| {
            let display = |id: &str| support::display_id(model, id);
            let amount = |payment: &Payment| amount(model, payment);
            let text = sentence(&origin, &when_text(model, height, time), &model.coin(), &display, &amount);
            sentence_label.set_text(&text);
            canvas.update_property(&[gtk::accessible::Property::Label(&text)]);
            canvas.queue_draw();
        }
    };
    update(&model);
    model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &canvas, update);
}

fn dashed_stroke(dashed: bool) -> gsk::Stroke {
    let stroke = gsk::Stroke::new(1.0);
    stroke.set_line_cap(gsk::LineCap::Butt);
    if dashed {
        stroke.set_dash(&[5.0, 4.0]);
    }
    stroke
}

const REGISTERED: font::Role = font::Role { tracking: 1.1, ..font::HEADING };

#[allow(clippy::too_many_arguments)]
fn draw_origin(canvas: &Canvas, snapshot: &gtk::Snapshot, width: f32, height: f32, model: &AppModel, origin: &Origin, when: &str) {
    let mid_y = height / 2.0;
    let centre = (width / 2.0 - 80.0, mid_y - 28.0, 160.0, 56.0);
    let side = (width * 0.27).min(220.0);
    let slot = |index: usize, count: usize| height * (index as f32 + 0.5) / count.max(1) as f32;
    let display = |id: &str| support::display_id(model, id);
    let line = |x0: f32, y0: f32, x1: f32, y1: f32, color: Color, dashed: bool| {
        let builder = gsk::PathBuilder::new();
        builder.move_to(x0, y0);
        builder.line_to(x1, y1);
        snapshot.append_stroke(&builder.to_path(), &dashed_stroke(dashed), &color.rgba());
    };
    let boxed = |x: f32, y: f32, w: f32, h: f32, title: &str, subtitle: &str, dashed: bool, color: Color| {
        draw::fill_rect(snapshot, x + 0.5, y + 0.5, w - 1.0, h - 1.0, tone::SCREEN);
        snapshot.append_stroke(&draw::rect_path(x + 0.5, y + 0.5, w - 1.0, h - 1.0), &dashed_stroke(dashed), &color.rgba());
        // `.callout.weight(.semibold)` and `.caption2`: 12 and 10.
        let layout = super::px_layout(canvas, title, 12.0, 600);
        draw::draw_layout(snapshot, &layout, x + w / 2.0, y + h / 2.0 - 7.0, draw::Anchor::Center, tone::VALUE);
        let layout = super::px_layout(canvas, subtitle, 10.0, 400);
        draw::draw_layout(snapshot, &layout, x + w / 2.0, y + h / 2.0 + 9.0, draw::Anchor::Center, tone::LABEL);
    };
    for (index, payer) in origin.payers.iter().enumerate() {
        let y = slot(index, origin.payers.len());
        line(side, y, centre.0, mid_y, tone::AXIS, true);
        boxed(0.0, y - 20.0, side, 40.0, &display(payer), "possible payer", true, tone::AXIS);
    }
    let dashed = !origin.verified;
    let color = if origin.verified { tone::GOOD } else { tone::AXIS };
    for (index, payment) in origin.payments.iter().enumerate() {
        let y = slot(index, origin.payments.len());
        let x = width - side;
        line(centre.0 + centre.2, mid_y, x, y, color, dashed);
        let money = amount(model, payment);
        let subtitle = match &payment.annotation {
            Some(annotation) => format!("{annotation}: {money}"),
            None => format!("was paid {money}"),
        };
        boxed(x, y - 20.0, side, 40.0, &display(&payment.recipient), &subtitle, dashed, color);
    }
    let (cx, cy, cw, ch) = centre;
    draw::fill_rect(snapshot, cx + 0.5, cy + 0.5, cw - 1.0, ch - 1.0, tone::SCREEN);
    draw::fill_rect(snapshot, cx + 0.5, cy + 0.5, cw - 1.0, ch - 1.0, tone::RECORD.opacity(0.08));
    draw::stroke(snapshot, &draw::rect_path(cx + 0.5, cy + 0.5, cw - 1.0, ch - 1.0), tone::RECORD, 1.0);
    draw::text(snapshot, canvas, REGISTERED, "REGISTERED", cx + cw / 2.0, cy + ch / 2.0 - 8.0, draw::Anchor::Center, tone::RECORD);
    draw::figure(snapshot, canvas, when, cx + cw / 2.0, cy + ch / 2.0 + 11.0, draw::Anchor::Center, tone::LABEL);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentence_for_inferred_payments() {
        let origin = Origin {
            payers: vec!["RPayer".into(), "RTwo".into()],
            payments: vec![Payment { recipient: "iRef".into(), sats: 500_000_000, currency: String::new(), annotation: None }],
            verified: false,
            spent: Some(10_000_000_000),
        };
        let display = |id: &str| id.to_string();
        let amount = |p: &Payment| format!("{} VRSC", format::coins(p.sats));
        assert_eq!(
            sentence(&origin, "Dec 4, 2020", "VRSC", &display, &amount),
            "Registered on Dec 4, 2020, possibly paid for by RPayer and 1 more, at a cost of 100 VRSC. The registration paid 5 VRSC to iRef. Such a payment is usually a referral or a charge from the issuer. Carbide did not verify it."
        );
        let verified = Origin {
            payers: vec![],
            payments: vec![Payment { recipient: "iRef".into(), sats: 100_000_000, currency: "VRSC".into(), annotation: Some("Direct Referral Payment".into()) }],
            verified: true,
            spent: None,
        };
        assert_eq!(
            sentence(&verified, "block 5", "VRSC", &display, &amount),
            "Registered on block 5. The registration paid 1 VRSC to iRef (Direct Referral Payment)."
        );
    }
}
