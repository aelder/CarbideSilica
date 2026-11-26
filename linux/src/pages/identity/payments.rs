//! How an identity registration's cost was split, checked as Carbide checks it: referral
//! payments, the issuer's charge, the native instruction budget and the transaction fee,
//! against the issuer's own fee rules (`RegistrationPaymentsCard` in
//! `RegistrationPayments.swift`). Nothing shows for a transaction that does not register
//! an identity.

use std::cell::Cell;
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;

use crate::bridge;
use crate::bridge::api::{Confidence, PaymentRow, PaymentsStatus, RegistrationPayments, Route};
use crate::format;
use crate::model::Topic;
use crate::page::{self, load::clear, Card, Page};
use crate::shared::confidence::confidence_mark;
use crate::shared::identity_support::{self as support, small_progress};
use crate::theme::{self, font};
use crate::widgets::display;
use crate::widgets::lamp;

enum Phase {
    Checking,
    Failed(String),
    Done(RegistrationPayments),
}

pub struct RegistrationPaymentsCard {
    pub card: Card,
    page: Weak<Page>,
    txid: String,
    attempt: Cell<u64>,
    phase: std::cell::RefCell<Phase>,
}

impl RegistrationPaymentsCard {
    /// The card for the registration transaction of an identity.
    pub fn new(page: &Rc<Page>, txid: &str) -> Rc<Self> {
        let card = page.card("Registration payments", None);
        let this = Rc::new(Self {
            card,
            page: Rc::downgrade(page),
            txid: txid.to_string(),
            attempt: Cell::new(0),
            phase: std::cell::RefCell::new(Phase::Checking),
        });
        let weak = Rc::downgrade(&this);
        page.model.watch(&[Topic::Connection], &this.card.root, move |_| {
            if let Some(this) = weak.upgrade() {
                this.load();
            }
        });
        let weak = Rc::downgrade(&this);
        page.model.watch(&[Topic::Names, Topic::Display], &this.card.root, move |_| {
            if let Some(this) = weak.upgrade() {
                this.render();
            }
        });
        this.load();
        this
    }

    fn load(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else { return };
        let model = page.model.clone();
        let attempt = self.attempt.get() + 1;
        self.attempt.set(attempt);
        self.phase.replace(Phase::Checking);
        self.render();
        let explorer = match model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                self.phase.replace(Phase::Failed(message));
                self.render();
                return;
            }
        };
        let txid = self.txid.clone();
        let generation = model.generation();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.registration_payments(txid).await }).await;
            let Some(this) = weak.upgrade() else { return };
            if this.attempt.get() != attempt || model.generation() != generation {
                return;
            }
            match result {
                Ok(payments) => {
                    let mut ids: Vec<String> = payments.referrals.iter().flat_map(|r| [r.recipient.clone(), r.currency.clone()]).collect();
                    ids.extend(payments.issuer.clone());
                    ids.extend(payments.charge.as_ref().map(|c| c.recipient.clone()));
                    this.phase.replace(Phase::Done(payments));
                    model.resolve_names(ids);
                }
                Err(error) => {
                    this.phase.replace(Phase::Failed(bridge::describe(&error)));
                }
            }
            this.render();
        });
    }

    fn render(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else { return };
        clear(&self.card.content);
        let phase = self.phase.borrow();
        match &*phase {
            Phase::Checking => {
                self.card.root.set_visible(true);
                self.card.append(&small_progress("Checking historical evidence"));
            }
            Phase::Failed(message) => {
                self.card.root.set_visible(true);
                self.card.append(&page::note(message, true));
                let weak = Rc::downgrade(self);
                self.card.append(&lamp::button("Try Again", move || {
                    if let Some(this) = weak.upgrade() {
                        this.load();
                    }
                }));
            }
            Phase::Done(payments) => {
                let shown = payments.status != PaymentsStatus::NotRegistration;
                self.card.root.set_visible(shown);
                if shown {
                    content(&page, &self.card, payments);
                }
            }
        }
    }
}

