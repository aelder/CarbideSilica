//! Sources: nodes, public APIs, the cache of observations, and which chain is in use.
//!
//! Part of the bridge's Swift-facing API. See `ffi.rs` for the shared types and helpers.
//!
//! The model is Carbide's (`src/sources.rs`, `CROSS_CHAIN_ARCHITECTURE.md`):
//!
//! - Each node and each API is a source with its own state: disabled, enabled or
//!   monitored. A disabled source receives no request. An API cannot be monitored.
//! - The states live in Carbide's own settings file, read and written by
//!   `sources::Registry`, so the terminal app and this app share them. A change applies to
//!   the session at once and is written to the file only when the person saves.
//! - A primary request that the local node cannot answer asks the person before it uses
//!   a disabled API (`next_permission_request`, `answer_permission`).
//! - Enabled nodes of the current chain, and monitored nodes of any chain, are reconciled
//!   in the background with Carbide's `Reconciler`, which keeps the block history current.

#[allow(unused_imports)]
use super::*;

use std::collections::HashMap;
use std::path::Path;

use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::sources::{
    ApiConsent, ApiDecision, ApiEndpoint, ApiPool, ChainCheck, Communication, CommunicationOutcome,
    ConnectionState, Control, Registry,
};

#[cfg(test)]
#[path = "ffi_sources_tests.rs"]
mod tests;

// --------------------------------------------------------------- records

/// Carbide's three connection states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SourceState {
    /// The app sends no request to the source.
    Disabled,
    /// The source answers explicit requests. A node of the current chain also keeps the
    /// chain's block history current.
    Enabled,
    /// A node only: enabled, and the app also follows its tip and mempool in the background.
    Monitored,
}

/// What the app observed of a source's traffic in this session. Independent of the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SourceTraffic {
    /// The source is disabled.
    Off,
    /// The app has not sent a request to it in this session.
    NotUsed,
    /// A request is in progress.
    Working,
    /// The source answered recently.
    Responding,
    /// The source answered, but not recently.
    Idle,
    /// The last request did not reach the source or got no valid answer.
    Unavailable,
}

/// One node or API.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SourceRow {
    /// The key to use with `set_source_state` and `source_activity`. Valid for this explorer.
    pub key: String,
    /// Carbide's stable ID for the source, as in `Provenance::source_id`.
    pub source_id: String,
    pub kind: SourceKind,
    /// The chain's name as Carbide knows it: VRSC, vrsctest, or a PBaaS chain's folder name.
    pub chain: String,
    pub system_id: String,
    /// `http://host:port` for a node, the URL for an API.
    pub endpoint: String,
    pub config_file: Option<String>,
    /// A node whose configuration file has no RPC user and password cannot be used.
    pub has_credentials: bool,
    pub state: SourceState,
    /// True for the sources of the chain the app is showing.
    pub current: bool,
    pub traffic: SourceTraffic,
    /// Seconds since the last successful answer in this session.
    pub last_success_age: Option<u64>,
    /// For nodes: the last chain check (reconciliation), in Carbide's words.
    pub chain_check: Option<String>,
    /// True when the last chain check failed recently.
    pub chain_check_problem: bool,
    /// The chain's name when it is known, such as vDEX for a PBaaS chain whose folder is
    /// named by its hex ID. Resolve `system_id` with `names` to learn more names.
    pub name: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct SourcesOverview {
    pub rows: Vec<SourceRow>,
    /// Carbide's settings file, which the terminal app also reads.
    pub settings_file: Option<String>,
    /// Set when the settings file cannot be read. States then apply to this session only.
    pub settings_problem: Option<String>,
    /// True when a state changed since the last save.
    pub unsaved: bool,
    /// Set when the cache cannot be used. Answers are then not recorded.
    pub cache_problem: Option<String>,
}

/// A significant event that the cache recorded, such as a reorganization or a
/// disagreement between sources.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ActivityRow {
    pub observed_at: u64,
    /// "Reconciled", "Not rechecked" or "Recorded", as in the terminal app.
    pub status: String,
    pub summary: String,
    /// The full message, with source IDs replaced by their endpoints.
    pub message: String,
    pub system_id: Option<String>,
}

/// A primary request that the local node cannot answer, and the disabled APIs that could.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PermissionRequest {
    pub id: u64,
    pub chain: String,
    /// The RPC method of the request.
    pub method: String,
    /// Why the local node did not answer.
    pub failure: String,
    /// The disabled APIs, by origin. `PermissionAnswer::Enable` takes an index in this list.
    pub endpoints: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PermissionAnswer {
    /// Ask the local node again.
    Retry,
    /// Enable one API for this session, then send the request again.
    Enable { index: u32 },
    /// Send nothing to an API. The request fails, or shows a retained observation.
    Cancel,
}

/// The last observed membership of a block, as a source reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Membership {
    /// The source reported the block on its active chain.
    Active,
    /// The source reported the block off its active chain.
    Orphaned,
    /// No stored report says which. This is not evidence of either.
    Unverified,
}

/// A block from stored observations: a block that contained a transaction, or a child
/// of a block.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ObservedBlock {
    pub block_hash: String,
    pub height: Option<u64>,
    /// The block's own time, when a stored header or body has it.
    pub time: Option<u64>,
    pub source_id: String,
    pub source: String,
    pub membership: Membership,
    /// When the source last reported this membership, in Unix seconds.
    pub observed_at: Option<u64>,
    /// True when the cache has the block's full body, so it opens without a source.
    pub cached_body: bool,
}

/// One page of stored observations, oldest height first. Different sources stay separate.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ObservedBlockPage {
    pub rows: Vec<ObservedBlock>,
    pub page: u32,
    /// True when another page follows.
    pub more: bool,
}

