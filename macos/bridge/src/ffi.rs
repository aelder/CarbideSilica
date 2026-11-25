//! The Swift-facing API of the desktop bridge.
//!
//! Everything here is read-only, like Carbide itself: each call maps to one or
//! more query RPCs issued through Carbide's `RpcClient`. There is deliberately
//! no generic "call any RPC method" entry point, because the local daemon may
//! hold a wallet.
//!
//! Records carry the fields the views show plus, where useful, the daemon's
//! JSON so the app can offer a raw view of the same data.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use serde_json::Value;

use crate::block_time::{TimeBlock, TimeSource};
use crate::cli::Cli;
use crate::config::Config;
use crate::domain::{BlockSummary, ObjectRoute, TransactionSummary};
use crate::rpc::{BlockId, RpcCallOutput, RpcClient};
use crate::world::Network;

// ---------------------------------------------------------------- errors

#[derive(Debug, uniffi::Error)]
pub enum CarbideError {
    Failed { message: String },
}

impl std::fmt::Display for CarbideError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Failed { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for CarbideError {}

impl From<anyhow::Error> for CarbideError {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed {
            message: readable(&error),
        }
    }
}

/// Carbide formats daemon errors as `RPC error from <method>: <json>`.
/// Show the daemon's own message when there is one.
fn readable(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    if let Some(start) = text.find("RPC error from ")
        && let Some((_, json)) = text[start..].split_once(": ")
        && let Ok(value) = serde_json::from_str::<Value>(json)
        && let Some(message) = value["message"].as_str()
    {
        return message.trim().to_string();
    }
    if text.contains("error sending request") {
        return "The app cannot connect to the Verus node. Make sure that the node is running, or enable \
                a public API in Sources."
            .into();
    }
    if text.contains("HTTP 401") {
        return "The node refused the request. The RPC user and password in its configuration \
                file are not correct, or the file has none."
            .into();
    }
    // Carbide's terminal wording names its F2 key. The app has a Sources page instead.
    if text.contains("enable it in Sources (F2)") {
        return "The source is disabled. Enable it in Sources.".into();
    }
    if text.contains("API action cancelled") {
        return "The request was cancelled. No public API was used.".into();
    }
    text
}

type Outcome<T> = std::result::Result<T, CarbideError>;

// --------------------------------------------------------------- records

#[derive(Clone, Default, uniffi::Record)]
pub struct ConnectionOptions {
    pub chain: String,
    pub testnet: bool,
    /// Absolute path to a daemon `.conf` file. `None` uses the standard location.
    pub config_file: Option<String>,
    /// A public JSON-RPC endpoint the user has chosen to allow as a fallback.
    pub api_endpoint: Option<String>,
    /// Folder for Carbide's cache of looked-up data. `None` uses the platform cache folder.
    pub cache_dir: Option<String>,
    /// The node's data directory, as Carbide's `-datadir`. The configuration file is then
    /// looked for there, not in the standard location.
    #[uniffi(default = None)]
    pub data_dir: Option<String>,
    /// Carbide's `-rpcconnect`, `-rpcport`, `-rpcuser` and `-rpcpassword`. Each overrides the
    /// value in the configuration file.
    #[uniffi(default = None)]
    pub rpc_host: Option<String>,
    #[uniffi(default = None)]
    pub rpc_port: Option<u16>,
    #[uniffi(default = None)]
    pub rpc_user: Option<String>,
    #[uniffi(default = None)]
    pub rpc_password: Option<String>,
}

// Never print the RPC password.
impl std::fmt::Debug for ConnectionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionOptions")
            .field("chain", &self.chain)
            .field("testnet", &self.testnet)
            .field("config_file", &self.config_file)
            .field("api_endpoint", &self.api_endpoint)
            .field("cache_dir", &self.cache_dir)
            .field("data_dir", &self.data_dir)
            .field("rpc_host", &self.rpc_host)
            .field("rpc_port", &self.rpc_port)
            .field("rpc_user", &self.rpc_user)
            .field("rpc_password", &self.rpc_password.as_ref().map(|_| "<hidden>"))
            .finish()
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConnectionInfo {
    pub chain: String,
    pub testnet: bool,
    pub daemon_url: String,
    pub config_file: Option<String>,
    pub config_found: bool,
    pub has_credentials: bool,
    pub api_endpoint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DataSource {
    LocalDaemon,
    PublicApi,
}

/// The kind of source behind an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SourceKind {
    LocalNode,
    PublicApi,
    /// A source this session does not know, such as one recorded by the terminal app.
    Unknown,
}

