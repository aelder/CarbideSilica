//! App-wide state: the connection, the navigation path and the dashboard's data
//! (`AppModel.swift`).
//!
//! The model lives on the GTK main thread as `Rc<AppModel>`. Its state is in a `RefCell`.
//! A change sends a [`Topic`] to the widgets that watch it ([`AppModel::watch`]). A watcher
//! is tied to a widget and stops when the widget is gone, so a page does not need to
//! remove what it added.

use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Route, Explorer};
use crate::format;
use crate::launch;
use crate::settings::{self, key};

/// The places the sidebar can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    Dashboard,
    Mempool,
    Charts,
    Currencies,
    Marketplace,
    Bookmarks,
    Node,
    Sources,
}

/// What changed. A watcher names the topics that it redraws for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Topic {
    /// A new connection started or finished: `generation` changed. Pages reload.
    Connection,
    /// Chain status and the recent blocks.
    Status,
    /// Node status: synchronization, mempool size, peers, speeds.
    Node,
    /// The navigation path or the place.
    Path,
    /// The objects opened this session.
    Recent,
    /// Names resolved for IDs.
    Names,
    Bookmarks,
    /// A search started, ended or failed.
    Search,
    /// A primary request waits for a decision about a public API.
    Permission,
    /// A display setting changed: names, dates, UTC, sounds.
    Display,
    /// Dates for heights arrived (`HeightDates`).
    HeightDates,
    /// A page changed its hooks: its keys, adjacent objects or menu functions.
    Hooks,
}

/// One measurement of how fast the node is validating blocks.
#[derive(Clone, Debug)]
pub struct SpeedSample {
    pub id: u64,
    pub time: f64,
    pub blocks_per_second: f64,
}

/// A transfer trace that the transaction page starts when it opens.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceRequest {
    pub txid: String,
    pub output: u32,
}

/// A currency state that the currency page shows when it opens.
#[derive(Clone, Debug, PartialEq)]
pub struct CurrencyStateRequest {
    pub currency: String,
    pub height: u64,
}

#[derive(Default)]
pub struct State {
    pub path: Vec<Route>,
    pub place: Option<Place>,
    /// Objects opened this session, newest first.
    pub recent: Vec<Route>,
    /// Names resolved for identity and currency IDs.
    pub names: HashMap<String, String>,
    name_requests: HashSet<String>,
    opened_start_object: bool,
    /// True until the person switches the chain: start-up arguments then stop applying.
    uses_launch_arguments: bool,

    pub bookmarks: Vec<api::BookmarkRow>,
    pub bookmark_error: Option<String>,

    pub node: Option<api::NodeStatus>,
    pub node_error: Option<String>,
    /// Validation speed over the last few minutes, oldest first.
    pub speeds: Vec<SpeedSample>,
    last_height_sample: Option<(f64, u64)>,

    pub explorer: Option<Explorer>,
    pub connection: Option<api::ConnectionInfo>,
    /// Bumped on every reconnect so that open pages reload.
    pub generation: u64,

    pub status: Option<api::ChainStatus>,
    pub status_error: Option<String>,
    pub blocks: Vec<api::BlockRow>,
    /// When this app first saw each block that arrived while it was running, by hash.
    pub arrivals: HashMap<String, f64>,
    pub last_refresh: Option<f64>,

    pub is_searching: bool,
    search_run: u64,
    pub search_error: Option<String>,
    /// The searches that found something, newest first. The app keeps 30.
    pub search_history: Vec<String>,

    /// Primary requests that wait for the person's decision about a public API.
    pub permission_requests: VecDeque<api::PermissionRequest>,

    /// The block the Charts page opens at. None opens the latest range.
    pub chart_anchor: Option<u64>,
    /// The hash of that block, when it is not on the active chain.
    pub chart_anchor_hash: Option<String>,
    /// Set by `open_trace`. The transaction page reads it once, then sets it to None.
    pub pending_trace: Option<TraceRequest>,
    /// Set by `open_currency_at`. The currency page reads it once, then sets it to None.
    pub pending_currency_state: Option<CurrencyStateRequest>,
}

