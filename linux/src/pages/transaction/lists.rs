//! The inputs and the outputs (`InputRow`, `OutputRow`, `ReferenceRow`, `AnnotationRow`,
//! `ValueText` and `IndexLabel` in `TransactionView.swift`). A list longer than 40 rows
//! shows its first 20 until the person asks for all. A row that the statement marks has a
//! faint field.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

use crate::bridge::api::{self, Route};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::shared::json_tree::json_tree;
use crate::page::{self, load::clear, Page};
use crate::route as routes;
use crate::theme::tone;
use crate::widgets::icons::{self, Icon};
use crate::widgets::display::{self, PlateFill};
use crate::widgets::{lamp, link};

use super::bytes::bytes_view;
use super::layout::Selection;
use super::overview::{callout, reserve_amount};
use super::Ctx;

const LONG_LIST: usize = 40;
const FIRST_ROWS: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Inputs,
    Outputs,
}

impl Side {
    fn noun(self) -> &'static str {
        match self {
            Side::Inputs => "inputs",
            Side::Outputs => "outputs",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Side::Inputs => "input-",
            Side::Outputs => "output-",
        }
    }

    fn marked(self, selection: Option<&Selection>) -> Vec<u32> {
        selection
            .map(|s| match self {
                Side::Inputs => s.inputs.iter().copied().collect(),
                Side::Outputs => s.outputs.iter().copied().collect(),
            })
            .unwrap_or_default()
    }
}

/// One list: its rows, its "Show all" key, and the marks of the statement.
struct List {
    side: Side,
    body: gtk::Box,
    /// The person asked for all rows.
    all: Cell<bool>,
    /// The rows that are built, by index.
    rows: RefCell<Vec<(u32, glib::WeakRef<gtk::Widget>)>>,
    /// The number of rows built, and whether all were.
    shown_all: Cell<Option<bool>>,
}

use gtk::glib;

pub fn inputs_card(ctx: &Rc<Ctx>) -> gtk::Box {
    list_card(ctx, Side::Inputs)
}

pub fn outputs_card(ctx: &Rc<Ctx>) -> gtk::Box {
    list_card(ctx, Side::Outputs)
}

fn list_card(ctx: &Rc<Ctx>, side: Side) -> gtk::Box {
    let Some(page) = ctx.page() else { return gtk::Box::new(gtk::Orientation::Vertical, 0) };
    let count = match side {
        Side::Inputs => ctx.tx.inputs.len(),
        Side::Outputs => ctx.tx.outputs.len(),
    };
    let card = page.card(if side == Side::Inputs { "Inputs" } else { "Outputs" }, Some(&count.to_string()));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.append(&body);
    let list = Rc::new(List { side, body, all: Cell::new(false), rows: Default::default(), shown_all: Cell::new(None) });
    build_rows(ctx, &list, true);

    // A row that the statement marks can be out of the first rows: the list then shows all.
    let (weak_ctx, weak_list) = (Rc::downgrade(ctx), Rc::downgrade(&list));
    ctx.on_marked(move |selection| {
        let (Some(ctx), Some(list)) = (weak_ctx.upgrade(), weak_list.upgrade()) else { return };
        if list.shown_all.get() != Some(shows_all(&list, selection)) {
            build_rows(&ctx, &list, false);
        } else {
            paint_marks(&list, selection);
        }
    });
    let (weak_ctx, weak_list) = (Rc::downgrade(ctx), Rc::downgrade(&list));
    ctx.show_all.borrow_mut().push(Box::new(move |target: &str| {
        let (Some(ctx), Some(list)) = (weak_ctx.upgrade(), weak_list.upgrade()) else { return };
        if target.starts_with(list.side.prefix()) && !list.all.get() {
            list.all.set(true);
            build_rows(&ctx, &list, false);
        }
    }));
    if side == Side::Outputs {
        let (weak_ctx, weak_list) = (Rc::downgrade(ctx), Rc::downgrade(&list));
        *ctx.refresh_outputs.borrow_mut() = Some(Box::new(move || {
            if let (Some(ctx), Some(list)) = (weak_ctx.upgrade(), weak_list.upgrade()) {
                build_rows(&ctx, &list, false);
            }
        }));
    }
    // The rows follow the display settings: dates in the inputs, names in the tags.
    let (weak_ctx, weak_list) = (Rc::downgrade(ctx), Rc::downgrade(&list));
    ctx.model.watch(&[Topic::Display], &list.body, move |_| {
        if let (Some(ctx), Some(list)) = (weak_ctx.upgrade(), weak_list.upgrade()) {
            build_rows(&ctx, &list, false);
        }
    });
    ctx.keep(list);
    card.root
}