/// The result of checking a notarization's proof root on the source of the chain it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ProofRootStatus {
    /// The block is on the source's active chain.
    Active,
    /// The source knows the block, off its active chain. This can indicate a fork.
    OffChain,
    /// The source knows the block, at another height than the proof root states.
    HeightMismatch,
    /// The source knows the block but did not report its membership.
    PresentMembershipUnknown,
    /// The source does not know the block. This can indicate a fork.
    NotFound,
    /// The source does not know the block, and its tip is below the proof root's height.
    SourceBehind,
    /// No node or API is configured for the chain.
    NoSourceConfigured,
    /// A source is configured, but each one is disabled.
    NoSourceEnabled,
    /// The source did not report its chain status.
    ChainQueryFailed,
    /// The block request failed for another reason than "not found".
    LookupFailed,
    /// The source returned a header for a different block.
    InvalidHeader,
    /// The proof root has no block hash, so there is nothing to check.
    MissingHash,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ProofRootCheck {
    pub status: ProofRootStatus,
    /// Carbide's wording for the status.
    pub label: String,
    pub source_id: Option<String>,
    pub source: Option<String>,
    /// The checked source's tip height, when it reported one.
    pub source_tip: Option<u64>,
    pub found_height: Option<u64>,
    pub found_time: Option<u64>,
}

// ------------------------------------------------------- session states

/// Source states changed in this process and not yet saved. They outlive one explorer, so
/// that switching the current chain keeps them, as the terminal app's session does.
#[derive(Default)]
struct Session {
    states: HashMap<String, ConnectionState>,
    unsaved: bool,
}

static SESSION: LazyLock<Mutex<Session>> = LazyLock::new(Default::default);

/// The background monitor running for each node, so that a new explorer replaces the
/// previous explorer's monitor instead of running a second one.
static RUNNING: LazyLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = LazyLock::new(Default::default);

fn session() -> std::sync::MutexGuard<'static, Session> {
    SESSION.lock().unwrap_or_else(|e| e.into_inner())
}

fn daemon_key(path: &Path) -> String {
    format!("daemon:{}", path.display())
}

fn api_key(system: &str, url: &str) -> String {
    format!("api:{system}:{}", url.trim_end_matches('/'))
}

fn same_url(a: &str, b: &str) -> bool {
    a.trim_end_matches('/') == b.trim_end_matches('/')
}

// ---------------------------------------------------------------- sources

struct Pending {
    reply: tokio::sync::oneshot::Sender<ApiDecision>,
    chain: String,
    endpoints: Vec<String>,
}

pub(super) struct Sources {
    /// The current chain first, then every other configured chain. Clones share each
    /// source's `Control`, so a state change reaches every client at once.
    configs: Vec<Config>,
    registry: Mutex<Option<Registry>>,
    settings_path: Option<PathBuf>,
    settings_problem: Option<String>,
    /// Source ID to a description and kind.
    labels: HashMap<String, (String, SourceKind)>,
    consent: tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<ApiConsent>>,
    pending: Mutex<HashMap<u64, Pending>>,
    next_request: AtomicU64,
    started: AtomicBool,
    monitors: Mutex<Vec<Arc<AtomicBool>>>,
    /// Counts committed changes of the stored branches in this explorer.
    revision: Arc<tokio::sync::watch::Sender<u64>>,
}

impl Drop for Sources {
    fn drop(&mut self) {
        for alive in self
            .monitors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            alive.store(false, Ordering::Release);
        }
    }
}

