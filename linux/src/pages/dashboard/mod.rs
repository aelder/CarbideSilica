//! The Dashboard: chain status, the chain pulse, recent blocks and recent confirmed
//! transactions, refreshed while the page is on screen (`DashboardView.swift`).

mod chain_pulse;
mod feed;
mod lists;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::api;
use crate::format;
use crate::launch;
use crate::model::{AppModel, Place, Topic};
use crate::page::load::clear;
use crate::page::{self, Page, PlaceActions, SourceLine, Toggle};
use crate::settings::{self, key};
use crate::theme::tone;
use crate::widgets::display::{self, reserve_width};
use crate::widgets::icons::{self, Icon};
use crate::widgets::{lamp, loader};
use chain_pulse::ChainPulse;
use feed::Feed;

/// Keep the newest block selected and in view, as the terminal's "Stream Blocks" row.
fn streaming() -> bool {
    settings::bool(key::DASHBOARD_STREAMS_BLOCKS, true)
}

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>) -> Rc<Page> {
    let page = Page::scrolling(model);
    // The page shows "Connecting…", the dashboard, or the state of no connection.
    let views = gtk::Stack::new();
    views.set_vexpand(true);
    let connecting = loader::progress("Connecting…");
    views.add_named(&connecting, Some("connecting"));
    let not_connected = gtk::Box::new(gtk::Orientation::Vertical, 0);
    views.add_named(&not_connected, Some("not-connected"));
    let scroller = page.root.clone();
    views.add_named(&scroller, Some("page"));
    let page = Rc::new(Page {
        model: page.model.clone(),
        root: views.clone().upcast(),
        content: page.content.clone(),
        scroller: page.scroller.clone(),
        cursor: page.cursor.clone(),
        hooks: Default::default(),
    });

    let feed = Feed::new();
    let status = StatusLine::new();
    page.append(&status.root);
    let stored_note = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&stored_note);
    let sync_banner = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&sync_banner);
    let error_note = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&error_note);

    let pulse_card = page.card("Chain pulse", None);
    let pulse = ChainPulse::new(model);
    pulse_card.append(&pulse.root);
    page.append(&pulse_card.root);

    let lists = lists::Lists::new(&page, &feed);
    page.append(&lists.root);

    // Stream blocks: a menu command and a check box.
    let set_stream: Rc<dyn Fn(bool)> = {
        let (lists, feed, model) = (lists.clone(), feed.clone(), Rc::downgrade(model));
        Rc::new(move |on: bool| {
            settings::set_bool(key::DASHBOARD_STREAMS_BLOCKS, on);
            lists.set_streaming(on);
            if on && let Some(model) = model.upgrade() {
                let first = model.state().blocks.first().map(|b| b.hash.clone());
                feed.select_block(first, &lists);
            }
            if let Some(model) = model.upgrade() {
                model.notify(Topic::Display);
            }
        })
    };
    lists.on_stream(set_stream.clone());
    {
        let weak = Rc::downgrade(model);
        let mut hooks = page.hooks();
        hooks.refresh = Some(Rc::new(move || {
            if let Some(model) = weak.upgrade() {
                glib::spawn_future_local(async move { model.refresh().await });
            }
        }));
        hooks.place = PlaceActions {
            stream_blocks: Some(Toggle { get: Rc::new(streaming), set: set_stream.clone() }),
            ..Default::default()
        };
    }

    // Selecting an older block stops the stream, as moving off the terminal's "Stream
    // Blocks" row does. The chain pulse draws the selected block as a hovered one.
    {
        let (feed, pulse, lists, model, set_stream) = (feed.clone(), pulse.clone(), lists.clone(), Rc::downgrade(model), set_stream.clone());
        page.cursor.on_select(move |id| {
            let Some(id) = id else { return };
            let Some(model) = model.upgrade() else { return };
            let is_block = lists.has_block(id);
            if is_block {
                feed.selected_block.replace(Some(id.to_string()));
                pulse.set_selected(Some(id.to_string()));
                let first = model.state().blocks.first().map(|b| b.hash.clone());
                if Some(id.to_string()) != first && streaming() {
                    set_stream(false);
                }
            } else {
                feed.selected_transaction.replace(Some(id.to_string()));
            }
        });
    }

    // A stored answer for a short time is usual: at the start, and while the app
    // reconciles new blocks. The source line says it. The large note comes only when the
    // state stays, so that the page does not move down and up again.
    let stored_since: Rc<Cell<Option<f64>>> = Rc::default();
    let last_first = Rc::new(std::cell::RefCell::new(None::<String>));
    let update = {
        let page = Rc::downgrade(&page);
        let (views, not_connected, status, stored_note, sync_banner, error_note, pulse, pulse_card, lists, feed) = (
            views.clone(), not_connected.clone(), status.clone(), stored_note.clone(), sync_banner.clone(),
            error_note.clone(), pulse.clone(), pulse_card.root.clone(), lists.clone(), feed.clone(),
        );
        let stored_since = stored_since.clone();
        move |model: &Rc<AppModel>| {
            let Some(_page) = page.upgrade() else { return };
            let state = model.state();
            let Some(chain) = state.status.clone() else {
                let error = state.status_error.clone();
                drop(state);
                match error {
                    Some(message) => {
                        fill_not_connected(model, &not_connected, &message);
                        views.set_visible_child_name("not-connected");
                    }
                    None => views.set_visible_child_name("connecting"),
                }
                return;
            };
            let status_error = state.status_error.clone();
            let blocks = state.blocks.clone();
            let waiting = state.node.as_ref().and_then(|n| n.mempool_count);
            drop(state);
            views.set_visible_child_name("page");
            let sync = model.sync_fraction();
            let retained = chain.provenance.retained;
            status.set(model, &chain);

            // The note of a stored status, after 45 seconds of it.
            if retained {
                let since = stored_since.get().unwrap_or_else(format::now);
                stored_since.set(Some(since));
                clear(&stored_note);
                if format::now() - since >= 45.0 {
                    stored_note.append(&page::provenance_note(&chain.provenance));
                }
            } else {
                stored_since.set(None);
                clear(&stored_note);
            }
            clear(&sync_banner);
            if let Some(fraction) = sync {
                sync_banner.append(&sync_banner_view(model, fraction, chain.tip_time));
            }
            clear(&error_note);
            if let Some(error) = status_error {
                error_note.append(&page::note(&error, true));
            }
            // An empty part takes no room in the column.
            for part in [&stored_note, &sync_banner, &error_note] {
                part.set_visible(part.first_child().is_some());
            }
            pulse_card.set_visible(!blocks.is_empty());
            let live = sync.is_none() && !retained;
            pulse.set(&blocks, live, if retained { None } else { waiting }, feed.selected_block.borrow().clone());
            lists.set_blocks(model, &blocks, &feed);
            // The newest block is selected while the stream is on.
            let first = blocks.first().map(|b| b.hash.clone());
            if streaming() && *last_first.borrow() != first {
                feed.select_block(first.clone(), &lists);
            }
            *last_first.borrow_mut() = first;
        }
    };
    update(model);
    let owner = page.root.clone();
    model.watch(&[Topic::Status, Topic::Node, Topic::Connection, Topic::Display], &owner, update.clone());
    // The stored note waits 45 seconds; the status line counts seconds.
    {
        let (weak, status, update) = (Rc::downgrade(model), status.clone(), update.clone());
        let ticks = Rc::new(Cell::new(0u32));
        page::every(&owner, Duration::from_secs(1), move || {
            let Some(model) = weak.upgrade() else { return };
            status.tick(&model);
            ticks.set(ticks.get() + 1);
            if stored_since.get().is_some() && ticks.get() % 5 == 0 {
                update(&model);
            }
        });
    }

    // While the page is on screen: refresh the chain, then the transactions, then wait.
    // A new connection starts the loop again at once.
    {
        let (weak_model, weak_root, feed, lists) = (Rc::downgrade(model), owner.downgrade(), feed.clone(), lists.clone());
        glib::spawn_future_local(async move {
            let mut last: Option<(u64, f64)> = None;
            loop {
                let (Some(model), Some(root)) = (weak_model.upgrade(), weak_root.upgrade()) else { return };
                let generation = model.generation();
                let due = match last {
                    Some((g, at)) => g != generation || format::now() - at >= launch::refresh_interval().as_secs_f64(),
                    None => true,
                };
                let shown = root.is_mapped() && model.place() == Place::Dashboard && model.state().path.is_empty();
                if due && shown && model.explorer().is_some() {
                    last = Some((generation, format::now()));
                    model.refresh().await;
                    feed.update(&model, &lists).await;
                } else if !shown && last.is_some_and(|(g, _)| g == generation) {
                    // Off screen, the loop waits. On screen again, it reads at once, as the
                    // Swift view's task starts again when the view appears.
                    last = Some((generation, 0.0));
                }
                drop(model);
                drop(root);
                glib::timeout_future(Duration::from_millis(500)).await;
            }
        });
    }
    page
}

