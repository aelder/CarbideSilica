//! A transaction as a statement (`TransactionStatementView` and `NetBar` in
//! `TransactionStatement.swift`): one sentence, then who ended with less and who ended
//! with more in each currency, as a table or as a flow. A row marks its inputs and outputs
//! in the lists below.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::gsk;
use gtk::prelude::*;

use crate::bridge::api::{self, ClosingKind};
use crate::format;
use crate::model::Topic;
use crate::page::{self, load::clear, Page};
use crate::route as routes;
use crate::settings;
use crate::theme::{draw, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;
use crate::widgets::{lamp, link};

use super::flow::Flow;
use super::layout::{RowStyle, Selection, Spread, StatementLayout, StatementRow};
use super::overview::callout;
use crate::shared::statement_words::{self as words, Namer, StatementWords};
use super::Ctx;

/// True shows the statement as a flow picture, false as the table (`statementShowsFlow`).
const SHOWS_FLOW: &str = "statementShowsFlow";
const LABEL_WIDTH: i32 = 230;

pub struct StatementView {
    pub root: gtk::Box,
    ctx: Weak<Ctx>,
    /// The one currency to show, when the statement has more columns than fit.
    only: RefCell<Option<Option<String>>>,
    /// The column whose amounts the flow picture shows.
    flow_column: Cell<usize>,
    /// The level of the flow.
    depth: Rc<Cell<usize>>,
    /// The parts that follow the mark.
    grid: RefCell<Option<glib::WeakRef<gtk::Box>>>,
    flow: RefCell<Option<Rc<Flow>>>,
    marked_line: gtk::Box,
    layout: RefCell<Option<Rc<StatementLayout>>>,
}

impl StatementView {
    pub fn new(ctx: &Rc<Ctx>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let view = Rc::new(Self {
            root,
            ctx: Rc::downgrade(ctx),
            only: RefCell::new(None),
            flow_column: Cell::new(0),
            depth: Rc::new(Cell::new(0)),
            grid: RefCell::new(None),
            flow: RefCell::new(None),
            marked_line: gtk::Box::new(gtk::Orientation::Horizontal, 8),
            layout: RefCell::new(None),
        });
        view.build();
        let weak = Rc::downgrade(&view);
        ctx.on_marked(move |selection| {
            if let Some(view) = weak.upgrade() {
                view.mark_changed(selection);
            }
        });
        // The words follow the names and the setting that shows them.
        let weak = Rc::downgrade(&view);
        ctx.model.watch(&[Topic::Names, Topic::Display], &view.root, move |_| {
            if let Some(view) = weak.upgrade() {
                view.build();
            }
        });
        view
    }

    fn words<'a>(&self, tx: &'a api::TransactionDetail, namer: &'a Namer) -> StatementWords<'a> {
        StatementWords {
            statement: &tx.statement,
            coin: &tx.coin,
            tags: &tx.tags,
            coinbase: tx.accounting.inputs.coinbase,
            fee_pool: tx.fee_pool_checkpoint,
            namer,
        }
    }

    fn build(self: &Rc<Self>) {
        let Some(ctx) = self.ctx.upgrade() else { return };
        let Some(page) = ctx.page() else { return };
        clear(&self.root);
        *self.flow.borrow_mut() = None;
        *self.grid.borrow_mut() = None;
        let tx = ctx.tx.clone();
        let statement = &tx.statement;
        let namer = ctx.namer();
        let words = self.words(&tx, &namer);
        let layout = Rc::new(StatementLayout::new(statement, self.only.borrow().clone(), |p| words.row_title(p)));
        *self.layout.borrow_mut() = Some(layout.clone());

        let sentence = page::paragraph(&words.sentence(), &["tx-sentence"]);
        sentence.set_selectable(true);
        self.root.append(&sentence);

        if statement.columns.len() > StatementLayout::MAX_COLUMNS {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let name = display::name_text("Currency");
            name.set_valign(gtk::Align::Center);
            row.append(&name);
            let mut titles: Vec<String> =
                vec![format!("First {} of {}", StatementLayout::MAX_COLUMNS, statement.columns.len())];
            titles.extend(statement.columns.iter().map(|c| words.currency(c.as_deref())));
            let options: Vec<Option<Option<String>>> =
                std::iter::once(None).chain(statement.columns.iter().map(|c| Some(c.clone()))).collect();
            let selected = options.iter().position(|o| *o == *self.only.borrow()).unwrap_or(0);
            let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
            let weak = Rc::downgrade(self);
            let menu = lamp::Menu::new(&refs, selected, move |index| {
                if let Some(view) = weak.upgrade() {
                    *view.only.borrow_mut() = options.get(index).cloned().flatten();
                    view.build_later();
                }
            });
            row.append(&menu.button);
            self.root.append(&row);
        }

        let shows_flow = settings::bool(SHOWS_FLOW, false);
        let column = self.flow_column.get().min(layout.columns.len().saturating_sub(1));
        if !layout.rows.is_empty() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
            let weak = Rc::downgrade(self);
            let segments = lamp::Segments::new(&["Table", "Flow"], usize::from(shows_flow), move |index| {
                settings::set_bool(SHOWS_FLOW, index == 1);
                if let Some(view) = weak.upgrade() {
                    view.build_later();
                }
            });
            row.append(&segments.root);
            if shows_flow && layout.columns.len() > 1 {
                let titles: Vec<String> = layout.columns.iter().map(|c| words.currency(c.as_deref())).collect();
                let refs: Vec<&str> = titles.iter().map(String::as_str).collect();
                let weak = Rc::downgrade(self);
                let columns = lamp::Segments::new(&refs, column, move |index| {
                    if let Some(view) = weak.upgrade()
                        && view.flow_column.get() != index
                    {
                        view.flow_column.set(index);
                        view.depth.set(0);
                        view.build_later();
                    }
                });
                row.append(&columns.root);
            }
            self.root.append(&row);
        }

        if !layout.rows.is_empty() || !layout.spread.is_empty() {
            let inside = gtk::Box::new(gtk::Orientation::Vertical, 12);
            if !layout.rows.is_empty() {
                if shows_flow {
                    let unit = words.currency(layout.columns.get(column).cloned().flatten().as_deref());
                    let flow = Flow::new(&ctx, layout.clone(), column, unit, self.depth.clone());
                    inside.append(&flow.root);
                    *self.flow.borrow_mut() = Some(flow);
                } else {
                    let grid = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    self.fill_grid(&grid, &page, &layout, &namer, &tx);
                    *self.grid.borrow_mut() = Some(grid.downgrade());
                    inside.append(&grid);
                }
            }
            if !layout.rows.is_empty() && !layout.spread.is_empty() {
                inside.append(&display::rule());
            }
            if !layout.spread.is_empty() {
                inside.append(&spread(&layout.spread, &words.currency(layout.columns.first().cloned().flatten().as_deref())));
            }
            self.root.append(&display::panel(&inside, 16));
        }

        self.root.append(&self.marked_line);
        self.fill_marked_line(ctx.marked().as_ref());

        if let Some(routes) = self.routes(&ctx, &page, &namer, &words) {
            self.root.append(&routes);
        }
        if let Some(carried) = carried(&tx, &words) {
            self.root.append(&carried);
        }
        if let Some(marks) = marks(&ctx, &namer) {
            self.root.append(&marks);
        }
        if statement.closing_kind == ClosingKind::InputsLessOutputs {
            self.root.append(&page::paragraph(
                "Protocol outputs account for value in other transactions. The row \"Inputs less outputs\" is not a fee.",
                &["caption-text", "secondary"],
            ));
        }
        if !statement.complete {
            self.root.append(&page::note("A value is missing from the source. The statement is not complete.", true));
        }
    }

    /// Builds again after the key that asked for it has finished its work.
    fn build_later(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(view) = weak.upgrade() {
                view.build();
            }
        });
    }

    fn mark_changed(self: &Rc<Self>, selection: Option<&Selection>) {
        self.fill_marked_line(selection);
        if let Some(flow) = self.flow.borrow().as_ref() {
            flow.set_chosen(selection.map(|s| s.id.as_str()));
        }
        let grid = self.grid.borrow().as_ref().and_then(|g| g.upgrade());
        if let (Some(grid), Some(layout), Some(ctx)) = (grid, self.layout.borrow().clone(), self.ctx.upgrade())
            && let Some(page) = ctx.page()
        {
            clear(&grid);
            self.fill_grid(&grid, &page, &layout, &ctx.namer(), &ctx.tx);
        }
    }

    /// "2 inputs and 1 output of this row are marked in the lists below."
    fn fill_marked_line(self: &Rc<Self>, selection: Option<&Selection>) {
        clear(&self.marked_line);
        let Some(selection) = selection.filter(|s| !(s.inputs.is_empty() && s.outputs.is_empty())) else {
            self.marked_line.set_visible(false);
            return;
        };
        self.marked_line.set_visible(true);
        self.marked_line.append(&callout(&marked_text(selection), true));
        let weak = self.ctx.clone();
        let (first_input, first_output) = (selection.inputs.first().copied(), selection.outputs.first().copied());
        let show = link::link("Show in the Lists", false, move || {
            let Some(ctx) = weak.upgrade() else { return };
            if let Some(input) = first_input {
                ctx.scroll(&format!("input-{input}"));
            } else if let Some(output) = first_output {
                ctx.scroll(&format!("output-{output}"));
            }
        });
        if let Some(label) = link::label_of(&show) {
            label.add_css_class("callout");
        }
        self.marked_line.append(&show);
    }

    // MARK: The grid

    fn fill_grid(&self, grid: &gtk::Box, page: &Rc<Page>, layout: &Rc<StatementLayout>, namer: &Namer, tx: &api::TransactionDetail) {
        let Some(ctx) = self.ctx.upgrade() else { return };
        let words = self.words(tx, namer);
        if !layout.columns.is_empty() {
            let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            header.set_margin_bottom(4);
            let space = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            space.set_size_request(LABEL_WIDTH, 1);
            header.append(&space);
            for column in &layout.columns {
                let name = display::name_text(&words.currency(column.as_deref()));
                name.set_hexpand(true);
                name.set_xalign(0.5);
                name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                header.append(&name);
            }
            grid.append(&header);
        }
        let chosen = ctx.marked().map(|s| s.id);
        for (position, row) in layout.rows.iter().enumerate() {
            if position > 0 {
                grid.append(&display::rule());
            }
            grid.append(&self.line(&ctx, page, row, layout, &words, chosen.as_deref() == Some(row.id.as_str())));
        }
    }

    fn line(&self, ctx: &Rc<Ctx>, page: &Rc<Page>, row: &StatementRow, layout: &StatementLayout, words: &StatementWords, chosen: bool) -> gtk::Box {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        line.set_margin_top(0);
        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        inner.set_margin_top(5);
        inner.set_margin_bottom(5);
        inner.set_hexpand(true);
        if chosen {
            line.add_css_class("tx-chosen-line");
        }
        let title: gtk::Widget = match &row.address {
            Some(address) => page::route_link(page, address, routes::holder(address), true).upcast(),
            None => {
                let label = page::text(&row.title, &[if matches!(row.style, RowStyle::Closing | RowStyle::Rest) { "label" } else { "value" }]);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.upcast()
            }
        };
        let title_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        title_box.set_size_request(LABEL_WIDTH, -1);
        title.set_valign(gtk::Align::Center);
        title.set_margin_start(if chosen { 4 } else { 0 });
        title_box.append(&title);
        inner.append(&title_box);

        let cells = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        cells.set_hexpand(true);
        cells.set_homogeneous(true);
        if row.style == RowStyle::Unchanged {
            let name = display::name_text("No change");
            name.set_xalign(0.5);
            name.set_hexpand(true);
            cells.append(&name);
        } else {
            for (column, sats) in row.cells.iter().enumerate() {
                cells.append(&cell(*sats, layout.scale[column], row.style, chosen, &words.currency(layout.columns[column].as_deref())));
            }
        }
        let selectable = row.selectable();
        if selectable {
            cells.set_tooltip_text(Some("Mark the inputs and outputs of this row in the lists below."));
            let click = gtk::GestureClick::new();
            let (weak, row_owned) = (Rc::downgrade(ctx), row.clone());
            click.connect_released(move |gesture, presses, _, _| {
                if presses != 1 {
                    return;
                }
                gesture.set_state(gtk::EventSequenceState::Claimed);
                let Some(ctx) = weak.upgrade() else { return };
                let marked = ctx.marked();
                ctx.set_marked(if marked.is_some_and(|m| m.id == row_owned.id) { None } else { Some(Selection::of(&row_owned)) });
            });
            cells.add_controller(click);
        }
        cells.update_property(&[gtk::accessible::Property::Label(&spoken(row, layout, words))]);
        inner.append(&cells);
        line.append(&inner);
        line
    }

    // MARK: Below the grid

    fn routes(&self, ctx: &Rc<Ctx>, _page: &Rc<Page>, namer: &Namer, words: &StatementWords) -> Option<gtk::Box> {
        let statement = &ctx.tx.statement;
        if statement.routes.is_empty() {
            return None;
        }
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.append(&display::name_text("Asked for"));
        for route in &statement.routes {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let mut parts = vec![words::list(&route.amounts.iter().map(|a| words.amount(a)).collect::<Vec<_>>())];
            if let Some(via) = &route.via {
                parts.push(namer.currency_name(via));
            }
            if let Some(to) = &route.to_currency {
                parts.push(namer.currency_name(to));
            }
            let mut text = parts.join(" → ");
            if let Some(destination) = &route.destination {
                text += &format!(", for {}", namer.display_id(destination));
            }
            row.append(&page::text(&text, &["callout", "tabular"]));
            if ctx.tx.trace_roots.iter().any(|r| r.output == route.output) {
                let (weak, output) = (Rc::downgrade(ctx), route.output);
                let follow = link::link("Follow", false, move || {
                    if let Some(ctx) = weak.upgrade() {
                        ctx.start(&format!("trace-{output}"));
                    }
                });
                if let Some(label) = link::label_of(&follow) {
                    label.add_css_class("callout");
                }
                follow.set_tooltip_text(Some(&format!("Start the transfer trace of output {}.", route.output)));
                row.append(&follow);
            }
            column.append(&row);
        }
        Some(column)
    }
}

