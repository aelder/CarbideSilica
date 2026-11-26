//! The cards below the charts: go to a block or a time (`SeekCard`), the selected block
//! (`SelectedBlockCard`) and the samples as a table (`SampleTable`), in ChartsView.swift.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api};
use crate::format;
use crate::page::{self, Page};
use crate::widgets::{lamp, loader};

use super::fee_pool::{moves, set_disclosure_title};
use super::format as chart_format;
use super::parts::{self, caption, caption_note, Holder};
use super::state::Charts;

fn unix_now() -> i64 {
    format::now() as i64
}

// MARK: - Seek

struct Seek {
    entry: gtk::Entry,
    preview: gtk::Label,
    go: gtk::Button,
    cancel: gtk::Button,
    spinner: gtk::Widget,
    working_text: gtk::Label,
    problem: gtk::Box,
    future: gtk::Button,
    now: Cell<i64>,
    working: Cell<bool>,
    /// Changes with each request, so that a cancelled answer is discarded.
    task: Cell<u64>,
    last_text: RefCell<String>,
}

/// Go to a block height, Unix seconds, or a date and time, with a preview before submitting.
pub fn seek_card(page: &Rc<Page>, charts: &Rc<Charts>) {
    let card = page.card("Go to a block or a time", None);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let entry = gtk::Entry::new();
    entry.add_css_class("display-field");
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("Height, Unix seconds or date. Examples: 4000000, 2026-11-07 14:30, height:3000000"));
    row.append(&entry);
    let spinner = loader::small();
    row.append(&spinner);
    let seek: Rc<RefCell<Option<Rc<Seek>>>> = Rc::default();
    let weak_seek = Rc::downgrade(&seek);
    let cancel = lamp::button("Cancel", move || {
        if let Some(seek) = weak_seek.upgrade().and_then(|s| s.borrow().clone()) {
            seek.task.set(seek.task.get() + 1);
            seek.working.set(false);
            seek.update();
        }
    });
    row.append(&cancel);
    let (weak_seek, weak_charts) = (Rc::downgrade(&seek), Rc::downgrade(charts));
    let go = lamp::button("Go", move || {
        if let (Some(seek), Some(charts)) = (weak_seek.upgrade().and_then(|s| s.borrow().clone()), weak_charts.upgrade()) {
            seek.submit(&charts, None);
        }
    });
    row.append(&go);
    card.append(&row);
    let preview = page::paragraph("", &["callout", "secondary"]);
    card.append(&preview);
    let working_text = caption("Locating the block. A date search reads up to 129 blocks around the best match.");
    card.append(&working_text);
    let problem = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.append(&problem);
    let (weak_seek, weak_charts) = (Rc::downgrade(&seek), Rc::downgrade(charts));
    let future = lamp::button("Show the latest block", move || {
        if let (Some(seek), Some(charts)) = (weak_seek.upgrade().and_then(|s| s.borrow().clone()), weak_charts.upgrade()) {
            seek.submit(&charts, Some(api::SeekTarget::Latest));
        }
    });
    card.append(&future);
    card.append(&caption("Dates are in UTC unless you give an offset. A date without a time means midnight. height: and unix: remove doubt about a number. A target in the loaded range is selected at once. Other targets load a range of the current span centred on them."));
    page.append(&card.root);

    let state = Rc::new(Seek {
        entry: entry.clone(),
        preview,
        go,
        cancel,
        spinner: spinner.upcast(),
        working_text,
        problem,
        future,
        now: Cell::new(unix_now()),
        working: Cell::new(false),
        task: Cell::new(0),
        last_text: RefCell::new(String::new()),
    });
    *seek.borrow_mut() = Some(state.clone());

    let (weak, weak_charts) = (Rc::downgrade(&state), Rc::downgrade(charts));
    entry.connect_changed(move |entry| {
        let (Some(seek), Some(charts)) = (weak.upgrade(), weak_charts.upgrade()) else { return };
        let text = entry.text().to_string();
        if seek.last_text.borrow().is_empty() && !text.is_empty() {
            seek.now.set(unix_now());
        }
        *seek.last_text.borrow_mut() = text;
        page::load::clear(&seek.problem);
        seek.future.set_visible(false);
        seek.update_preview(&charts);
        seek.update();
    });
    let (weak, weak_charts) = (Rc::downgrade(&state), Rc::downgrade(charts));
    entry.connect_activate(move |_| {
        if let (Some(seek), Some(charts)) = (weak.upgrade(), weak_charts.upgrade()) {
            seek.submit(&charts, None);
        }
    });
    // The key g moves the focus here.
    let weak_entry = entry.downgrade();
    *charts.focus_seek.borrow_mut() = Some(Rc::new(move || {
        if let Some(entry) = weak_entry.upgrade() {
            entry.grab_focus();
        }
    }));
    // The state lives as long as the page: the listener keeps it.
    let kept = state.clone();
    charts.listen(move |charts| kept.update_preview(charts));
    state.future.set_visible(false);
    state.update();
}

