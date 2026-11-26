//! Follows one transfer from its source output as far as the evidence reaches, using
//! Carbide's DeFi trail (the terminal's F8): `TransferTraceCard`, `LegDetail`, `RateLines`
//! and `PoolComparisonView` in `TransferTrace.swift`. The card shows one verdict, then a
//! receipt, the stages on an axis of blocks, and the place of the transfer in its export
//! batch.
//!
//! While the search runs, the card shows the trail so far. For a cross-system import, the
//! search can jump to a destination height, step through adjacent 1,000-block windows, or
//! scan up to 32 older windows.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Confidence, api::Route};
use crate::format;
use crate::page::{self, load::clear, Page};
use crate::shared::confidence::confidence_mark;
use crate::theme::{self, font, tone};
use crate::widgets::display;
use crate::widgets::{lamp, link, loader};

use super::extras::{arrow, foreign_id, transaction_target};
use super::overview::{callout, reserve_amount};
use super::trace_route;
use super::Ctx;

pub struct TraceCard {
    pub root: gtk::Box,
    ctx: Weak<Ctx>,
    trace_root: api::TraceRoot,
    content: gtk::Box,
    trace: RefCell<Option<api::TransferTrace>>,
    progress: RefCell<Option<api::TransferTrace>>,
    failure: RefCell<Option<String>>,
    running: Cell<bool>,
    show_evidence: Cell<bool>,
    /// True once the trail has been shown: the evidence opens by itself only then.
    shown_once: Cell<bool>,
    jump_height: RefCell<String>,
    legs: RefCell<Vec<Rc<LegDetail>>>,
    disclosure: RefCell<Option<Rc<lamp::Disclosure>>>,
}

impl TraceCard {
    pub fn new(ctx: &Rc<Ctx>, root: api::TraceRoot) -> Rc<Self> {
        let page = ctx.page().expect("the page builds its cards");
        let card = page.card("Transfer trace", Some(&format!("{} · output {}", root.label, root.output)));
        let trace = Rc::new(Self {
            root: card.root.clone(),
            ctx: Rc::downgrade(ctx),
            trace_root: root,
            content: card.content.clone(),
            trace: RefCell::new(None),
            progress: RefCell::new(None),
            failure: RefCell::new(None),
            running: Cell::new(false),
            show_evidence: Cell::new(false),
            shown_once: Cell::new(false),
            jump_height: RefCell::new(String::new()),
            legs: RefCell::new(Vec::new()),
            disclosure: RefCell::new(None),
        });
        trace.fill();
        trace
    }

    /// Starts the trace from a menu command or another page, once.
    pub fn start(self: &Rc<Self>) {
        if self.trace.borrow().is_none() && !self.running.get() {
            self.run(api::TraceRequest::Initial, None, None);
        }
    }

