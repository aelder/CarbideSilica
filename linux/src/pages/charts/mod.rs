//! The Charts page (`ChartsView.swift`, `ChartStack.swift`, `ChartPlot.swift`,
//! `FeePoolChart.swift`): charts of block intervals, mining and staking difficulty, the share
//! of staked blocks and the fee pool, over a range of header time that can be paged and
//! sought. Below them, the estimator for future blocks.
//!
//! Other pages open the charts at a block with `model.show_charts(height, hash)`. Without an
//! anchor the charts open at the chain tip. A block on a side branch also needs its hash:
//! the chart then follows parent hashes from it.

mod cards;
mod fee_pool;
mod format;
mod light;
pub mod model;
mod parts;
mod stack;
mod state;

use std::cell::Cell;
use std::rc::Rc;

use gtk::gdk::Key;
use gtk::glib;
use gtk::prelude::*;

use crate::format as fmt;
use crate::model::{AppModel, Topic};
use crate::page::{self, Page};
use crate::theme::{self, font};
use crate::widgets::display;
use crate::widgets::{lamp, link, loader};

use model::Span;
use parts::{caption, caption_note, Holder, Mark};
use stack::ChartStack;
use state::Charts;

const KEYS_TEXT: &str = "Keys: ← → or j k select a block, Home End ends, [ ] ranges, − + span, g go to, Return open";
const CLICK_TEXT: &str = "Click the panels to use the keys.";

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>) -> Rc<Page> {
    let page = Page::scrolling(model);
    // The charts have their own keys, on the panels.
    page.cursor.enabled.set(false);
    let charts = Charts::new(model);
    *charts.root.borrow_mut() = Some(page.root.downgrade());

    let card = page.card("Charts", None);
    let trailing = card.trailing.clone();

    // MARK: Controls
    let controls = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let span_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let span_name = display::name_text("Span");
    span_row.append(&span_name);
    let titles: Vec<&str> = Span::ALL.iter().map(|s| s.title()).collect();
    let weak = Rc::downgrade(&charts);
    let spans = lamp::Segments::new(&titles, charts.state.borrow().span.index(), move |index| {
        if let Some(charts) = weak.upgrade() {
            charts.set_span(Span::ALL[index]);
        }
    });
    span_row.append(&spans.root);
    controls.append(&span_row);

    let keys_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let weak = Rc::downgrade(&charts);
    let older = parts::key("Older range", Mark::Left, move || {
        if let Some(charts) = weak.upgrade() {
            charts.older();
        }
    });
    older.set_tooltip_text(Some("Load the range before this one ( [ or Page Up )"));
    keys_row.append(&older);
    let weak = Rc::downgrade(&charts);
    let newer = parts::key("Newer range", Mark::Right, move || {
        if let Some(charts) = weak.upgrade() {
            charts.newer();
        }
    });
    newer.set_tooltip_text(Some("Load the range after this one ( ] or Page Down )"));
    keys_row.append(&newer);
    // The tip moves on after a range loads, so this stays available.
    let weak = Rc::downgrade(&charts);
    let latest = lamp::button("Latest", move || {
        if let Some(charts) = weak.upgrade() {
            charts.latest();
        }
    });
    latest.set_tooltip_text(Some("Load the range that ends at the current chain tip"));
    keys_row.append(&latest);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    keys_row.append(&spacer);
    let work = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let work_spinner = loader::small();
    work.append(&work_spinner);
    let work_text = page::text("", &["caption-text", "secondary", "tabular"]);
    work.append(&work_text);
    let weak = Rc::downgrade(&charts);
    let stop = lamp::button("Stop", move || {
        if let Some(charts) = weak.upgrade() {
            charts.stop();
        }
    });
    work.append(&stop);
    keys_row.append(&work);
    let weak = Rc::downgrade(&charts);
    let refresh = parts::key("Refresh", Mark::Refresh, move || {
        if let Some(charts) = weak.upgrade() {
            charts.refresh();
        }
    });
    refresh.set_tooltip_text(Some("Load this range again and check that its newest block is on the active chain (Ctrl+R)"));
    keys_row.append(&refresh);
    controls.append(&keys_row);
    card.append(&controls);

    // The note of a public API.
    let api_note = caption_note(
        "A public API supplies blocks one at a time and slowly. The charts open with a 2-hour span. A longer span can take many minutes. A local node loads 7 days in seconds.",
        false,
    );
    card.append(&api_note);

    // MARK: Coverage
    let coverage = Holder::new(4);
    card.append(&coverage.root);
    let provenance = Holder::new(0);
    card.append(&provenance.root);

    // MARK: Plot
    let failure = Holder::new(0);
    card.append(&failure.root);
    let stopped = Holder::new(0);
    card.append(&stopped.root);
    let (open_weak, select_weak) = (Rc::downgrade(&charts), Rc::downgrade(&charts));
    let stack = ChartStack::new(
        move |hash| {
            if let Some(charts) = select_weak.upgrade() {
                charts.select(hash);
            }
        },
        move |hash| {
            if let Some(charts) = open_weak.upgrade() {
                charts.open(hash);
            }
        },
    );
    card.append(&stack.root);
    let keys_text = theme::label(font::Role { size: 10.0, ..font::CAPTION }, CLICK_TEXT, "label");
    card.append(&keys_text);
    let loading_view = loader::progress("Loading block headers…");
    loading_view.set_size_request(-1, 300);
    loading_view.set_halign(gtk::Align::Fill);
    card.append(&loading_view);
    let empty = page::text("No blocks in this range.", &["secondary"]);
    empty.set_xalign(0.5);
    empty.set_size_request(-1, 300);
    card.append(&empty);

    // MARK: Notes
    let marks = parts::disclosure("What the marks mean", |_| {});
    let marks_text = gtk::Box::new(gtk::Orientation::Vertical, 4);
    marks_text.set_margin_top(4);
    for text in [
        "All panels share one time axis. One line shows the selected block in each panel. A grey band is one of the longest gaps of the range. The chart marks the longest hundredth of the gaps, and 12 at most.",
        "In the block rate panel, each point is one period. The band is the mean with limits of 2.66 times the mean change between periods. The dashed line is the target. A red point is outside the limits.",
        "In the difficulty panels, each dot is one block of that kind, or a bar from the lowest to the highest value where the dots would merge. The line is MA100, the mean over the trailing 100 chain heights, so it turns after the dots turn. A gap means that the trailing 100 heights are incomplete or hold a block of the kind without a value. The axes are fitted to the values and do not start at zero.",
        "Staking difficulty is the fixed initial Verus LWMA posLimit divided by the observed PoS target of the block. The target comes from the nonce and must equal postarget.",
        "In the staked share panel, the violet line is the share over 100 blocks and the blue line is the share over 1,000 blocks. The band is three standard deviations of chance for 100 blocks. A share line needs a complete window of known blocks.",
        "In the fee pool panel, a tick at the top is a block where the pool rose. This is a balance chart. It does not separate inflow from payout.",
    ] {
        marks_text.append(&page::paragraph(text, &[]));
    }
    marks.content.append(&marks_text);
    card.append(&marks.root);
    let notes = Holder::new(4);
    card.append(&notes.root);
    page.append(&card.root);

    // MARK: The cards below
    cards::seek_card(&page, &charts);
    cards::selected_card(&page, &charts);
    fee_pool::section(&page, &charts);
    cards::sample_table(&page, &charts);
    page.append(&crate::shared::estimator::block_estimator(&page, None));

    // MARK: Keys
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&charts);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(charts) = weak.upgrade() else { return glib::Propagation::Proceed };
        if modifiers.intersects(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK) {
            return glib::Propagation::Proceed;
        }
        if handle_key(&charts, key) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
    });
    stack.root.add_controller(keys);
    let focus = gtk::EventControllerFocus::new();
    let label = keys_text.clone();
    focus.connect_enter(move |_| theme::set_label(&label, font::Role { size: 10.0, ..font::CAPTION }, KEYS_TEXT));
    let label = keys_text.clone();
    focus.connect_leave(move |_| theme::set_label(&label, font::Role { size: 10.0, ..font::CAPTION }, CLICK_TEXT));
    stack.root.add_controller(focus);
    {
        let weak = stack.root.downgrade();
        let scroller = page.scroller.as_ref().map(|s| s.downgrade());
        *charts.focus_plot.borrow_mut() = Some(Rc::new(move || {
            if let Some(root) = weak.upgrade()
                && root.is_visible()
                && root.is_mapped()
            {
                // The focus does not move the page: the panels take it where they are.
                let adjustment = scroller.as_ref().and_then(|s| s.upgrade()).map(|s| s.vadjustment());
                let at = adjustment.as_ref().map(|a| a.value());
                root.grab_focus();
                if let (Some(adjustment), Some(at)) = (adjustment, at) {
                    adjustment.set_value(at);
                    glib::idle_add_local_once(move || adjustment.set_value(at));
                }
            }
        }));
    }

    // MARK: Updates
    let had_page = Rc::new(Cell::new(false));
    let page_weak = Rc::downgrade(&page);
    let stack_ref = stack.clone();
    charts.listen(move |charts| {
        let Some(page) = page_weak.upgrade() else { return };
        let model = page.model.clone();
        let coin = {
            let state = charts.state.borrow();
            state.fee.current.as_ref().map(|c| c.currencies.native_label.clone()).unwrap_or_else(|| model.coin())
        };
        let range = charts.range_label();
        match &range {
            Some(text) => {
                theme::set_label(&trailing, font::SMALL, text);
                trailing.set_visible(true);
            }
            None => trailing.set_visible(false),
        }
        let can_older = charts.can_page_older();
        let can_newer = charts.can_page_newer();
        let loading_text = charts.loading_text();
        let selected = charts.selected_sample().map(|s| s.hash);
        let state = charts.state.borrow();
        spans.set_selected(state.span.index());
        older.set_sensitive(can_older);
        newer.set_sensitive(can_newer);
        if state.loading {
            work.set_visible(true);
            stop.set_visible(true);
            work_text.set_text(&loading_text);
        } else if state.fee.loading {
            work.set_visible(true);
            stop.set_visible(false);
            work_text.set_text("Reading fee pool checkpoints");
        } else {
            work.set_visible(false);
        }
        api_note.set_visible(state.via_api);

        // Coverage.
        let page_data = state.page.clone();
        let short = charts.coverage_short();
        let sentence = charts.coverage_sentence();
        let signature = format!(
            "{:?}|{short}|{sentence}|{}",
            page_data.as_ref().map(|p| (p.samples.first().map(|s| s.hash.clone()), p.samples.last().map(|s| s.hash.clone()))),
            crate::display::uses_utc()
        );
        coverage.set(signature, |root| {
            let Some(data) = &page_data else { return };
            let (Some(first), Some(last)) = (data.samples.first(), data.samples.last()) else { return };
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
            row.append(&endpoint(charts, "Start", first));
            row.append(&endpoint(charts, "End", last));
            let covers = gtk::Box::new(gtk::Orientation::Vertical, 3);
            covers.append(&display::name_text("Covers"));
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 5);
            let figure = theme::label(font::FIGURE_SMALL, &format::duration(last.time as i64 - first.time as i64), "value");
            figure.set_valign(gtk::Align::Baseline);
            line.append(&figure);
            let words = theme::label(font::CAPTION, "of header time", "label");
            words.set_valign(gtk::Align::Baseline);
            line.append(&words);
            covers.append(&line);
            row.append(&covers);
            root.append(&row);
            let text = theme::label(font::CAPTION, &short, "label");
            text.set_tooltip_text(Some(&sentence));
            root.append(&text);
        });
        let retained = page_data.as_ref().and_then(|p| p.provenance.clone()).filter(|p| p.retained);
        provenance.set(format!("{:?}", retained.as_ref().map(|p| (&p.source, p.observed_at))), |root| {
            if let Some(provenance) = &retained {
                root.append(&page::provenance_note(provenance));
            }
        });

        // Plot.
        failure.set(format!("{:?}", state.failure), |root| {
            if let Some(text) = &state.failure {
                root.append(&page::note(text, true));
            }
        });
        stopped.set(format!("{:?}", state.stopped), |root| {
            if let Some(text) = &state.stopped {
                root.append(&page::note(text, true));
            }
        });
        let shown = page_data.as_ref().filter(|p| !p.samples.is_empty());
        stack_ref.root.set_visible(shown.is_some());
        keys_text.set_visible(shown.is_some());
        loading_view.set_visible(shown.is_none() && state.loading);
        empty.set_visible(shown.is_none() && !state.loading && state.failure.is_none());
        if let Some(data) = shown {
            stack_ref.set(data, state.span.seconds(), &state.fee, &coin, selected);
        }

        // Notes.
        marks.root.set_visible(page_data.is_some());
        let seek_note = state.seek_note.clone();
        let signature = format!(
            "{:?}|{:?}",
            page_data.as_ref().map(|p| (p.negative_intervals, p.unavailable_stake)),
            seek_note
        );
        notes.set(signature, |root| {
            let Some(data) = &page_data else { return };
            if data.negative_intervals > 0 {
                root.append(&caption(&if data.negative_intervals == 1 {
                    "1 block in this range has a timestamp before its parent's. Select it to see its signed interval.".to_string()
                } else {
                    format!(
                        "{} blocks in this range have a timestamp before their parent's. Select a block to see its signed interval.",
                        data.negative_intervals
                    )
                }));
            }
            if data.unavailable_stake > 0 {
                root.append(&caption(&format!(
                    "{} staked blocks have no staking difficulty. Select one to see why.",
                    data.unavailable_stake
                )));
            }
            if let Some(note) = &seek_note {
                root.append(&caption(note));
            }
        });
        drop(state);

        // The plot exists only once a range has loaded. Give it focus then, once.
        if shown.is_some() && !had_page.get() {
            had_page.set(true);
            let focus = charts.focus_plot.borrow().clone();
            if let Some(focus) = focus {
                glib::idle_add_local_once(move || focus());
            }
        } else if page_data.is_none() {
            had_page.set(false);
        }
    });

    // MARK: Page hooks
    {
        let mut hooks = page.hooks();
        let weak = Rc::downgrade(&charts);
        hooks.refresh = Some(Rc::new(move || {
            if let Some(charts) = weak.upgrade() {
                charts.refresh();
            }
        }));
        let weak = Rc::downgrade(&charts);
        hooks.shown = Some(Rc::new(move || {
            if let Some(charts) = weak.upgrade() {
                charts.shown();
                // The keys work while the plot has focus, so it takes focus when the page opens.
                let focus = charts.focus_plot.borrow().clone();
                if let Some(focus) = focus {
                    glib::idle_add_local_once(move || focus());
                }
            }
        }));
    }

    // A new connection: the latest range again. A display setting: the dates again.
    let generation = Rc::new(Cell::new(model.generation()));
    let weak = Rc::downgrade(&charts);
    model.watch(&[Topic::Connection, Topic::Status], &page.root, move |model| {
        let Some(charts) = weak.upgrade() else { return };
        if model.generation() != generation.get() {
            generation.set(model.generation());
            charts.reconnected(charts.via_api_now());
        }
    });
    let (weak, stack_ref) = (Rc::downgrade(&charts), stack.clone());
    model.watch(&[Topic::Display], &page.root, move |_| {
        if let Some(charts) = weak.upgrade() {
            stack_ref.redraw();
            charts.changed();
        }
    });

    let (anchor, hash) = {
        let state = model.state();
        (state.chart_anchor, state.chart_anchor_hash.clone())
    };
    charts.prepare(anchor, hash, charts.via_api_now());
    charts.changed();
    // Keep the state and the stack alive with the page.
    let keep = (charts.clone(), stack.clone());
    page.root.connect_destroy(move |_| {
        let _ = &keep;
    });
    page
}