impl Seek {
    fn update(&self) {
        let working = self.working.get();
        self.spinner.set_visible(working);
        self.cancel.set_visible(working);
        self.go.set_visible(!working);
        self.go.set_sensitive(!self.entry.text().trim().is_empty());
        self.working_text.set_visible(working);
        self.problem.set_visible(self.problem.first_child().is_some());
    }

    fn update_preview(&self, charts: &Charts) {
        let tip = charts
            .state
            .borrow()
            .page
            .as_ref()
            .map(|p| p.tip)
            .or_else(|| charts.model.upgrade().and_then(|m| m.state().status.as_ref().map(|s| s.blocks)));
        let text = api::chart_seek_preview(self.entry.text().to_string(), self.now.get(), tip);
        if self.preview.text() != text {
            self.preview.set_text(&text);
        }
    }

    fn set_problem(&self, text: &str) {
        page::load::clear(&self.problem);
        self.problem.append(&page::note(text, true));
    }

    fn submit(self: &Rc<Self>, charts: &Rc<Charts>, forced: Option<api::SeekTarget>) {
        page::load::clear(&self.problem);
        self.future.set_visible(false);
        let target = match forced {
            Some(target) => target,
            None => match api::chart_seek_parse(self.entry.text().to_string(), self.now.get()) {
                Ok(target) => target,
                Err(error) => {
                    self.set_problem(&bridge::describe(&error));
                    self.update();
                    return;
                }
            },
        };
        if let Some((hash, note)) = loaded_match(charts, target) {
            charts.select(&hash);
            charts.state.borrow_mut().seek_note = Some(note);
            charts.changed();
            self.update();
            return;
        }
        let Some(explorer) = charts.model.upgrade().and_then(|m| m.require_explorer().ok()) else { return };
        let span = charts.state.borrow().span.seconds();
        self.working.set(true);
        let task = self.task.get() + 1;
        self.task.set(task);
        self.update();
        let (seek, charts) = (self.clone(), charts.clone());
        glib::spawn_future_local(async move {
            let outcome = bridge::run(async move { explorer.chart_seek(target, span).await }).await;
            if seek.task.get() != task {
                return;
            }
            seek.working.set(false);
            match outcome {
                Ok(outcome) if outcome.future => {
                    seek.set_problem(&outcome.note);
                    seek.future.set_visible(true);
                }
                Ok(outcome) => charts.show(outcome),
                Err(error) => seek.set_problem(&bridge::describe(&error)),
            }
            seek.update();
        });
    }
}

/// A target inside the loaded range, selected without a request. Ported from Carbide's
/// `chart_ui::seek::loaded`.
fn loaded_match(charts: &Charts, target: api::SeekTarget) -> Option<(String, String)> {
    let tip = charts.state.borrow().page.as_ref().filter(|p| !p.samples.is_empty())?.tip;
    if let api::SeekTarget::Number { value } = target
        && value > tip
    {
        return loaded_match(charts, api::SeekTarget::Time { unix: value.min(i64::MAX as u64) as i64 });
    }
    let state = charts.state.borrow();
    let page = state.page.as_ref()?;
    match target {
        api::SeekTarget::Number { value } | api::SeekTarget::Height { value } => {
            let sample = page.samples.iter().find(|s| s.height == value)?;
            Some((sample.hash.clone(), format!("Block {} is in the loaded range.", format::integer(sample.height))))
        }
        api::SeekTarget::Time { unix } => {
            let low = page.samples.iter().map(|s| s.time as i64).min()?;
            let high = page.samples.iter().map(|s| s.time as i64).max()?;
            if !(low..=high).contains(&unix) {
                return None;
            }
            let best = page
                .samples
                .iter()
                .min_by_key(|s| ((s.time as i64 - unix).abs(), s.time, s.height))?;
            let difference = best.time as i64 - unix;
            Some((
                best.hash.clone(),
                format!(
                    "Requested {}. Matched block {} at {}, a difference of {}{} s. This is the nearest of {} loaded blocks.",
                    chart_format::iso(unix),
                    format::integer(best.height),
                    chart_format::iso(best.time as i64),
                    if difference >= 0 { "+" } else { "" },
                    difference,
                    page.samples.len()
                ),
            ))
        }
        api::SeekTarget::Latest => None,
    }
}

// MARK: - Selected block

