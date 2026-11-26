//! References between this transaction's outputs and other outputs, and the signers of
//! notarization evidence (`ProtocolLinksCard`, `LinkList`, `WitnessCard` and
//! `SignatureGroup` in `TransactionExtras.swift`).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::gsk;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Confidence, api::Route};
use crate::format;
use crate::page::{self, load::clear, Card, Page};
use crate::shared::confidence::confidence_mark;
use crate::theme::{self, draw, font, tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;
use crate::widgets::icons::{self, Icon};
use crate::widgets::{lamp, link, loader};

use super::overview::callout;
use super::Ctx;

/// A small arrow to the left or to the right (`arrow.left`, `arrow.right`).
pub fn arrow(left: bool, color: Color) -> Canvas {
    let mark = Canvas::new(11, 9, move |_, snapshot, w, h| {
        let mid = h / 2.0;
        let (tip, tail) = if left { (0.5, w - 0.5) } else { (w - 0.5, 0.5) };
        draw::rule(snapshot, tail, mid, tip, mid, color, 1.2, &[]);
        let head = gsk::PathBuilder::new();
        let back = if left { tip + 3.5 } else { tip - 3.5 };
        head.move_to(back, mid - 3.5);
        head.line_to(tip, mid);
        head.line_to(back, mid + 3.5);
        draw::stroke(snapshot, &head.to_path(), color, 1.2);
    });
    mark.set_valign(gtk::Align::Center);
    mark.set_can_target(false);
    mark
}

/// A filled circle with a check or a cross (`checkmark.circle.fill`, `xmark.circle.fill`).
fn sealed(check: bool, color: Color) -> Canvas {
    let mark = Canvas::new(13, 13, move |_, snapshot, w, h| {
        let (cx, cy, r) = (w / 2.0, h / 2.0, w.min(h) / 2.0 - 0.5);
        let circle = gsk::PathBuilder::new();
        circle.add_circle(&draw::point(cx, cy), r);
        draw::fill(snapshot, &circle.to_path(), color);
        let line = gsk::PathBuilder::new();
        if check {
            line.move_to(cx - r * 0.45, cy);
            line.line_to(cx - r * 0.1, cy + r * 0.35);
            line.line_to(cx + r * 0.5, cy - r * 0.35);
        } else {
            line.move_to(cx - r * 0.38, cy - r * 0.38);
            line.line_to(cx + r * 0.38, cy + r * 0.38);
            line.move_to(cx + r * 0.38, cy - r * 0.38);
            line.line_to(cx - r * 0.38, cy + r * 0.38);
        }
        draw::stroke(snapshot, &line.to_path(), tone::SCREEN, 1.6);
    });
    mark.set_valign(gtk::Align::Center);
    mark
}

/// An ID on another system: shortened, monospaced and selectable, the whole ID in the help.
pub fn foreign_id(txid: &str) -> gtk::Label {
    let label = page::text(&format::short(txid, 10, 8), &["mono"]);
    label.set_selectable(true);
    label.set_tooltip_text(Some(txid));
    label.set_valign(gtk::Align::Baseline);
    label
}

/// A transaction on this chain is a link. On another system it is its ID.
pub fn transaction_target(page: &Page, chain: Option<&str>, system: &str, txid: &str) -> gtk::Widget {
    if chain == Some(system) {
        page::route_link(page, txid, Route::Transaction { txid: txid.to_string() }, true).upcast()
    } else {
        foreign_id(txid).upcast()
    }
}

fn small_progress(text: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&loader::small());
    row.append(&page::text(text, &["secondary"]));
    row
}

/// A warning in Carbide's words, with the triangle, in the colour of an alert.
fn problem(text: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    row.append(&icons::icon(Icon::Warning, 13, tone::ALERT));
    let label = gtk::Label::new(None);
    label.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(text)));
    label.add_css_class("callout");
    label.add_css_class("alert");
    row.append(&label);
    row
}

// MARK: References

struct Links {
    ctx: Weak<Ctx>,
    card: Card,
    report: RefCell<Option<api::LinkReport>>,
    failure: RefCell<Option<String>>,
}

