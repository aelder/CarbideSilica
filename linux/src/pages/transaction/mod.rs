//! The Transaction page (`TransactionView.swift`, with `TransactionStatement.swift`,
//! `TransactionExtras.swift`, `TransferTrace.swift` and `TraceRoute.swift`): the overview
//! as Carbide's terminal shows it (status, value accounting and fee), the statement of
//! where the coins went, the investigations, then each input and output with its
//! annotations, the blocks that held it, and the node's JSON.
//!
//! Names and dates follow the app's display settings.

mod bytes;
mod extras;
mod flow;
mod flow_scene;
pub mod layout;
mod lists;
mod overview;
mod statement;
mod style;
mod trace;
mod trace_route;
pub mod words;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Route};
use crate::model::AppModel;
use crate::page::{self, Adjacent, Investigations, LoadView, Page};
use crate::shared::{inclusion, json_tree};
use crate::theme::{self, draw, font, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;
use crate::widgets::lamp;

use layout::Selection;

/// What the parts of one loaded transaction share: the marked row of the statement, the
/// places that the page scrolls to, and the cards that the investigation keys start.
pub(crate) struct Ctx {
    /// The page holds the parts, and the parts hold this: a weak link, so that the page
    /// goes when it leaves the path.
    page: std::rc::Weak<Page>,
    pub model: Rc<AppModel>,
    pub tx: Rc<api::TransactionDetail>,
    /// The row of the statement whose inputs and outputs are marked in the lists.
    marked: RefCell<Option<Selection>>,
    mark_watchers: RefCell<Vec<Box<dyn Fn(Option<&Selection>)>>>,
    /// Widgets the page scrolls to, by their ID: `input-3`, `output-0`, `trace-1`, `links`.
    anchors: RefCell<HashMap<String, glib::WeakRef<gtk::Widget>>>,
    /// Shows all rows of the inputs or the outputs, when the list is long.
    pub show_all: RefCell<Vec<Box<dyn Fn(&str)>>>,
    pub traces: RefCell<BTreeMap<u32, Rc<trace::TraceCard>>>,
    pub witnesses: RefCell<BTreeMap<u32, Rc<extras::WitnessCard>>>,
    /// Carbide's verified registration payment annotations, by output.
    pub payments: RefCell<HashMap<u32, String>>,
    /// Builds the rows of the outputs again, when the payment annotations arrive.
    pub refresh_outputs: RefCell<Option<Box<dyn Fn()>>>,
    /// The parts that have state of their own. They live as long as the transaction is
    /// shown.
    kept: RefCell<Vec<Rc<dyn std::any::Any>>>,
}

impl Ctx {
    pub fn marked(&self) -> Option<Selection> {
        self.marked.borrow().clone()
    }

    pub fn set_marked(&self, selection: Option<Selection>) {
        if *self.marked.borrow() == selection {
            return;
        }
        *self.marked.borrow_mut() = selection.clone();
        for watcher in self.mark_watchers.borrow().iter() {
            watcher(selection.as_ref());
        }
    }

    /// Calls `watcher` after each change of the mark.
    pub fn on_marked(&self, watcher: impl Fn(Option<&Selection>) + 'static) {
        self.mark_watchers.borrow_mut().push(Box::new(watcher));
    }

    /// Keeps a part with state of its own for as long as the transaction is shown.
    pub fn keep(&self, part: Rc<dyn std::any::Any>) {
        self.kept.borrow_mut().push(part);
    }

    pub fn anchor(&self, id: &str, widget: &impl IsA<gtk::Widget>) {
        self.anchors.borrow_mut().insert(id.to_string(), widget.upcast_ref::<gtk::Widget>().downgrade());
    }

    /// Scrolls the page so that the row or card with this ID is at the top. A row of a long
    /// list can be out of the first rows: the list then shows all first.
    pub fn scroll(self: &Rc<Self>, target: &str) {
        for show in self.show_all.borrow().iter() {
            show(target);
        }
        let weak = Rc::downgrade(self);
        let target = target.to_string();
        glib::timeout_add_local_once(Duration::from_millis(60), move || {
            let Some(ctx) = weak.upgrade() else { return };
            let widget = ctx.anchors.borrow().get(&target).and_then(|w| w.upgrade());
            let Some(page) = ctx.page() else { return };
            if let (Some(widget), Some(scroller)) = (widget, page.scroller.as_ref())
                && let Some(bounds) = widget.compute_bounds(&page.content)
            {
                // The column has a margin of 24 above the first part.
                scroller.vadjustment().set_value((bounds.y() as f64 + 24.0 - 8.0).max(0.0));
            }
        });
    }

    /// Starts an investigation card and scrolls to it.
    pub fn start(self: &Rc<Self>, card: &str) {
        if let Some(output) = card.strip_prefix("trace-").and_then(|o| o.parse::<u32>().ok()) {
            let found = self.traces.borrow().get(&output).cloned();
            if let Some(found) = found {
                found.start();
            }
        } else if let Some(output) = card.strip_prefix("witness-").and_then(|o| o.parse::<u32>().ok()) {
            let found = self.witnesses.borrow().get(&output).cloned();
            if let Some(found) = found {
                found.start();
            }
        }
        self.scroll(card);
    }

    /// The output that the cursor of the page is in.
    fn cursor_output(&self) -> Option<u32> {
        let anchors = self.anchors.borrow();
        anchors.iter().find_map(|(id, widget)| {
            let output = id.strip_prefix("output-")?.parse::<u32>().ok()?;
            let widget = widget.upgrade()?;
            has_cursor(&widget).then_some(output)
        })
    }

    pub fn page(&self) -> Option<Rc<Page>> {
        self.page.upgrade()
    }

    pub fn namer(&self) -> words::Namer {
        words::Namer::from_model(&self.model)
    }

    pub fn chain_id(&self) -> Option<String> {
        self.model.state().status.as_ref().and_then(|s| s.chain_id.clone())
    }
}

/// True when the widget or a widget in it is the stop under the page's cursor.
fn has_cursor(widget: &gtk::Widget) -> bool {
    if widget.has_css_class("cursor-on") {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if has_cursor(&c) {
            return true;
        }
        child = c.next_sibling();
    }
    false
}

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>, txid: &str) -> Rc<Page> {
    style::install();
    let page = Page::scrolling(model);
    let txid = txid.to_string();
    let page_weak = Rc::downgrade(&page);
    let model_weak = Rc::downgrade(model);
    let load = LoadView::new(
        model,
        {
            let txid = txid.clone();
            move |explorer| {
                let txid = txid.clone();
                async move { explorer.transaction(txid).await }
            }
        },
        move |holder, transaction| {
            let (Some(page), Some(model)) = (page_weak.upgrade(), model_weak.upgrade()) else { return };
            show(&page, &model, holder, transaction);
        },
    );
    page.append(&load.root);
    // The page keeps the loader through its key for a refresh.
    page.hooks().refresh = Some(Rc::new(move || load.reload()));
    page
}

