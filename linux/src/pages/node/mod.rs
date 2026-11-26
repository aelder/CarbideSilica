//! The Node and Peers page: what the local node is doing, how far it has validated, how
//! fast, and who it is talking to. Port of NodeView.swift, PeerControls.swift, PeerMap.swift,
//! PeerOutlineMap.swift and NodeDiagrams.swift (`shared/branches.rs`) in
//! `macos/Sources/CarbideSilica/Views`.

mod columns;
mod commands;
mod detail;
mod feed;
mod geometry;
mod map;
mod peers;
mod pulses;
mod sync;


use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::api;
use crate::format;
use crate::model::{AppModel, Place, Topic};
use crate::page::load::clear;
use crate::page::{self, Card, Page, PlaceActions, SourceLine, Toggle};
use crate::settings;
use crate::shared::branches;
use crate::theme::{draw, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::loader;
use commands::{window_of, ControlsCard, PeerCommands};
use feed::PeerFeed;

/// The setting of the peer map, as `@AppStorage("peerMapShown")`.
const PEER_MAP_SHOWN: &str = "peerMapShown";

fn shows_map() -> bool {
    settings::bool(PEER_MAP_SHOWN, true)
}

/// True while the node is more than 20 blocks behind the longest chain.
pub fn is_syncing(node: &api::NodeStatus) -> bool {
    node.longest_chain.is_some_and(|longest| longest > node.blocks + 20)
}

/// Average speed over roughly the last half minute.
fn recent_speed(model: &AppModel) -> Option<f64> {
    let state = model.state();
    let recent: Vec<f64> = state.speeds.iter().rev().take(15).map(|s| s.blocks_per_second).collect();
    (!recent.is_empty()).then(|| recent.iter().sum::<f64>() / recent.len() as f64)
}

/// The mark of a server (`server.rack`), for the state of no connection.
fn server_mark() -> Canvas {
    let mark = Canvas::new(34, 34, |_, snapshot, w, h| {
        for row in 0..3 {
            let y = h * 0.14 + row as f32 * h * 0.26;
            draw::stroke(snapshot, &draw::rect_path(w * 0.12, y, w * 0.76, h * 0.2), tone::LABEL, 1.4);
            draw::fill(snapshot, &draw::ellipse_path(w * 0.22, y + h * 0.07, h * 0.06, h * 0.06), tone::LABEL);
            draw::rule(snapshot, w * 0.5, y + h * 0.1, w * 0.78, y + h * 0.1, tone::LABEL, 1.2, &[]);
        }
    });
    mark.set_halign(gtk::Align::Center);
    mark
}

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>) -> Rc<Page> {
    let page = Page::scrolling(model);
    // The page shows "Connecting to the node…", the node, or the state of no connection.
    let views = gtk::Stack::new();
    views.set_vexpand(true);
    views.add_named(&loader::progress("Connecting to the node…"), Some("connecting"));
    let unavailable = gtk::Box::new(gtk::Orientation::Vertical, 10);
    unavailable.set_halign(gtk::Align::Center);
    unavailable.set_valign(gtk::Align::Center);
    unavailable.append(&server_mark());
    let title = gtk::Label::new(Some("No connection to the node"));
    title.add_css_class("dialog-title");
    unavailable.append(&title);
    let reason = page::paragraph("", &["secondary"]);
    reason.set_xalign(0.5);
    reason.set_justify(gtk::Justification::Center);
    reason.set_max_width_chars(70);
    unavailable.append(&reason);
    views.add_named(&unavailable, Some("unavailable"));
    views.add_named(&page.root, Some("page"));
    let page = Rc::new(Page {
        model: page.model.clone(),
        root: views.clone().upcast(),
        content: page.content.clone(),
        scroller: page.scroller.clone(),
        cursor: page.cursor.clone(),
        hooks: Default::default(),
    });

    let feed = feed::shared();
    let commands = PeerCommands::new(model, &feed);
    let open: Rc<dyn Fn(i64)> = {
        let (model, feed, commands, root) = (Rc::downgrade(model), feed.clone(), commands.clone(), page.root.downgrade());
        Rc::new(move |id| {
            let Some(model) = model.upgrade() else { return };
            let parent = root.upgrade().and_then(|r| window_of(&r));
            detail::show(&model, &feed, &commands, id, parent);
        })
    };

    let source = SourceLine::new();
    source.root.add_css_class("callout");
    page.append(&source.root);
    let provenance = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&provenance);
    let sync_card = sync::SyncCard::new(&page);
    page.append(&sync_card.root);
    let speed_card = sync::SpeedCard::new(&page);
    page.append(&speed_card.root);
    let map_card = map::MapCard::new(&page, &feed, open.clone());
    page.append(&map_card.card.root);
    let peers_card = peers::PeersCard::new(&page, &feed, open.clone());
    page.append(&peers_card.card.root);
    let controls = ControlsCard::new(&page, &commands);
    page.append(&controls.card.root);
    // Peers of the public API: the card says why there are none.
    let no_peers = Card::new("Peers", None, true);
    let no_peers_text = page::paragraph("", &["secondary"]);
    no_peers.append(&no_peers_text);
    page.append(&no_peers.root);
    let branches_card = branches::branches_card(&page);
    page.append(&branches_card.root);

    let update = {
        let (views, reason, provenance) = (views.clone(), reason.clone(), provenance.clone());
        let (sync_card, speed_card, map_card, peers_card, controls) = (sync_card.clone(), speed_card.clone(), map_card.clone(), peers_card.clone(), controls.clone());
        let (no_peers_root, commands, feed, page_weak) = (no_peers.root.clone(), commands.clone(), feed.clone(), Rc::downgrade(&page));
        let source = Rc::new(source);
        Rc::new(move |model: &Rc<AppModel>| {
            let Some(page) = page_weak.upgrade() else { return };
            let node = model.state().node.clone();
            let Some(node) = node else {
                let error = { let s = model.state(); s.node_error.clone().or_else(|| s.status_error.clone()) };
                match error {
                    Some(error) => {
                        reason.set_text(&error);
                        views.set_visible_child_name("unavailable");
                    }
                    None => views.set_visible_child_name("connecting"),
                }
                return;
            };
            views.set_visible_child_name("page");
            source.set(Some(&node.provenance), None, true);
            clear(&provenance);
            if node.provenance.retained {
                provenance.append(&page::provenance_note(&node.provenance));
            }
            provenance.set_visible(node.provenance.retained);
            sync_card.update(&node, recent_speed(model));
            speed_card.root.set_visible(is_syncing(&node));
            if is_syncing(&node) {
                speed_card.update(&model.state().speeds);
            }
            let peers_ok = node.peers_error.is_none();
            map_card.card.root.set_visible(peers_ok && shows_map());
            peers_card.card.root.set_visible(peers_ok);
            controls.card.root.set_visible(peers_ok);
            no_peers_root.set_visible(!peers_ok);
            if peers_ok {
                if shows_map() {
                    map_card.update(&node.peers);
                }
                peers_card.update(&node);
                controls.set_banned(feed.banned.borrow().len(), &commands);
            } else {
                no_peers_text.set_text(&if node.source == api::DataSource::PublicApi {
                    "Only a local node supplies peer data. The public API does not.".to_string()
                } else {
                    node.peers_error.clone().unwrap_or_default()
                });
            }
            set_hooks(&page, model, &commands, &feed, peers_ok);
        })
    };
    update(model);
    {
        let update = update.clone();
        model.watch(&[Topic::Node, Topic::Connection, Topic::Status, Topic::Display], &page.root, move |model| update(model));
    }

    // While the page is on screen: the totals once per visit, then the node, the response
    // times and the ban list every two seconds. The locations when the peers change.
    {
        let (weak_model, weak_root, feed, update) = (Rc::downgrade(model), page.root.downgrade(), feed.clone(), update.clone());
        glib::spawn_future_local(async move {
            let mut visit: Option<u64> = None;
            let mut last_poll = 0.0f64;
            let mut located: Option<(u64, Vec<String>)> = None;
            loop {
                let (Some(model), Some(root)) = (weak_model.upgrade(), weak_root.upgrade()) else { return };
                let generation = model.generation();
                let shown = root.is_mapped() && model.place() == Place::Node && model.state().path.is_empty();
                if !shown {
                    // A new visit reads the totals again, as the Swift view's task does.
                    visit = None;
                } else if model.explorer().is_some() {
                    if visit != Some(generation) {
                        visit = Some(generation);
                        feed.load_totals(&model).await;
                        last_poll = 0.0;
                    }
                    if format::now() - last_poll >= 2.0 {
                        last_poll = format::now();
                        model.refresh_node().await;
                        let peers = model.state().node.as_ref().map(|n| n.peers.clone()).unwrap_or_default();
                        feed.note(&peers);
                        feed.load_bans(&model).await;
                        let mut addresses: Vec<String> = peers.iter().map(|p| p.address.clone()).collect();
                        addresses.sort();
                        let key = (model.generation(), addresses.clone());
                        if located.as_ref() != Some(&key) {
                            located = Some(key);
                            feed.load_locations(&model, addresses).await;
                        }
                        update(&model);
                    }
                }
                drop(model);
                drop(root);
                glib::timeout_future(Duration::from_millis(250)).await;
            }
        });
    }
    page
}