/// Where an answer came from. Every record that shows an object can carry one.
///
/// A live answer came from a source during this request. A retained answer is the
/// latest stored observation of the same request, served because no source answered.
/// It shows what the source reported then; it does not show the chain now.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Provenance {
    /// Carbide's stable ID for the source, such as `local-daemon:<hash>`.
    pub source_id: String,
    /// The source in words: a node's chain and address, or an API's URL.
    pub source: String,
    pub kind: SourceKind,
    /// True when the answer is a stored observation, not a live answer.
    pub retained: bool,
    /// When the source gave the answer, in Unix seconds.
    pub observed_at: u64,
    /// The block height that the answer names, when it names one.
    pub observed_height: Option<u64>,
    /// For a retained answer: why no source answered now.
    pub failure: Option<String>,
    /// For a retained answer: a limit of the stored data, in words.
    pub caution: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ChainStatus {
    pub name: String,
    pub chain_id: Option<String>,
    pub version: Option<String>,
    pub blocks: u64,
    pub longest_chain: Option<u64>,
    pub connections: Option<u64>,
    pub difficulty: Option<f64>,
    pub tip_time: Option<u64>,
    pub testnet: bool,
    pub source: DataSource,
    /// Where the status came from. Retained when no source answered and the app shows the
    /// latest stored `getinfo` instead.
    pub provenance: Provenance,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BlockRow {
    pub height: u64,
    pub hash: String,
    pub time: u64,
    pub transaction_count: u32,
    pub size: Option<u64>,
    /// "mined", "staked", or the daemon's own block type.
    pub production: String,
    pub staked: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Field {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TransactionRow {
    pub txid: String,
    pub tags: Vec<String>,
    pub input_count: u32,
    pub output_count: u32,
    pub output_sats: Option<i64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CurrencyAmount {
    pub currency: String,
    pub amount: f64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ReserveRow {
    pub currency_id: String,
    pub name: Option<String>,
    pub weight: Option<f64>,
    pub reserves: Option<f64>,
    pub price_in_reserve: Option<f64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CurrencyDetail {
    pub name: String,
    pub fully_qualified_name: Option<String>,
    pub currency_id: Option<String>,
    pub parent: Option<String>,
    pub system_id: Option<String>,
    pub launch_status: String,
    pub option_tags: Vec<String>,
    pub options: Option<u64>,
    pub proof_protocol: Option<u64>,
    pub start_block: Option<u64>,
    pub end_block: Option<u64>,
    pub supply: Option<f64>,
    pub reserves: Vec<ReserveRow>,
    pub definition_txid: Option<String>,
    pub last_confirmed_height: Option<u64>,
    pub json: String,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PeerRow {
    pub id: i64,
    pub address: String,
    pub inbound: bool,
    /// Round-trip time of the last ping, in milliseconds.
    pub ping_ms: Option<f64>,
    /// The last block this peer and the node have in common.
    pub synced_blocks: Option<i64>,
    /// The peer's own chain height when it connected.
    pub starting_height: Option<i64>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub version: Option<String>,
    pub connected_since: Option<u64>,
    pub json: String,
}

/// What the local node is doing: how far it has validated and who it is connected to.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NodeStatus {
    pub blocks: u64,
    pub longest_chain: Option<u64>,
    /// Timestamp of the newest validated block.
    pub tip_time: Option<u64>,
    /// The furthest block header any peer has delivered; headers arrive ahead of blocks.
    pub synced_headers: Option<u64>,
    pub mempool_count: Option<u64>,
    pub peers: Vec<PeerRow>,
    /// Set when peers could not be listed (they are only available from a local daemon).
    pub peers_error: Option<String>,
    pub source: DataSource,
    /// Where the height figures came from. Peers and the mempool are always live.
    pub provenance: Provenance,
}

/// A resolved name for an identity or currency ID.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NameEntry {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ReservePoint {
    pub currency_id: String,
    pub reserves: Option<f64>,
    pub price_in_reserve: Option<f64>,
    /// What the most recent import batch before this block moved into the pool.
    pub last_in: f64,
    /// What that batch paid out of the pool.
    pub last_out: f64,
    pub last_fees: f64,
}

/// A currency's recorded state at one block.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CurrencyPoint {
    pub height: u64,
    pub time: Option<u64>,
    pub supply: Option<f64>,
    pub reserves: Vec<ReservePoint>,
}

/// The states of a currency over a range of blocks.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CurrencyHistory {
    pub points: Vec<CurrencyPoint>,
    /// For a currency of a different system: the last block of this chain that a
    /// confirmed notarization of that system commits to. The range stops there.
    pub limit: Option<u64>,
    /// False when the node was slow and the app stopped before the end of the range.
    pub complete: bool,
}

/// How well a fact is established. Carbide's many provenance labels reduce to these four.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Confidence {
    /// Proven by exact match or replay.
    Verified,
    /// Reported by a source at a known block; in progress or not re-checked.
    Observed,
    /// A reasoned guess.
    Candidate,
    /// A gap in what can be shown.
    Unknown,
}

/// The end of a chain of blocks the node knows about.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChainTip {
    pub height: u64,
    pub hash: String,
    /// Blocks on this branch since it left the active chain. Zero for the active chain.
    pub branch_length: u64,
    /// The daemon's status: active, valid-fork, valid-headers, headers-only or invalid.
    pub status: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OfferKind {
    IdentityForSale,
    IdentityWanted,
    CurrencySwap,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BookmarkRow {
    pub index: u32,
    pub kind: String,
    pub target: String,
    pub description: String,
    pub created_at: u64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CacheInfo {
    /// The database file for this network.
    pub path: String,
    /// The database and its write-ahead files together.
    pub bytes: u64,
    /// The folder the app uses when no folder is chosen.
    pub default_folder: String,
    /// Set when the app cannot use the cache, with the reason. Answers are then not recorded.
    pub problem: Option<String>,
}

/// A block a transaction appears in.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Inclusion {
    pub block_hash: String,
    pub height: Option<u64>,
    /// True when the block is on the active chain.
    pub active: bool,
}

/// A change the app can ask the local daemon to make to its peer connections.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum PeerAction {
    AddNode { address: String },
    PingAll,
    Disconnect { address: String },
    Ban { ip: String },
    ClearBans,
}

/// A daemon configuration found on this computer.
#[derive(Debug, Clone, uniffi::Record)]
pub struct KnownChain {
    /// The name to connect with: VRSC, vrsctest, or a PBaaS chain's folder name.
    pub chain: String,
    pub config_file: String,
    pub testnet: bool,
}

/// Where a search query leads.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Route {
    Block { id: String },
    Transaction { txid: String },
    Identity { name: String },
    Currency { name: String },
    Address { address: String },
    /// The currency browser, from the `currencies` search.
    Currencies,
    /// A future block or moment, from `estimate:` or a search above the tip. `query` is an
    /// estimator target frozen at search time: a height, or `unix:` and seconds.
    Estimate { query: String },
}

// -------------------------------------------------------------- explorer

#[derive(uniffi::Object)]
pub struct Explorer {
    /// For every request that must not ask the person anything: automatic enrichment,
    /// lists and background work. It never asks for API permission. When the cache is
    /// open, every answer is recorded there as an observation, as in the terminal app.
    client: RpcClient,
    /// For explicit primary requests only: opening a block, transaction, identity,
    /// currency or address. When the local node cannot answer and a disabled API could,
    /// it waits for the person's decision (see `next_permission_request`).
    primary: RpcClient,
    /// Carbide's cache of observations. `None` when it cannot be opened (`cache_problem`).
    cache: Option<crate::cache::Cache>,
    cache_problem: Option<String>,
    /// Every configured node and API, their states, and pending permission requests.
    sources: sources::Sources,
    config: Config,
    api_endpoint: Option<String>,
    config_found: bool,
    /// Other chains configured on this computer, so traces can follow transfers into them.
    other_chains: Vec<Config>,
    /// Names already resolved for identity and currency IDs.
    names: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

const MAX_RECENT_BLOCKS: u32 = 100;
const MAX_NAME_LOOKUPS: usize = 48;
const REVERSE_WINDOW: u64 = 1_000;
const MAX_BRANCH_WALK: u64 = 20;
const LOG_TAIL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_LOG_LINES: usize = 200;
const MAX_HISTORY_SAMPLES: u32 = 200;
/// The states of one request to the node.
const HISTORY_CHUNK: u64 = 15;
/// The app asks for no more states after this time.
const HISTORY_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);
const MAX_BLOCK_TX_BODIES: usize = 150;

#[uniffi::export(async_runtime = "tokio")]
impl Explorer {
    /// Reads the daemon configuration. Does not contact the daemon.
    #[uniffi::constructor]
    pub fn new(options: ConnectionOptions) -> Outcome<Arc<Self>> {
        Ok(Arc::new(Self::open(options)?))
    }

    /// Like `new`, and also lists `offered_apis` as disabled public API sources for this
    /// chain, so that the app can offer them when the local node cannot answer. Listing an
    /// API does not permit any request to it.
    #[uniffi::constructor]
    pub fn with_sources(
        options: ConnectionOptions,
        offered_apis: Vec<String>,
    ) -> Outcome<Arc<Self>> {
        Ok(Arc::new(Self::open_with(options, &offered_apis)?))
    }

    pub fn connection(&self) -> ConnectionInfo {
        ConnectionInfo {
            chain: self.config.chain.name.clone(),
            testnet: self.config.chain.testnet,
            daemon_url: format!(
                "http://{}:{}",
                self.config.daemon.host, self.config.daemon.port
            ),
            config_file: self
                .config
                .daemon
                .config_file
                .as_ref()
                .map(|path| path.display().to_string()),
            config_found: self.config_found,
            has_credentials: self.config.daemon.user.is_some()
                && self.config.daemon.password.is_some(),
            api_endpoint: self.api_endpoint.clone(),
        }
    }

    /// The chain's status. Automatic: it never asks for permission. When no source answers,
    /// it serves the latest stored status, marked as retained.
    pub async fn chain_status(&self) -> Outcome<ChainStatus> {
        let output = self
            .observed::<Value>("getinfo", &[], self.client.get_info().await)
            .await?;
        let info = &output.result;
        Ok(ChainStatus {
            name: text(&info["name"]).unwrap_or_else(|| self.config.chain.name.clone()),
            chain_id: text(&info["chainid"]),
            version: text(&info["VRSCversion"])
                .or_else(|| info["version"].as_u64().map(|v| v.to_string())),
            blocks: info["blocks"].as_u64().unwrap_or(0),
            longest_chain: info["longestchain"].as_u64(),
            connections: info["connections"].as_u64(),
            difficulty: info["difficulty"].as_f64(),
            tip_time: info["tiptime"].as_u64(),
            testnet: info["testnet"]
                .as_bool()
                .unwrap_or(self.config.chain.testnet),
            source: source_of(&output.source_id),
            provenance: output.provenance,
        })
    }

    /// Sync progress, mempool size and peers. Peers come only from the local daemon.
    pub async fn node_status(&self) -> Outcome<NodeStatus> {
        let output = self
            .observed::<Value>("getinfo", &[], self.client.get_info().await)
            .await?;
        let info = &output.result;
        let (peers, peers_error) = match self.settle(
            self.client
                .call_with_raw::<Vec<Value>>("getpeerinfo", &[])
                .await,
        ) {
            Ok(list) => (list.result.iter().map(peer_row).collect::<Vec<_>>(), None),
            Err(error) => (Vec::new(), Some(readable(&error))),
        };
        let mempool_count = self
            .settle(
                self.client
                    .call_with_raw::<Value>("getmempoolinfo", &[])
                    .await,
            )
            .ok()
            .and_then(|pool| pool.result["size"].as_u64());
        Ok(NodeStatus {
            blocks: info["blocks"].as_u64().unwrap_or(0),
            longest_chain: info["longestchain"].as_u64(),
            tip_time: info["tiptime"].as_u64(),
            synced_headers: peers.iter().filter_map(|peer| peer.synced_headers).max(),
            mempool_count,
            peers: peers.into_iter().map(|peer| peer.row).collect(),
            peers_error,
            source: source_of(&output.source_id),
            provenance: output.provenance,
        })
    }

    /// The `count` blocks ending at `before` (or at the tip), newest first.
    pub async fn recent_blocks(&self, before: Option<u64>, count: u32) -> Outcome<Vec<BlockRow>> {
        // Automatic: never asks. Blocks that no source serves come from stored
        // observations; `chain_status` says when the dashboard shows stored data.
        let tip = match before {
            Some(height) => height,
            None => self.chain_status().await?.blocks,
        };
        let count = u64::from(count.clamp(1, MAX_RECENT_BLOCKS));
        let first = tip.saturating_sub(count - 1);
        let mut tasks = tokio::task::JoinSet::new();
        for height in first..=tip {
            let client = self.client.clone();
            tasks.spawn(async move {
                (height, fetch_block(&client, BlockId::Height(height)).await)
            });
        }
        let mut rows = Vec::new();
        let mut failure = None;
        while let Some(joined) = tasks.join_next().await {
            let (height, live) = joined.context("block request stopped")?;
            let params = [block_selector(&BlockId::Height(height))];
            match self.observed("getblock", &params, live).await {
                Ok(answer) => rows.push(block_row(&answer.result)),
                Err(error) => failure = Some(error),
            }
        }
        if rows.is_empty()
            && let Some(error) = failure
        {
            return Err(error.into());
        }
        rows.sort_by(|a, b| b.height.cmp(&a.height));
        Ok(rows)
    }

    /// Classifies a search query with Carbide's resolver and confirms what it names.
    /// The rules are in `ffi_shell.rs`.
    pub async fn resolve(&self, query: String) -> Outcome<Route> {
        self.resolve_query(&query).await
    }

    /// Resolves identity and currency IDs to names. IDs that cannot be resolved are left out.
    pub async fn names(&self, ids: Vec<String>) -> Vec<NameEntry> {
        let mut found = Vec::new();
        let mut tasks = tokio::task::JoinSet::new();
        for id in ids.into_iter().take(MAX_NAME_LOOKUPS) {
            if let Some(name) = self.cached_name(&id) {
                found.push(NameEntry { id, name });
                continue;
            }
            let client = self.client.clone();
            tasks.spawn(async move {
                let result = client.get_identity(&id, None).await;
                (id, result)
            });
        }
        while let Some(Ok((id, result))) = tasks.join_next().await {
            if let Ok(output) = self.settle(result)
                && let Some(name) = text(&output.result["friendlyname"])
                    .or_else(|| text(&output.result["fullyqualifiedname"]))
            {
                let name = self.short_name(&name);
                self.names
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id.clone(), name.clone());
                found.push(NameEntry { id, name });
            }
        }
        found
    }

    /// A currency's state sampled evenly across a range of heights, oldest first.
    pub async fn currency_history(
        &self,
        name: String,
        from_height: u64,
        to_height: u64,
        samples: u32,
    ) -> Outcome<CurrencyHistory> {
        let name = name.trim().trim_end_matches('@');
        // The node finds a state after the last confirmed notarization of a different
        // system from each block since then. That can take minutes for one state, and
        // the node answers no other request during that time.
        let limit = self.state_limit(name).await;
        let to_height = limit.map_or(to_height, |limit| to_height.min(limit));
        if to_height <= from_height {
            return match limit {
                Some(_) => Ok(CurrencyHistory { points: Vec::new(), limit, complete: true }),
                None => Err(fail("The range is empty.")),
            };
        }
        let samples = u64::from(samples.clamp(2, MAX_HISTORY_SAMPLES));
        let step = ((to_height - from_height) / (samples - 1)).max(1);
        // Short requests in sequence: the node can answer other requests between them,
        // and the app can stop when the node is slow.
        let started = std::time::Instant::now();
        let mut entries: Vec<Value> = Vec::new();
        let mut complete = true;
        let mut first = from_height;
        while first <= to_height {
            let last = (first + (HISTORY_CHUNK - 1) * step).min(to_height);
            let range = format!("{first},{last},{step}");
            let output = self.settle(
                self.client
                    .call_with_raw::<Vec<Value>>(
                        "getcurrencystate",
                        &[Value::from(name), Value::from(range)],
                    )
                    .await,
            )?;
            entries.extend(output.result);
            first += HISTORY_CHUNK * step;
            if first <= to_height && started.elapsed() > HISTORY_BUDGET {
                complete = false;
                break;
            }
        }
        let points = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let state = &entry["currencystate"];
                CurrencyPoint {
                    height: entry["height"]
                        .as_u64()
                        .unwrap_or(from_height + index as u64 * step),
                    time: entry["blocktime"].as_u64(),
                    supply: state["supply"].as_f64(),
                    reserves: state["reservecurrencies"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|reserve| {
                            let currency_id = text(&reserve["currencyid"])?;
                            let flow = &state["currencies"][&currency_id];
                            Some(ReservePoint {
                                reserves: reserve["reserves"].as_f64(),
                                price_in_reserve: reserve["priceinreserve"].as_f64(),
                                last_in: flow["reservein"].as_f64().unwrap_or(0.0),
                                last_out: flow["reserveout"].as_f64().unwrap_or(0.0),
                                last_fees: flow["conversionfees"].as_f64().unwrap_or(0.0),
                                currency_id,
                            })
                        })
                        .collect(),
                }
            })
            .collect();
        Ok(CurrencyHistory { points, limit, complete })
    }

    /// Every chain tip the node knows, tallest first: the active chain and any side branches.
    pub async fn chain_tips(&self) -> Outcome<Vec<ChainTip>> {
        let output = self.settle(
            self.client
                .call_with_raw::<Vec<Value>>("getchaintips", &[])
                .await,
        )?;
        let mut tips: Vec<ChainTip> = output
            .result
            .iter()
            .map(|tip| ChainTip {
                height: tip["height"].as_u64().unwrap_or(0),
                hash: text(&tip["hash"]).unwrap_or_default(),
                branch_length: tip["branchlen"].as_u64().unwrap_or(0),
                status: text(&tip["status"]).unwrap_or_else(|| "unknown".into()),
            })
            .collect();
        tips.sort_by(|a, b| b.height.cmp(&a.height));
        Ok(tips)
    }

    /// Saved bookmarks for this network, in Carbide's own file so both apps share them.
    pub fn bookmarks(&self) -> Outcome<Vec<BookmarkRow>> {
        Ok(self
            .book()?
            .entries
            .iter()
            .enumerate()
            .map(|(index, (_, bookmark))| BookmarkRow {
                index: index as u32,
                kind: bookmark.kind.label().to_string(),
                target: bookmark.target.clone(),
                description: bookmark.description.clone(),
                created_at: bookmark.created_at,
            })
            .collect())
    }

    /// `kind` is block, transaction, address, identity or currency.
    pub fn add_bookmark(&self, kind: String, target: String, description: String) -> Outcome<()> {
        use crate::bookmarks::Kind;
        let kind = match kind.as_str() {
            "block" => Kind::Block,
            "transaction" => Kind::Transaction,
            "address" => Kind::Address,
            "identity" => Kind::Identity,
            "currency" => Kind::Currency,
            _ => Kind::Auto,
        };
        let system_id = self
            .config
            .chain
            .system_id
            .get()
            .cloned()
            .ok_or_else(|| fail("The system ID of the chain is not known."))?;
        let bookmark = crate::bookmarks::Bookmark::new(system_id, kind, target, description)?;
        Ok(self.book()?.add(bookmark)?)
    }

    pub fn remove_bookmark(&self, index: u32) -> Outcome<()> {
        Ok(self.book()?.remove(index as usize)?)
    }

    pub fn edit_bookmark(&self, index: u32, description: String) -> Outcome<()> {
        Ok(self.book()?.edit(index as usize, description)?)
    }

    /// Where Carbide's cache of observations lives and how large it is.
    pub fn cache_info(&self) -> CacheInfo {
        CacheInfo {
            path: self.config.cache_path.display().to_string(),
            bytes: crate::cache::cache_disk_usage(&self.config.cache_path).unwrap_or(0),
            default_folder: default_cache_folder().display().to_string(),
            problem: self.cache_problem.clone(),
        }
    }

    /// Empties the cache for this network. Bookmarks, settings and the other network's
    /// cache are kept.
    pub fn clear_cache(&self) -> Outcome<()> {
        if self.config.cache_path.exists() {
            crate::cache::Cache::clear_at(&self.config.cache_path)?;
        }
        Ok(())
    }

    /// The blocks a transaction is known to be in: its block on the active chain, and any
    /// block on a side branch the node still holds near that height.
    pub async fn inclusions(&self, txid: String) -> Outcome<Vec<Inclusion>> {
        let txid = txid.trim().to_string();
        let raw = self
            .settle(self.client.get_raw_transaction(&txid).await)?
            .result;
        let height = raw["height"].as_u64();
        let mut found = Vec::new();
        if let Some(hash) = text(&raw["blockhash"]) {
            found.push(Inclusion {
                block_hash: hash,
                height,
                active: raw["confirmations"].as_i64().unwrap_or(0) > 0,
            });
        }
        let Some(height) = height else {
            return Ok(found);
        };
        // Walk back along each validated side branch that spans this height.
        for tip in self.chain_tips().await.unwrap_or_default() {
            if tip.status != "valid-fork"
                || tip.branch_length == 0
                || tip.branch_length > MAX_BRANCH_WALK
                || tip.height < height
                || tip.height - tip.branch_length >= height
            {
                continue;
            }
            let mut hash = Some(tip.hash.clone());
            for _ in 0..tip.branch_length {
                let Some(current) = hash.take() else { break };
                let Ok(block) =
                    self.settle(fetch_block(&self.client, BlockId::Hash(current.clone())).await)
                else {
                    break;
                };
                if block.result.tx.iter().any(|tx| tx.txid == txid)
                    && !found.iter().any(|seen| seen.block_hash == current)
                {
                    found.push(Inclusion {
                        block_hash: current,
                        height: Some(block.result.height),
                        active: false,
                    });
                }
                hash = block.result.previousblockhash;
            }
        }
        Ok(found)
    }

    /// Recent lines from the daemon's log that mention a peer. Local daemons only.
    pub fn peer_log(&self, address: String) -> Outcome<Vec<String>> {
        let ip = peer_ip(&address);
        let path = crate::log_feed::daemon_log_path(self.config.daemon.config_file.as_deref())
            .ok_or_else(|| fail("The location of the node log file is not known."))?;
        let text = read_tail(&path, LOG_TAIL_BYTES)?;
        let mut lines: Vec<String> = text
            .lines()
            .filter(|line| line.contains(&ip))
            .map(str::to_owned)
            .collect();
        let skip = lines.len().saturating_sub(MAX_LOG_LINES);
        Ok(lines.split_off(skip))
    }

    /// Asks the local daemon to change its peer connections. These are the only calls in
    /// the bridge that change anything; none touches funds, keys or the chain.
    pub async fn peer_action(&self, action: PeerAction) -> Outcome<()> {
        use serde_json::json;
        let (method, params) = match action {
            PeerAction::AddNode { address } => {
                ("addnode", vec![json!(address.trim()), json!("add")])
            }
            PeerAction::PingAll => ("ping", vec![]),
            PeerAction::Disconnect { address } => ("disconnectnode", vec![json!(address.trim())]),
            PeerAction::Ban { ip } => ("setban", vec![json!(peer_ip(&ip)), json!("add")]),
            PeerAction::ClearBans => ("clearbanned", vec![]),
        };
        self.client.call_local(method, &params).await?;
        Ok(())
    }

    pub async fn currency(&self, name: String) -> Outcome<CurrencyDetail> {
        let name = name.trim().trim_end_matches('@');
        let params = [Value::from(name)];
        let output = self
            .observed(
                "getcurrency",
                &params,
                self.primary.get_currency(name).await,
            )
            .await?;
        let provenance = output.provenance;
        let raw = output.result;
        let state = crate::currency_status::reported_state(&raw);
        let names = &raw["currencynames"];
        let reserves = state["reservecurrencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|reserve| {
                let currency_id = text(&reserve["currencyid"])?;
                Some(ReserveRow {
                    name: text(&names[&currency_id]),
                    weight: reserve["weight"].as_f64(),
                    reserves: reserve["reserves"].as_f64(),
                    price_in_reserve: reserve["priceinreserve"].as_f64(),
                    currency_id,
                })
            })
            .collect();
        Ok(CurrencyDetail {
            name: text(&raw["name"]).unwrap_or_else(|| name.to_string()),
            fully_qualified_name: text(&raw["fullyqualifiedname"]),
            currency_id: text(&raw["currencyid"]),
            parent: text(&raw["parent"]),
            system_id: text(&raw["systemid"]),
            launch_status: crate::currency_status::classify(state).label().to_string(),
            option_tags: option_tags(raw["options"].as_u64()),
            options: raw["options"].as_u64(),
            proof_protocol: raw["proofprotocol"].as_u64(),
            start_block: raw["startblock"].as_u64(),
            end_block: raw["endblock"].as_u64(),
            supply: state["supply"].as_f64(),
            reserves,
            definition_txid: text(&raw["definitiontxid"]),
            last_confirmed_height: raw["lastconfirmedheight"].as_u64(),
            json: pretty(&raw),
            provenance,
        })
    }

}