/// The references between this transaction's outputs and other outputs: the notarization,
/// evidence, finalization and import/export links Carbide follows with F3, forward and
/// reverse, each with Carbide's resolution status.
pub fn links_card(ctx: &Rc<Ctx>) -> gtk::Box {
    let Some(page) = ctx.page() else { return gtk::Box::new(gtk::Orientation::Vertical, 0) };
    let card = page.card("References", Some(""));
    let links = Rc::new(Links { ctx: Rc::downgrade(ctx), card, report: RefCell::new(None), failure: RefCell::new(None) });
    links.fill();
    let root = links.card.root.clone();
    ctx.keep(links.clone());
    let Some(explorer) = ctx.model.explorer() else {
        *links.failure.borrow_mut() = ctx.model.require_explorer().err();
        links.fill();
        return root;
    };
    let txid = ctx.tx.txid.clone();
    let generation = ctx.model.generation();
    let weak = Rc::downgrade(&links);
    glib::spawn_future_local(async move {
        let found = bridge::run(async move { explorer.protocol_links(txid).await }).await;
        let Some(links) = weak.upgrade() else { return };
        let Some(ctx) = links.ctx.upgrade() else { return };
        if ctx.model.generation() != generation {
            return;
        }
        match found {
            Ok(report) => {
                ctx.model.resolve_names(report.links.iter().map(|l| l.target_system.clone()));
                *links.report.borrow_mut() = Some(report);
            }
            Err(error) => *links.failure.borrow_mut() = Some(bridge::describe(&error)),
        }
        links.fill();
    });
    root
}

impl Links {
    fn fill(&self) {
        let (Some(ctx), content) = (self.ctx.upgrade(), &self.card.content) else { return };
        let Some(page) = ctx.page() else { return };
        clear(content);
        let report = self.report.borrow();
        let failure = self.failure.borrow();
        let links: Vec<api::ProtocolLink> = report.as_ref().map(|r| r.links.clone()).unwrap_or_else(|| ctx.tx.links.clone());
        self.card.set_trailing(Some(&format!("{} link{}", links.len(), if links.len() == 1 { "" } else { "s" })));
        let forward: Vec<&api::ProtocolLink> = links.iter().filter(|l| !l.reverse).collect();
        let reverse: Vec<&api::ProtocolLink> = links.iter().filter(|l| l.reverse).collect();
        if forward.is_empty() {
            content.append(&page::paragraph("This transaction has no explicit reference to another transaction.", &["secondary"]));
        } else {
            content.append(&link_list(&ctx, &page, &forward, report.is_none() && failure.is_none()));
        }
        let heading = display::name_text("Referenced by");
        heading.set_margin_top(4);
        content.append(&heading);
        if let Some(report) = report.as_ref() {
            if !reverse.is_empty() {
                content.append(&link_list(&ctx, &page, &reverse, false));
            } else if let Some(reason) = &report.reverse_unavailable {
                content.append(&page::note(reason, true));
            } else {
                content.append(&page::paragraph("The cache holds no reference to this transaction from another transaction.", &["secondary"]));
            }
            content.append(&page::paragraph(
                "Reverse references come from Carbide's cache. They include only transactions that the app or the terminal has already loaded.",
                &["caption-text", "secondary"],
            ));
        } else if let Some(failure) = failure.as_ref() {
            content.append(&page::note(failure, true));
        } else {
            content.append(&small_progress("Checking the other side of each reference…"));
        }
    }
}

fn link_list(ctx: &Rc<Ctx>, page: &Page, links: &[&api::ProtocolLink], checking: bool) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let chain = ctx.chain_id();
    for (index, link) in links.iter().enumerate() {
        if index > 0 {
            column.append(&page::divider());
        }
        let block = gtk::Box::new(gtk::Orientation::Vertical, 4);
        block.set_margin_top(7);
        block.set_margin_bottom(7);
        let first = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let output = display::name_text(&format!("Output {}", link.output));
        output.set_size_request(76, -1);
        output.set_valign(gtk::Align::Center);
        first.append(&output);
        first.append(&page::text(&link.label, &["medium"]));
        first.append(&arrow(link.reverse, tone::SECONDARY));
        // The other end can be on another system.
        first.append(&transaction_target(page, chain.as_deref(), &link.target_system, &link.target_txid));
        first.append(&page::text(&format!("output {} on", link.target_output), &["secondary"]));
        first.append(&page::route_link(page, &link.target_system, Route::Currency { name: link.target_system.clone() }, true));
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        first.append(&spacer);
        first.append(&page::copy_button(&format!("{}:{}", link.target_txid, link.target_output)));
        block.append(&first);
        let second = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        space.set_size_request(76, -1);
        second.append(&space);
        second.append(&status(link, checking));
        second.append(&page::text(&link.field, &["caption-text", "mono", "tertiary"]));
        block.append(&second);
        column.append(&block);
    }
    column
}

fn status(link: &api::ProtocolLink, checking: bool) -> gtk::Widget {
    match &link.status {
        Some(status) if link.resolved => confidence_mark(Confidence::Verified, Some(status)).upcast(),
        Some(status) if link.problem => problem(status).upcast(),
        Some(status) => confidence_mark(Confidence::Unknown, Some(status)).upcast(),
        None if checking => confidence_mark(Confidence::Observed, Some("Checking")).upcast(),
        None => confidence_mark(Confidence::Unknown, Some("Unresolved")).upcast(),
    }
}