fn shows_all(list: &List, selection: Option<&Selection>) -> bool {
    list.all.get() || !list.side.marked(selection).is_empty()
}

fn paint_marks(list: &List, selection: Option<&Selection>) {
    let marked = list.side.marked(selection);
    for (index, row) in list.rows.borrow().iter() {
        if let Some(row) = row.upgrade() {
            if marked.contains(index) { row.add_css_class("tx-marked") } else { row.remove_css_class("tx-marked") }
        }
    }
}

fn build_rows(ctx: &Rc<Ctx>, list: &Rc<List>, _first: bool) {
    let Some(page) = ctx.page() else { return };
    let selection = ctx.marked();
    let all = shows_all(list, selection.as_ref());
    list.shown_all.set(Some(all));
    clear(&list.body);
    list.rows.borrow_mut().clear();
    let tx = &ctx.tx;
    let count = match list.side {
        Side::Inputs => tx.inputs.len(),
        Side::Outputs => tx.outputs.len(),
    };
    let long = count > LONG_LIST;
    let shown = if long && !all { FIRST_ROWS } else { count };
    for position in 0..shown {
        if position > 0 {
            list.body.append(&page::divider());
        }
        let (index, row) = match list.side {
            Side::Inputs => {
                let input = &tx.inputs[position];
                (input.index, input_row(&page, ctx, input))
            }
            Side::Outputs => {
                let output = &tx.outputs[position];
                (output.index, output_row(&page, ctx, output))
            }
        };
        ctx.anchor(&format!("{}{index}", list.side.prefix()), &row);
        list.rows.borrow_mut().push((index, row.upcast_ref::<gtk::Widget>().downgrade()));
        list.body.append(&row);
    }
    if long {
        let key = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        key.set_margin_top(8);
        let noun = list.side.noun();
        let title = if all { format!("Show the first {FIRST_ROWS}") } else { format!("Show all {count} {noun}") };
        let (weak_ctx, weak_list) = (Rc::downgrade(ctx), Rc::downgrade(list));
        key.append(&lamp::button(&title, move || {
            if let (Some(ctx), Some(list)) = (weak_ctx.upgrade(), weak_list.upgrade()) {
                list.all.set(!list.all.get());
                build_rows(&ctx, &list, false);
            }
        }));
        if !all {
            let note = page::text(&format!("The first {FIRST_ROWS} of {count} {noun} are shown."), &["caption-text", "label"]);
            key.append(&note);
        }
        list.body.append(&key);
    }
    paint_marks(list, selection.as_ref());
}

fn index_label(index: u32) -> gtk::Label {
    let label = page::text(&index.to_string(), &["callout", "tabular", "secondary"]);
    label.set_xalign(1.0);
    label.set_size_request(28, -1);
    label.set_valign(gtk::Align::Start);
    label.set_margin_top(1);
    label
}