impl Explorer {
    fn open(options: ConnectionOptions) -> Result<Self> {
        Self::open_with(options, &[])
    }

    fn open_with(options: ConnectionOptions, offered_apis: &[String]) -> Result<Self> {
        let chain = options.chain.trim();
        let chain = if chain.is_empty() { "VRSC" } else { chain };
        let network =
            Network::from_testnet(options.testnet || chain.eq_ignore_ascii_case("vrsctest"));
        let args = cli_arguments(&options, chain, network)?;
        let cli = Cli::try_parse_from(args).context("invalid connection options")?;
        let mut config = Config::load(&cli).context("the node configuration did not load")?;
        if let Some(dir) = options
            .cache_dir
            .as_deref()
            .map(str::trim)
            .filter(|dir| !dir.is_empty())
        {
            config.cache_path = PathBuf::from(dir).join(network.database_name());
        }
        let config_found = config
            .daemon
            .config_file
            .as_deref()
            .is_some_and(|path| path.is_file());

        let api_endpoint = options
            .api_endpoint
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty());
        if let Some(url) = &api_endpoint {
            check_api_url(url)?;
        }
        let offered: Vec<String> = offered_apis
            .iter()
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty() && check_api_url(url).is_ok())
            .collect();

        // The cache records every answer. When it cannot be opened (another Carbide process
        // holds it, or its folder is on a disk that is not connected), the app still works
        // and says that nothing is recorded.
        let (cache, cache_problem) = match crate::cache::Cache::open(&config.cache_path)
            .and_then(|cache| cache.bind_network(network).map(|()| cache))
        {
            Ok(cache) => (Some(cache), None),
            Err(error) => (None, Some(cache_problem_text(&error))),
        };

        // Every node and API in Carbide's settings file and in the standard locations on
        // this Mac. The first entry is the chain this explorer was opened for.
        let sources = sources::Sources::load(config, network, api_endpoint.as_deref(), &offered);
        let config = sources.current().clone();
        let other_chains = sources
            .configs()
            .iter()
            .skip(1)
            .filter(|other| other.daemon.config_file.is_some())
            .cloned()
            .collect();
        if let Some(cache) = &cache {
            sources.register(cache);
        }

        let (client, primary) = if cache.is_some() {
            (
                RpcClient::for_config(&config.noninteractive())?,
                RpcClient::for_config(&config)?,
            )
        } else {
            (
                RpcClient::new(config.daemon.clone())?.noninteractive(),
                RpcClient::new(config.daemon.clone())?,
            )
        };
        Ok(Self {
            client,
            primary,
            cache,
            cache_problem,
            sources,
            config,
            api_endpoint,
            config_found,
            other_chains,
            names: Default::default(),
        })
    }

    fn cached_name(&self, id: &str) -> Option<String> {
        if self.config.chain.system_id.get().map(AsRef::as_ref) == Some(id) {
            return Some(self.config.chain.name.clone());
        }
        self.names
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
    }

    /// Friendly names omit the chain's own root, as Carbide's do: `alice.VRSC@` is `alice@`.
    fn short_name(&self, name: &str) -> String {
        let root = format!(".{}@", self.config.chain.name);
        match name.strip_suffix(root.as_str()) {
            Some(base) if !base.is_empty() => format!("{base}@"),
            _ => name.to_string(),
        }
    }

    /// This chain first, then every other chain configured on this computer.
    fn all_configs(&self) -> Vec<Config> {
        let mut configs = vec![self.config.clone()];
        configs.extend(self.other_chains.iter().cloned());
        configs
    }

    fn book(&self) -> Result<crate::bookmarks::Book> {
        let network = Network::from_testnet(self.config.chain.testnet);
        crate::bookmarks::Book::load(crate::bookmarks::path(network)?)
    }

    /// Clears API back-off after work done by Carbide's own clients, which share the pool.
    fn forgive(&self) {
        for endpoint in &self.config.daemon.api_pool.endpoints {
            *endpoint
                .retry_after
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    /// Carbide backs off an API endpoint for 30 seconds after any error. A
    /// daemon-level answer such as "not found" proves the endpoint is healthy,
    /// so do not let it block the next lookup.
    fn settle<T>(&self, result: Result<T>) -> Result<T> {
        if let Err(error) = &result
            && error
                .downcast_ref::<crate::rpc::ApplicationError>()
                .is_some()
        {
            for endpoint in &self.config.daemon.api_pool.endpoints {
                *endpoint
                    .retry_after
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = None;
            }
        }
        result
    }

    // ------------------------------------------------------ observation layer
    //
    // Every answer from `self.client` and `self.primary` is recorded in Carbide's cache as
    // an observation, with its source, by Carbide's own `RpcClient`. The helpers below add
    // the other half: they say where an answer came from, and when no source answers they
    // serve the latest stored observation of the same request, marked as retained.

    /// Settles a request and records where its answer came from. When no source answered
    /// (the node is not reachable, every source is disabled, or the person cancelled the
    /// permission request), serves the latest stored observation of the same request.
    /// `method` and `params` must be exactly what the request sent, so that the stored
    /// observation is the same request. A source's own answer, such as "not found", is
    /// returned as it is: it is an answer, so no stored observation replaces it.
    pub(crate) async fn observed<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: &[Value],
        live: Result<RpcCallOutput<T>>,
    ) -> Result<Answer<T>> {
        match self.settle(live) {
            Ok(output) => {
                let height = observed_height(&output.response_json);
                if let Some(height) = height
                    && matches!(method, "getblock" | "getrawtransaction")
                {
                    self.queue_reconciliation(&output.source_id, height);
                }
                Ok(Answer {
                    provenance: self.provenance(&output.source_id, height),
                    result: output.result,
                    response_json: output.response_json,
                    source_id: output.source_id,
                })
            }
            Err(error) if source_answered(&error) => Err(error),
            Err(error) => match self.retained::<T>(method, params, Some(&error)).await {
                Some(answer) => Ok(answer),
                None => Err(error),
            },
        }
    }

    /// The latest stored observation of a request, from any source for this chain. Reads
    /// only the cache. `failure` is why no source answered, when that is the reason to ask.
    pub(crate) async fn retained<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: &[Value],
        failure: Option<&anyhow::Error>,
    ) -> Option<Answer<T>> {
        let path = self.config.cache_path.clone();
        let system = self.config.chain_cache_key();
        let (method_owned, params_owned) = (method.to_string(), params.to_vec());
        let row = tokio::task::spawn_blocking(move || {
            retained_row(&path, &system, &method_owned, &params_owned)
        })
        .await
        .ok()?
        .ok()??;
        let response: Value = serde_json::from_str(&row.payload).ok()?;
        let value = response
            .get("result")
            .filter(|value| !value.is_null())?
            .clone();
        let result = serde_json::from_value::<T>(value).ok()?;
        let mut provenance = self.provenance(&row.source, observed_height(&row.payload));
        provenance.retained = true;
        provenance.observed_at = row.observed_at;
        provenance.failure = failure.map(readable);
        // A height names whatever block is at that height now. The stored block was there
        // when it was observed.
        if method == "getblock"
            && params
                .first()
                .and_then(Value::as_str)
                .is_some_and(|id| id.len() != 64)
        {
            provenance.caution = Some(
                "This block was at this height when the source reported it. A reorganization \
                 can have replaced it since then."
                    .into(),
            );
        }
        Some(Answer {
            result,
            response_json: row.payload,
            source_id: row.source,
            provenance,
        })
    }

    /// A client like `self.client` that does not record its answers in the cache. For bulk
    /// reads that are cheap to repeat and are not evidence, such as a chart's thousands of
    /// blocks: one 7-day chart recorded adds approximately 170 MiB, because Carbide keeps
    /// every block body it observes and never prunes it. It uses the same sources and
    /// states, never asks for permission, and has no retained fallback.
    pub(crate) fn unrecorded_client(&self) -> RpcClient {
        RpcClient::new(self.config.daemon.clone())
            .map(RpcClient::noninteractive)
            .unwrap_or_else(|_| self.client.clone())
    }

    /// A live answer's provenance, for records that other areas build from an
    /// `RpcCallOutput`: `self.provenance(&output.source_id, None)`.
    pub(crate) fn provenance(&self, source_id: &str, observed_height: Option<u64>) -> Provenance {
        let (source, kind) = self.sources.label(source_id);
        Provenance {
            source_id: source_id.to_string(),
            source,
            kind,
            retained: false,
            observed_at: now_seconds(),
            observed_height,
            failure: None,
            caution: None,
        }
    }

    /// Asks the local node's reconciler to certify a block height that a view opened, as
    /// the terminal app does, so the block history can say whether it is on the active chain.
    fn queue_reconciliation(&self, source_id: &str, height: u64) {
        if source_id == self.config.daemon_cache_key() {
            let mut heights = self
                .config
                .daemon
                .control
                .reconciliation_heights
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if heights.len() < 64 {
                heights.insert(height);
            }
        }
    }

    /// `getinfo` rather than `getblockcount`: public APIs commonly allow only the former.
    async fn tip_height(&self) -> Result<u64> {
        self.settle(self.client.get_info().await)?.result["blocks"]
            .as_u64()
            .context("the source did not report a block height")
    }

    async fn block_near(&self, query: &str) -> Result<u64> {
        let query = query
            .split_once(':')
            .filter(|(prefix, _)| {
                ["time", "date", "unix"]
                    .iter()
                    .any(|name| prefix.eq_ignore_ascii_case(name))
            })
            .map_or(query, |(_, rest)| rest.trim());
        let target = match query.parse::<i64>() {
            Ok(unix) => unix,
            Err(_) => crate::block_time::parse(query)?.timestamp(),
        };
        let mut source = ClientTime(self.client.clone());
        let tip = source.block_at(self.tip_height().await?).await?;
        Ok(crate::block_time::locate(&mut source, target, tip)
            .await?
            .block
            .height)
    }
}

