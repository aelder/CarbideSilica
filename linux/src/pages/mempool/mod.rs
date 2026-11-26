//! The Mempool page (`MempoolView.swift`): unconfirmed transactions in the local node's
//! mempool, as a waiting list or as a table.
//!
//! The mempool is empty most of the time, and each block takes all that waits. Thus the
//! first view is a waiting list on a time record: what waits, how long against the usual
//! wait for a block, and what left with the last blocks. The table shows the same data as
//! the terminal's mempool view.

mod feed;
mod left;
mod list;
mod logic;
mod record;
mod rows;
mod text;
mod waiting;
mod words;

use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::api;
use crate::format;
use crate::launch;
use crate::model::{AppModel, Place, Topic};
use crate::page::{self, Card, Page, PlaceActions, SourceLine, Toggle};
use crate::settings;
use crate::wait_record;
use crate::widgets::{display, lamp, loader};

/// The setting of the second view (`@AppStorage("mempoolShowsList")`).
const SHOWS_LIST: &str = "mempoolShowsList";

fn shows_list() -> bool {
    settings::bool(SHOWS_LIST, false)
}

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>) -> Rc<Page> {
    waiting::install_style();
    let page = Page::scrolling(model);

    let totals = Rc::new(Totals::new());
    page.append(&totals.root);

    let segments = Rc::new(lamp::Segments::new(&["Waiting", "List"], usize::from(shows_list()), |_| {}));
    segments.root.set_halign(gtk::Align::Start);
    page.append(&segments.root);

    // The card of the list has its title. The card of the waiting list does not: its
    // instrument says what it is.
    let list_card = page.card("Unconfirmed transactions", None);
    let waiting_card = Card::new("Waiting for a block", None, false);
    page.cursor.add_card(&waiting_card.title, &waiting_card.root);
    let list_state = States::new();
    let waiting_state = States::new();
    list_card.append(&list_state.stack);
    waiting_card.append(&waiting_state.stack);
    let list = list::MempoolList::new(&page, model);
    list_state.content.append(&list.root);
    let room = waiting::WaitingRoom::new(&page, model);
    waiting_state.content.append(&room.root);
    page.append(&list_card.root);
    page.append(&waiting_card.root);

    let left_card = page.card("Left with recent blocks", None);
    let left = left::LeftList::new(model);
    left_card.append(&left.root);
    page.append(&left_card.root);

    let update: Rc<dyn Fn()> = {
        let (model, page_weak) = (Rc::downgrade(model), Rc::downgrade(&page));
        let (totals, list, room, left) = (totals.clone(), list.clone(), room.clone(), left.clone());
        let (list_root, waiting_root, left_root) = (list_card.root.clone(), waiting_card.root.clone(), left_card.root.clone());
        Rc::new(move || {
            let (Some(model), Some(_page)) = (model.upgrade(), page_weak.upgrade()) else { return };
            let showing_list = shows_list();
            totals.update(&model);
            list_root.set_visible(showing_list);
            waiting_root.set_visible(!showing_list);
            let (failure, loaded, empty) = feed::with(|f| (f.failure.clone(), f.loaded, f.entries.is_empty()));
            let public = model.state().status.as_ref().is_some_and(|s| s.source == api::DataSource::PublicApi);
            let state = match &failure {
                Some(failure) if empty => Shown::Failure(if public {
                    "Only a local node supplies the mempool. The public API does not.".to_string()
                } else {
                    failure.clone()
                }),
                _ if !loaded => Shown::Loading,
                _ => Shown::Content,
            };
            if showing_list {
                let state = if state == Shown::Content && empty { Shown::Empty } else { state };
                list_state.show(&state);
                if state == Shown::Content {
                    list.update();
                }
            } else {
                waiting_state.show(&state);
                if state == Shown::Content {
                    room.update();
                }
            }
            let shows_left = !showing_list && loaded && (failure.is_none() || !empty);
            left_root.set_visible(shows_left);
            if shows_left {
                left.update();
            }
        })
    };

    // The second view: a key of the page, a menu command and a line of the command list.
    let set_list: Rc<dyn Fn(bool)> = {
        let (update, segments, model) = (update.clone(), segments.clone(), Rc::downgrade(model));
        Rc::new(move |on: bool| {
            settings::set_bool(SHOWS_LIST, on);
            segments.set_selected(usize::from(on));
            if !on && let Some(model) = model.upgrade() {
                wait_record::keep(&model, true);
            }
            update();
        })
    };
    {
        let set_list = set_list.clone();
        segments_changed(&segments, move |index| set_list(index == 1));
    }
    {
        let weak = Rc::downgrade(model);
        let mut hooks = page.hooks();
        hooks.refresh = Some(Rc::new(move || {
            if let Some(model) = weak.upgrade() {
                glib::spawn_future_local(async move { feed::refresh(&model).await });
            }
        }));
        hooks.place = PlaceActions {
            mempool_as_list: Some(Toggle { get: Rc::new(shows_list), set: set_list.clone() }),
            ..Default::default()
        };
    }

    // The waiting list sets the present wait against the record of waits.
    if !shows_list() {
        wait_record::keep(model, true);
    }

    // Redraws: after each change of the feed, once a second for the times, and when the
    // chain, the names or the display settings change.
    let owner = page.root.clone();
    {
        let (update, owner_weak) = (update.clone(), owner.downgrade());
        feed::watch(&owner, move || {
            if owner_weak.upgrade().is_some_and(|o| o.is_mapped()) {
                update();
            }
        });
    }
    {
        let update = update.clone();
        page::every(&owner, Duration::from_secs(1), move || update());
    }
    {
        let update = update.clone();
        model.watch(
            &[Topic::Status, Topic::Node, Topic::Connection, Topic::Display, Topic::Names, Topic::HeightDates],
            &owner,
            move |_| update(),
        );
    }
    {
        // On screen again: the record of waits is read while the waiting list shows it.
        let (update, weak) = (update.clone(), Rc::downgrade(model));
        owner.connect_map(move |_| {
            if !shows_list() && let Some(model) = weak.upgrade() {
                wait_record::keep(&model, true);
            }
            update();
        });
    }
    update();

    // The page sets the mempool against the newest blocks, thus it keeps the chain status
    // fresh while it is open, as the dashboard does.
    {
        let (weak_model, weak_root) = (Rc::downgrade(model), owner.downgrade());
        glib::spawn_future_local(async move {
            let mut last: Option<(u64, f64)> = None;
            loop {
                let (Some(model), Some(root)) = (weak_model.upgrade(), weak_root.upgrade()) else { return };
                let generation = model.generation();
                let due = match last {
                    Some((g, at)) => g != generation || format::now() - at >= launch::refresh_interval().as_secs_f64(),
                    None => true,
                };
                let shown = root.is_mapped() && model.place() == Place::Mempool && model.state().path.is_empty();
                if due && shown && model.explorer().is_some() {
                    last = Some((generation, format::now()));
                    model.refresh().await;
                } else if !shown && last.is_some_and(|(g, _)| g == generation) {
                    // On screen again, the page reads at once, as the Swift view's task
                    // starts again when the view appears.
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

/// Work for the whole session: the Swift sidebar keeps the record of the mempool on every
/// page (`MempoolFeed.refresh` every `MempoolFeed.interval`), so that the Mempool page has
/// the last minutes when a person opens it.
pub fn background(model: &Rc<AppModel>) {
    feed::keep(model);
}

fn segments_changed(segments: &Rc<lamp::Segments>, changed: impl Fn(usize) + 'static) {
    // `Segments` takes its action when it is made; the page needs the action of the page,
    // which needs the segments. The keys are the children of the row, in order.
    let changed = Rc::new(changed);
    let mut index = 0;
    let mut child = segments.root.first_child();
    while let Some(key) = child {
        if let Some(button) = key.downcast_ref::<gtk::Button>() {
            let (changed, at) = (changed.clone(), index);
            button.connect_clicked(move |_| changed(at));
        }
        index += 1;
        child = key.next_sibling();
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Shown {
    Failure(String),
    Loading,
    Empty,
    Content,
}

/// What a card of the page shows: the failure, the loader, "empty", or its content.
struct States {
    stack: gtk::Stack,
    failure: gtk::Label,
    content: gtk::Box,
}

impl States {
    fn new() -> Self {
        let stack = gtk::Stack::new();
        stack.set_vhomogeneous(false);
        stack.set_hhomogeneous(false);
        stack.set_interpolate_size(false);
        let failure = page::paragraph("", &["secondary"]);
        stack.add_named(&failure, Some("failure"));
        let loading = loader::progress("Loading the mempool…");
        loading.set_size_request(-1, 200);
        loading.set_hexpand(true);
        stack.add_named(&loading, Some("loading"));
        stack.add_named(&page::text("The mempool is empty.", &["secondary"]), Some("empty"));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        stack.add_named(&content, Some("content"));
        Self { stack, failure, content }
    }

    fn show(&self, shown: &Shown) {
        let name = match shown {
            Shown::Failure(text) => {
                if self.failure.text() != text.as_str() {
                    self.failure.set_text(text);
                }
                "failure"
            }
            Shown::Loading => "loading",
            Shown::Empty => "empty",
            Shown::Content => "content",
        };
        if self.stack.visible_child_name().as_deref() != Some(name) {
            self.stack.set_visible_child_name(name);
        }
    }
}

/// The terminal's mempool summary: the source and the refresh age (`Totals`). The
/// instrument below has the count, the size and the last block.
struct Totals {
    root: gtk::Box,
    source: SourceLine,
    refresh: gtk::Label,
}

impl Totals {
    fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.add_css_class("callout");
        let source = SourceLine::new();
        root.append(&source.root);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let name = display::name_text("Refresh");
        name.set_valign(gtk::Align::Baseline);
        row.append(&name);
        let refresh = page::text("", &["semibold", "tabular"]);
        refresh.set_valign(gtk::Align::Baseline);
        refresh.set_single_line_mode(true);
        row.append(&refresh);
        root.append(&row);
        Self { root, source, refresh }
    }

    fn update(&self, model: &Rc<AppModel>) {
        let (provenance, last, refreshing) = feed::with(|f| (f.provenance.clone(), f.last_refresh, f.refreshing));
        let chain = model.current_chain();
        self.source.set(provenance.as_ref(), Some(&chain), true);
        let text = match last {
            None => "every 2 s, waiting for the first refresh".to_string(),
            Some(_) if refreshing => "every 2 s, refreshing".to_string(),
            Some(last) => format!("every 2 s, updated {} s ago", ((format::now() - last) as i64).max(0)),
        };
        if self.refresh.text() != text {
            self.refresh.set_text(&text);
        }
    }
}
