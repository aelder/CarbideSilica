//! The currency browser's list, filters and selection, kept for the session so that they
//! survive going back and visits to other places in the sidebar (`CurrencyBrowserState`,
//! `CurrencySort` and `LaunchState.word` in `CurrencyStores.swift`).
//!
//! This is a stand-in inside the Currencies folder. `PORT.md` puts `CurrencyBrowserState`
//! in `shared/currency_stores.rs`, which another engineer owns; when it is there, this file
//! goes and the page uses it. The page is the only user of the state.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use crate::bridge::api::{self, CurrencyFilter, CurrencyListing, LaunchState, Provenance};

/// The columns that can give the order of the currency list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurrencySort {
    Name,
    Reserves,
    Supply,
    Start,
}

impl CurrencySort {
    /// The name of the column (`rawValue`).
    pub fn title(self) -> &'static str {
        match self {
            CurrencySort::Name => "Currency",
            CurrencySort::Reserves => "Reserves",
            CurrencySort::Supply => "Supply",
            CurrencySort::Start => "Start",
        }
    }
}

/// The launch states in the order of the keys (`LaunchState.all`).
pub const LAUNCH_STATES: [LaunchState; 4] =
    [LaunchState::Launched, LaunchState::Prelaunch, LaunchState::Failed, LaunchState::Unknown];

/// The launch state as a word (`LaunchState.word`).
pub fn launch_word(state: LaunchState) -> &'static str {
    match state {
        LaunchState::Launched => "Launched",
        LaunchState::Prelaunch => "Prelaunch",
        LaunchState::Failed => "Failed launch (refunding)",
        LaunchState::Unknown => "Unknown",
    }
}

pub struct BrowserState {
    pub rows: Vec<CurrencyListing>,
    pub source: Option<String>,
    /// Where the list came from: live, or stored from the last answer.
    pub provenance: Option<Provenance>,
    pub failure: Option<String>,
    pub loading: bool,
    /// The connection the rows came from. A new connection loads a new list.
    pub generation: Option<u64>,
    pub filter: CurrencyFilter,
    /// The currency opened last, so the list shows where the user was.
    pub selected: Option<String>,
    pub show_flags: bool,
    /// The column that gives the order of the list. None is the order of the source.
    pub sort: Option<CurrencySort>,
    pub descending: bool,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            source: None,
            provenance: None,
            failure: None,
            loading: false,
            generation: None,
            filter: api::default_currency_filter(),
            selected: None,
            show_flags: false,
            sort: None,
            descending: false,
        }
    }
}

/// The state and the pages that show it. The Currencies place and a `currencies` search
/// both show the browser; a change in one shows in the other.
#[derive(Default)]
pub struct Store {
    pub state: RefCell<BrowserState>,
    listeners: RefCell<Vec<Weak<dyn Fn()>>>,
}

impl Store {
    /// Calls `listener` after each change, while the caller keeps it.
    pub fn listen(&self, listener: &Rc<dyn Fn()>) {
        self.listeners.borrow_mut().push(Rc::downgrade(listener));
    }

    /// Tells each page that the state changed. Call it with no borrow of the state.
    pub fn changed(&self) {
        let listeners: Vec<Rc<dyn Fn()>> = {
            let mut list = self.listeners.borrow_mut();
            list.retain(|l| l.upgrade().is_some());
            list.iter().filter_map(Weak::upgrade).collect()
        };
        for listener in listeners {
            listener();
        }
    }
}

thread_local! {
    static STORE: Rc<Store> = Rc::new(Store::default());
}

/// The browser state of the session (`CurrencyBrowserState.shared`).
pub fn store() -> Rc<Store> {
    STORE.with(Rc::clone)
}