/// `getblock` with the daemon's default verbosity (transaction IDs only).
/// Carbide's `RpcClient::get_block` always sends a verbosity argument, which
/// some public APIs reject; once a source rejects a method, Carbide stops
/// using that method on it for the rest of the session.
async fn fetch_block(
    client: &RpcClient,
    block: BlockId,
) -> Result<crate::rpc::RpcCallOutput<BlockSummary>> {
    let selector = match block {
        BlockId::Height(height) => Value::from(height.to_string()),
        BlockId::Hash(hash) => Value::from(hash),
    };
    client.call_with_raw("getblock", &[selector]).await
}

struct ClientTime(RpcClient);

impl TimeSource for ClientTime {
    async fn block_at(&mut self, height: u64) -> Result<TimeBlock> {
        let block = fetch_block(&self.0, BlockId::Height(height)).await?.result;
        Ok(TimeBlock {
            height: block.height,
            hash: block.hash,
            parent: block.previousblockhash,
            timestamp: block.time as i64,
        })
    }
}

/// Carbide's own discovery uses the Linux locations. On macOS the daemon
/// keeps its data under `~/Library/Application Support`.
fn platform_config_path(chain: &str, network: Network) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let support = directories::BaseDirs::new()?
        .home_dir()
        .join("Library/Application Support");
    let name = crate::world::chain_directory(chain, network).ok()?;
    let directory = if name == "VRSC" || name == "vrsctest" {
        support.join("Komodo").join(&name)
    } else {
        support
            .join(if network == Network::Testnet {
                "VerusTest"
            } else {
                "Verus"
            })
            .join("pbaas")
            .join(&name)
    };
    let path = directory.join(format!("{name}.conf"));
    path.is_file().then_some(path)
}