/// An annotation or status badge, in Carbide's words.
fn annotation_row(annotations: &[String], badges: &[String]) -> Option<gtk::Box> {
    if annotations.is_empty() && badges.is_empty() {
        return None;
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for annotation in annotations {
        row.append(&display::plate(annotation, PlateFill::Record));
    }
    for badge in badges {
        let label = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        label.append(&icons::icon(Icon::Hourglass, 11, tone::MINED));
        let text = page::text(badge, &["caption-small", "medium", "mined"]);
        label.append(&text);
        label.set_tooltip_text(Some(if badge == "Immature" {
            "A coinbase output cannot be spent until it is 100 blocks deep."
        } else {
            "An output can stake only after 150 confirmations."
        }));
        row.append(&label);
    }
    Some(row)
}

/// A native value: exact, derived from the shielded value balance, or hidden.
fn value_text(model: &AppModel, sats: Option<i64>, derived: bool, shielded: bool) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.set_halign(gtk::Align::End);
    match sats {
        Some(sats) => {
            let amount = gtk::Label::new(None);
            amount.set_markup(&format!(
                "<span font_features=\"tnum\">{}</span>",
                page::coin_markup(sats, &model.coin(), tone::VALUE)
            ));
            amount.set_xalign(1.0);
            amount.set_halign(gtk::Align::End);
            column.append(&amount);
            if derived {
                let note = page::text("Derived from Sapling valueBalance", &["caption-small", "secondary"]);
                note.set_xalign(1.0);
                column.append(&note);
            }
        }
        None if shielded => {
            let hidden = page::text("Hidden", &["secondary"]);
            hidden.set_xalign(1.0);
            hidden.set_tooltip_text(Some("A shielded value is private. Carbide shows it only when the value balance proves it."));
            column.append(&hidden);
        }
        None => {
            let missing = page::text("Value unavailable", &["secondary"]);
            missing.set_xalign(1.0);
            column.append(&missing);
        }
    }
    column
}

/// The values at the right of a row: the native value, then the tokens.
fn values(page: &Page, model: &AppModel, sats: Option<i64>, derived: bool, shielded: bool, reserves: &[api::CurrencyAmount]) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.set_valign(gtk::Align::Start);
    column.set_halign(gtk::Align::End);
    column.append(&value_text(model, sats, derived, shielded));
    for amount in reserves {
        let reserve = reserve_amount(page, amount, "");
        reserve.set_halign(gtk::Align::End);
        column.append(&reserve);
    }
    column
}

/// A line of small parts in the callout size: "spends <link> output 0".
fn callout_line() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Horizontal, 4)
}

fn callout_link(page: &Page, id: &str, route: Route, shorten: bool) -> gtk::Button {
    let link = page::route_link(page, id, route, shorten);
    if let Some(label) = link::label_of(&link) {
        label.add_css_class("callout");
    }
    link
}