// MARK: Witnesses

/// Who signed the notarization evidence in one output, using Carbide's witness
/// investigation (the terminal's F4). Signatures are grouped by identity, signing height,
/// statement, signing system and signed target.
pub struct WitnessCard {
    pub root: gtk::Box,
    ctx: Weak<Ctx>,
    root_output: api::TraceRoot,
    content: gtk::Box,
    report: RefCell<Option<api::WitnessReport>>,
    failure: RefCell<Option<String>>,
    running: Cell<bool>,
    disclosure: RefCell<Option<Rc<lamp::Disclosure>>>,
}

impl WitnessCard {
    pub fn new(ctx: &Rc<Ctx>, root: api::TraceRoot) -> Rc<Self> {
        let page = ctx.page().expect("the page builds its cards");
        let card = page.card("Signers", Some(&format!("{} · output {}", root.label, root.output)));
        let witness = Rc::new(Self {
            root: card.root.clone(),
            ctx: Rc::downgrade(ctx),
            root_output: root,
            content: card.content.clone(),
            report: RefCell::new(None),
            failure: RefCell::new(None),
            running: Cell::new(false),
            disclosure: RefCell::new(None),
        });
        witness.fill();
        witness
    }

    /// Starts the investigation from a menu command, once.
    pub fn start(self: &Rc<Self>) {
        if self.report.borrow().is_none() && !self.running.get() {
            self.run(false);
        }
    }

    fn run(self: &Rc<Self>, refresh: bool) {
        let Some(ctx) = self.ctx.upgrade() else { return };
        let explorer = match ctx.model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                *self.failure.borrow_mut() = Some(message);
                self.fill();
                return;
            }
        };
        self.running.set(true);
        *self.failure.borrow_mut() = None;
        self.fill();
        let (txid, output) = (ctx.tx.txid.clone(), self.root_output.output);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let found = bridge::run(async move { explorer.witnesses(txid, output, refresh).await }).await;
            let Some(card) = weak.upgrade() else { return };
            card.running.set(false);
            match found {
                Ok(report) => {
                    if let Some(ctx) = card.ctx.upgrade() {
                        ctx.model.resolve_names(
                            report.signatures.iter().flat_map(|s| [s.identity.clone(), s.system.clone(), s.target_system.clone()]),
                        );
                    }
                    *card.report.borrow_mut() = Some(report);
                }
                Err(error) => *card.failure.borrow_mut() = Some(bridge::describe(&error)),
            }
            card.fill();
        });
    }

    fn fill(self: &Rc<Self>) {
        let Some(ctx) = self.ctx.upgrade() else { return };
        let Some(page) = ctx.page() else { return };
        clear(&self.content);
        let report = self.report.borrow();
        if let Some(report) = report.as_ref() {
            let summary = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let groups = report.signatures.len();
            let text = page::text(
                &format!(
                    "{groups} signature group{} · {} transactions · {} outputs · {} multipart set{} decoded",
                    if groups == 1 { "" } else { "s" },
                    report.transactions_checked,
                    report.outputs_checked,
                    report.multipart_sets,
                    if report.multipart_sets == 1 { "" } else { "s" }
                ),
                &["secondary"],
            );
            text.set_hexpand(true);
            summary.append(&text);
            let weak = Rc::downgrade(self);
            let refresh = lamp::button("Refresh", move || {
                if let Some(card) = weak.upgrade() {
                    card.run(true);
                }
            });
            refresh.set_sensitive(!self.running.get());
            refresh.set_tooltip_text(Some("Request every reachable transaction again, instead of the answers already received."));
            summary.append(&refresh);
            self.content.append(&summary);
            if report.signatures.is_empty() {
                self.content.append(&page::paragraph("No witness signatures are in the evidence that the search reached.", &["secondary"]));
            } else {
                let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
                for (index, signature) in report.signatures.iter().enumerate() {
                    if index > 0 {
                        list.append(&page::divider());
                    }
                    list.append(&signature_group(&ctx, &page, signature));
                }
                self.content.append(&list);
            }
            if !report.finalizations.is_empty() {
                let heading = display::name_text("Observed finalization fields");
                heading.set_margin_top(4);
                self.content.append(&heading);
                for line in &report.finalizations {
                    let text = page::text(line, &["callout", "mono", "secondary"]);
                    text.set_selectable(true);
                    text.set_wrap(true);
                    self.content.append(&text);
                }
            }
            if !report.issues.is_empty() {
                let heading = theme::label(font::SMALL, "Investigation incomplete", "mined");
                heading.set_margin_top(4);
                self.content.append(&heading);
                for issue in &report.issues {
                    let note = page::note(issue, true);
                    note.add_css_class("callout");
                    self.content.append(&note);
                }
            }
            self.content.append(&page::paragraph(
                "These groups are not a quorum count. The investigation covers reachable evidence. It does not find every witness broadcast, and it does not verify signatures, proofs, quorum membership or consensus.",
                &["caption-text", "secondary"],
            ));
            let disclosure = lamp::Disclosure::new("Decoded report", false, |_| {});
            disclosure.content.set_margin_top(6);
            disclosure.content.append(&json_text(&report.json));
            self.content.append(&disclosure.root);
            // The disclosure lives as long as the card shows it.
            *self.disclosure.borrow_mut() = Some(disclosure);
        } else if self.running.get() {
            self.content.append(&small_progress("Searching the evidence…"));
        } else {
            if let Some(failure) = self.failure.borrow().as_ref() {
                self.content.append(&page::note(failure, true));
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let text = page::text("Find the identities that signed for or against this notarization.", &["secondary"]);
            text.set_hexpand(true);
            row.append(&text);
            let weak = Rc::downgrade(self);
            row.append(&lamp::button("Find signers", move || {
                if let Some(card) = weak.upgrade() {
                    card.run(false);
                }
            }));
            self.content.append(&row);
        }
    }
}