    fn prompt(&self) -> &'static str {
        match self.trace_root.label.as_str() {
            "Fee pool checkpoint" => "Compare this fee pool checkpoint with the adjacent coinbase checkpoints.",
            "Currency import" => "Open this import's source export and the outputs it generated.",
            _ => "Trace this transfer through its export, its import and its result.",
        }
    }

    fn fill(self: &Rc<Self>) {
        clear(&self.content);
        let shown = self.progress.borrow().clone().map(|p| (p, true)).or_else(|| self.trace.borrow().clone().map(|t| (t, false)));
        if let Some((trace, live)) = shown {
            self.content(&trace, live);
        } else if self.running.get() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.append(&loader::small());
            row.append(&page::text("Tracing the transfer…", &["secondary"]));
            self.content.append(&row);
        }
        if let Some(failure) = self.failure.borrow().as_ref() {
            self.content.append(&page::note(failure, true));
        }
        if self.trace.borrow().is_none() && !self.running.get() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let text = page::text(self.prompt(), &["secondary"]);
            text.set_hexpand(true);
            text.set_wrap(true);
            row.append(&text);
            let weak = Rc::downgrade(self);
            row.append(&lamp::button("Trace", move || {
                if let Some(card) = weak.upgrade() {
                    card.run(api::TraceRequest::Initial, None, None);
                }
            }));
            self.content.append(&row);
        }
    }

    fn content(self: &Rc<Self>, trace: &api::TransferTrace, live: bool) {
        let Some(ctx) = self.ctx.upgrade() else { return };
        let Some(page) = ctx.page() else { return };
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        if live {
            head.append(&loader::small());
            head.append(&page::text("Searching. This is the trail so far.", &["secondary"]));
        } else {
            let mark = confidence_mark(trace.confidence, None);
            mark.set_valign(gtk::Align::Center);
            head.append(&mark);
            let verdict = page::paragraph(&trace.verdict, &["tx-verdict"]);
            verdict.set_hexpand(true);
            head.append(&verdict);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        head.append(&spacer);
        if !live {
            let weak = Rc::downgrade(self);
            let again = lamp::button("Trace again", move || {
                if let Some(card) = weak.upgrade() {
                    card.run(api::TraceRequest::Initial, None, None);
                }
            });
            again.set_sensitive(!self.running.get());
            head.append(&again);
        }
        self.content.append(&head);

        let namer = ctx.namer();
        if !trace.legs.is_empty() {
            self.content.append(&trace_route::route(trace, live, &namer));
        }

        // Each leg keeps its state (the currencies shown, the comparison) while the trail
        // grows.
        {
            let mut legs = self.legs.borrow_mut();
            while legs.len() < trace.legs.len() {
                let index = legs.len() as u32;
                legs.push(LegDetail::new(&ctx, self.trace_root.output, index));
            }
            legs.truncate(trace.legs.len());
            for (index, leg) in trace.legs.iter().enumerate() {
                let detail = &legs[index];
                detail.set(leg.clone(), (trace.legs.len() > 1).then_some(index + 1), live);
                self.content.append(&detail.holder);
            }
        }

        if let Some(stop) = &trace.stop {
            self.content.append(&page::note(stop, true));
        }
        if let Some(search) = &trace.search {
            self.content.append(&self.search_controls(search, trace, live, &namer));
        }

        // The evidence is open at the first view of a trail with no legs.
        if !self.shown_once.get() {
            self.shown_once.set(true);
            if trace.legs.is_empty() {
                self.show_evidence.set(true);
            }
        }
        let count = trace.steps.len();
        let weak = Rc::downgrade(self);
        let disclosure = lamp::Disclosure::new(
            &format!("Evidence ({count} step{})", if count == 1 { "" } else { "s" }),
            self.show_evidence.get(),
            move |open| {
                if let Some(card) = weak.upgrade() {
                    card.show_evidence.set(open);
                }
            },
        );
        let evidence = gtk::Box::new(gtk::Orientation::Vertical, 10);
        evidence.set_margin_top(8);
        let chain = ctx.chain_id();
        for step in &trace.steps {
            let block = gtk::Box::new(gtk::Orientation::Vertical, 3);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.append(&page::text(&step.label, &["semibold"]));
            line.append(&transaction_target(&page, chain.as_deref(), &step.system_id, &step.txid));
            line.append(&page::text(&format!("output {} on", step.output), &["secondary"]));
            line.append(&page::route_link(&page, &step.system_id, Route::Currency { name: step.system_id.clone() }, true));
            block.append(&line);
            let detail = page::paragraph(&step.detail, &["caption-text", "mono", "secondary"]);
            detail.set_selectable(true);
            block.append(&detail);
            evidence.append(&block);
        }
        for note in &trace.notes {
            evidence.append(&page::paragraph(note, &["caption-text", "secondary"]));
        }
        disclosure.content.append(&evidence);
        self.content.append(&disclosure.root);
        *self.disclosure.borrow_mut() = Some(disclosure);
    }

    /// The searched range, and the controls to search other windows of the destination chain.
    fn search_controls(self: &Rc<Self>, search: &api::TraceSearch, trace: &api::TransferTrace, live: bool, namer: &super::words::Namer) -> gtk::Box {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let windows = search.windows;
        column.append(&page::paragraph(
            &format!(
                "Searched heights {} to {} of {} in {windows} window{}. The tip is {}.",
                format::integer(search.first),
                format::integer(search.last),
                namer.short_name(&search.system_id),
                if windows == 1 { "" } else { "s" },
                format::integer(search.tip)
            ),
            &["callout"],
        ));
        if trace.confidence != Confidence::Verified {
            let note = page::note("A missing import in the searched windows is not proof that none exists.", false);
            note.add_css_class("callout");
            note.add_css_class("secondary");
            column.append(&note);
        }
        if !live && trace.confidence != Confidence::Verified {
            let running = self.running.get();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let target = trace.search_root.clone();
            let previous = trace.clone();
            let window = |title: &str, request: api::TraceRequest, enabled: bool, help: &str| {
                let weak = Rc::downgrade(self);
                let (target, previous) = (target.clone(), previous.clone());
                let key = lamp::button(title, move || {
                    if let Some(card) = weak.upgrade() {
                        card.run(request.clone(), target.clone(), Some(previous.clone()));
                    }
                });
                key.set_sensitive(enabled);
                key.set_tooltip_text(Some(help));
                key
            };
            row.append(&window(
                "Earlier window",
                api::TraceRequest::WindowEndingAt { height: search.first.saturating_sub(1) },
                !running && search.first != 0,
                "Search the 1,000 blocks before this range.",
            ));
            row.append(&window(
                "Later window",
                api::TraceRequest::WindowEndingAt { height: (search.last + 1000).min(search.tip) },
                !running && search.last < search.tip,
                "Search the 1,000 blocks after this range.",
            ));
            if search.more_older {
                row.append(&window(
                    "Scan older windows",
                    api::TraceRequest::BackwardFrom { height: search.first },
                    !running,
                    "Search up to 32 older 1,000-block windows. Select it again to continue.",
                ));
            }
            let divider = display::vertical_rule();
            divider.set_size_request(1, 18);
            divider.set_vexpand(false);
            divider.set_valign(gtk::Align::Center);
            row.append(&divider);
            let field = gtk::Entry::new();
            field.add_css_class("display-field");
            field.set_placeholder_text(Some("Destination height"));
            field.set_text(&self.jump_height.borrow());
            field.set_width_chars(16);
            field.set_size_request(150, -1);
            let go = {
                let weak = Rc::downgrade(self);
                let (target, previous) = (target.clone(), previous.clone());
                Rc::new(move || {
                    if let Some(card) = weak.upgrade()
                        && let Some(height) = jump_value(&card.jump_height.borrow())
                    {
                        card.run(api::TraceRequest::WindowEndingAt { height }, target.clone(), Some(previous.clone()));
                    }
                })
            };
            let run = go.clone();
            let go_key = lamp::button("Go", move || run());
            go_key.set_sensitive(!running && jump_value(&self.jump_height.borrow()).is_some());
            go_key.set_tooltip_text(Some("Search the 1,000-block window that ends at this height."));
            {
                let (weak, key) = (Rc::downgrade(self), go_key.downgrade());
                field.connect_changed(move |field| {
                    let Some(card) = weak.upgrade() else { return };
                    *card.jump_height.borrow_mut() = field.text().to_string();
                    if let Some(key) = key.upgrade() {
                        key.set_sensitive(!card.running.get() && jump_value(&field.text()).is_some());
                    }
                });
            }
            let run = go.clone();
            field.connect_activate(move |_| run());
            row.append(&field);
            row.append(&go_key);
            column.append(&row);
        }
        column
    }

    /// Runs the trace. A search of one leg keeps the other legs' import heights.
    fn run(self: &Rc<Self>, request: api::TraceRequest, target: Option<api::Outpoint>, previous: Option<api::TransferTrace>) {
        if self.running.get() {
            return;
        }
        let Some(ctx) = self.ctx.upgrade() else { return };
        self.running.set(true);
        *self.failure.borrow_mut() = None;
        *self.progress.borrow_mut() = None;
        let explorer = match ctx.model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                self.running.set(false);
                *self.failure.borrow_mut() = Some(message);
                self.fill();
                return;
            }
        };
        self.fill();
        let token: u64 = rand::random_range(1..=u64::MAX);
        // The trail so far, read every 400 milliseconds while the search runs.
        {
            let (weak, explorer) = (Rc::downgrade(self), explorer.clone());
            let poll = move || {
                let Some(card) = weak.upgrade() else { return glib::ControlFlow::Break };
                if !card.running.get() {
                    return glib::ControlFlow::Break;
                }
                if let Some(partial) = explorer.trace_progress(token) {
                    *card.progress.borrow_mut() = Some(partial);
                    card.fill();
                }
                glib::ControlFlow::Continue
            };
            if poll() == glib::ControlFlow::Continue {
                glib::timeout_add_local(Duration::from_millis(400), poll);
            }
        }
        let plan = api::TracePlan {
            request,
            target,
            known_imports: previous.map(|p| p.import_heights).unwrap_or_default(),
        };
        let (txid, output) = (ctx.tx.txid.clone(), self.trace_root.output);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.trace_transfer(txid, output, plan, token).await }).await;
            let Some(card) = weak.upgrade() else { return };
            card.running.set(false);
            *card.progress.borrow_mut() = None;
            match result {
                Ok(trace) => {
                    if let Some(ctx) = card.ctx.upgrade() {
                        let mut ids: Vec<String> = trace.legs.iter().flat_map(|l| l.currencies.iter().cloned()).collect();
                        ids.extend(trace.steps.iter().map(|s| s.system_id.clone()));
                        ids.extend(trace.search.iter().map(|s| s.system_id.clone()));
                        ctx.model.resolve_names(ids);
                    }
                    *card.trace.borrow_mut() = Some(trace);
                }
                Err(error) => *card.failure.borrow_mut() = Some(bridge::describe(&error)),
            }
            card.fill();
        });
    }
}