fn input_row(page: &Page, ctx: &Rc<Ctx>, input: &api::TransactionInput) -> gtk::Box {
    let tx = &ctx.tx;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.append(&index_label(input.index));
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_hexpand(true);
    if input.coinbase {
        column.append(&display::plate("Coinbase", PlateFill::Mined));
    } else if let Some(pool) = &input.shielded_pool {
        column.append(&page::text(&format!("Shielded pool · {pool}"), &["secondary"]));
        column.append(&callout("The address is shielded and not known.", true));
    }
    for address in &input.addresses {
        column.append(&page::route_link(page, address, routes::holder(address), false));
    }
    if !input.coinbase && input.shielded_pool.is_none() && input.addresses.is_empty() {
        column.append(&page::text("Address unavailable", &["secondary"]));
    }
    if let Some(previous) = &input.previous_txid {
        let line = callout_line();
        line.append(&callout("spends", true));
        line.append(&callout_link(page, previous, Route::Transaction { txid: previous.clone() }, true));
        if let Some(output) = input.previous_output {
            line.append(&callout(&format!("output {output}"), true));
        }
        column.append(&line);
    }
    // Script type, sequence and the confirmations the spent output had.
    let details = gtk::Box::new(gtk::Orientation::Vertical, 2);
    let parts: Vec<String> = [
        input.script_type.as_ref().map(|s| format!("script {s}")),
        input.sequence.map(|s| format!("sequence {s}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !parts.is_empty() {
        details.append(&callout(&parts.join(" · "), true));
    }
    if input.previous_txid.is_some() {
        details.append(&callout(&spent_context(input, tx.block_hash.is_some()), true));
    }
    column.append(&details);
    let annotations: Vec<String> = input.annotations.iter().filter(|a| !a.starts_with("Derived from ")).cloned().collect();
    if let Some(annotations) = annotation_row(&annotations, &[]) {
        column.append(&annotations);
    }
    row.append(&column);
    row.append(&values(
        page,
        &ctx.model,
        input.value_sats,
        input.derived_value,
        input.shielded_pool.is_some(),
        &input.reserve_amounts,
    ));
    row
}

fn spent_context(input: &api::TransactionInput, spent_in_block: bool) -> String {
    let time = input.source_block_time.map(|t| format!("Block time {}", format::date(t))).unwrap_or_else(|| "Block time unknown".into());
    if !spent_in_block {
        return format!("{time} · confirmations when spent: not yet confirmed");
    }
    let height = input.source_height.map(|h| format!(" · height {}", format::integer(h))).unwrap_or_default();
    if crate::display::shows_dates()
        && let Some(time) = input.source_block_time
    {
        return format!("Confirmed since {} when spent{height}", format::date(time));
    }
    let count = input.confirmations_when_spent.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into());
    format!("{time} · confirmations when spent {count}{height}")
}

fn output_row(page: &Page, ctx: &Rc<Ctx>, output: &api::TransactionOutput) -> gtk::Box {
    let model = &ctx.model;
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 6);
    outer.set_margin_top(8);
    outer.set_margin_bottom(8);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.append(&index_label(output.index));
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_hexpand(true);
    if let Some(pool) = &output.shielded_pool {
        column.append(&page::text(&format!("Shielded pool · {pool}"), &["secondary"]));
        column.append(&callout("The address is shielded and not known.", true));
    } else if output.addresses.is_empty() {
        column.append(&page::text("Address unavailable", &["secondary"]));
    }
    for address in &output.addresses {
        if let Some(routing) = output.routing.iter().find(|r| &r.label == address) {
            column.append(&callout(&routing.value, true));
        }
        column.append(&page::route_link(page, address, routes::holder(address), false));
    }
    // Value, script type and whether the output type can be spent.
    let parts: Vec<String> = [
        output.script_type.as_ref().map(|s| format!("script {s}")),
        output.spendable.map(|s| if s { "spendable output type".to_string() } else { "protocol output type".to_string() }),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !parts.is_empty() {
        column.append(&callout(&parts.join(" · "), true));
    }
    if !output.tag_details.is_empty() {
        column.append(&tags(ctx, output));
    }
    let mut annotations: Vec<String> = output.annotations.iter().filter(|a| !a.starts_with("Derived from ")).cloned().collect();
    if let Some(payment) = ctx.payments.borrow().get(&output.index) {
        annotations.push(payment.clone());
    }
    if let Some(annotations) = annotation_row(&annotations, &output.badges) {
        column.append(&annotations);
    }
    // The bytes of unrecognised data open below the row.
    let bytes_holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    bytes_holder.set_margin_start(40);
    bytes_holder.set_visible(false);
    for reference in &output.references {
        column.append(&reference_row(page, ctx, reference, &bytes_holder));
    }
    for note in &output.reference_notes {
        let line = callout(note, true);
        line.set_wrap(true);
        column.append(&line);
    }
    if let Some(spender) = &output.spent_txid {
        let line = callout_line();
        line.append(&callout("spent by", true));
        line.append(&callout_link(page, spender, Route::Transaction { txid: spender.clone() }, true));
        column.append(&line);
    }
    row.append(&column);
    row.append(&values(
        page,
        model,
        output.value_sats,
        output.derived_value,
        output.shielded_pool.is_some(),
        &output.reserve_amounts,
    ));
    outer.append(&row);
    outer.append(&bytes_holder);
    if !output.json.is_empty() {
        let json_holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        json_holder.set_margin_start(40);
        json_holder.set_visible(false);
        let key = gtk::Button::new();
        key.add_css_class("tx-icon-key");
        let mark = crate::theme::label(crate::theme::font::MONO_CALLOUT, "{}", "secondary");
        key.set_child(Some(&mark));
        key.set_valign(gtk::Align::Start);
        key.set_tooltip_text(Some("Show the JSON of this output"));
        key.update_property(&[gtk::accessible::Property::Label(&format!("Show JSON for output {}", output.index))]);
        lamp::key_sounds(&key, false);
        let (json, holder, ctx) = (output.json.clone(), json_holder.downgrade(), Rc::downgrade(ctx));
        key.connect_clicked(move |_| {
            let Some(holder) = holder.upgrade() else { return };
            if holder.first_child().is_none() {
                let Some(page) = ctx.upgrade().and_then(|c| c.page()) else { return };
                holder.append(&json_view(&page, &json));
            }
            holder.set_visible(!holder.is_visible());
        });
        row.append(&key);
        outer.append(&json_holder);
    }
    outer
}

/// The JSON of one output as a tree, on the black of the display in a thin frame.
fn json_view(page: &Rc<Page>, json: &str) -> gtk::Box {
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("tx-json");
    frame.append(&json_tree(page, json, "carbide.json"));
    frame
}

/// Carbide's output tags. A DeFi tag names its currency. A tag that opens something is a
/// key.
fn tags(ctx: &Rc<Ctx>, output: &api::TransactionOutput) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for tag in &output.tag_details {
        if let Some(target) = &tag.target {
            let key = plate_key(&format!("{} ↗", tag.label), PlateFill::None);
            key.set_tooltip_text(Some(&format!("Open output {} of {}", target.output, format::short(&target.txid, 10, 8))));
            let (weak, target) = (Rc::downgrade(ctx), target.clone());
            key.connect_clicked(move |_| {
                let Some(ctx) = weak.upgrade() else { return };
                let chain = ctx.chain_id();
                if chain.is_none() || chain.as_deref() == Some(target.system_id.as_str()) {
                    ctx.model.open(Route::Transaction { txid: target.txid.clone() });
                }
            });
            row.append(&key);
        } else if let Some(currency) = &tag.currency_id {
            let key = plate_key(&format!("{}: {}", tag.label, ctx.namer().short_name(currency)), PlateFill::None);
            key.set_tooltip_text(Some(currency));
            let (weak, id) = (Rc::downgrade(ctx), currency.clone());
            key.connect_clicked(move |_| {
                if let Some(ctx) = weak.upgrade() {
                    ctx.model.open(Route::Currency { name: id.clone() });
                }
            });
            // The name of the currency arrives later.
            let (weak_ctx, weak_key, label, id) = (Rc::downgrade(ctx), key.downgrade(), tag.label.clone(), currency.clone());
            ctx.model.watch(&[Topic::Names], &key, move |_| {
                if let (Some(ctx), Some(key)) = (weak_ctx.upgrade(), weak_key.upgrade())
                    && let Some(plate) = key.child().and_downcast::<gtk::Label>()
                {
                    crate::theme::set_label(&plate, crate::theme::font::PLATE, &format!("{label}: {}", ctx.namer().short_name(&id)));
                }
            });
            row.append(&key);
        } else {
            let plate = display::plate(&tag.label, PlateFill::None);
            if tag.traceable {
                plate.set_tooltip_text(Some("The transfer trace for this output is above."));
            }
            row.append(&plate);
        }
    }
    row
}