/// Fills the page with a loaded transaction.
fn show(page: &Rc<Page>, model: &Rc<AppModel>, holder: &gtk::Box, transaction: api::TransactionDetail) {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 20);
    holder.append(&column);
    let tx = Rc::new(transaction);
    let ctx = Rc::new(Ctx {
        page: Rc::downgrade(page),
        model: model.clone(),
        tx: tx.clone(),
        marked: RefCell::new(None),
        mark_watchers: Default::default(),
        anchors: Default::default(),
        show_all: Default::default(),
        traces: Default::default(),
        witnesses: Default::default(),
        payments: Default::default(),
        refresh_outputs: Default::default(),
        kept: Default::default(),
    });
    // The column keeps the parts of the transaction while it shows them. Each closure in
    // the parts holds a weak link to them.
    {
        let owned = ctx.clone();
        column.connect_destroy(move |_| {
            let _ = &owned;
        });
    }

    // The heading.
    let tags: Vec<(String, display::PlateFill)> =
        tx.tags.iter().map(|t| (t.clone(), display::signal_tag_fill(t))).collect();
    column.append(&page::page_header(&crate::format::short(&tx.txid, 16, 16), Some(&tx.txid), &tags));
    column.append(&page::provenance_note(&tx.provenance));
    if let Some(height) = tx.height {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let key = charts_key(model, height);
        key.set_halign(gtk::Align::End);
        key.set_hexpand(true);
        row.append(&key);
        column.append(&row);
    }

    column.append(&overview::card(&ctx));

    let statement = page.card("Statement", None);
    let view = statement::StatementView::new(&ctx);
    statement.append(&view.root);
    ctx.keep(view);
    column.append(&statement.root);

    for root in &tx.trace_roots {
        let card = trace::TraceCard::new(&ctx, root.clone());
        ctx.anchor(&format!("trace-{}", root.output), &card.root);
        column.append(&card.root);
        ctx.traces.borrow_mut().insert(root.output, card);
    }
    for root in &tx.witness_roots {
        let card = extras::WitnessCard::new(&ctx, root.clone());
        ctx.anchor(&format!("witness-{}", root.output), &card.root);
        column.append(&card.root);
        ctx.witnesses.borrow_mut().insert(root.output, card);
    }
    let links = extras::links_card(&ctx);
    ctx.anchor("links", &links);
    column.append(&links);

    if tx.tags.iter().any(|t| t == "Identity Registration") {
        // `RegistrationPaymentsCard` is a part of the Identity page (RegistrationPayments.swift).
        // It has no shared form yet; the card says where the payments are.
        let card = page.card("Registration payments", None);
        card.append(&page::paragraph(
            "The identity page shows the registration payments of this transaction. The shared card is not ported yet.",
            &["secondary"],
        ));
        column.append(&card.root);
    }

    column.append(&lists::inputs_card(&ctx));
    column.append(&lists::outputs_card(&ctx));

    if tx.height.is_some() {
        let card = inclusion::inclusion_card(page, &tx.txid);
        ctx.anchor("inclusions", &card.root);
        column.append(&card.root);
    }

    // `JSONSection` registers Tab with the page.
    column.append(&json_tree::json_section(page, &tx.json, &format!("tx-{}.json", tx.txid)));

    install_hooks(&ctx);
    consume_pending_trace(&ctx);
    model.resolve_names(ids(&tx));
    load_siblings(&ctx);
    load_payments(&ctx);
}