/// The height in the field: its digits.
fn jump_value(text: &str) -> Option<u64> {
    text.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
}

/// What one leg of the transfer asked for and what it produced, with its fee budgets kept
/// apart. Budgets are never added across legs: the next leg spends the same funds.
struct LegDetail {
    holder: gtk::Box,
    ctx: Weak<Ctx>,
    output: u32,
    index: u32,
    leg: RefCell<Option<(api::TraceLeg, Option<usize>, bool)>>,
    comparison: RefCell<Option<api::PoolComparison>>,
    comparing: Cell<bool>,
    comparison_failure: RefCell<Option<String>>,
    show_currencies: Cell<bool>,
}

impl LegDetail {
    fn new(ctx: &Rc<Ctx>, output: u32, index: u32) -> Rc<Self> {
        let holder = gtk::Box::new(gtk::Orientation::Vertical, 10);
        holder.add_css_class("tx-frame");
        holder.set_hexpand(true);
        Rc::new(Self {
            holder,
            ctx: Rc::downgrade(ctx),
            output,
            index,
            leg: RefCell::new(None),
            comparison: RefCell::new(None),
            comparing: Cell::new(false),
            comparison_failure: RefCell::new(None),
            show_currencies: Cell::new(false),
        })
    }

    fn set(self: &Rc<Self>, leg: api::TraceLeg, number: Option<usize>, live: bool) {
        *self.leg.borrow_mut() = Some((leg, number, live));
        self.fill();
    }