impl Sources {
    /// Reads Carbide's settings file and the node configurations on this Mac. Never fails:
    /// a settings file that cannot be read leaves the current chain alone, and says why.
    pub(super) fn load(
        invocation: Config,
        network: Network,
        enabled_api: Option<&str>,
        offered_apis: &[String],
    ) -> Self {
        // The app always shows its current chain, as the terminal's dashboard does, so an
        // enabled node of that chain keeps its block history current.
        invocation
            .daemon
            .control
            .visible
            .store(true, Ordering::Release);
        let (mut registry, settings_problem) = match Registry::load(invocation.clone()) {
            Ok(registry) => (Some(registry), None),
            Err(error) => (
                None,
                Some(format!(
                    "The app cannot read Carbide's settings file. Source states apply to this \
                     session only. {error:#}"
                )),
            ),
        };
        let settings_path = registry
            .as_ref()
            .map(|registry| registry.settings_path.clone())
            .or_else(|| {
                directories::BaseDirs::new()
                    .map(|dirs| dirs.config_dir().join("verus-explorer-tui/config.toml"))
            });
        let shared_sender = registry
            .as_ref()
            .and_then(|registry| registry.configs.first())
            .and_then(|config| config.daemon.api_pool.consent.clone());
        let (consent_tx, consent_rx) = match (&mut registry, shared_sender) {
            (Some(registry), Some(sender)) => (
                sender,
                std::mem::replace(
                    &mut registry.consent_rx,
                    tokio::sync::mpsc::unbounded_channel().1,
                ),
            ),
            _ => tokio::sync::mpsc::unbounded_channel(),
        };
        let mut configs = registry
            .as_ref()
            .map(|registry| registry.configs.clone())
            .unwrap_or_else(|| vec![invocation.clone()]);

        // Carbide's discovery reads the Linux locations. Add the macOS ones, disabled, as
        // Carbide adds what it discovers. Carbide keeps one node for each chain.
        for known in known_chains(network == Network::Testnet) {
            let path = PathBuf::from(&known.config_file);
            if configs
                .iter()
                .any(|config| config.daemon.config_file.as_deref() == Some(path.as_path()))
            {
                continue;
            }
            let Some(mut found) = daemon_config(&known.chain, &path, network) else {
                continue;
            };
            if found.daemon.user.is_none() || found.daemon.password.is_none() {
                continue;
            }
            found.cache_path = invocation.cache_path.clone();
            found.daemon.control.set(ConnectionState::Disabled);
            let system = found.chain_cache_key();
            match configs
                .iter_mut()
                .find(|config| config.chain_cache_key() == system)
            {
                // A chain known only by its saved APIs gets this node.
                Some(existing) if existing.daemon.config_file.is_none() => {
                    let pool = existing.daemon.api_pool.clone();
                    existing.daemon = found.daemon;
                    existing.daemon.api_pool = pool;
                }
                Some(_) => {}
                None => {
                    found.daemon.api_pool = Arc::new(ApiPool::new(
                        found.chain.name.clone(),
                        Vec::new(),
                        Some(consent_tx.clone()),
                    ));
                    configs.push(found);
                }
            }
        }

        // The current chain's APIs: the saved ones, then the ones the app offers.
        let current = &mut configs[0];
        let system = current.chain.system_id.get().cloned();
        let mut endpoints: Vec<ApiEndpoint> = current
            .daemon
            .api_pool
            .endpoints
            .iter()
            .map(|endpoint| ApiEndpoint {
                url: endpoint.url.clone(),
                control: endpoint.control.clone(),
                retry_after: Default::default(),
            })
            .collect();
        for url in offered_apis.iter().map(String::as_str).chain(enabled_api) {
            if !endpoints
                .iter()
                .any(|endpoint| same_url(&endpoint.url, url))
            {
                let control = Arc::new(Control::default());
                control.set(ConnectionState::Disabled);
                if let Some(system) = &system {
                    let _ = control.expected.set((system.clone(), network));
                }
                endpoints.push(ApiEndpoint {
                    url: url.to_string(),
                    control,
                    retry_after: Default::default(),
                });
            }
        }
        current.daemon.api_pool = Arc::new(ApiPool::new(
            current.chain.name.clone(),
            endpoints,
            Some(consent_tx),
        ));

        // States changed earlier in this session, then the API that Settings allows.
        {
            let session = session();
            for config in &configs {
                if let Some(path) = &config.daemon.config_file
                    && let Some(state) = session.states.get(&daemon_key(path))
                    && config.daemon.control.state() != *state
                {
                    config.daemon.control.set(*state);
                }
                for endpoint in &config.daemon.api_pool.endpoints {
                    if let Some(state) = session
                        .states
                        .get(&api_key(&config.chain_cache_key(), &endpoint.url))
                        && endpoint.control.state() != *state
                    {
                        endpoint.control.set(*state);
                    }
                }
            }
        }
        if let Some(url) = enabled_api {
            for endpoint in &configs[0].daemon.api_pool.endpoints {
                if same_url(&endpoint.url, url)
                    && endpoint.control.state() == ConnectionState::Disabled
                {
                    endpoint.control.set(ConnectionState::Enabled);
                }
            }
        }

        let mut labels = HashMap::new();
        for config in &configs {
            let chain = chain_label(&config.chain.name);
            if config.daemon.config_file.is_some() {
                labels.insert(
                    config.daemon_cache_key(),
                    (
                        format!(
                            "{chain} node at {}:{}",
                            config.daemon.host, config.daemon.port
                        ),
                        SourceKind::LocalNode,
                    ),
                );
            }
            for endpoint in &config.daemon.api_pool.endpoints {
                labels.insert(
                    crate::rpc::source_id("public-api", &endpoint.url),
                    (endpoint.url.clone(), SourceKind::PublicApi),
                );
            }
        }
        if let Some(registry) = &mut registry {
            registry.configs = configs.clone();
        }
        Self {
            configs,
            registry: Mutex::new(registry),
            settings_path,
            settings_problem,
            labels,
            consent: tokio::sync::Mutex::new(consent_rx),
            pending: Default::default(),
            next_request: AtomicU64::new(1),
            started: AtomicBool::new(false),
            monitors: Default::default(),
            revision: Arc::new(tokio::sync::watch::channel(0).0),
        }
    }

    pub(super) fn current(&self) -> &Config {
        &self.configs[0]
    }

    pub(super) fn configs(&self) -> &[Config] {
        &self.configs
    }

    /// The configuration of the chain with this system ID, current chain included.
    pub(super) fn config_for_system(&self, system_id: &str) -> Option<&Config> {
        self.configs
            .iter()
            .find(|config| config.chain_cache_key() == system_id)
    }

    /// A source ID in words, and its kind.
    pub(super) fn label(&self, source_id: &str) -> (String, SourceKind) {
        self.labels.get(source_id).cloned().unwrap_or_else(|| {
            let kind = if source_id.starts_with("local-daemon") {
                SourceKind::LocalNode
            } else if source_id.starts_with("public-api") {
                SourceKind::PublicApi
            } else {
                SourceKind::Unknown
            };
            (source_label(source_id), kind)
        })
    }

    /// Tells the cache each source's endpoint, so that recorded events name endpoints.
    pub(super) fn register(&self, cache: &crate::cache::Cache) {
        for config in &self.configs {
            let system = config.chain_cache_key();
            if config.daemon.config_file.is_some() {
                let endpoint = format!("http://{}:{}", config.daemon.host, config.daemon.port);
                let _ = cache.register_source(&config.daemon_cache_key(), &system, &endpoint);
            }
            for api in &config.daemon.api_pool.endpoints {
                let _ = cache.register_source(
                    &crate::rpc::source_id("public-api", &api.url),
                    &system,
                    &api.url,
                );
            }
        }
    }