// --------------------------------------------------------------- helpers

fn fail(message: &str) -> CarbideError {
    CarbideError::Failed {
        message: message.into(),
    }
}

/// Carbide's command line (`src/cli.rs`) for the connection options. Each value goes in the
/// `--flag=value` form, so that a value that starts with a dash stays a value.
///
/// Without a data directory, the configuration file is the chosen file or the standard
/// macOS location, and must be absolute. With one, Carbide finds the file in that directory,
/// and a relative `config_file` is relative to it, as for the node.
fn cli_arguments(options: &ConnectionOptions, chain: &str, network: Network) -> Result<Vec<String>> {
    let given = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let data_dir = given(&options.data_dir);
    let mut args = vec!["carbide".to_string(), format!("--chain={chain}")];
    if network == Network::Testnet {
        args.push("--testnet".into());
    }
    if let Some(dir) = &data_dir {
        anyhow::ensure!(
            std::path::Path::new(dir).is_absolute(),
            "The path of the node's data directory must be absolute."
        );
        args.push(format!("--datadir={dir}"));
    }
    let config_file = match (given(&options.config_file), &data_dir) {
        (Some(path), _) => Some(PathBuf::from(path)),
        (None, None) => platform_config_path(chain, network),
        (None, Some(_)) => None,
    };
    if let Some(path) = &config_file {
        anyhow::ensure!(
            data_dir.is_some() || path.is_absolute(),
            "The path of the node configuration file must be absolute."
        );
        args.push(format!("--conf={}", path.display()));
    }
    if let Some(host) = given(&options.rpc_host) {
        args.push(format!("--rpcconnect={host}"));
    }
    if let Some(port) = options.rpc_port {
        args.push(format!("--rpcport={port}"));
    }
    if let Some(user) = given(&options.rpc_user) {
        args.push(format!("--rpcuser={user}"));
    }
    // The password is passed as it is: spaces can be part of it.
    if let Some(password) = options.rpc_password.as_ref().filter(|p| !p.is_empty()) {
        args.push(format!("--rpcpassword={password}"));
    }
    Ok(args)
}