    fn fill(self: &Rc<Self>) {
        clear(&self.holder);
        let (Some(ctx), Some((leg, number, live))) = (self.ctx.upgrade(), self.leg.borrow().clone()) else { return };
        let Some(page) = ctx.page() else { return };
        let chain = ctx.chain_id();
        let on_chain = |system: Option<&String>| system.is_some() && system == chain.as_ref();

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        if let Some(number) = number {
            let name = display::name_text(&format!("Leg {number}"));
            name.set_valign(gtk::Align::Center);
            head.append(&name);
        }
        head.append(&confidence_mark(leg.confidence, Some(&leg.stage)));
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        head.append(&spacer);
        if on_chain(Some(&leg.source_system)) && leg.source_txid != ctx.tx.txid {
            head.append(&page::route_link(&page, &leg.source_txid, Route::Transaction { txid: leg.source_txid.clone() }, true));
        }
        self.holder.append(&head);

        let columns = gtk::Box::new(gtk::Orientation::Horizontal, 28);
        columns.append(&amount_column(&page, "Sent", &leg.principal, !leg.principal_known));
        if let Some(pool) = &leg.pool {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
            column.append(&display::name_text("Through"));
            column.append(&page::route_link(&page, pool, Route::Currency { name: pool.clone() }, true));
            columns.append(&column);
        }
        if let Some(result) = &leg.result {
            columns.append(&amount_column(&page, result, &leg.result_amounts, false));
        } else if let Some(burned) = &leg.burned {
            columns.append(&amount_column(&page, "Burned", std::slice::from_ref(burned), false));
        } else {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
            column.append(&display::name_text(if leg.stage.contains("burn") { "Burned" } else { "Received" }));
            column.append(&page::text("Unresolved", &["secondary"]));
            columns.append(&column);
        }
        if let Some(height) = leg.import_height {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
            column.append(&display::name_text("Imported at"));
            if on_chain(leg.execution_system.as_ref()) {
                let model = Rc::downgrade(&ctx.model);
                let open = Rc::new(move || {
                    if let Some(model) = model.upgrade() {
                        model.open(Route::Block { id: height.to_string() });
                    }
                });
                let run = open.clone();
                let block = link::link(&format!("block {}", format::integer(height)), false, move || run());
                page.stop(&block, None, Some(height.to_string()), move || open());
                column.append(&block);
            } else {
                column.append(&page::text(&format!("height {}", format::integer(height)), &[]));
            }
            columns.append(&column);
        }
        self.holder.append(&columns);

        // Instruction principal, transfer custody and fees, each on its own line.
        let budgets = gtk::Box::new(gtk::Orientation::Vertical, 3);
        match &leg.transfer_fee_budget {
            Some(budget) => budgets.append(&amount_line(&page, "Fee budget", budget, None)),
            None => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let name = display::name_text("Fee budget");
                name.set_size_request(170, -1);
                row.append(&name);
                row.append(&callout("Unknown", true));
                budgets.append(&row);
            }
        }
        if let Some(fee) = &leg.conversion_fee {
            budgets.append(&amount_line(&page, "Conversion fee", fee, None));
        }
        if let Some(budget) = &leg.forwarding_budget {
            budgets.append(&amount_line(&page, "Forward budget", budget, None));
        }
        if let Some(fee) = &leg.forwarding_conversion_fee {
            budgets.append(&amount_line(&page, "Forward conversion fee", fee, None));
        }
        if let Some(budget) = &leg.next_fee_budget {
            budgets.append(&amount_line(&page, "Next fee budget", budget, Some("carried into the next instruction")));
        }
        self.holder.append(&budgets);