/// The terminal's status line: tip against the longest chain, the age of the last block,
/// the node and API state, and the peer count.
#[derive(Clone)]
struct StatusLine {
    root: gtk::Box,
    source: Rc<SourceLine>,
    row: gtk::Box,
    last_block: Rc<std::cell::RefCell<Option<(gtk::Label, Option<u64>)>>>,
}

impl StatusLine {
    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let source = Rc::new(SourceLine::new());
        source.root.add_css_class("callout");
        root.append(&source.root);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        row.add_css_class("callout");
        root.append(&row);
        Self { root, source, row, last_block: Default::default() }
    }

    fn set(&self, model: &Rc<AppModel>, status: &api::ChainStatus) {
        let chain = model.current_chain();
        self.source.set(Some(&status.provenance), Some(&chain), false);
        clear(&self.row);
        // True when the node is at the tip and the data is not a stored observation.
        let live = model.sync_fraction().is_none() && !status.provenance.retained;
        let node_on = status.source == api::DataSource::LocalDaemon
            && model.state().status_error.is_none()
            && !status.provenance.retained;
        let behind = match status.longest_chain {
            None => "longest chain unknown".to_string(),
            Some(longest) => {
                let delta = longest as i64 - status.blocks as i64;
                if delta <= 0 {
                    "at the longest chain".to_string()
                } else {
                    format!("{} behind {}", format::integer(delta as u64), format::integer(longest))
                }
            }
        };
        // When the node is not at the tip, the line has more items: no fixed width.
        self.row.append(&item("Height", &format::integer(status.blocks), Some(&behind), live.then_some("0,000,000  at the longest chain")));
        // The chain pulse shows the time since the last block. When the node is not at the
        // tip, the pulse does not, thus the line shows it.
        *self.last_block.borrow_mut() = None;
        if !live {
            let value = status.tip_time.map(|t| format::age(t, format::now())).unwrap_or_else(|| "unknown".into());
            let widget = item("Last block", &value, None, None);
            let label = widget.last_child().and_then(|c| c.first_child()).and_downcast::<gtk::Label>();
            if let Some(label) = label {
                *self.last_block.borrow_mut() = Some((label, status.tip_time));
            }
            self.row.append(&widget);
        }
        // The usual state has no label. The line says when the node is off.
        if !node_on {
            self.row.append(&state_item("Node", false, false));
        }
        self.row.append(&state_item(
            "Public API",
            model.state().connection.as_ref().is_some_and(|c| c.api_endpoint.is_some()),
            status.source == api::DataSource::PublicApi,
        ));
        self.row.append(&item("Peers", &status.connections.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into()), None, Some("00")));
        if let Some(difficulty) = status.difficulty {
            self.row.append(&item("Difficulty", &format::difficulty(difficulty), None, Some("0.00 T")));
        }
        if let Some(version) = &status.version {
            self.row.append(&item("Version", version, None, None));
        }
    }

    /// The age of the last block counts seconds.
    fn tick(&self, _model: &Rc<AppModel>) {
        if let Some((label, Some(time))) = self.last_block.borrow().as_ref() {
            label.set_text(&format::age(*time, format::now()));
        }
    }
}