/// An answer with its provenance. The first three fields have the names of
/// `RpcCallOutput`'s, so code written for one reads the other.
#[derive(Debug, Clone)]
pub(crate) struct Answer<T> {
    pub result: T,
    pub response_json: String,
    pub source_id: String,
    pub provenance: Provenance,
}

/// True when a source received the request and gave its own answer, such as "not found".
/// "Method not found" is a refusal, not an answer about the object.
fn source_answered(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<crate::rpc::ApplicationError>()
        .is_some_and(|error| !format!("{error}").contains("-32601"))
}

/// The block height that a raw JSON-RPC answer names, when it names one.
fn observed_height(response_json: &str) -> Option<u64> {
    let response = serde_json::from_str::<Value>(response_json).ok()?;
    let result = response.get("result")?.as_object()?;
    result
        .get("height")
        .or_else(|| result.get("blockheight"))?
        .as_u64()
}

/// One stored observation: its source, the full JSON-RPC response and when it was observed.
#[derive(Debug, Clone, PartialEq)]
struct RetainedRow {
    source: String,
    payload: String,
    observed_at: u64,
}

/// Reads the latest stored observation of a request from Carbide's cache, from any source
/// for `system`. Blocks and transactions named by hash come from the object observations,
/// which keep each source's answer; other requests come from the stored responses.
///
/// The queries follow Carbide's `StoreConnection::bookmark_response` in `src/cache.rs`, and
/// add the observation time. They use a read-only connection, so they also work while
/// another Carbide process owns the cache, and they never write.
fn retained_row(
    path: &std::path::Path,
    system: &str,
    method: &str,
    params: &[Value],
) -> Result<Option<RetainedRow>> {
    use rusqlite::{OpenFlags, OptionalExtension, params};
    if !path.is_file() {
        return Ok(None);
    }
    let connection = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(2))?;
    let row = |r: &rusqlite::Row<'_>| {
        Ok(RetainedRow {
            source: r.get(0)?,
            payload: r.get(1)?,
            observed_at: r.get::<_, i64>(2)?.max(0) as u64,
        })
    };
    let target = params.first().and_then(Value::as_str).unwrap_or_default();
    let kind = match method {
        "getrawtransaction" => Some("transaction"),
        "getblock" if target.len() == 64 => Some("block"),
        _ => None,
    };
    if let Some(kind) = kind {
        let found = connection
            .query_row(
                "select o.source,b.payload,o.observed_at from object_observations o
                 join raw_blobs b on b.hash=o.payload_hash
                 where o.system_id=?1 and o.kind=?2 and o.object_id=?3
                   and (?2!='block' or json_type(b.payload,'$.result.tx')='array')
                 order by o.request_order desc limit 1",
                params![system, kind, target.to_ascii_lowercase()],
                row,
            )
            .optional()?;
        if found.is_some() {
            return Ok(found);
        }
    }
    let parameters = serde_json::to_string(params)?;
    Ok(connection
        .query_row(
            "select r.source,b.payload,r.fetched_at from raw_rpc_cache r
             join raw_blobs b on b.hash=r.payload_hash
             where r.system_id=?1 and r.method=?2 and (
                r.params_json=?3
                or (?2='getidentity' and ?4<>'' and (
                    json_extract(b.payload,'$.result.identity.identityaddress')=?4
                    or lower(json_extract(b.payload,'$.result.fullyqualifiedname'))=lower(?4)))
                or (?2='getcurrency' and ?4<>'' and (
                    json_extract(b.payload,'$.result.currencyid')=?4
                    or lower(json_extract(b.payload,'$.result.name'))=lower(?4)
                    or lower(json_extract(b.payload,'$.result.fullyqualifiedname'))=lower(?4))))
             order by r.fetched_at desc,r.request_order desc limit 1",
            params![system, method, parameters, target],
            row,
        )
        .optional()?)
}