/// JSON as monospaced text, on the black of the display.
pub fn json_text(json: &str) -> gtk::Box {
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("tx-json");
    let pretty = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| json.to_string());
    let text = page::text(&pretty, &["mono"]);
    text.set_selectable(true);
    text.set_wrap(true);
    text.set_wrap_mode(gtk::pango::WrapMode::Char);
    frame.append(&text);
    frame
}

/// One identity's signatures over one target, with the key signatures and the outputs that
/// hold the evidence.
fn signature_group(ctx: &Rc<Ctx>, page: &Page, signature: &api::WitnessSignature) -> gtk::Box {
    let chain = ctx.chain_id();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.set_margin_top(7);
    column.set_margin_bottom(7);
    let first = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let verdict = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    verdict.set_size_request(110, -1);
    let color = if signature.confirms { tone::GOOD } else { tone::ALERT };
    verdict.append(&sealed(signature.confirms, color));
    verdict.append(&page::text(
        if signature.confirms { "Confirming" } else { "Rejecting" },
        &[if signature.confirms { "good" } else { "alert" }],
    ));
    first.append(&verdict);
    first.append(&page::route_link(page, &signature.identity, Route::Identity { name: signature.identity.clone() }, true));
    first.append(&page::text("on", &["secondary"]));
    first.append(&page::route_link(page, &signature.system, Route::Currency { name: signature.system.clone() }, true));
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    first.append(&spacer);
    first.append(&page::text(
        &signature.height.map(|h| format!("signing height {}", format::integer(h))).unwrap_or_else(|| "signing height unknown".into()),
        &["secondary", "tabular"],
    ));
    let details = gtk::Box::new(gtk::Orientation::Vertical, 4);
    details.set_margin_start(122);
    details.set_visible(false);
    let keys = signature.key_count;
    let toggle = {
        let details = details.downgrade();
        link::link(&format!("{keys} distinct key signature{}", if keys == 1 { "" } else { "s" }), false, move || {
            if let Some(details) = details.upgrade() {
                details.set_visible(!details.is_visible());
            }
        })
    };
    first.append(&toggle);
    column.append(&first);

    let second = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = display::name_text("Signed target");
    name.set_valign(gtk::Align::Center);
    second.append(&name);
    second.append(&transaction_target(page, chain.as_deref(), &signature.target_system, &signature.target_txid));
    second.append(&callout(&format!("output {} on", signature.target_output), true));
    second.append(&page::route_link(page, &signature.target_system, Route::Currency { name: signature.target_system.clone() }, true));
    column.append(&second);

    for key in &signature.key_signatures {
        let text = page::text(key, &["caption-text", "mono"]);
        text.set_selectable(true);
        text.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        details.append(&text);
    }
    for origin in &signature.origins {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let name = display::name_text("Evidence");
        name.set_valign(gtk::Align::Center);
        row.append(&name);
        if chain.as_deref() == Some(origin.output.system_id.as_str()) {
            row.append(&page::route_link(page, &origin.output.txid, Route::Transaction { txid: origin.output.txid.clone() }, true));
        } else {
            row.append(&page::text(&format::short(&origin.output.txid, 10, 8), &["mono"]));
        }
        row.append(&callout(&format!("output {} · {}", origin.output.output, origin.source), true));
        details.append(&row);
    }
    column.append(&details);
    column
}