/// `reserve` is the widest text of the value and its detail. The item keeps that width, so
/// that the items after it do not move when the text gets longer or shorter.
fn item(label: &str, value: &str, detail: Option<&str>, reserve: Option<&str>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = display::name_text(label);
    name.set_valign(gtk::Align::Baseline);
    row.append(&name);
    let values = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    values.set_valign(gtk::Align::Baseline);
    let figure = gtk::Label::new(Some(value));
    figure.add_css_class("semibold");
    figure.add_css_class("tabular");
    figure.set_valign(gtk::Align::Baseline);
    values.append(&figure);
    if let Some(detail) = detail {
        let text = gtk::Label::new(Some(detail));
        text.add_css_class("secondary");
        text.set_valign(gtk::Align::Baseline);
        values.append(&text);
    }
    if let Some(reserve) = reserve {
        let probe = gtk::Label::new(None);
        probe.add_css_class("callout");
        probe.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(reserve)));
        let (_, natural, _, _) = probe.measure(gtk::Orientation::Horizontal, -1);
        values.set_size_request(natural, -1);
    }
    row.append(&values);
    let _ = reserve_width;
    row
}

/// State is a shape, a colour and a word.
fn state_item(label: &str, on: bool, in_use: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let mark = icons::icon(if on { Icon::Dot } else { Icon::Ring }, 8, if on { tone::GOOD } else { tone::LABEL });
    row.append(&mark);
    let name = display::name_text(label);
    name.set_valign(gtk::Align::Baseline);
    row.append(&name);
    let word = gtk::Label::new(Some(if on { if in_use { "in use" } else { "on" } } else { "off" }));
    word.add_css_class("semibold");
    word.set_valign(gtk::Align::Baseline);
    row.append(&word);
    row
}