fn carried(tx: &api::TransactionDetail, words: &StatementWords) -> Option<gtk::Box> {
    let lines: Vec<&api::StatementParty> = tx.statement.parties.iter().filter(|p| !p.carried.is_empty() && !p.net.is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.append(&display::name_text("Passed through with no change"));
    for party in lines {
        let amounts: Vec<String> = party.carried.iter().map(|a| words.amount(a)).collect();
        let line = page::paragraph(&format!("{}: {}", words::capital(&words.name(party)), words::list(&amounts)), &["callout"]);
        line.set_selectable(true);
        column.append(&line);
    }
    Some(column)
}

fn marks(ctx: &Rc<Ctx>, namer: &Namer) -> Option<gtk::Box> {
    let statement = &ctx.tx.statement;
    if statement.marks.is_empty() {
        return None;
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let spent: Vec<&api::StatementMark> = statement.marks.iter().filter(|m| m.spent).collect();
    let written: Vec<&api::StatementMark> = statement.marks.iter().filter(|m| !m.spent).collect();
    for (title, marks) in [("Spends", spent), ("Writes", written)] {
        if marks.is_empty() {
            continue;
        }
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = display::name_text(title);
        name.set_size_request(56, -1);
        name.set_valign(gtk::Align::Start);
        name.set_margin_top(3);
        line.append(&name);
        let flow = gtk::FlowBox::new();
        flow.set_selection_mode(gtk::SelectionMode::None);
        flow.set_column_spacing(6);
        flow.set_row_spacing(6);
        flow.set_hexpand(true);
        flow.set_max_children_per_line(64);
        for mark in marks {
            let text = match &mark.currency_id {
                Some(id) => format!("{} · {}", mark.label, namer.currency_name(id)),
                None => mark.label.clone(),
            };
            let key = gtk::Button::new();
            key.add_css_class("tx-plate-key");
            key.set_child(Some(&display::plate(&text, display::PlateFill::None)));
            key.set_cursor_from_name(Some("pointer"));
            let noun = if mark.spent { "input" } else { "output" };
            key.set_tooltip_text(Some(&format!("Go to {noun} {} in the lists below.", mark.index)));
            let (weak, spent, index) = (Rc::downgrade(ctx), mark.spent, mark.index);
            key.connect_clicked(move |_| {
                let Some(ctx) = weak.upgrade() else { return };
                let mut selection = Selection { id: format!("mark-{spent}-{index}"), inputs: Default::default(), outputs: Default::default() };
                if spent {
                    selection.inputs.insert(index);
                } else {
                    selection.outputs.insert(index);
                }
                ctx.set_marked(Some(selection));
                ctx.scroll(&format!("{}-{index}", if spent { "input" } else { "output" }));
            });
            flow.insert(&key, -1);
        }
        line.append(&flow);
        column.append(&line);
    }
    Some(column)
}

fn marked_text(selection: &Selection) -> String {
    let (inputs, outputs) = (selection.inputs.len(), selection.outputs.len());
    let mut parts = Vec::new();
    if inputs > 0 {
        parts.push(format!("{inputs} {}", if inputs == 1 { "input" } else { "inputs" }));
    }
    if outputs > 0 {
        parts.push(format!("{outputs} {}", if outputs == 1 { "output" } else { "outputs" }));
    }
    let count = inputs + outputs;
    format!("{} of this row {} marked in the lists below.", words::list(&parts), if count == 1 { "is" } else { "are" })
}

fn spoken(row: &StatementRow, layout: &StatementLayout, words: &StatementWords) -> String {
    let parts: Vec<String> = row
        .cells
        .iter()
        .zip(&layout.columns)
        .filter(|(sats, _)| **sats != 0)
        .map(|(sats, column)| {
            let money = format!("{} {}", format::coins(sats.abs()), words.currency(column.as_deref()));
            if row.style == RowStyle::Closing {
                return if *sats < 0 { format!("minus {money}") } else { money };
            }
            format!("{} {money}", if *sats < 0 { "gave" } else { "received" })
        })
        .collect();
    if parts.is_empty() { format!("{}, no change", row.title) } else { format!("{} {}", row.title, words::list(&parts)) }
}

fn cell(sats: i64, scale: i64, style: RowStyle, chosen: bool, currency: &str) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.set_hexpand(true);
    column.set_margin_start(6);
    column.set_margin_end(6);
    let fraction = sats as f64 / scale as f64;
    let bar = Canvas::new(0, 9, move |_, snapshot, w, h| net_bar(snapshot, w, h, fraction, style, chosen));
    bar.set_hexpand(true);
    column.append(&bar);
    let tone_class = if matches!(style, RowStyle::Closing | RowStyle::Rest) { "label" } else { "value" };
    let text = if sats == 0 { " ".to_string() } else { words::signed(sats) };
    let figure = page::text(&text, &["callout", "tabular", tone_class]);
    figure.set_xalign(0.5);
    figure.set_ellipsize(gtk::pango::EllipsizeMode::End);
    column.append(&figure);
    if sats != 0 {
        column.set_tooltip_text(Some(&format!(
            "{} {currency} {}",
            format::coins(sats.abs()),
            if sats < 0 { "given" } else { "received" }
        )));
    }
    column
}

/// One bar of the statement. A hollow bar to the left of the axis is an amount given. A
/// solid bar to the right is an amount received. A dashed bar has a hidden recipient.
fn net_bar(snapshot: &gtk::Snapshot, width: f32, height: f32, fraction: f64, style: RowStyle, chosen: bool) {
    let middle = width / 2.0;
    draw::rule(snapshot, middle, 0.0, middle, height, tone::AXIS, 1.0, &[]);
    if fraction == 0.0 {
        return;
    }
    let length = (fraction.abs() as f32 * (middle - 1.0)).max(2.0);
    let (x, y, w, h) = (if fraction < 0.0 { middle - length } else { middle }, 0.75, length, height - 1.5);
    match style {
        RowStyle::Closing | RowStyle::Rest => draw::fill_rect(snapshot, x, y, w, h, tone::LABEL.opacity(0.55)),
        RowStyle::Hidden => {
            draw::fill_rect(snapshot, x, y, w, h, tone::RECORD.opacity(0.15));
            let stroke = gsk::Stroke::new(1.0);
            stroke.set_line_cap(gsk::LineCap::Butt);
            stroke.set_dash(&[4.0, 3.0]);
            snapshot.append_stroke(&draw::rect_path(x + 0.5, y + 0.5, w - 1.0, h - 1.0), &stroke, &tone::RECORD.rgba());
        }
        _ => {
            if fraction < 0.0 {
                draw::stroke(snapshot, &draw::rect_path(x + 0.5, y + 0.5, w - 1.0, h - 1.0), if chosen { tone::PRESENT } else { tone::VALUE }, 1.0);
            } else if chosen {
                draw::lit(snapshot, &draw::rect_path(x, y, w, h), tone::PRESENT, 0.5);
            } else {
                draw::fill_rect(snapshot, x, y, w, h, tone::RECORD);
            }
        }
    }
}

/// The spread of the receivers in the first column: one segment for each address, largest
/// first, and the smallest together in one hatched segment.
fn spread(values: &[i64], name: &str) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let figures = Spread::new(values);
    let note = if figures.small > 0 {
        format!(
            "One segment is one address. The segment with lines is the {} smallest together: {} {name}.",
            figures.small,
            words::brief(figures.small_total)
        )
    } else {
        "One segment is one address.".to_string()
    };
    let (single, small, total) = (figures.single.clone(), figures.small, figures.total);
    let bar = Canvas::new(0, 20, move |_, snapshot, w, h| {
        let mut x = 0.0f32;
        for (index, value) in single.iter().enumerate() {
            let width = *value as f32 / total.max(1) as f32 * w;
            draw::fill_rect(
                snapshot,
                x,
                0.0,
                (width - 0.6).max(0.3),
                h,
                tone::RECORD.opacity(if index % 2 == 0 { 0.9 } else { 0.55 }),
            );
            x += width;
        }
        if small > 0 && w > x {
            // The group of small addresses: an outline with lines across it.
            let bounds = draw::rect(x, 0.0, w - x, h);
            snapshot.push_clip(&bounds);
            let lines = gsk::PathBuilder::new();
            let mut at = x - h;
            while at < w {
                lines.move_to(at, h);
                lines.line_to(at + h, 0.0);
                at += 5.0;
            }
            draw::stroke(snapshot, &lines.to_path(), tone::RECORD.opacity(0.6), 1.0);
            snapshot.pop();
            draw::stroke(snapshot, &draw::rect_path(x + 0.5, 0.5, w - x - 1.0, h - 1.0), tone::RECORD.opacity(0.8), 1.0);
        }
    });
    bar.set_hexpand(true);
    bar.update_property(&[gtk::accessible::Property::Label(&format!(
        "The amount of each of {} addresses, largest first.",
        values.len()
    ))]);
    column.append(&bar);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 26);
    for (label, value) in [
        ("Addresses".to_string(), values.len().to_string()),
        ("Largest".to_string(), format!("{} {name}", words::brief(values[0]))),
        ("Median".to_string(), format!("{} {name}", words::brief(figures.median))),
        (format!("{} largest", StatementLayout::KEPT_RECEIVERS), format!("{}% of the total", figures.share)),
    ] {
        let pair = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        let name_label = display::name_text(&label);
        name_label.set_valign(gtk::Align::Baseline);
        pair.append(&name_label);
        let figure = page::text(&value, &["callout", "tabular", "value"]);
        figure.set_valign(gtk::Align::Baseline);
        pair.append(&figure);
        row.append(&pair);
    }
    column.append(&row);
    column.append(&page::paragraph(&note, &["caption-text", "label"]));
    column
}