/// The key "Open in Charts", with the mark of a chart.
fn charts_key(model: &Rc<AppModel>, height: u64) -> gtk::Button {
    let weak = Rc::downgrade(model);
    let key = lamp::button("Open in Charts", move || {
        if let Some(model) = weak.upgrade() {
            model.show_charts(Some(height), None);
        }
    });
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    let mark = Canvas::new(13, 11, |_, snapshot, w, h| {
        // `chart.xyaxis.line`: two axes and a line that rises.
        let color = tone::VALUE.opacity(0.85);
        draw::rule(snapshot, 0.5, 0.0, 0.5, h - 0.5, color, 1.0, &[]);
        draw::rule(snapshot, 0.5, h - 0.5, w, h - 0.5, color, 1.0, &[]);
        let line = gtk::gsk::PathBuilder::new();
        line.move_to(2.5, h - 3.0);
        line.line_to(w * 0.42, h * 0.45);
        line.line_to(w * 0.62, h * 0.62);
        line.line_to(w - 0.5, 1.5);
        draw::stroke(snapshot, &line.to_path(), color, 1.0);
    });
    mark.set_valign(gtk::Align::Center);
    mark.set_can_target(false);
    row.append(&mark);
    row.append(&theme::label(font::KEY, "Open in Charts", ""));
    key.set_child(Some(&row));
    key.set_tooltip_text(Some("Open Charts at the block that holds this transaction."));
    key
}