        if let Some(batch) = &leg.batch_burned {
            self.holder.append(&amount_line(&page, "Batch supply reduction", batch, None));
        }
        if !leg.proceeds.is_empty() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&callout("Currency proceeds", true));
            for amount in &leg.proceeds {
                row.append(&reserve_amount(&page, amount, ""));
            }
            if let Some(outpoint) = &leg.proceeds_output {
                row.append(&callout(&format!("at output {} (currency identity)", outpoint.output), true));
            }
            self.holder.append(&row);
        }
        if let (Some(txid), Some(output)) = (&leg.result_txid, leg.result_output) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.append(&callout(&format!("{} output", leg.result.as_deref().unwrap_or("Result")), true));
            if on_chain(leg.result_system.as_ref()) {
                row.append(&page::route_link(&page, txid, Route::Transaction { txid: txid.clone() }, true));
            } else {
                row.append(&foreign_id(txid));
            }
            row.append(&callout(&format!("output {output}"), true));
            if let Some(system) = &leg.result_system {
                row.append(&callout("on", true));
                row.append(&page::route_link(&page, system, Route::Currency { name: system.clone() }, true));
            }
            self.holder.append(&row);
        }

        let namer = ctx.namer();
        for rate in &leg.rates {
            self.holder.append(&rate_lines(&page, rate, &namer));
        }

        let keys = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        let weak = Rc::downgrade(self);
        let currencies = link::link(
            &if self.show_currencies.get() { "Hide currencies".to_string() } else { format!("Currencies ({})", leg.currencies.len()) },
            false,
            move || {
                if let Some(detail) = weak.upgrade() {
                    detail.show_currencies.set(!detail.show_currencies.get());
                    detail.fill_later();
                }
            },
        );
        keys.append(&small(currencies));
        if let Some(pool) = &leg.pool {
            let (model, pool_id, height, executed_here) = (Rc::downgrade(&ctx.model), pool.clone(), leg.import_height, on_chain(leg.execution_system.as_ref()));
            let open = link::link("Open pool", false, move || {
                let Some(model) = model.upgrade() else { return };
                // The pool at the import height when it is on the connected chain, else its
                // current state.
                match height {
                    Some(height) if executed_here => model.open_currency_at(pool_id.clone(), height),
                    _ => model.open(Route::Currency { name: pool_id.clone() }),
                }
            });
            open.set_tooltip_text(Some(&pool_help(&leg, executed_here, &namer)));
            keys.append(&small(open));
        }
        if leg.pool_inspection.as_ref().is_some_and(|p| p.previous.is_some()) && !live {
            let weak = Rc::downgrade(self);
            let compare = link::link(
                if self.comparing.get() { "Comparing…" } else { "Compare with the previous pool notarization" },
                false,
                move || {
                    if let Some(detail) = weak.upgrade() {
                        detail.compare();
                    }
                },
            );
            compare.set_sensitive(!self.comparing.get());
            keys.append(&small(compare));
        }
        self.holder.append(&keys);
        if self.show_currencies.get() {
            let list = gtk::Box::new(gtk::Orientation::Vertical, 3);
            for id in &leg.currencies {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let name = display::name_text(if Some(id) == leg.pool.as_ref() { "Pool" } else { "Currency" });
                name.set_size_request(64, -1);
                name.set_valign(gtk::Align::Center);
                row.append(&name);
                row.append(&page::route_link(&page, id, Route::Currency { name: id.clone() }, true));
                row.append(&page::text(id, &["caption-text", "mono", "tertiary"]));
                list.append(&row);
            }
            self.holder.append(&list);
        }
        if let Some(failure) = self.comparison_failure.borrow().as_ref() {
            let note = page::note(failure, true);
            note.add_css_class("callout");
            self.holder.append(&note);
        }
        if let Some(comparison) = self.comparison.borrow().as_ref() {
            self.holder.append(&pool_comparison(&ctx, &page, comparison));
        }
    }

    /// Builds again after the link that asked for it has finished its work.
    fn fill_later(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(detail) = weak.upgrade() {
                detail.fill();
            }
        });
    }

    fn compare(self: &Rc<Self>) {
        let Some(ctx) = self.ctx.upgrade() else { return };
        self.comparing.set(true);
        *self.comparison_failure.borrow_mut() = None;
        let explorer = match ctx.model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                self.comparing.set(false);
                *self.comparison_failure.borrow_mut() = Some(message);
                self.fill_later();
                return;
            }
        };
        self.fill_later();
        let (txid, output, leg) = (ctx.tx.txid.clone(), self.output, self.index);
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let found = bridge::run(async move { explorer.pool_comparison(txid, output, leg).await }).await;
            let Some(detail) = weak.upgrade() else { return };
            detail.comparing.set(false);
            match found {
                Ok(comparison) => {
                    if let Some(ctx) = detail.ctx.upgrade() {
                        ctx.model.resolve_names(comparison.changes.iter().map(|c| c.label.clone()));
                    }
                    *detail.comparison.borrow_mut() = Some(comparison);
                }
                Err(error) => *detail.comparison_failure.borrow_mut() = Some(bridge::describe(&error)),
            }
            detail.fill();
        });
    }
}