    /// The control behind a row key: `node:<chain>` or `api:<chain>:<api>`.
    fn control(&self, key: &str) -> Option<(&Config, Option<&ApiEndpoint>)> {
        let mut parts = key.split(':');
        let kind = parts.next()?;
        let config = self.configs.get(parts.next()?.parse::<usize>().ok()?)?;
        match kind {
            "node" if config.daemon.config_file.is_some() => Some((config, None)),
            "api" => Some((
                config,
                Some(
                    config
                        .daemon
                        .api_pool
                        .endpoints
                        .get(parts.next()?.parse::<usize>().ok()?)?,
                ),
            )),
            _ => None,
        }
    }

    fn rows(&self) -> Vec<SourceRow> {
        let mut rows = Vec::new();
        for (index, config) in self.configs.iter().enumerate() {
            let stale_after =
                Duration::from_secs(config.poll_interval_seconds.saturating_mul(2).max(30));
            let system_id = config.chain_cache_key();
            if config.daemon.config_file.is_some() {
                let control = &config.daemon.control;
                let (chain_check, chain_check_problem) = chain_check_text(control, stale_after);
                rows.push(SourceRow {
                    key: format!("node:{index}"),
                    source_id: config.daemon_cache_key(),
                    kind: SourceKind::LocalNode,
                    chain: config.chain.name.clone(),
                    system_id: system_id.clone(),
                    endpoint: format!("http://{}:{}", config.daemon.host, config.daemon.port),
                    config_file: config
                        .daemon
                        .config_file
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    has_credentials: config.daemon.user.is_some()
                        && config.daemon.password.is_some(),
                    state: state_of(control.state()),
                    current: index == 0,
                    traffic: traffic(control.state(), &control.communication(), stale_after),
                    last_success_age: control
                        .communication()
                        .last_success
                        .map(|time| time.elapsed().as_secs()),
                    chain_check: Some(chain_check),
                    chain_check_problem,
                    name: None,
                });
            }
            for (api, endpoint) in config.daemon.api_pool.endpoints.iter().enumerate() {
                let control = &endpoint.control;
                rows.push(SourceRow {
                    key: format!("api:{index}:{api}"),
                    source_id: crate::rpc::source_id("public-api", &endpoint.url),
                    kind: SourceKind::PublicApi,
                    chain: config.chain.name.clone(),
                    system_id: system_id.clone(),
                    endpoint: endpoint.url.clone(),
                    config_file: None,
                    has_credentials: true,
                    state: state_of(control.state()),
                    current: index == 0,
                    traffic: traffic(control.state(), &control.communication(), stale_after),
                    last_success_age: control
                        .communication()
                        .last_success
                        .map(|time| time.elapsed().as_secs()),
                    chain_check: None,
                    chain_check_problem: false,
                    name: None,
                });
            }
        }
        rows
    }
}

/// Loads one node's configuration the way `Explorer::open` does for the current chain.
fn daemon_config(chain: &str, path: &Path, network: Network) -> Option<Config> {
    let mut args = vec![
        "carbide".to_string(),
        "--chain".into(),
        chain.to_string(),
        "--conf".into(),
        path.display().to_string(),
    ];
    if network == Network::Testnet {
        args.push("--testnet".into());
    }
    Config::load(&Cli::try_parse_from(args).ok()?).ok()
}

/// PBaaS chains are stored under their 40-character hex ID. Show a short form.
fn chain_label(chain: &str) -> String {
    if chain.len() == 40 && chain.bytes().all(|b| b.is_ascii_hexdigit()) {
        format!("PBaaS chain {}", &chain[..8])
    } else {
        chain.to_string()
    }
}

/// A source ID that this session does not know: its kind and the start of its hash.
/// Ported from `source_label` in `src/app/notarization_ui.rs`.
fn source_label(source: &str) -> String {
    source.split_once(':').map_or_else(
        || source.to_string(),
        |(kind, id)| format!("{kind}:{}", &id[..id.len().min(12)]),
    )
}

fn state_of(state: ConnectionState) -> SourceState {
    match state {
        ConnectionState::Disabled => SourceState::Disabled,
        ConnectionState::Enabled => SourceState::Enabled,
        ConnectionState::Monitored => SourceState::Monitored,
    }
}

fn connection_state(state: SourceState) -> ConnectionState {
    match state {
        SourceState::Disabled => ConnectionState::Disabled,
        SourceState::Enabled => ConnectionState::Enabled,
        SourceState::Monitored => ConnectionState::Monitored,
    }
}

/// Ported from `communication_label` in `src/app/source_ui.rs`.
fn traffic(state: ConnectionState, status: &Communication, stale_after: Duration) -> SourceTraffic {
    if state == ConnectionState::Disabled {
        SourceTraffic::Off
    } else if !status.in_flight.is_empty() {
        SourceTraffic::Working
    } else {
        match status.outcome {
            Some(CommunicationOutcome::Failed) => SourceTraffic::Unavailable,
            Some(CommunicationOutcome::Responding)
                if status
                    .last_contact
                    .is_some_and(|time| time.elapsed() <= stale_after) =>
            {
                SourceTraffic::Responding
            }
            Some(_) => SourceTraffic::Idle,
            None => SourceTraffic::NotUsed,
        }
    }
}