type Handler = Rc<dyn Fn(&Rc<AppModel>)>;

struct Watcher {
    topics: Vec<Topic>,
    owner: glib::WeakRef<glib::Object>,
    handler: Handler,
}

/// Requests from the model to the window shell: the shell owns these widgets.
#[derive(Default)]
pub struct ShellHooks {
    pub focus_search: Option<Box<dyn Fn()>>,
    pub show_search_text: Option<Box<dyn Fn(&str)>>,
    /// Drops a stored place page, so that it is built again the next time it shows.
    pub reset_place: Option<Box<dyn Fn(Place)>>,
}

pub struct AppModel {
    state: RefCell<State>,
    watchers: RefCell<Vec<Watcher>>,
    pub shell: RefCell<ShellHooks>,
    populated_bookmarks: Cell<bool>,
}

const RECENT_BLOCK_COUNT: u32 = 40;
const KEPT_BLOCK_COUNT: usize = 60;

impl AppModel {
    pub fn new() -> Rc<Self> {
        let model = Rc::new(Self {
            state: RefCell::new(State {
                place: Some(Place::Dashboard),
                uses_launch_arguments: true,
                search_history: settings::strings(key::SEARCH_HISTORY),
                ..Default::default()
            }),
            watchers: RefCell::new(Vec::new()),
            shell: RefCell::new(ShellHooks::default()),
            populated_bookmarks: Cell::new(false),
        });
        model
    }