/// The parameter `fetch_block` sends for a block.
pub(crate) fn block_selector(block: &BlockId) -> Value {
    match block {
        BlockId::Height(height) => Value::from(height.to_string()),
        BlockId::Hash(hash) => Value::from(hash.as_str()),
    }
}

/// Carbide's rule for API endpoints: HTTP(S) and no credentials in the URL.
fn check_api_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url).context("The API address is not a valid URL.")?;
    anyhow::ensure!(
        matches!(parsed.scheme(), "http" | "https")
            && parsed.username().is_empty()
            && parsed.password().is_none(),
        "API endpoints require HTTP(S) and no embedded credentials"
    );
    Ok(())
}

fn cache_problem_text(error: &anyhow::Error) -> String {
    let text = format!("{error:#}");
    if text.contains("already open by another explorer process") {
        return "Another Carbide process, possibly the terminal app, uses the cache in this folder. \
                The app does not record answers until that process stops, or until you choose \
                another folder."
            .into();
    }
    format!("The app cannot open the cache. It does not record answers. {text}")
}

/// The platform cache folder that Carbide uses when no folder is chosen.
fn default_cache_folder() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("verus-carbide")
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn source_of(source_id: &str) -> DataSource {
    if source_id.starts_with("public-api") {
        DataSource::PublicApi
    } else {
        DataSource::LocalDaemon
    }
}

struct Peer {
    row: PeerRow,
    synced_headers: Option<u64>,
}

fn peer_row(peer: &Value) -> Peer {
    // `inflight` lists every block being downloaded and can be long; leave it out of the raw view.
    let json = pretty(&without(peer, "inflight"));
    Peer {
        synced_headers: peer["synced_headers"].as_u64(),
        row: PeerRow {
            id: peer["id"].as_i64().unwrap_or(-1),
            address: text(&peer["addr"]).unwrap_or_default(),
            inbound: peer["inbound"].as_bool().unwrap_or(false),
            ping_ms: peer["pingtime"].as_f64().map(|seconds| seconds * 1000.0),
            synced_blocks: peer["synced_blocks"].as_i64(),
            starting_height: peer["startingheight"].as_i64(),
            bytes_sent: peer["bytessent"].as_u64().unwrap_or(0),
            bytes_received: peer["bytesrecv"].as_u64().unwrap_or(0),
            version: text(&peer["subver"]),
            connected_since: peer["conntime"].as_u64(),
            json,
        },
    }
}