/// The keys of the page: b for the block, the investigations (F8, F4, F3, F7), and n and p
/// once the block's transactions are known.
fn install_hooks(ctx: &Rc<Ctx>) {
    let tx = &ctx.tx;
    let Some(page) = ctx.page() else { return };
    let mut hooks = page.hooks();
    hooks.keys.clear();
    if let Some(hash) = tx.block_hash.clone() {
        let model = Rc::downgrade(&ctx.model);
        hooks.keys.insert(
            "b".into(),
            (
                "Open the Block of this Transaction".into(),
                Rc::new(move || {
                    if let Some(model) = model.upgrade() {
                        model.open(Route::Block { id: hash.clone() });
                    }
                }),
            ),
        );
    }
    let weak = Rc::downgrade(ctx);
    let follow: Option<page::Action> = tx.trace_roots.first().map(|first| {
        let first = first.output;
        let weak = weak.clone();
        Rc::new(move || {
            // The output under the cursor, when it has the function. If not, the first that has.
            let Some(ctx) = weak.upgrade() else { return };
            let chosen = ctx.cursor_output();
            let output = ctx.tx.trace_roots.iter().find(|r| Some(r.output) == chosen).map(|r| r.output).unwrap_or(first);
            ctx.start(&format!("trace-{output}"));
        }) as page::Action
    });
    let witnesses: Option<page::Action> = tx.witness_roots.first().map(|first| {
        let first = first.output;
        let weak = weak.clone();
        Rc::new(move || {
            let Some(ctx) = weak.upgrade() else { return };
            let chosen = ctx.cursor_output();
            let output = ctx.tx.witness_roots.iter().find(|r| Some(r.output) == chosen).map(|r| r.output).unwrap_or(first);
            ctx.start(&format!("witness-{output}"));
        }) as page::Action
    });
    let links: page::Action = {
        let weak = weak.clone();
        Rc::new(move || {
            if let Some(ctx) = weak.upgrade() {
                ctx.scroll("links");
            }
        })
    };
    let inclusions: Option<page::Action> = tx.height.map(|_| {
        let weak = weak.clone();
        Rc::new(move || {
            if let Some(ctx) = weak.upgrade() {
                ctx.scroll("inclusions");
            }
        }) as page::Action
    });
    hooks.investigations = Investigations {
        follow_transfer: follow,
        witnesses,
        protocol_links: Some(links),
        inclusions,
        shared_authorities: None,
    };
    hooks.adjacent = Some(Adjacent { noun: "Transaction in Block".into(), previous: None, next: None });
}

/// The transactions of the block that holds this one, in order: n and p go to the
/// transaction after and before this one in its block, in place.
fn load_siblings(ctx: &Rc<Ctx>) {
    let Some(hash) = ctx.tx.block_hash.clone() else { return };
    let Some(explorer) = ctx.model.explorer() else { return };
    let generation = ctx.model.generation();
    let weak = Rc::downgrade(ctx);
    glib::spawn_future_local(async move {
        let found = bridge::run(async move { explorer.block(hash).await }).await;
        let Some(ctx) = weak.upgrade() else { return };
        if ctx.model.generation() != generation {
            return;
        }
        let siblings: Vec<String> = found.map(|b| b.transactions.into_iter().map(|t| t.txid).collect()).unwrap_or_default();
        let Some(index) = siblings.iter().position(|t| *t == ctx.tx.txid) else { return };
        let step = |offset: isize| -> Option<page::Action> {
            let at = index as isize + offset;
            let txid = siblings.get(usize::try_from(at).ok()?)?.clone();
            let model = Rc::downgrade(&ctx.model);
            Some(Rc::new(move || {
                if let Some(model) = model.upgrade() {
                    model.step(Route::Transaction { txid: txid.clone() });
                }
            }))
        };
        let Some(page) = ctx.page() else { return };
        page.hooks().adjacent =
            Some(Adjacent { noun: "Transaction in Block".into(), previous: step(-1), next: step(1) });
    });
}

