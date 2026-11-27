//! Helpers of the currency pages
//! (`CurrencyStores.swift`): `knownName`, `observationSource`, `heightText`,
//! `OtherChainNote`, the words of `LaunchState` and `DataSource`, the session store of the
//! currency browser (`CurrencyBrowserState`, used by the Currencies page) and the requests
//! for a notarization perspective (`NotarizationRequests`, used by the Bookmarks page and
//! the Currency page).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gtk::prelude::*;

use crate::bridge::api;
use crate::display;
use crate::format;
use crate::model::AppModel;
use crate::page;
use crate::widgets::link;

/// The currency browser's list, filters and selection, kept for the session so that they
/// survive going back and visits to other places in the sidebar (`CurrencyBrowserState`).
/// Read and change it through [`browser_state`].
#[derive(Clone, Debug)]
pub struct CurrencyBrowserState {
    pub rows: Vec<api::CurrencyListing>,
    pub source: Option<String>,
    /// Where the list came from: live, or stored from the last answer.
    pub provenance: Option<api::Provenance>,
    pub failure: Option<String>,
    pub loading: bool,
    /// The connection the rows came from. A new connection loads a new list.
    pub generation: Option<u64>,
    pub filter: api::CurrencyFilter,
    /// The currency opened last, so the list shows where the user was.
    pub selected: Option<String>,
    pub show_flags: bool,
    /// The column that gives the order of the list. None is the order of the source.
    pub sort: Option<CurrencySort>,
    pub descending: bool,
}

impl Default for CurrencyBrowserState {
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

/// The columns that can give the order of the currency list (`CurrencySort`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurrencySort {
    Name,
    Reserves,
    Supply,
    Start,
}

impl CurrencySort {
    /// The column's title, which is the raw value of the Swift enum.
    pub fn title(self) -> &'static str {
        match self {
            CurrencySort::Name => "Currency",
            CurrencySort::Reserves => "Reserves",
            CurrencySort::Supply => "Supply",
            CurrencySort::Start => "Start",
        }
    }
}

/// The browser state and the pages that show it. The Currencies place and a `currencies`
/// search both show the browser, and a change in one shows in the other, as the Swift
/// `@Observable` class does.
#[derive(Default)]
pub struct CurrencyBrowserStore {
    pub state: RefCell<CurrencyBrowserState>,
    listeners: RefCell<Vec<Weak<dyn Fn()>>>,
}

impl CurrencyBrowserStore {
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
    static BROWSER_STATE: Rc<CurrencyBrowserStore> = Rc::new(CurrencyBrowserStore::default());
    static NOTARIZATION_REQUESTS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// The one store of the currency browser for the session (`CurrencyBrowserState.shared`).
pub fn browser_state() -> Rc<CurrencyBrowserStore> {
    BROWSER_STATE.with(Rc::clone)
}

/// Requests to open a currency page with its notarization browser on one perspective. A
/// notarization bookmark saves the observing chain, and this carries it to the page
/// (`NotarizationRequests`). `currency` is the currency name or ID as routed; `observer` is
/// the observing system's ID.
pub fn request_notarization_view(currency: &str, observer: &str) {
    NOTARIZATION_REQUESTS.with(|requests| {
        requests.borrow_mut().insert(currency.to_string(), observer.to_string());
    });
}

/// Returns and forgets a request for any of the currency's names.
pub fn take_notarization_request(keys: &[Option<&str>]) -> Option<String> {
    NOTARIZATION_REQUESTS.with(|requests| {
        let mut requests = requests.borrow_mut();
        keys.iter().flatten().find_map(|key| requests.remove(*key))
    })
}

/// The word for a launch state (`LaunchState.word`).
pub fn launch_word(state: api::LaunchState) -> &'static str {
    match state {
        api::LaunchState::Launched => "Launched",
        api::LaunchState::Prelaunch => "Prelaunch",
        api::LaunchState::Failed => "Failed launch (refunding)",
        api::LaunchState::Unknown => "Unknown",
    }
}