/// The keys of the panels (`ChartState.handle`).
fn handle_key(charts: &Rc<Charts>, key: Key) -> bool {
    match key {
        Key::Down | Key::Left => charts.move_by(-1),
        Key::Up | Key::Right => charts.move_by(1),
        Key::Home => charts.move_to_end(false),
        Key::End => charts.move_to_end(true),
        Key::Page_Up => charts.older(),
        Key::Page_Down => charts.newer(),
        Key::Return | Key::KP_Enter => {
            if let Some(sample) = charts.selected_sample() {
                charts.open(&sample.hash);
            }
        }
        _ => {
            let Some(typed) = key.to_unicode() else { return false };
            match typed {
                'j' => charts.move_by(-1),
                'k' => charts.move_by(1),
                '[' => charts.older(),
                ']' => charts.newer(),
                '-' => charts.step_span(-1),
                '+' | '=' => charts.step_span(1),
                'g' => {
                    let focus = charts.focus_seek.borrow().clone();
                    if let Some(focus) = focus {
                        focus();
                    }
                }
                'v' => {
                    charts.state.borrow_mut().fee.cycle_currency();
                    charts.changed();
                }
                'z' => {
                    charts.state.borrow_mut().fee.cycle_scale();
                    charts.changed();
                }
                't' => {
                    {
                        let mut state = charts.state.borrow_mut();
                        state.fee.detail = !state.fee.detail;
                    }
                    charts.changed();
                }
                _ => return false,
            }
        }
    }
    true
}

/// The start or end of the range: its height as a link, and its date.
fn endpoint(charts: &Rc<Charts>, label: &str, sample: &crate::bridge::api::ChartSample) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 3);
    column.append(&display::name_text(label));
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let (weak, hash) = (Rc::downgrade(charts), sample.hash.clone());
    let height = link::link_in(font::FIGURE_SMALL, &format!("#{}", fmt::integer(sample.height)), move || {
        if let Some(charts) = weak.upgrade() {
            charts.open(&hash);
        }
    });
    height.set_tooltip_text(Some(&sample.hash));
    line.append(&height);
    let date = theme::label(font::CAPTION, &fmt::date(sample.time), "value");
    date.add_css_class("tabular");
    date.set_valign(gtk::Align::Baseline);
    line.append(&date);
    column.append(&line);
    column
}