/// A link in the callout size.
fn small(link: gtk::Button) -> gtk::Button {
    if let Some(label) = link::label_of(&link) {
        label.add_css_class("callout");
    }
    link
}

fn pool_help(leg: &api::TraceLeg, executed_here: bool, namer: &super::words::Namer) -> String {
    if let Some(height) = leg.import_height
        && executed_here
    {
        return format!("Opens the pool's state at height {}, where this leg executed.", format::integer(height));
    }
    let mut text = "Opens the pool's current state. The pool is on another chain.".to_string();
    if let Some(height) = leg.import_height {
        text += &format!(" This leg executed at height {}", format::integer(height));
        if let Some(system) = &leg.execution_system {
            text += &format!(" on {}", namer.name_for(system, true));
        }
        text += ".";
    }
    text
}

fn amount_line(page: &Page, label: &str, amount: &api::CurrencyAmount, note: Option<&str>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = display::name_text(label);
    name.set_size_request(170, -1);
    name.set_valign(gtk::Align::Center);
    row.append(&name);
    row.append(&reserve_amount(page, amount, ""));
    if let Some(note) = note {
        row.append(&callout(&format!("({note})"), true));
    }
    row
}

fn amount_column(page: &Page, title: &str, amounts: &[api::CurrencyAmount], unknown: bool) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.append(&display::name_text(title));
    if amounts.is_empty() {
        column.append(&page::text(if unknown { "Unknown" } else { "–" }, &["secondary"]));
    }
    for amount in amounts {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let figure = page::text(&format::amount(amount.amount), &["tabular", "medium"]);
        figure.set_valign(gtk::Align::Baseline);
        row.append(&figure);
        row.append(&page::route_link(page, &amount.currency, Route::Currency { name: amount.currency.clone() }, true));
        column.append(&row);
    }
    column
}