/// Ported from `short_age` in `src/app/source_ui.rs`.
fn short_age(seconds: u64) -> String {
    match seconds {
        0 => "<1s".into(),
        1..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

/// Ported from `chain_check_line` in `src/app/source_ui.rs`. The bool is true for a
/// recent failure.
fn chain_check_text(control: &Control, stale_after: Duration) -> (String, bool) {
    match control.chain_check() {
        Some(ChainCheck::Checking) if control.polling() => ("Checking".into(), false),
        Some(ChainCheck::Finished { at, error }) => {
            let fresh = control.polling() && at.elapsed() <= stale_after;
            let age = short_age(at.elapsed().as_secs());
            match error {
                Some(error) => {
                    let label = if fresh {
                        "Needs attention"
                    } else {
                        "Last check incomplete"
                    };
                    let reason = error.lines().next().unwrap_or_default();
                    (format!("{label} · {age} ago · {reason}"), fresh)
                }
                None if fresh => (format!("Reconciled · {age} ago"), false),
                None => (format!("Last reconciled {age} ago"), false),
            }
        }
        _ if control.polling() => ("Awaiting first check".into(), false),
        _ => ("Not polling".into(), false),
    }
}

// ---------------------------------------------------- recorded activity
//
// Ported from `impl SourceEvent` in `src/app/source_ui/activity.rs`. Event keys are
// `system:height:sourceA:sourceB:hashA:hashB` for a block change or disagreement, and
// `reorg:system:source:…` for a committed reorganization.

fn event_block_scope(key: &str) -> Option<(&str, &str, &str)> {
    let (system, rest) = key.split_once(':')?;
    let (height, rest) = rest.split_once(':')?;
    height.parse::<u64>().ok()?;
    // The two trailing components are block hashes. Endpoint IDs themselves contain a
    // colon, so keep the remaining pair intact for matching.
    let (sources, _) = rest.rsplit_once(':')?.0.rsplit_once(':')?;
    Some((system, height, sources))
}

fn event_reorg_scope(key: &str) -> Option<(&str, &str)> {
    let (system, rest) = key.strip_prefix("reorg:")?.split_once(':')?;
    Some((system, rest.rsplit_once(':')?.0))
}

fn event_system(key: &str) -> Option<&str> {
    event_reorg_scope(key)
        .map(|(system, _)| system)
        .or_else(|| event_block_scope(key).map(|(system, _, _)| system))
}

fn event_involves(key: &str, system: &str, source: &str) -> bool {
    if let Some(scope) = event_reorg_scope(key) {
        return scope == (system, source);
    }
    event_block_scope(key).is_some_and(|(chain, _, sources)| {
        chain == system
            && (sources
                .strip_prefix(source)
                .is_some_and(|s| s.starts_with(':'))
                || sources
                    .strip_suffix(source)
                    .is_some_and(|s| s.ends_with(':')))
    })
}

fn event_status(key: &str, message: &str) -> &'static str {
    if event_reorg_scope(key).is_some() {
        "Reconciled"
    } else if message.contains(": sources disagree at height ") {
        "Not rechecked"
    } else {
        "Recorded"
    }
}

fn event_summary(key: &str, message: &str) -> String {
    if let Some((_, height, _)) = event_block_scope(key) {
        let label = if event_status(key, message) == "Not rechecked" {
            "Sources differed"
        } else {
            "Block changed"
        };
        return format!("{label} at height {height}");
    }
    if event_reorg_scope(key).is_some()
        && let Some((_, message)) = message
            .lines()
            .next()
            .and_then(|line| line.split_once(": "))
    {
        let summary = message
            .rsplit_once(" (")
            .map_or(message, |(summary, _)| summary);
        return format!("Reorg · {summary}");
    }
    message.to_string()
}

fn activity_rows(
    events: Vec<crate::cache::SourceEvent>,
    scope: Option<(&str, &str)>,
) -> Vec<ActivityRow> {
    events
        .into_iter()
        .filter(|event| {
            scope.is_none_or(|(system, source)| event_involves(&event.key, system, source))
        })
        .map(|event| ActivityRow {
            observed_at: event.observed_at,
            status: event_status(&event.key, &event.message).to_string(),
            summary: event_summary(&event.key, &event.message),
            system_id: event_system(&event.key).map(str::to_owned),
            message: event.message,
        })
        .collect()
}

// ------------------------------------------------------ block history

fn membership(text: &str) -> Membership {
    match text {
        "member" => Membership::Active,
        "orphan" => Membership::Orphaned,
        _ => Membership::Unverified,
    }
}

// --------------------------------------------------------- proof roots

/// A block header as a proof-root check needs it. Ported from `header` in
/// `src/app/notarization_ui.rs`.
#[derive(Debug, Clone, PartialEq)]
struct RootHeader {
    height: u64,
    time: Option<u64>,
    confirmations: Option<i64>,
}

fn root_header(value: &Value, hash: &str) -> Option<RootHeader> {
    if value["hash"].as_str()? != hash {
        return None;
    }
    Some(RootHeader {
        height: value["height"].as_u64()?,
        time: value["time"].as_u64(),
        confirmations: value["confirmations"].as_i64(),
    })
}

/// Classifies a found header against the proof root. Ported from `root_status` in
/// `src/app/notarization_ui.rs`, which also supplies the wording.
fn found_status(root_height: Option<u64>, header: &RootHeader) -> (ProofRootStatus, String) {
    match (root_height, header.confirmations) {
        (Some(height), _) if height != header.height => (
            ProofRootStatus::HeightMismatch,
            format!("Height mismatch (found #{})", header.height),
        ),
        (_, Some(-1)) => (
            ProofRootStatus::OffChain,
            "Known off-chain • possible fork".into(),
        ),
        (_, Some(n)) if n > 0 => (ProofRootStatus::Active, "Active".into()),
        _ => (
            ProofRootStatus::PresentMembershipUnknown,
            "Present • membership unavailable".into(),
        ),
    }
}

fn missing_status(root_height: Option<u64>, tip: Option<u64>) -> (ProofRootStatus, String) {
    match tip {
        Some(tip) if root_height.is_some_and(|height| height > tip) => (
            ProofRootStatus::SourceBehind,
            format!("Not found • source tip #{tip} is behind"),
        ),
        _ => (
            ProofRootStatus::NotFound,
            "Not found • possible fork".into(),
        ),
    }
}

/// Carbide's `CheckFailure` labels. "see F2" names the terminal's key; the app has Sources.
fn failure_label(status: ProofRootStatus) -> &'static str {
    match status {
        ProofRootStatus::NoSourceConfigured => "No source configured for this chain",
        ProofRootStatus::NoSourceEnabled => "No enabled source (see Sources)",
        ProofRootStatus::ChainQueryFailed => "Could not query chain status",
        ProofRootStatus::LookupFailed => "Block header lookup failed",
        ProofRootStatus::InvalidHeader => "Source returned an invalid block header",
        ProofRootStatus::MissingHash => "Proof root has no block hash",
        _ => "",
    }
}