/// The selected block's exact figures, with controls to move the selection and open it.
pub fn selected_card(page: &Rc<Page>, charts: &Rc<Charts>) {
    let card = page.card("Selected block", None);
    let holder = Holder::new(12);
    card.append(&holder.root);
    page.append(&card.root);
    let page_weak = Rc::downgrade(page);
    charts.listen(move |charts| {
        let Some(page) = page_weak.upgrade() else { return };
        let sample = charts.selected_sample();
        let loading = charts.state.borrow().loading;
        let signature = format!(
            "{:?}|{loading}|{}|{}",
            sample.as_ref().map(|s| (&s.hash, s.mining_average.map(|a| a.samples), s.share100.map(f64::to_bits), s.interval)),
            crate::display::uses_utc(),
            crate::display::shows_names()
        );
        holder.set(signature, |root| match &sample {
            Some(sample) => {
                root.append(&moves(charts, &sample.hash));
                let route = || Some(api::Route::Block { id: sample.hash.clone() });
                root.append(&page::field_row(&page, "Height", &format::integer(sample.height), route(), false));
                root.append(&page::field_row(&page, "Hash", &sample.hash, route(), true));
                root.append(&page::field_row(&page, "Time", &format!("{} ({})", format::date(sample.time), sample.time), None, false));
                root.append(&page::field_row(&page, "Produced by", chart_format::production(sample.production), None, false));
                for (label, value, mono) in rows(sample) {
                    root.append(&page::field_row(&page, &label, &value, None, mono));
                }
                if !sample.linked {
                    root.append(&caption_note(
                        "This block does not link to a loaded parent. Its interval and averages are not available.",
                        false,
                    ));
                }
            }
            None if loading => root.append(&page::text("Loading.", &["secondary"])),
            None => root.append(&page::text("No applicable block in this range. A missing value is not zero.", &["secondary"])),
        });
    });
}

fn rows(sample: &api::ChartSample) -> Vec<(String, String, bool)> {
    let average = |value: Option<api::ChartAverage>, kind: &str| match value {
        None => "Not available. The trailing 100 heights are incomplete, or hold a block without a value.".to_string(),
        Some(value) => format!("{} from {} {kind} blocks", chart_format::exact(value.value), value.samples),
    };
    let mut rows = vec![
        (
            "Interval".to_string(),
            sample.interval.map(chart_format::signed_seconds).unwrap_or_else(|| "Not available. The parent is not loaded or not linked.".into()),
            false,
        ),
        (
            "Mining difficulty".into(),
            sample.mining.map(chart_format::exact).unwrap_or_else(|| "Not a mined block, or no value".into()),
            false,
        ),
        ("Mining MA100".into(), average(sample.mining_average, "mined"), false),
        (
            "Staking difficulty".into(),
            sample.staking.map(chart_format::exact).unwrap_or_else(|| "Not a staked block, or no value".into()),
            false,
        ),
        ("Staking MA100".into(), average(sample.staking_average, "staked"), false),
    ];
    if let Some(target) = &sample.pos_target {
        rows.push(("PoS target".into(), target.clone(), true));
    }
    if let Some(problem) = &sample.problem {
        rows.push(("Problem".into(), problem.clone(), false));
    }
    rows.push(("100 blocks".into(), chart_format::window(&sample.window100, sample.share100), false));
    rows.push(("1,000 blocks".into(), chart_format::window(&sample.window1000, sample.share1000), false));
    rows
}

// MARK: - Table

/// The same samples as a table, for exact reading and copying.
pub fn sample_table(page: &Rc<Page>, charts: &Rc<Charts>) {
    let card = page.card("Samples as a table", None);
    let shown = Rc::new(Cell::new(false));
    let table = Holder::new(0);
    let weak = Rc::downgrade(charts);
    let opened = shown.clone();
    let disclosure = parts::disclosure("", move |open| {
        opened.set(open);
        if let Some(charts) = weak.upgrade() {
            charts.changed();
        }
    });
    disclosure.header.append(&crate::theme::label(crate::theme::font::SMALL, "", ""));
    disclosure.content.append(&table.root);
    card.append(&disclosure.root);
    page.append(&card.root);
    let root = card.root.clone();
    charts.listen(move |charts| {
        let state = charts.state.borrow();
        let samples = state.page.as_ref().map(|p| p.samples.clone()).unwrap_or_default();
        root.set_visible(!samples.is_empty());
        let open = shown.get();
        set_disclosure_title(
            &disclosure,
            &if open { "Hide".to_string() } else { format!("Show all {} blocks of this range", samples.len()) },
        );
        let signature = if open {
            format!("{}|{}|{}", samples.len(), samples.last().map(|s| s.hash.as_str()).unwrap_or(""), crate::display::uses_utc())
        } else {
            "closed".into()
        };
        table.set(signature, |root| {
            if !open {
                return;
            }
            let dash = || "–".to_string();
            let rows: Vec<parts::Row> = samples
                .iter()
                .rev()
                .map(|s| {
                    vec![
                        s.height.to_string(),
                        format::date(s.time),
                        chart_format::production(s.production).to_string(),
                        s.interval.map(chart_format::signed_seconds).unwrap_or_else(dash),
                        s.mining.map(format::difficulty).unwrap_or_else(dash),
                        s.staking.map(format::difficulty).unwrap_or_else(dash),
                        s.share100.map(format::percent).unwrap_or_else(dash),
                        s.share1000.map(format::percent).unwrap_or_else(dash),
                    ]
                })
                .collect();
            root.append(&parts::table(
                &[
                    ("Height", 80),
                    ("Time", 180),
                    ("Produced", 70),
                    ("Interval", 70),
                    ("Mining", 70),
                    ("Staking", 70),
                    ("Staked, 100", 80),
                    ("Staked, 1,000", 90),
                ],
                rows,
                320,
            ));
        });
    });
}