    pub fn state(&self) -> Ref<'_, State> {
        self.state.borrow()
    }

    fn state_mut(&self) -> RefMut<'_, State> {
        self.state.borrow_mut()
    }

    // MARK: Watching

    /// Calls `handler` after each change of one of `topics`, while `owner` exists.
    pub fn watch(
        &self,
        topics: &[Topic],
        owner: &impl IsA<glib::Object>,
        handler: impl Fn(&Rc<AppModel>) + 'static,
    ) {
        self.watchers.borrow_mut().push(Watcher {
            topics: topics.to_vec(),
            owner: owner.upcast_ref::<glib::Object>().downgrade(),
            handler: Rc::new(handler),
        });
    }

    /// Tells the watchers of `topic` that it changed.
    pub fn notify(self: &Rc<Self>, topic: Topic) {
        let handlers: Vec<Handler> = {
            let mut watchers = self.watchers.borrow_mut();
            watchers.retain(|watcher| watcher.owner.upgrade().is_some());
            watchers
                .iter()
                .filter(|watcher| watcher.topics.contains(&topic))
                .map(|watcher| watcher.handler.clone())
                .collect()
        };
        for handler in handlers {
            handler(self);
        }
    }

    // MARK: Simple reads

    pub fn explorer(&self) -> Option<Explorer> {
        self.state().explorer.clone()
    }

    pub fn generation(&self) -> u64 {
        self.state().generation
    }

    /// The coin's ticker for native amounts.
    pub fn coin(&self) -> String {
        let state = self.state();
        state
            .status
            .as_ref()
            .map(|status| status.name.clone())
            .or_else(|| state.connection.as_ref().map(|c| c.chain.clone()))
            .unwrap_or_else(|| "VRSC".into())
    }

    // The AppModel extension of `IdentitySupport.swift`.

    /// A currency's display name: the chain's own coin, a resolved name without "@", or a
    /// shortened ID (`currencyName`).
    pub fn currency_name(&self, id: &str) -> String {
        let coin = self.coin();
        let state = self.state();
        let chain_id = state.status.as_ref().and_then(|s| s.chain_id.as_deref());
        currency_name_in(&state.names, crate::display::shows_names(), chain_id, &coin, id)
    }

    /// An identity or currency ID as "Show Names for IDs" says: the name, or a shortened
    /// ID (`displayID`).
    pub fn display_id(&self, id: &str) -> String {
        display_id_in(&self.state().names, crate::display::shows_names(), id)
    }

    /// A block height as "Show Dates for Heights" says (`heightText`). `time` is the
    /// block's time when it is known.
    pub fn height_text(self: &Rc<Self>, height: u64, time: Option<u64>) -> String {
        if crate::display::shows_dates() {
            crate::display::height_text(height, time, self)
        } else {
            format!("block {}", crate::format::integer(height))
        }
    }

    /// An amount in satoshis with its ticker (`amountText`).
    pub fn amount_text(&self, sats: i64, currency: &str) -> String {
        format!("{} {}", crate::format::coins(sats), self.currency_name(currency))
    }

    pub fn current_chain(&self) -> String {
        self.state()
            .connection
            .as_ref()
            .map(|c| c.chain.clone())
            .or_else(|| settings::string(key::CHAIN))
            .unwrap_or_else(|| "VRSC".into())
    }

    /// Node configurations found on this computer, for the chain menu.
    pub fn known_chain_list(&self) -> Vec<api::KnownChain> {
        api::known_chains(settings::bool(key::TESTNET, false))
    }

    /// How much of the chain the node has validated, when it is still catching up.
    pub fn sync_fraction(&self) -> Option<f64> {
        let state = self.state();
        let blocks = state.node.as_ref().map(|n| n.blocks).or(state.status.as_ref().map(|s| s.blocks))?;
        let longest = state
            .node
            .as_ref()
            .and_then(|n| n.longest_chain)
            .or(state.status.as_ref().and_then(|s| s.longest_chain))?;
        (longest > blocks + 20).then(|| blocks as f64 / longest as f64)
    }

    /// The name to show for an identity or currency ID, or the ID when names are off or the
    /// name is not known. A currency name has no trailing "@".
    pub fn display_name(&self, id: &str, currency: bool) -> String {
        if !crate::display::shows_names() {
            return id.to_string();
        }
        match self.state().names.get(id) {
            Some(name) if currency => name.strip_suffix('@').unwrap_or(name).to_string(),
            Some(name) => name.clone(),
            None => id.to_string(),
        }
    }

    // MARK: Connection

    /// Opens the bridge with the saved settings. It reads configuration files only, off the
    /// main thread, and sends no request.
    pub fn connect(self: &Rc<Self>) {
        let chain = settings::string(key::CHAIN).unwrap_or_else(|| "VRSC".into());
        let config_file = settings::string(key::CONFIG_FILE).unwrap_or_default();
        let api_url = settings::string(key::API_URL).unwrap_or_else(|| key::DEFAULT_API.into());
        let mut options = api::ConnectionOptions {
            chain,
            testnet: settings::bool(key::TESTNET, false),
            config_file: (!config_file.is_empty()).then_some(config_file),
            api_endpoint: settings::bool(key::API_ENABLED, false).then(|| api_url.clone()),
            cache_dir: settings::string(key::CACHE_DIR).filter(|dir| !dir.is_empty()),
            ..Default::default()
        };
        // Start-up arguments override the saved connection for this run. They are not
        // saved, and they stop applying when the person switches the chain.
        let launch = launch::arguments();
        if self.state().uses_launch_arguments && launch.problem.is_none() {
            if let Some(named) = &launch.chain {
                options.chain = named.clone();
                // A chain named on the command line does not use another chain's saved file.
                options.config_file = None;
            }
            if launch.testnet {
                options.testnet = true;
            }
            // With a data directory, Carbide looks for the file there, as the node does.
            if launch.data_dir.is_some() {
                options.config_file = None;
            }
            if let Some(file) = &launch.config_file {
                options.config_file = Some(file.clone());
            }
            options.data_dir = launch.data_dir.clone();
            options.rpc_host = launch.rpc_host.clone();
            options.rpc_port = launch.rpc_port;
            options.rpc_user = launch.rpc_user.clone();
            options.rpc_password = launch.rpc_password.clone();
        }
        // The API in Settings is always listed as a source, disabled unless it is allowed,
        // so that the app can offer it when the node cannot answer. Listing sends nothing.
        let offered: Vec<String> = if api_url.is_empty() { vec![] } else { vec![api_url] };
        // Requests of the previous connection must not wait for an answer that never comes.
        let previous = {
            let mut state = self.state_mut();
            let previous = state.explorer.take();
            let requests: Vec<u64> = state.permission_requests.drain(..).map(|r| r.id).collect();
            if let Some(explorer) = &previous {
                for id in requests {
                    let _ = explorer.answer_permission(id, api::PermissionAnswer::Cancel);
                }
            }
            state.names.clear();
            state.name_requests.clear();
            state.status = None;
            state.status_error = None;
            state.blocks.clear();
            state.node = None;
            state.node_error = None;
            state.speeds.clear();
            state.last_height_sample = None;
            state.generation += 1;
            previous
        };
        drop(previous);
        let attempt = self.generation();
        self.notify(Topic::Connection);
        self.notify(Topic::Status);
        self.notify(Topic::Permission);
        let model = self.clone();
        glib::spawn_future_local(async move {
            let result = bridge::run_blocking(move || api::Explorer::with_sources(options, offered)).await;
            model.finish_connecting(result, attempt);
        });
    }

    fn finish_connecting(self: &Rc<Self>, result: Result<Explorer, api::CarbideError>, attempt: u64) {
        if attempt != self.generation() {
            return;
        }
        match result {
            Ok(explorer) => {
                {
                    let mut state = self.state_mut();
                    state.connection = Some(explorer.connection());
                    state.explorer = Some(explorer.clone());
                }
                self.listen_for_permission_requests(explorer.clone());
                // Reconciliation of the current chain, and of monitored nodes, as the
                // terminal does.
                let background = explorer.clone();
                bridge::runtime().spawn(async move { background.start_background().await });
                self.populate_bookmarks_once(explorer);
                let model = self.clone();
                glib::spawn_future_local(async move { model.refresh().await });
            }
            Err(error) => {
                let mut state = self.state_mut();
                state.connection = None;
                state.status_error = Some(bridge::describe(&error));
            }
        }
        // Pages reload when the generation changes, so they start once the bridge is ready.
        self.state_mut().generation += 1;
        self.notify(Topic::Connection);
        self.notify(Topic::Status);
        self.reload_bookmarks();
        self.open_start_object();
    }

    /// Carbide's start-up population of uncached bookmarks, once per launch. It does
    /// nothing unless Carbide's config.toml turns it on, and it never asks for permission.
    fn populate_bookmarks_once(self: &Rc<Self>, explorer: Explorer) {
        if self.populated_bookmarks.replace(true) {
            return;
        }
        let model = self.clone();
        glib::spawn_future_local(async move {
            let ex = explorer.clone();
            let _ = bridge::run(async move { ex.populate_bookmarks().await }).await;
            if model.explorer().is_some_and(|current| std::sync::Arc::ptr_eq(&current, &explorer)) {
                model.reload_bookmarks();
            }
        });
    }

    /// Opens the object named on the command line, once, as Carbide's `--open`.
    fn open_start_object(self: &Rc<Self>) {
        {
            let mut state = self.state_mut();
            if state.opened_start_object || state.explorer.is_none() {
                return;
            }
            state.opened_start_object = true;
        }
        let Some(query) = launch::arguments().open_object.clone().filter(|q| !q.is_empty()) else {
            return;
        };
        self.search(query);
    }

    /// Switches to another configured chain. Each chain remembers the config file it was
    /// last used with, so a custom path for one chain survives a visit to another.
    pub fn switch_chain(self: &Rc<Self>, chain: &api::KnownChain) {
        let mut remembered = settings::string_map(key::CHAIN_CONFIGS);
        let saved = settings::string(key::CHAIN).unwrap_or_else(|| "VRSC".into());
        remembered.insert(saved, settings::string(key::CONFIG_FILE).unwrap_or_default());
        settings::set_string_map(key::CHAIN_CONFIGS, &remembered);
        settings::set_string(key::CHAIN, &chain.chain);
        let file = remembered.get(&chain.chain).cloned().unwrap_or_else(|| chain.config_file.clone());
        settings::set_string(key::CONFIG_FILE, &file);
        {
            let mut state = self.state_mut();
            state.path.clear();
            state.recent.clear();
            state.uses_launch_arguments = false;
        }
        self.notify(Topic::Path);
        self.notify(Topic::Recent);
        self.connect();
    }

    /// Makes the chain of a node in Sources the current chain.
    pub fn switch_chain_to_source(self: &Rc<Self>, source: &api::SourceRow) {
        let Some(file) = source.config_file.clone() else { return };
        self.switch_chain(&api::KnownChain {
            chain: source.chain.clone(),
            config_file: file,
            testnet: settings::bool(key::TESTNET, false),
        });
    }

    /// Waits for primary requests that need permission to use a public API, and queues
    /// them for the dialog. It stops when the connection changes.
    fn listen_for_permission_requests(self: &Rc<Self>, explorer: Explorer) {
        let model = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            loop {
                let ex = explorer.clone();
                let request = bridge::run(async move { ex.next_permission_request().await }).await;
                let Some(model) = model.upgrade() else { return };
                if !model.explorer().is_some_and(|current| std::sync::Arc::ptr_eq(&current, &explorer)) {
                    return;
                }
                if let Some(request) = request {
                    model.state_mut().permission_requests.push_back(request);
                    model.notify(Topic::Permission);
                }
            }
        });
    }

    /// Answers the oldest permission request.
    pub fn answer_permission(self: &Rc<Self>, answer: api::PermissionAnswer) {
        let (explorer, request) = {
            let mut state = self.state_mut();
            let Some(explorer) = state.explorer.clone() else { return };
            let Some(request) = state.permission_requests.pop_front() else { return };
            (explorer, request)
        };
        let _ = explorer.answer_permission(request.id, answer);
        self.notify(Topic::Permission);
    }

    /// Turns on the public API fallback. Only called from an explicit action of the person.
    pub fn enable_public_api(self: &Rc<Self>) {
        if settings::string(key::API_URL).unwrap_or_default().is_empty() {
            settings::set_string(key::API_URL, key::DEFAULT_API);
        }
        settings::set_bool(key::API_ENABLED, true);
        self.connect();
    }

    // MARK: Refresh

    /// Refreshes the chain status and fetches only the blocks that are new.
    pub async fn refresh(self: &Rc<Self>) {
        let Some(explorer) = self.explorer() else { return };
        let generation = self.generation();
        let ex = explorer.clone();
        let status = bridge::run(async move { ex.chain_status().await }).await;
        if generation != self.generation() {
            return;
        }
        match status {
            Ok(status) => {
                let tip = status.blocks;
                let newest = self.state().blocks.first().map(|b| b.height);
                {
                    let mut state = self.state_mut();
                    state.status = Some(status);
                    state.status_error = None;
                }
                let missing = newest.map(|newest| tip.saturating_sub(newest));
                if missing == Some(0) {
                    self.state_mut().last_refresh = Some(format::now());
                    self.notify(Topic::Status);
                    self.refresh_node().await;
                    return;
                }
                let wanted = missing.unwrap_or(RECENT_BLOCK_COUNT as u64).min(RECENT_BLOCK_COUNT as u64) as u32;
                let ex = explorer.clone();
                let fetched = bridge::run(async move { ex.recent_blocks(Some(tip), wanted).await }).await;
                if generation != self.generation() {
                    return;
                }
                match fetched {
                    Ok(fetched) => {
                        let mut state = self.state_mut();
                        let known: HashSet<String> = fetched.iter().map(|b| b.hash.clone()).collect();
                        // A gap larger than one page means the old rows are no longer
                        // contiguous.
                        let older: Vec<api::BlockRow> = if missing.unwrap_or(0) > RECENT_BLOCK_COUNT as u64 {
                            vec![]
                        } else {
                            state.blocks.iter().filter(|b| !known.contains(&b.hash)).cloned().collect()
                        };
                        // Only a small step forward is an arrival. A first load or a long
                        // catch-up is not.
                        if let Some(missing) = missing
                            && missing <= 5
                        {
                            let now = format::now();
                            for block in &fetched {
                                state.arrivals.entry(block.hash.clone()).or_insert(now);
                            }
                        }
                        let mut blocks: Vec<api::BlockRow> = fetched.into_iter().chain(older).collect();
                        blocks.truncate(KEPT_BLOCK_COUNT);
                        let kept: HashSet<String> = blocks.iter().map(|b| b.hash.clone()).collect();
                        state.arrivals.retain(|hash, _| kept.contains(hash));
                        state.blocks = blocks;
                        state.last_refresh = Some(format::now());
                    }
                    Err(error) => self.state_mut().status_error = Some(bridge::describe(&error)),
                }
            }
            Err(error) => self.state_mut().status_error = Some(bridge::describe(&error)),
        }
        self.notify(Topic::Status);
        self.refresh_node().await;
    }

    /// Refreshes synchronization progress, mempool size and peers, and records the
    /// validation speed.
    pub async fn refresh_node(self: &Rc<Self>) {
        let Some(explorer) = self.explorer() else { return };
        let generation = self.generation();
        let result = bridge::run(async move { explorer.node_status().await }).await;
        if generation != self.generation() {
            return;
        }
        {
            let mut state = self.state_mut();
            match result {
                Ok(status) => {
                    let now = format::now();
                    if let Some((time, height)) = state.last_height_sample {
                        let elapsed = now - time;
                        if elapsed >= 1.0 {
                            let gained = status.blocks.saturating_sub(height);
                            let id = state.speeds.last().map(|s| s.id).unwrap_or(0) + 1;
                            state.speeds.push(SpeedSample {
                                id,
                                time: now,
                                blocks_per_second: gained as f64 / elapsed,
                            });
                            let excess = state.speeds.len().saturating_sub(150);
                            state.speeds.drain(..excess);
                            state.last_height_sample = Some((now, status.blocks));
                        }
                    } else {
                        state.last_height_sample = Some((now, status.blocks));
                    }
                    state.node = Some(status);
                    state.node_error = None;
                }
                Err(error) => state.node_error = Some(bridge::describe(&error)),
            }
        }
        self.notify(Topic::Node);
    }

    // MARK: Bookmarks

    pub fn reload_bookmarks(self: &Rc<Self>) {
        let Some(explorer) = self.explorer() else { return };
        {
            let mut state = self.state_mut();
            match explorer.bookmarks() {
                Ok(list) => {
                    state.bookmarks = list;
                    state.bookmark_error = None;
                }
                Err(error) => state.bookmark_error = Some(bridge::describe(&error)),
            }
        }
        self.notify(Topic::Bookmarks);
    }

    /// Saves the object as a bookmark in Carbide's bookmark file.
    pub fn add_bookmark(self: &Rc<Self>, route: &Route, description: &str) {
        let Some(explorer) = self.explorer() else { return };
        let Some((kind, target)) = crate::route::bookmark_target(route) else { return };
        let result = explorer.add_bookmark(kind.into(), target, description.into());
        self.state_mut().bookmark_error = result.err().map(|e| bridge::describe(&e));
        self.reload_bookmarks();
    }

    pub fn remove_bookmark(self: &Rc<Self>, index: u32) {
        let Some(explorer) = self.explorer() else { return };
        let result = explorer.remove_bookmark(index);
        self.state_mut().bookmark_error = result.err().map(|e| bridge::describe(&e));
        self.reload_bookmarks();
    }

    pub fn edit_bookmark(self: &Rc<Self>, index: u32, description: &str) {
        let Some(explorer) = self.explorer() else { return };
        let result = explorer.edit_bookmark(index, description.into());
        self.state_mut().bookmark_error = result.err().map(|e| bridge::describe(&e));
        self.reload_bookmarks();
    }

    // MARK: Names

    /// Looks up names for the identity and currency IDs that are not yet known. Other
    /// strings are ignored.
    pub fn resolve_names(self: &Rc<Self>, ids: impl IntoIterator<Item = String>) {
        let Some(explorer) = self.explorer() else { return };
        let wanted: Vec<String> = {
            let mut state = self.state_mut();
            let wanted: HashSet<String> = ids
                .into_iter()
                .filter(|id| {
                    id.starts_with('i')
                        && id.chars().count() == 34
                        && !state.names.contains_key(id)
                        && !state.name_requests.contains(id)
                })
                .collect();
            state.name_requests.extend(wanted.iter().cloned());
            wanted.into_iter().collect()
        };
        if wanted.is_empty() {
            return;
        }
        let generation = self.generation();
        let model = self.clone();
        glib::spawn_future_local(async move {
            let found = bridge::run(async move { explorer.names(wanted).await }).await;
            if generation != model.generation() {
                return;
            }
            {
                let mut state = model.state_mut();
                for entry in found {
                    state.names.insert(entry.id, entry.name);
                }
            }
            model.notify(Topic::Names);
        });
    }

    // MARK: Navigation

    pub fn place(&self) -> Place {
        self.state().place.unwrap_or(Place::Dashboard)
    }

    pub fn path(&self) -> Vec<Route> {
        self.state().path.clone()
    }

    pub fn show(self: &Rc<Self>, place: Place) {
        {
            let mut state = self.state_mut();
            state.place = Some(place);
            state.path.clear();
        }
        self.notify(Topic::Path);
    }

    /// Replaces the path, as a link of the path bar or a key of the Recent list does.
    pub fn set_path(self: &Rc<Self>, path: Vec<Route>) {
        self.state_mut().path = path;
        self.notify(Topic::Path);
    }

    pub fn open(self: &Rc<Self>, route: Route) {
        {
            let mut state = self.state_mut();
            if state.path.last() != Some(&route) {
                state.path.push(route.clone());
            }
        }
        self.remember(route);
        self.notify(Topic::Path);
    }

    /// Replaces the open page with its neighbour, such as the next block. Back then goes
    /// to the page before the first of them, as in the terminal.
    pub fn step(self: &Rc<Self>, route: Route) {
        {
            let mut state = self.state_mut();
            match state.path.last_mut() {
                Some(last) => *last = route,
                None => state.path.push(route),
            }
        }
        self.notify(Topic::Path);
    }

    fn remember(self: &Rc<Self>, route: Route) {
        {
            let mut state = self.state_mut();
            state.recent.retain(|r| r != &route);
            state.recent.insert(0, route);
            state.recent.truncate(20);
        }
        self.notify(Topic::Recent);
        let ids: Vec<String> = self
            .state()
            .recent
            .iter()
            .filter_map(|route| match route {
                Route::Identity { name } | Route::Currency { name } => Some(name.clone()),
                _ => None,
            })
            .collect();
        self.resolve_names(ids);
    }

    // MARK: Requests from one page to another

    /// Opens the Charts page with `height` in view. Pass `hash` for a block on a side branch.
    pub fn show_charts(self: &Rc<Self>, height: Option<u64>, hash: Option<String>) {
        {
            let mut state = self.state_mut();
            state.chart_anchor = height;
            state.chart_anchor_hash = hash;
        }
        // The Swift app builds the charts again for a new anchor (`.id`). So does this app.
        if let Some(reset) = &self.shell.borrow().reset_place {
            reset(Place::Charts);
        }
        self.show(Place::Charts);
    }

    /// Opens a transaction and starts the transfer trace at one of its outputs.
    pub fn open_trace(self: &Rc<Self>, txid: String, output: u32) {
        self.state_mut().pending_trace = Some(TraceRequest { txid: txid.clone(), output });
        self.open(Route::Transaction { txid });
    }

    /// Reads the pending trace once.
    pub fn take_pending_trace(&self) -> Option<TraceRequest> {
        self.state_mut().pending_trace.take()
    }

    /// Opens a currency with its state at `height` in the exact-state card.
    pub fn open_currency_at(self: &Rc<Self>, name: String, height: u64) {
        self.state_mut().pending_currency_state =
            Some(CurrencyStateRequest { currency: name.clone(), height });
        self.open(Route::Currency { name });
    }

    pub fn take_pending_currency_state(&self) -> Option<CurrencyStateRequest> {
        self.state_mut().pending_currency_state.take()
    }

    // MARK: Search

    /// Resolves a search and navigates to what it names.
    pub fn search(self: &Rc<Self>, query: String) {
        let query = query.trim().to_string();
        if query.is_empty() || self.state().is_searching {
            return;
        }
        let Some(explorer) = self.explorer() else {
            let message = self.state().status_error.clone().unwrap_or_else(|| "Not connected.".into());
            self.state_mut().search_error = Some(message);
            self.notify(Topic::Search);
            return;
        };
        let run = {
            let mut state = self.state_mut();
            state.is_searching = true;
            state.search_run += 1;
            state.search_run
        };
        self.notify(Topic::Search);
        let model = self.clone();
        glib::spawn_future_local(async move {
            let asked = query.clone();
            let result = bridge::run(async move { explorer.resolve(asked).await }).await;
            // The person stopped this search, or started a different one.
            if model.state().search_run != run {
                return;
            }
            model.state_mut().is_searching = false;
            match result {
                Ok(route) => {
                    if let Some(show) = &model.shell.borrow().show_search_text {
                        show("");
                    }
                    model.record_search(&query);
                    // A search starts a new trail, as in the terminal app.
                    model.state_mut().path = vec![route.clone()];
                    model.remember(route);
                    model.notify(Topic::Path);
                }
                Err(error) => model.state_mut().search_error = Some(bridge::describe(&error)),
            }
            model.notify(Topic::Search);
        });
    }

    /// Stops the search that runs. The app does not use its answer.
    pub fn cancel_search(self: &Rc<Self>) {
        {
            let mut state = self.state_mut();
            state.search_run += 1;
            state.is_searching = false;
        }
        self.notify(Topic::Search);
    }

    pub fn take_search_error(&self) -> Option<String> {
        self.state_mut().search_error.take()
    }

    pub fn set_search_error(self: &Rc<Self>, message: String) {
        self.state_mut().search_error = Some(message);
        self.notify(Topic::Search);
    }

    fn record_search(&self, query: &str) {
        let mut state = self.state_mut();
        state.search_history.retain(|q| q != query);
        state.search_history.insert(0, query.to_string());
        state.search_history.truncate(30);
        settings::set_strings(key::SEARCH_HISTORY, &state.search_history);
    }

    /// Asks the search field to take the keyboard focus.
    pub fn focus_search(&self) {
        if let Some(focus) = &self.shell.borrow().focus_search {
            focus();
        }
    }

    /// Changes a display setting and redraws the pages that show it.
    pub fn toggle_display(self: &Rc<Self>, key: &str, default: bool) {
        settings::toggle(key, default);
        self.notify(Topic::Display);
    }

    pub fn set_display(self: &Rc<Self>, key: &str, on: bool) {
        settings::set_bool(key, on);
        self.notify(Topic::Display);
    }

    pub fn require_explorer(&self) -> Result<Explorer, String> {
        self.explorer().ok_or_else(|| self.state().status_error.clone().unwrap_or_else(|| "Not connected.".into()))
    }
}

/// `AppModel.currencyName` over a set of names: the chain's own coin, a resolved name
/// without "@", or a shortened ID.
pub fn currency_name_in(names: &HashMap<String, String>, shows_names: bool, chain_id: Option<&str>, coin: &str, id: &str) -> String {
    if chain_id == Some(id) || id == coin {
        return coin.to_string();
    }
    if !shows_names {
        return crate::format::short(id, 8, 6);
    }
    match names.get(id) {
        Some(name) => name.strip_suffix('@').unwrap_or(name).to_string(),
        None => crate::format::short(id, 8, 6),
    }
}

/// `AppModel.displayID` over a set of names: the name, or a shortened ID.
pub fn display_id_in(names: &HashMap<String, String>, shows_names: bool, id: &str) -> String {
    if shows_names
        && let Some(name) = names.get(id)
    {
        return name.clone();
    }
    crate::format::short(id, 8, 6)
}