/// The net amounts of a conversion, its effective rate and the import batch's named prices.
fn rate_lines(page: &Page, rate: &api::TraceRate, namer: &super::words::Namer) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 3);
    let labelled = |label: &str| {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let name = display::name_text(label);
        name.set_size_request(170, -1);
        name.set_valign(gtk::Align::Center);
        row.append(&name);
        row
    };
    let net = labelled(&format!("{} net", rate.purpose));
    net.append(&reserve_amount(page, &rate.net_input, ""));
    net.append(&arrow(false, tone::SECONDARY));
    net.append(&reserve_amount(page, &rate.output, ""));
    column.append(&net);
    if let Some(sats) = rate.rate_sats {
        let row = labelled("Execution rate");
        row.append(&page::text(
            &format!(
                "{} {} {} per {}",
                if rate.exact { "=" } else { "≈" },
                format::coins(sats),
                namer.short_name(&rate.output.currency),
                namer.short_name(&rate.net_input.currency)
            ),
            &["callout", "tabular"],
        ));
        column.append(&row);
    }
    for price in &rate.prices {
        let row = labelled("Import batch");
        row.append(&page::text(
            &format!("{} {} = {}", namer.short_name(&price.currency), price.field, format::amount(price.value)),
            &["callout", "tabular"],
        ));
        column.append(&row);
    }
    if !rate.prices.is_empty() {
        column.append(&page::paragraph(
            "Import batch prices are the batch builder's fields. They are not market quotes.",
            &["caption-text", "secondary"],
        ));
    }
    column
}