/// Describes how an identity version differs from the one before it, in plain words.
fn identity_changes(previous: Option<&Value>, current: &Value) -> Vec<String> {
    let Some(previous) = previous else {
        return vec!["Registered".into()];
    };
    let mut changes = Vec::new();
    let keys = |identity: &Value| strings(&identity["primaryaddresses"]);
    let required = |identity: &Value| identity["minimumsignatures"].as_u64().unwrap_or(1);
    if keys(previous) != keys(current) {
        changes.push(format!(
            "Signing keys changed ({} of {} needed)",
            required(current),
            keys(current).len()
        ));
    } else if required(previous) != required(current) {
        changes.push(format!(
            "Signatures needed changed from {} to {}",
            required(previous),
            required(current)
        ));
    }
    for (key, label) in [
        ("revocationauthority", "Revocation authority changed"),
        ("recoveryauthority", "Recovery authority changed"),
        ("privateaddress", "Private address changed"),
    ] {
        if previous[key] != current[key] {
            changes.push(label.into());
        }
    }
    let flags = |identity: &Value| identity["flags"].as_u64().unwrap_or(0);
    // CIdentity flags: 1 active currency, 2 locked, 0x8000 revoked.
    for (bit, on, off) in [
        (1, "Currency defined", "Currency flag cleared"),
        (2, "Locked", "Unlocked"),
        (0x8000, "Revoked", "Recovered from revocation"),
    ] {
        match (flags(previous) & bit != 0, flags(current) & bit != 0) {
            (false, true) => changes.push(on.into()),
            (true, false) => changes.push(off.into()),
            _ => {}
        }
    }
    if previous["timelock"] != current["timelock"] {
        changes.push("Timelock changed".into());
    }
    if previous["contentmap"] != current["contentmap"]
        || previous["contentmultimap"] != current["contentmultimap"]
    {
        changes.push("Attached data changed".into());
    }
    if changes.is_empty() {
        changes.push("Updated. Control did not change.".into());
    }
    changes
}

// CCurrencyDefinition::EOptions, as tabulated in Carbide's currency viewer.
const OPTION_TAGS: &[(u64, &str)] = &[
    (1, "Fractional"),
    (2, "Restricted ID issuance"),
    (4, "ID staking"),
    (8, "ID referrals"),
    (0x10, "Referrer required"),
    (0x20, "Token"),
    (0x40, "Single currency"),
    (0x80, "Gateway"),
    (0x100, "PBaaS chain"),
    (0x200, "Gateway converter"),
    (0x400, "Gateway name control"),
    (0x800, "NFT"),
    (0x1000, "No ID issuance"),
];

fn option_tags(options: Option<u64>) -> Vec<String> {
    let Some(options) = options else {
        return Vec::new();
    };
    OPTION_TAGS
        .iter()
        .filter(|(bit, _)| options & bit != 0)
        .map(|(_, name)| (*name).to_string())
        .collect()
}

/// The IP part of a peer address such as `203.0.113.18:27485` or `[2001:db8::1]:27485`.
fn peer_ip(address: &str) -> String {
    let address = address.trim();
    if let Some(rest) = address.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest).to_string();
    }
    match address.rsplit_once(':') {
        Some((ip, port)) if !ip.contains(':') && port.bytes().all(|b| b.is_ascii_digit()) => {
            ip.to_string()
        }
        _ => address.to_string(),
    }
}

/// The last `limit` bytes of a file as text, starting at a line boundary.
fn read_tail(path: &std::path::Path, limit: u64) -> Result<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file =
        std::fs::File::open(path).with_context(|| format!("could not open {}", path.display()))?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(limit)))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(match (length > limit, text.split_once('\n')) {
        (true, Some((_, rest))) => rest.to_string(),
        _ => text.into_owned(),
    })
}

/// Daemon configurations in the standard macOS locations: the main chain and any PBaaS
/// chains. Reads folder names only; it does not open wallets or contact any daemon.
#[uniffi::export]
pub fn known_chains(testnet: bool) -> Vec<KnownChain> {
    let network = Network::from_testnet(testnet);
    let mut chains = Vec::new();
    let main = if testnet { "vrsctest" } else { "VRSC" };
    if let Some(path) = platform_config_path(main, network) {
        chains.push(KnownChain {
            chain: main.into(),
            config_file: path.display().to_string(),
            testnet,
        });
    }
    let Some(base) = directories::BaseDirs::new() else {
        return chains;
    };
    let root = base
        .home_dir()
        .join("Library/Application Support")
        .join(if testnet { "VerusTest" } else { "Verus" })
        .join("pbaas");
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let config = entry.path().join(format!("{name}.conf"));
        if name.len() == 40 && name.bytes().all(|b| b.is_ascii_hexdigit()) && config.is_file() {
            chains.push(KnownChain {
                chain: name,
                config_file: config.display().to_string(),
                testnet,
            });
        }
    }
    chains
}

fn block_row(block: &BlockSummary) -> BlockRow {
    BlockRow {
        height: block.height,
        hash: block.hash.clone(),
        time: block.time,
        transaction_count: block.tx_count() as u32,
        size: block.size,
        production: block.production_label(),
        // The daemon reports proof-of-stake blocks as "minted".
        staked: block.production_kind() == crate::domain::BlockProductionKind::Staked
            || block
                .blocktype
                .as_deref()
                .is_some_and(|kind| kind.eq_ignore_ascii_case("minted")),
    }
}

fn labels(tx: &TransactionSummary) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for tag in tx.tags() {
        let label = tag.label().to_string();
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels
}

fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn display(value: &Value) -> Option<String> {
    match value {
        Value::String(text) if !text.is_empty() => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(text)
        .collect()
}

fn count(value: Option<&Value>) -> u32 {
    value
        .and_then(Value::as_array)
        .map_or(0, |items| items.len() as u32)
}

/// Native-coin value of an input or output in satoshis.
fn sats(value: &Value) -> Option<i64> {
    ["valueSat", "valueZat", "satoshis"]
        .iter()
        .find_map(|key| value[*key].as_i64())
        .or_else(|| {
            value["value"]
                .as_f64()
                .map(|coins| (coins * 100_000_000.0).round() as i64)
        })
}

fn addresses(value: &Value) -> Vec<String> {
    let mut found = strings(&value["addresses"]);
    if let Some(address) = text(&value["address"])
        && !found.contains(&address)
    {
        found.push(address);
    }
    found
}

fn amounts(value: &Value) -> Vec<CurrencyAmount> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(currency, amount)| {
            Some(CurrencyAmount {
                currency: currency.clone(),
                amount: amount.as_f64()?,
            })
        })
        .collect()
}

fn without(value: &Value, key: &str) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove(key);
    }
    value
}

/// The `result` member of a raw JSON-RPC response.
fn result_of(response_json: &str) -> Value {
    serde_json::from_str::<Value>(response_json)
        .ok()
        .and_then(|mut response| response.get_mut("result").map(Value::take))
        .unwrap_or(Value::Null)
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

// One module per area of the app. Each adds its own records and its own
// `#[uniffi::export] impl Explorer` block, and can use everything in this file.
#[path = "ffi_charts.rs"]
mod charts;
#[path = "ffi_currency.rs"]
mod currency;
#[path = "ffi_identity.rs"]
mod identity;
#[path = "ffi_shell.rs"]
mod shell;
#[path = "ffi_sources.rs"]
mod sources;
#[path = "ffi_transaction.rs"]
mod transaction;

#[cfg(test)]
#[path = "ffi_tests.rs"]
mod tests;
