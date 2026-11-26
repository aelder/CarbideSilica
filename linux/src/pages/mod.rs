//! The registry of pages. Each place and each object has its own module folder below this
//! one, and a page is finished by changing only the files in its own folder.
//!
//! This file is shared. It maps each place and each route to the `build` function of its
//! module, with the fixed signatures below. It does not change when a page is ported:
//! the title, the keys, the commands and the background work of a page are in the page's
//! own module (see `linux/PORT.md`).

pub mod address;
pub mod block;
pub mod bookmarks;
pub mod charts;
pub mod currencies;
pub mod currency;
pub mod dashboard;
pub mod estimate;
pub mod identity;
pub mod marketplace;
pub mod mempool;
pub mod node;
pub mod sources;
pub mod transaction;

use std::rc::Rc;

use crate::bridge::api::Route;
use crate::model::{AppModel, Place};
use crate::page::Page;

/// A place in the sidebar (`SidebarPlace` and the groups of `Sidebar`).
pub struct PlaceSpec {
    pub place: Place,
    /// The title in the Go menu, the command list and the path bar.
    pub title: &'static str,
    /// The legend of the sidebar key.
    pub key_title: &'static str,
    /// The group of the sidebar: Watch, Explore, Library or Node.
    pub group: &'static str,
    /// The digit of Ctrl-1 to Ctrl-8 (Command-1 to Command-8 on macOS).
    pub digit: char,
    /// The key of the terminal, shown in the command list.
    pub terminal_key: &'static str,
    pub build: fn(&Rc<AppModel>) -> Rc<Page>,
}

/// The places in the sidebar, in order.
pub const PLACES: [PlaceSpec; 8] = [
    PlaceSpec { place: Place::Dashboard, title: "Dashboard", key_title: "Dashboard", group: "Watch", digit: '1', terminal_key: "d", build: dashboard::build },
    PlaceSpec { place: Place::Mempool, title: "Mempool", key_title: "Mempool", group: "Watch", digit: '2', terminal_key: "m", build: mempool::build },
    PlaceSpec { place: Place::Charts, title: "Charts", key_title: "Charts", group: "Watch", digit: '3', terminal_key: "F9", build: charts::build },
    PlaceSpec { place: Place::Currencies, title: "Currencies", key_title: "Currencies", group: "Explore", digit: '4', terminal_key: "c", build: currencies::build },
    PlaceSpec { place: Place::Marketplace, title: "Marketplace", key_title: "Marketplace", group: "Explore", digit: '5', terminal_key: "o", build: marketplace::build },
    PlaceSpec { place: Place::Bookmarks, title: "Bookmarks", key_title: "Bookmarks", group: "Library", digit: '6', terminal_key: "F6", build: bookmarks::build },
    PlaceSpec { place: Place::Node, title: "Node and Peers", key_title: "Node", group: "Node", digit: '7', terminal_key: "`", build: node::build },
    PlaceSpec { place: Place::Sources, title: "Sources", key_title: "Sources", group: "Node", digit: '8', terminal_key: "F2", build: sources::build },
];

pub fn spec(place: Place) -> &'static PlaceSpec {
    PLACES.iter().find(|spec| spec.place == place).expect("each place has a spec")
}

/// The page of an object.
pub fn build_route(model: &Rc<AppModel>, route: &Route) -> Rc<Page> {
    match route {
        Route::Block { id } => block::build(model, id),
        Route::Transaction { txid } => transaction::build(model, txid),
        Route::Identity { name } => identity::build(model, name),
        Route::Currency { name } => currency::build(model, name),
        Route::Address { address } => address::build(model, address),
        Route::Currencies => currencies::build(model),
        Route::Estimate { query } => estimate::build(model, query),
    }
}

/// Work that runs for the whole session from the start, whatever page is open. The Swift
/// app keeps the record of the mempool on every page, for example. Each page module owns
/// its `background` function; this list only calls them.
pub fn start_background(model: &Rc<AppModel>) {
    mempool::background(model);
}