/// The launch states in the order of the browser's filter (`LaunchState.all`).
pub const LAUNCH_STATES: [api::LaunchState; 4] =
    [api::LaunchState::Launched, api::LaunchState::Prelaunch, api::LaunchState::Failed, api::LaunchState::Unknown];

/// The word for a data source (`DataSource.sourceWord`).
pub fn source_word(source: api::DataSource) -> &'static str {
    match source {
        api::DataSource::LocalDaemon => "Local node",
        api::DataSource::PublicApi => "Public API",
    }
}

/// A currency or system name for display: a resolved name without the trailing "@", else
/// the given fallback, else a shortened ID. When names are off, the ID.
pub fn known_name(id: &str, model: &AppModel, fallback: Option<&str>) -> String {
    if !display::shows_names() {
        return id.to_string();
    }
    if model.state().status.as_ref().and_then(|s| s.chain_id.as_deref()) == Some(id) {
        return model.coin();
    }
    if let Some(name) = model.state().names.get(id) {
        return name.strip_suffix('@').unwrap_or(name).to_string();
    }
    match fallback.filter(|f| !f.is_empty()) {
        Some(fallback) => fallback.to_string(),
        None => format::short(id, 6, 4),
    }
}

/// Carbide's source IDs are `kind:digest`. Show the kind and a short digest.
pub fn observation_source(source: &str) -> String {
    let parts: Vec<&str> = source.splitn(2, ':').collect();
    if parts.len() != 2 {
        return source.to_string();
    }
    let word = match parts[0] {
        "local-daemon" => "Local node",
        "public-api" => "Public API",
        other => other,
    };
    format!("{word} {}", format::short(parts[1], 6, 4))
}

/// A height as a number, or as a date when dates are on.
pub fn height_text(height: u64, known: Option<u64>, model: &Rc<AppModel>) -> String {
    display::height_text(height, known, model)
}

/// Names the chain of a block or transaction that is on another chain. The app shows one
/// chain at a time, so the object stays text. When a node for that chain is configured on
/// this computer, the person can make it the current chain (`OtherChainNote`).
pub fn other_chain_note(model: &Rc<AppModel>, window: &gtk::Window, system_id: &str, fallback: Option<&str>) -> gtk::Box {
    let name = known_name(system_id, model, fallback);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&page::text(&format!("on {name}"), &["caption-text", "label"]));
    let node = model.explorer().and_then(|explorer| {
        explorer.sources_for_system(system_id.to_string()).into_iter().find(|source| {
            source.kind == crate::bridge::api::SourceKind::LocalNode && source.config_file.is_some() && !source.current
        })
    });
    if let Some(source) = node {
        let (weak, window, title) = (Rc::downgrade(model), window.downgrade(), name.clone());
        let switch = link::link("Switch Chain…", false, move || {
            let (Some(model), Some(window)) = (weak.upgrade(), window.upgrade()) else { return };
            let (source, weak) = (source.clone(), Rc::downgrade(&model));
            crate::shell::dialogs::confirm(
                &window,
                &format!("Switch to {title}?"),
                &format!("The app connects to the {title} node and closes the pages that are open now."),
                "Switch Chain",
                move || {
                    if let Some(model) = weak.upgrade() {
                        model.switch_chain_to_source(&source);
                    }
                },
            );
        });
        switch.add_css_class("caption-text");
        switch.set_tooltip_text(Some(&format!("Make {name} the current chain. The app then shows its blocks and transactions.")));
        row.append(&switch);
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notarization_requests_are_taken_once_by_any_name() {
        request_notarization_view("vETH", "iSystem");
        assert_eq!(take_notarization_request(&[None, Some("other"), Some("vETH")]), Some("iSystem".into()));
        assert_eq!(take_notarization_request(&[Some("vETH")]), None);
    }

    #[test]
    fn observation_sources_name_their_kind() {
        assert_eq!(observation_source("local-daemon:0123456789abcdef"), "Local node 012345…cdef");
        assert_eq!(observation_source("plain"), "plain");
        assert_eq!(launch_word(api::LaunchState::Failed), "Failed launch (refunding)");
        assert_eq!(CurrencySort::Name.title(), "Currency");
    }
}