/// Says plainly that the dashboard is showing history while the node catches up.
fn sync_banner_view(model: &Rc<AppModel>, fraction: f64, tip_time: Option<u64>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("panel");
    row.set_margin_top(0);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    inner.set_margin_top(14);
    inner.set_margin_bottom(14);
    inner.set_margin_start(14);
    inner.set_margin_end(14);
    inner.set_hexpand(true);
    let progress = loader::fraction_bar(fraction);
    progress.set_size_request(40, -1);
    progress.set_valign(gtk::Align::Center);
    inner.append(&progress);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let title = page::text(
        &format!("The node is not synchronized. It validated {} of the chain.", format::percent1(fraction)),
        &["semibold"],
    );
    text.append(&title);
    let detail = match tip_time {
        None => "This data is not current.".to_string(),
        Some(time) => format!(
            "This data is from {}. The node cannot find newer blocks, addresses or identities at this time.",
            format::month_year(time)
        ),
    };
    text.append(&page::paragraph(&detail, &["callout", "secondary"]));
    inner.append(&text);
    let weak = Rc::downgrade(model);
    inner.append(&lamp::button("Show progress", move || {
        if let Some(model) = weak.upgrade() {
            model.show(Place::Node);
        }
    }));
    row.append(&inner);
    row
}

/// Shown when neither the local node nor an allowed API answered.
fn fill_not_connected(model: &Rc<AppModel>, holder: &gtk::Box, message: &str) {
    clear(holder);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
    column.set_halign(gtk::Align::Center);
    column.set_valign(gtk::Align::Center);
    column.set_vexpand(true);
    let mark = icons::icon(Icon::Disconnected, 32, tone::LABEL);
    mark.set_halign(gtk::Align::Center);
    column.append(&mark);
    let title = gtk::Label::new(Some("Not connected to Verus"));
    title.add_css_class("dialog-title");
    column.append(&title);
    let text = page::paragraph(message, &["value"]);
    text.set_xalign(0.5);
    text.set_justify(gtk::Justification::Center);
    text.set_max_width_chars(70);
    column.append(&text);
    if let Some(connection) = model.state().connection.clone() {
        let detail = if connection.config_found {
            format!("The app tried the node at {}.", connection.daemon_url)
        } else {
            format!("The app found no node configuration for {}. It tried {}.", connection.chain, connection.daemon_url)
        };
        let line = page::paragraph(&detail, &["callout", "secondary"]);
        line.set_xalign(0.5);
        line.set_justify(gtk::Justification::Center);
        column.append(&line);
    }
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    keys.set_halign(gtk::Align::Center);
    let weak = Rc::downgrade(model);
    keys.append(&lamp::button("Try Again", move || {
        if let Some(model) = weak.upgrade() {
            glib::spawn_future_local(async move { model.refresh().await });
        }
    }));
    if model.state().connection.as_ref().is_none_or(|c| c.api_endpoint.is_none()) {
        let weak = Rc::downgrade(model);
        let api = lamp::button("Use Public API", move || {
            if let Some(model) = weak.upgrade() {
                model.enable_public_api();
            }
        });
        api.set_tooltip_text(Some(&format!(
            "Send requests to {} when the local node cannot answer.",
            key::DEFAULT_API
        )));
        keys.append(&api);
    }
    keys.append(&lamp::button("Settings…", move || {
        if let Some(app) = gtk::gio::Application::default() {
            app.activate_action("settings", None);
        }
    }));
    column.append(&keys);
    holder.append(&column);
}