fn content(page: &Rc<Page>, card: &Card, payments: &RegistrationPayments) {
    let model = page.model.clone();
    match &payments.status {
        PaymentsStatus::Unverified { reason } => {
            card.append(&confidence_mark(Confidence::Unknown, Some("Unverified")));
            card.append(&page::paragraph(reason, &["secondary"]));
            let caption = page::paragraph(
                "Carbide marks the registration payments verified only when every payment matches the issuer's fee rules and the referral chain.",
                &["secondary"],
            );
            super::set_px(&caption, 10.0, 400);
            card.append(&caption);
        }
        PaymentsStatus::Verified | PaymentsStatus::NotRegistration => {
            let heading = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            heading.append(&confidence_mark(Confidence::Verified, Some("Verified against the issuer's rules")));
            if let Some(issuer) = &payments.issuer {
                let name = display::name_text("Issuer");
                name.set_valign(gtk::Align::Center);
                heading.append(&name);
                heading.append(&support::id_link(page, issuer, Route::Currency { name: issuer.clone() }, true));
            }
            card.append(&heading);
            let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let referral_amount = match payments.referrals.first() {
                Some(first) => support::amount_text(&model, payments.referral_total_sats, &first.currency),
                None => support::amount_text(&model, 0, payments.issuer.as_deref().unwrap_or(&model.coin())),
            };
            rows.append(&row(&format!("Referrals ({})", payments.referrals.len()), &referral_amount, false));
            for referral in &payments.referrals {
                let line = payment_row(page, referral);
                line.set_margin_start(18);
                rows.append(&line);
            }
            let spaced_divider = || {
                let divider = page::divider();
                divider.set_margin_top(4);
                divider.set_margin_bottom(4);
                divider
            };
            let coin = model.coin();
            if let Some(charge) = &payments.charge {
                rows.append(&spaced_divider());
                rows.append(&payment_row(page, charge));
                rows.append(&row("Native instruction budget", &format!("{} {coin}", format::coins(payments.budget_sats)), false));
            }
            rows.append(&spaced_divider());
            rows.append(&row("Transaction fee", &format!("{} {coin}", format::coins(payments.fee_sats)), false));
            let native = format!("{} {coin}", format::coins(payments.total_native_sats));
            let total = match (payments.total_issuer_sats, &payments.issuer) {
                (Some(sats), Some(issuer)) => format!("{} + {native}", support::amount_text(&model, sats, issuer)),
                _ => native,
            };
            rows.append(&row("Total", &total, true));
            card.append(&rows);
        }
    }
}

fn payment_row(page: &Rc<Page>, payment: &PaymentRow) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    line.set_margin_top(4);
    line.set_margin_bottom(4);
    let annotation = page::text(&payment.annotation, &[]);
    annotation.set_size_request(210, -1);
    annotation.set_valign(gtk::Align::Baseline);
    line.append(&annotation);
    let output = page::text(&format!("output {}", payment.output), &["secondary", "tabular"]);
    output.set_valign(gtk::Align::Baseline);
    line.append(&output);
    line.append(&support::id_link(page, &payment.recipient, Route::Identity { name: payment.recipient.clone() }, true));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    line.append(&spacer);
    let amount = page::text(&support::amount_text(&page.model, payment.amount_sats, &payment.currency), &["tabular"]);
    amount.set_valign(gtk::Align::Baseline);
    line.append(&amount);
    line
}

fn row(label: &str, amount: &str, bold: bool) -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    line.set_margin_top(4);
    line.set_margin_bottom(4);
    let name = if bold { theme::label(font::SMALL, label, "value") } else { display::name_text(label) };
    name.set_valign(gtk::Align::Baseline);
    name.set_hexpand(true);
    line.append(&name);
    let figure = page::text(amount, if bold { &["tabular", "semibold", "value"] } else { &["tabular"] });
    figure.set_valign(gtk::Align::Baseline);
    line.append(&figure);
    line
}