/// Starts the trace that another page asked for, once, when it names this transaction.
fn consume_pending_trace(ctx: &Rc<Ctx>) {
    let pending = ctx.model.state().pending_trace.as_ref().is_some_and(|r| r.txid == ctx.tx.txid);
    if !pending {
        return;
    }
    let Some(request) = ctx.model.take_pending_trace() else { return };
    if ctx.tx.trace_roots.iter().any(|r| r.output == request.output) {
        let weak = Rc::downgrade(ctx);
        // The card starts once it is on screen and laid out.
        glib::idle_add_local_once(move || {
            if let Some(ctx) = weak.upgrade() {
                ctx.start(&format!("trace-{}", request.output));
            }
        });
    }
}

/// Carbide's registration payment annotations. Checking and unverified states add nothing.
fn load_payments(ctx: &Rc<Ctx>) {
    let json = &ctx.tx.json;
    if !(json.contains("\"identityreservation\"") || json.contains("\"identityadvancedreservation\"")) {
        return;
    }
    let Some(explorer) = ctx.model.explorer() else { return };
    let txid = ctx.tx.txid.clone();
    let generation = ctx.model.generation();
    let weak = Rc::downgrade(ctx);
    glib::spawn_future_local(async move {
        let found = bridge::run(async move { explorer.registration_payments(txid).await }).await;
        let Some(ctx) = weak.upgrade() else { return };
        if ctx.model.generation() != generation {
            return;
        }
        let Ok(payments) = found else { return };
        if payments.status != api::PaymentsStatus::Verified {
            return;
        }
        let mut annotations = HashMap::new();
        for row in payments.referrals.iter().chain(payments.charge.iter()) {
            annotations.insert(row.output, row.annotation.clone());
        }
        *ctx.payments.borrow_mut() = annotations;
        if let Some(refresh) = ctx.refresh_outputs.borrow().as_ref() {
            refresh();
        }
    });
}

/// Every identity and currency ID the overview can name.
pub fn ids(tx: &api::TransactionDetail) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for input in &tx.inputs {
        ids.extend(input.addresses.iter().cloned());
        ids.extend(input.reserve_amounts.iter().map(|a| a.currency.clone()));
    }
    for output in &tx.outputs {
        ids.extend(output.addresses.iter().cloned());
        ids.extend(output.reserve_amounts.iter().map(|a| a.currency.clone()));
        ids.extend(output.tag_details.iter().filter_map(|t| t.currency_id.clone()));
        for reference in &output.references {
            match &reference.target {
                api::ReferenceTarget::Currency { id } => ids.push(id.clone()),
                api::ReferenceTarget::Destination { address, system_id } => {
                    ids.push(address.clone());
                    ids.extend(system_id.iter().cloned());
                }
                api::ReferenceTarget::Output { outpoint } => ids.push(outpoint.system_id.clone()),
                _ => {}
            }
        }
    }
    let statement = &tx.statement;
    ids.extend(statement.columns.iter().flatten().cloned());
    ids.extend(statement.parties.iter().filter_map(|p| p.currency_id.clone()));
    ids.extend(statement.marks.iter().filter_map(|m| m.currency_id.clone()));
    for route in &statement.routes {
        ids.extend([&route.via, &route.to_currency, &route.destination].into_iter().flatten().cloned());
    }
    ids.extend(tx.accounting.inputs.reserves.iter().map(|r| r.currency.clone()));
    ids.extend(tx.accounting.outputs.reserves.iter().map(|r| r.currency.clone()));
    ids
}