fn root_failure(status: ProofRootStatus) -> ProofRootCheck {
    ProofRootCheck {
        status,
        label: failure_label(status).into(),
        source_id: None,
        source: None,
        source_tip: None,
        found_height: None,
        found_time: None,
    }
}

// --------------------------------------------------------- monitoring

/// One node's background work, as the terminal's dashboard poller does it: Carbide's
/// reconciler keeps the stored block history current, and a monitored node also reports
/// its mempool. It never uses an API. It stops when `alive` is cleared.
async fn monitor(
    mut config: Config,
    alive: Arc<AtomicBool>,
    recorded: bool,
    revision: Arc<tokio::sync::watch::Sender<u64>>,
) {
    config.daemon.api_pool = Arc::default();
    config.daemon.background = true;
    config.daemon.observations = None;
    let control = config.daemon.control.clone();
    let mempool = if recorded {
        RpcClient::for_config(&config).ok()
    } else {
        RpcClient::new(config.daemon.clone()).ok()
    }
    .map(RpcClient::noninteractive);
    let mut reconciler: Option<crate::reconciliation::Reconciler> = None;
    let mut failures = 0u32;
    while alive.load(Ordering::Acquire) {
        if !control.polling() {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        let started = Instant::now();
        let generation = control.generation();
        let mut healthy = true;
        if !recorded {
            control.record_chain_check(
                generation,
                ChainCheck::Finished {
                    at: Instant::now(),
                    error: Some("The cache is not open. The chain check needs the cache.".into()),
                },
            );
        } else if reconciler.is_none() {
            match crate::reconciliation::Reconciler::new(config.clone()).await {
                Ok(created) => reconciler = Some(created),
                Err(error) => {
                    healthy = false;
                    control.record_chain_check(
                        generation,
                        ChainCheck::Finished {
                            at: Instant::now(),
                            error: Some(format!("{error:#}")),
                        },
                    );
                }
            }
        }
        if let Some(reconciler) = reconciler.as_mut() {
            let result = reconciler
                .reconcile(|| control.record_chain_check(generation, ChainCheck::Checking))
                .await;
            healthy &= result.is_ok();
            // A committed change of the stored branch: views of stored history reload.
            if let Ok(change) = &result
                && (!change.attached.is_empty() || !change.detached.is_empty())
            {
                revision.send_modify(|value| *value += 1);
            }
            control.record_chain_check(
                generation,
                ChainCheck::Finished {
                    at: Instant::now(),
                    error: result.err().map(|error| format!("{error:#}")),
                },
            );
        }
        if control.state() == ConnectionState::Monitored
            && let Some(client) = &mempool
        {
            healthy &= client
                .call_with_raw::<Value>("getrawmempool", &[])
                .await
                .is_ok();
        }
        failures = if healthy {
            0
        } else {
            failures.saturating_add(1)
        };
        let interval = Duration::from_secs(
            config
                .poll_interval_seconds
                .max(5)
                .saturating_mul(1u64 << failures.min(5))
                .min(120),
        );
        while alive.load(Ordering::Acquire) && control.polling() && started.elapsed() < interval {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
}

// ----------------------------------------------------------- Swift API

#[uniffi::export(async_runtime = "tokio")]
impl Explorer {
    /// Every node and API, with its state and observed traffic.
    pub fn sources_overview(&self) -> SourcesOverview {
        SourcesOverview {
            rows: self.named_rows(),
            settings_file: self
                .sources
                .settings_path
                .as_ref()
                .map(|path| path.display().to_string()),
            settings_problem: self.sources.settings_problem.clone(),
            unsaved: session().unsaved,
            cache_problem: self.cache_problem.clone(),
        }
    }

    /// The sources of the chain with this system ID. Empty when none is configured.
    pub fn sources_for_system(&self, system_id: String) -> Vec<SourceRow> {
        self.named_rows()
            .into_iter()
            .filter(|row| row.system_id == system_id.trim())
            .collect()
    }

    /// Changes a source's state for this session. `save_sources` writes it to the file.
    pub fn set_source_state(&self, key: String, state: SourceState) -> Outcome<()> {
        let (config, api) = self
            .sources
            .control(&key)
            .ok_or_else(|| fail("The source is no longer available."))?;
        let (control, session_key) = match api {
            Some(endpoint) => {
                if state == SourceState::Monitored {
                    return Err(fail(
                        "An API cannot be monitored. The app never sends background requests to an API.",
                    ));
                }
                (
                    &endpoint.control,
                    api_key(&config.chain_cache_key(), &endpoint.url),
                )
            }
            None => (
                &config.daemon.control,
                daemon_key(
                    config
                        .daemon
                        .config_file
                        .as_deref()
                        .unwrap_or(Path::new("")),
                ),
            ),
        };
        let state = connection_state(state);
        if control.state() != state {
            control.set(state);
            let mut session = session();
            session.states.insert(session_key, state);
            session.unsaved = true;
        }
        Ok(())
    }

    /// Writes every source's state to Carbide's settings file, which the terminal app reads.
    pub fn save_sources(&self) -> Outcome<()> {
        let mut registry = self
            .sources
            .registry
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let registry = registry.as_mut().ok_or_else(|| {
            fail(
                self.sources
                    .settings_problem
                    .as_deref()
                    .unwrap_or("The settings file is not available."),
            )
        })?;
        registry.save()?;
        session().unsaved = false;
        Ok(())
    }

    /// Significant events that the cache recorded for one source, newest first, or for
    /// every source when `all` is true. The cache keeps at most 200.
    pub async fn source_activity(&self, key: String, all: bool) -> Outcome<Vec<ActivityRow>> {
        let cache = self.cache.clone().ok_or_else(|| {
            fail(
                self.cache_problem
                    .as_deref()
                    .unwrap_or("The cache is not open."),
            )
        })?;
        let scope = if all {
            None
        } else {
            let (config, api) = self
                .sources
                .control(&key)
                .ok_or_else(|| fail("The source is no longer available."))?;
            let source = match api {
                Some(endpoint) => crate::rpc::source_id("public-api", &endpoint.url),
                None => config.daemon_cache_key(),
            };
            Some((config.chain_cache_key(), source))
        };
        let events = tokio::task::spawn_blocking(move || cache.events())
            .await
            .context("the activity request stopped")??;
        Ok(activity_rows(
            events,
            scope
                .as_ref()
                .map(|(system, source)| (system.as_str(), source.as_str())),
        ))
    }

    /// Starts the background work for each node: reconciliation for enabled nodes of the
    /// current chain and for monitored nodes, and mempool reports for monitored nodes.
    /// Call it once after connecting. Later calls do nothing.
    pub async fn start_background(&self) {
        if self.sources.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
        let mut mine = self
            .sources
            .monitors
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for config in self.sources.configs() {
            if config.daemon.config_file.is_none() {
                continue;
            }
            let alive = Arc::new(AtomicBool::new(true));
            let key = format!("{}|{}", config.chain_cache_key(), config.daemon_cache_key());
            if let Some(previous) = running.insert(key, alive.clone()) {
                previous.store(false, Ordering::Release);
            }
            mine.push(alive.clone());
            tokio::spawn(monitor(
                config.clone(),
                alive,
                self.cache.is_some(),
                self.sources.revision.clone(),
            ));
        }
    }

    /// A count of committed changes of the stored block history in this connection: a new
    /// block or a reorganization, committed by the background reconciliation. Views of stored
    /// history reload when it changes, as the terminal's F7 panel does. Reads memory only.
    pub fn observation_revision(&self) -> u64 {
        *self.sources.revision.borrow()
    }

    /// Waits up to 30 seconds for `observation_revision` to pass `after`, and returns the
    /// revision. Sends no request: call it in a loop to reload when the history changes.
    pub async fn wait_for_revision(&self, after: u64) -> u64 {
        let mut receiver = self.sources.revision.subscribe();
        let _ = tokio::time::timeout(
            Duration::from_secs(30),
            receiver.wait_for(|revision| *revision > after),
        )
        .await;
        *self.sources.revision.borrow()
    }

    /// Waits up to 15 seconds for a primary request that needs the person's permission to
    /// use an API. Returns `None` when there is none. Call it in a loop while connected,
    /// and answer each request with `answer_permission`. Automatic requests never ask.
    pub async fn next_permission_request(&self) -> Option<PermissionRequest> {
        let mut receiver = self.sources.consent.lock().await;
        let consent = tokio::time::timeout(Duration::from_secs(15), receiver.recv())
            .await
            .ok()??;
        let id = self.sources.next_request.fetch_add(1, Ordering::AcqRel);
        let request = PermissionRequest {
            id,
            chain: consent.chain.clone(),
            method: consent.method.clone(),
            failure: readable(&anyhow::anyhow!(consent.local_failure.clone())),
            endpoints: consent.sources.clone(),
        };
        self.sources
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id,
                Pending {
                    reply: consent.reply,
                    chain: consent.chain,
                    endpoints: consent.sources,
                },
            );
        Some(request)
    }

    /// Answers a permission request. Enabling an API applies to this session; Sources can
    /// save it.
    pub fn answer_permission(&self, id: u64, answer: PermissionAnswer) -> Outcome<()> {
        let pending = self
            .sources
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id)
            .ok_or_else(|| fail("The request is no longer waiting."))?;
        let decision = match answer {
            PermissionAnswer::Retry => ApiDecision::Retry,
            PermissionAnswer::Cancel => ApiDecision::Cancel,
            PermissionAnswer::Enable { index } => {
                let origin = pending
                    .endpoints
                    .get(index as usize)
                    .ok_or_else(|| fail("The selected API is not in the list."))?;
                // Carbide's client enables the endpoint itself. Remember it for the session,
                // so that it stays enabled when the explorer reconnects.
                let mut session = session();
                for config in self
                    .sources
                    .configs()
                    .iter()
                    .filter(|config| config.chain.name == pending.chain)
                {
                    for endpoint in &config.daemon.api_pool.endpoints {
                        if origin_of(&endpoint.url) == *origin {
                            session.states.insert(
                                api_key(&config.chain_cache_key(), &endpoint.url),
                                ConnectionState::Enabled,
                            );
                            session.unsaved = true;
                        }
                    }
                }
                ApiDecision::Enable(index as usize)
            }
        };
        pending
            .reply
            .send(decision)
            .map_err(|_| fail("The request is no longer waiting."))?;
        Ok(())
    }

    /// Every block that a transaction is known to have been in, from stored observations
    /// only: the terminal's F7. Each source stays separate. Pages hold 100 rows. An empty
    /// list does not prove that no block contained the transaction.
    pub async fn inclusion_history(&self, txid: String, page: u32) -> Outcome<ObservedBlockPage> {
        self.investigate(
            crate::cache::investigation::Investigation::Inclusions(txid.trim().to_lowercase()),
            page,
        )
        .await
    }

    /// Every known child of a block, from stored observations only. More than one child
    /// means that sources reported more than one branch after this block.
    pub async fn block_children(
        &self,
        parent_hash: String,
        page: u32,
    ) -> Outcome<ObservedBlockPage> {
        self.investigate(
            crate::cache::investigation::Investigation::Children(parent_hash.trim().to_lowercase()),
            page,
        )
        .await
    }

    /// Checks a notarization's proof root on an enabled source of the chain that the root
    /// names, as Carbide's notarization browser does. `system_id` is the root's system,
    /// `block_hash` and `height` are the root's own values. The current chain is also
    /// accepted. The check never asks for API permission and never uses a disabled source.
    pub async fn check_proof_root(
        &self,
        system_id: String,
        block_hash: Option<String>,
        height: Option<u64>,
    ) -> ProofRootCheck {
        let Some(hash) = block_hash
            .map(|hash| hash.trim().to_lowercase())
            .filter(|hash| !hash.is_empty())
        else {
            return root_failure(ProofRootStatus::MissingHash);
        };
        let client = match self.client_for_system(system_id.trim()) {
            Ok(client) => client,
            Err(status) => return root_failure(status),
        };
        // Pin the lookup and its tip to the same endpoint.
        let Ok(tip) = client.get_blockchain_info().await else {
            return root_failure(ProofRootStatus::ChainQueryFailed);
        };
        let source = tip.source_id.clone();
        let source_tip = tip.result["blocks"].as_u64();
        let (label, _) = self.sources.label(&source);
        let mut check = ProofRootCheck {
            status: ProofRootStatus::LookupFailed,
            label: String::new(),
            source_id: Some(source.clone()),
            source: Some(label),
            source_tip,
            found_height: None,
            found_time: None,
        };
        let (status, text) = match client
            .call_on(&source, "getblockheader", &[json!(hash), json!(true)])
            .await
        {
            Ok(value) => match root_header(&value, &hash) {
                Some(header) => {
                    check.found_height = Some(header.height);
                    check.found_time = header.time;
                    found_status(height, &header)
                }
                None => (
                    ProofRootStatus::InvalidHeader,
                    failure_label(ProofRootStatus::InvalidHeader).into(),
                ),
            },
            Err(error)
                if error
                    .downcast_ref::<crate::rpc::ApplicationError>()
                    .is_some_and(|error| error.is_block_not_found()) =>
            {
                missing_status(height, source_tip)
            }
            Err(_) => (
                ProofRootStatus::LookupFailed,
                failure_label(ProofRootStatus::LookupFailed).into(),
            ),
        };
        check.status = status;
        check.label = text;
        check
    }
}

impl Explorer {
    /// A client for another chain's sources, for requests about that chain's objects. It
    /// never asks for API permission, records its answers in the cache, and uses only
    /// enabled sources. The current chain's system ID returns `self.client`.
    /// The error is a `ProofRootStatus` whose Carbide label `failure_label` gives.
    pub(crate) fn client_for_system(
        &self,
        system_id: &str,
    ) -> std::result::Result<RpcClient, ProofRootStatus> {
        use crate::sources::ConnectionState::Disabled;
        if system_id == self.config.chain_cache_key() {
            return Ok(self.client.clone());
        }
        let config = self
            .sources
            .config_for_system(system_id)
            .ok_or(ProofRootStatus::NoSourceConfigured)?;
        let daemon = &config.daemon;
        let usable_node = daemon.config_file.is_some() && daemon.control.state() != Disabled;
        if !usable_node
            && daemon
                .api_pool
                .endpoints
                .iter()
                .all(|api| api.control.state() == Disabled)
        {
            return Err(ProofRootStatus::NoSourceEnabled);
        }
        let client = if self.cache.is_some() {
            RpcClient::for_config(&config.noninteractive())
        } else {
            RpcClient::new(config.daemon.clone()).map(RpcClient::noninteractive)
        };
        client.map_err(|_| ProofRootStatus::ChainQueryFailed)
    }

    /// The rows, with each chain's name where a name is known. A PBaaS chain's folder is
    /// named by its hex ID, which is not a name.
    fn named_rows(&self) -> Vec<SourceRow> {
        let mut rows = self.sources.rows();
        for row in &mut rows {
            row.name = self
                .cached_name(&row.system_id)
                .map(|name| name.trim_end_matches('@').to_string())
                .or_else(|| {
                    let hex = row.chain.len() == 40
                        && row.chain.bytes().all(|b| b.is_ascii_hexdigit());
                    (!hex).then(|| row.chain.clone())
                });
        }
        rows
    }

    async fn investigate(
        &self,
        subject: crate::cache::investigation::Investigation,
        page: u32,
    ) -> Outcome<ObservedBlockPage> {
        let cache = self.cache.clone().ok_or_else(|| {
            fail(
                self.cache_problem
                    .as_deref()
                    .unwrap_or("The cache is not open."),
            )
        })?;
        let found = cache
            .investigate(self.config.chain_cache_key(), subject, page as usize)
            .await?;
        Ok(ObservedBlockPage {
            rows: found
                .rows
                .into_iter()
                .map(|row| ObservedBlock {
                    source: self.sources.label(&row.source).0,
                    membership: membership(&row.membership),
                    block_hash: row.hash,
                    height: row.height,
                    time: row.timestamp,
                    source_id: row.source,
                    observed_at: row.observed_at.and_then(|at| u64::try_from(at).ok()),
                    cached_body: row.cached,
                })
                .collect(),
            page,
            more: found.more,
        })
    }
}

/// The origin that Carbide shows for an API in a permission request.
fn origin_of(url: &str) -> String {
    reqwest::Url::parse(url)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| "Configured API".into())
}