/// The refresh, the peer commands and the map toggle of the place (`PlaceActions`).
fn set_hooks(page: &Rc<Page>, model: &Rc<AppModel>, commands: &Rc<PeerCommands>, feed: &Rc<PeerFeed>, peers_ok: bool) {
    let root = page.root.downgrade();
    let window = move || root.upgrade().and_then(|r| window_of(&r));
    let mut hooks = page.hooks();
    let (weak, f) = (Rc::downgrade(model), feed.clone());
    hooks.refresh = Some(Rc::new(move || {
        let Some(model) = weak.upgrade() else { return };
        let feed = f.clone();
        glib::spawn_future_local(async move {
            model.refresh_node().await;
            feed.load_bans(&model).await;
            model.notify(Topic::Node);
        });
    }));
    let banned = feed.banned.borrow().len();
    hooks.place = PlaceActions {
        add_node: peers_ok.then(|| {
            let (c, window) = (commands.clone(), window.clone());
            Rc::new(move || {
                if let Some(window) = window() {
                    c.start_add_node(&window);
                }
            }) as page::Action
        }),
        ping_all: peers_ok.then(|| {
            let c = commands.clone();
            Rc::new(move || c.ping_all()) as page::Action
        }),
        clear_bans: (banned > 0).then(|| {
            let (c, window, f) = (commands.clone(), window.clone(), feed.clone());
            Rc::new(move || {
                if let Some(window) = window() {
                    c.confirm(&window, commands::clear_bans_pending(f.banned.borrow().len()));
                }
            }) as page::Action
        }),
        show_map: Some(Toggle {
            get: Rc::new(shows_map),
            set: {
                let model = Rc::downgrade(model);
                Rc::new(move |on| {
                    settings::set_bool(PEER_MAP_SHOWN, on);
                    if let Some(model) = model.upgrade() {
                        model.notify(Topic::Display);
                    }
                })
            },
        }),
        ..Default::default()
    };
}