/// A plate that is a key: it opens what it names.
fn plate_key(text: &str, fill: PlateFill) -> gtk::Button {
    let key = gtk::Button::new();
    key.add_css_class("tx-plate-key");
    key.set_child(Some(&display::plate(text, fill)));
    key.set_halign(gtk::Align::Start);
    key.set_cursor_from_name(Some("pointer"));
    lamp::key_sounds(&key, false);
    key
}

/// One explicit reference from an output: a currency, a destination, another output, a
/// notarized block, or the raw bytes of unrecognised data.
fn reference_row(page: &Page, ctx: &Rc<Ctx>, reference: &api::OutputReference, bytes_holder: &gtk::Box) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&callout(&format!("{}:", reference.label), true));
    let chain = ctx.chain_id();
    match &reference.target {
        api::ReferenceTarget::Currency { id } => {
            row.append(&callout_link(page, id, Route::Currency { name: id.clone() }, true));
        }
        api::ReferenceTarget::Destination { address, system_id } => {
            row.append(&callout_link(page, address, routes::holder(address), true));
            if let Some(system) = system_id
                && Some(system) != chain.as_ref()
            {
                row.append(&callout("on", true));
                row.append(&callout_link(page, system, Route::Currency { name: system.clone() }, true));
            }
        }
        api::ReferenceTarget::Output { outpoint } => {
            if Some(&outpoint.system_id) == chain.as_ref() {
                row.append(&callout_link(page, &outpoint.txid, Route::Transaction { txid: outpoint.txid.clone() }, true));
            } else {
                let id = page::text(&format::short(&outpoint.txid, 10, 8), &["mono"]);
                id.set_selectable(true);
                row.append(&id);
            }
            row.append(&callout(&format!("output {} on", outpoint.output), true));
            row.append(&callout_link(page, &outpoint.system_id, Route::Currency { name: outpoint.system_id.clone() }, true));
        }
        api::ReferenceTarget::Block { hash, height } => {
            let text = format!("#{} {}", format::integer(*height as u64), format::short(hash, 10, 8));
            let model = Rc::downgrade(&ctx.model);
            let target = hash.clone();
            let open = move || {
                if let Some(model) = model.upgrade() {
                    model.open(Route::Block { id: target.clone() });
                }
            };
            let open = Rc::new(open);
            let run = open.clone();
            let link = link::link(&text, false, move || run());
            if let Some(label) = link::label_of(&link) {
                label.add_css_class("callout");
            }
            link.set_tooltip_text(Some(hash));
            page.stop(&link, None, Some(hash.clone()), move || open());
            row.append(&link);
        }
        api::ReferenceTarget::Bytes { hex, script_body } => {
            let count = hex.len() / 2;
            let kind = if *script_body { "script bytes" } else { "payload" };
            let closed = format!("View {count} bytes ({kind})");
            let holder = bytes_holder.downgrade();
            let (hex, script_body) = (hex.clone(), *script_body);
            let page_weak = ctx.page.clone();
            let key: Rc<RefCell<Option<glib::WeakRef<gtk::Button>>>> = Rc::default();
            let key_ref = key.clone();
            let button = link::link(&closed.clone(), false, move || {
                let Some(holder) = holder.upgrade() else { return };
                let open = !holder.is_visible();
                if open && holder.first_child().is_none()
                    && let Some(page) = page_weak.upgrade()
                {
                    holder.append(&bytes_view(&page, &hex, script_body));
                }
                if !open {
                    // The view gives the arrow keys back to the page when it goes.
                    clear(&holder);
                    if let Some(page) = page_weak.upgrade() {
                        page.cursor.suspended.set(false);
                    }
                }
                holder.set_visible(open);
                if let Some(button) = key_ref.borrow().as_ref().and_then(|k| k.upgrade()) {
                    link::set_text(&button, if open { "Hide bytes" } else { &closed });
                }
            });
            if let Some(label) = link::label_of(&button) {
                label.add_css_class("callout");
            }
            *key.borrow_mut() = Some(button.downgrade());
            row.append(&button);
        }
    }
    row
}