/// The pool's supply and reserves at the linked previous notarization and at the import.
fn pool_comparison(ctx: &Rc<Ctx>, page: &Page, comparison: &api::PoolComparison) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    column.set_margin_top(4);
    column.append(&theme::label(font::HEADING, "Pool states", "value"));
    column.append(&page::paragraph("Pool-wide changes between notarizations include other transfers and fees.", &["caption-text", "secondary"]));
    let chain = ctx.chain_id();
    let snapshot = |label: &str, snapshot: &api::PoolSnapshot| {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let name = display::name_text(label);
        name.set_size_request(170, -1);
        name.set_valign(gtk::Align::Center);
        row.append(&name);
        if chain.as_deref() == Some(snapshot.output.system_id.as_str()) {
            row.append(&page::route_link(page, &snapshot.output.txid, Route::Transaction { txid: snapshot.output.txid.clone() }, true));
        } else {
            row.append(&page::text(&format::short(&snapshot.output.txid, 10, 8), &["mono"]));
        }
        row.append(&callout(&format!("output {}", snapshot.output.output), true));
        if let Some(height) = snapshot.height {
            row.append(&callout(&format!("at height {}", format::integer(height)), true));
        }
        row
    };
    if let Some(previous) = &comparison.previous {
        column.append(&snapshot("Previous notarization", previous));
    }
    column.append(&snapshot("Import state", &comparison.imported));
    if let Some(note) = &comparison.note {
        let note = page::note(note, true);
        note.add_css_class("callout");
        column.append(&note);
    }
    let grid = gtk::Grid::new();
    grid.set_column_spacing(18);
    grid.set_row_spacing(4);
    for (index, title) in ["Before", "After", "Change"].iter().enumerate() {
        let name = display::name_text(title);
        name.set_xalign(1.0);
        grid.attach(&name, index as i32 + 1, 0, 1, 1);
    }
    let value = |number: Option<f64>, signed: bool| -> gtk::Label {
        let text = number
            .map(|n| format!("{}{}", if signed && n > 0.0 { "+" } else { "" }, format::amount(n)))
            .unwrap_or_else(|| "Unavailable".into());
        let label = page::text(&text, &["callout", "tabular", if number.is_none() { "secondary" } else { "value" }]);
        label.set_xalign(1.0);
        label
    };
    for (row, change) in comparison.changes.iter().enumerate() {
        let row = row as i32 + 1;
        let first: gtk::Widget = if change.is_supply {
            page::text("Supply", &["callout"]).upcast()
        } else {
            page::route_link(page, &change.label, Route::Currency { name: change.label.clone() }, true).upcast()
        };
        grid.attach(&first, 0, row, 1, 1);
        grid.attach(&value(change.before, false), 1, row, 1, 1);
        grid.attach(&value(change.after, false), 2, row, 1, 1);
        grid.attach(&value(change.change, true), 3, row, 1, 1);
    }
    column.append(&grid);
    if comparison.more_reserves {
        column.append(&page::paragraph(
            "The list shows the first 12 reserves. Open the pool for the complete state.",
            &["caption-text", "secondary"],
        ));
    }
    column
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_height_field_reads_digits() {
        assert_eq!(jump_value("4,339,600"), Some(4_339_600));
        assert_eq!(jump_value("abc"), None);
    }
}
