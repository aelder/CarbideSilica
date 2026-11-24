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
use crate::rpc::{BlockId, RpcClient};
use crate::sources::{ApiEndpoint, ApiPool, Control};
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
        return "Could not reach the Verus daemon. Check that it is running, or enable a public \
                API in Settings."
            .into();
    }
    text
}

type Outcome<T> = std::result::Result<T, CarbideError>;

// --------------------------------------------------------------- records

#[derive(Debug, Clone, uniffi::Record)]
pub struct ConnectionOptions {
    pub chain: String,
    pub testnet: bool,
    /// Absolute path to a daemon `.conf` file. `None` uses the standard location.
    pub config_file: Option<String>,
    /// A public JSON-RPC endpoint the user has chosen to allow as a fallback.
    pub api_endpoint: Option<String>,
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
pub struct BlockDetail {
    pub row: BlockRow,
    pub previous_hash: Option<String>,
    pub next_hash: Option<String>,
    pub confirmations: Option<i64>,
    pub fields: Vec<Field>,
    pub transactions: Vec<TransactionRow>,
    /// The block as the daemon reported it, with transactions reduced to IDs.
    pub json: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CurrencyAmount {
    pub currency: String,
    pub amount: f64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TransactionInput {
    pub index: u32,
    pub coinbase: bool,
    pub previous_txid: Option<String>,
    pub previous_output: Option<u32>,
    pub addresses: Vec<String>,
    pub value_sats: Option<i64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TransactionOutput {
    pub index: u32,
    pub value_sats: Option<i64>,
    pub addresses: Vec<String>,
    pub script_type: Option<String>,
    pub tags: Vec<String>,
    pub spent_txid: Option<String>,
    pub currency_values: Vec<CurrencyAmount>,
    pub json: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TransactionDetail {
    pub txid: String,
    pub block_hash: Option<String>,
    pub height: Option<u64>,
    pub confirmations: Option<i64>,
    pub time: Option<u64>,
    pub size: Option<u64>,
    pub version: Option<i64>,
    pub expiry_height: Option<u64>,
    pub tags: Vec<String>,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub private_inputs: u32,
    pub private_outputs: u32,
    /// Outputs from which a transfer can be followed.
    pub trace_roots: Vec<TraceRoot>,
    pub json: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct IdentityDetail {
    pub name: String,
    pub fully_qualified_name: Option<String>,
    pub identity_address: Option<String>,
    pub parent: Option<String>,
    pub system_id: Option<String>,
    pub status: Option<String>,
    pub primary_addresses: Vec<String>,
    pub minimum_signatures: Option<u64>,
    pub revocation_authority: Option<String>,
    pub recovery_authority: Option<String>,
    pub private_address: Option<String>,
    pub timelock: Option<u64>,
    pub flags: Option<u64>,
    pub defines_currency: bool,
    pub block_height: Option<u64>,
    pub txid: Option<String>,
    pub json: String,
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
    pub options: Option<u64>,
    pub proof_protocol: Option<u64>,
    pub start_block: Option<u64>,
    pub end_block: Option<u64>,
    pub supply: Option<f64>,
    pub reserves: Vec<ReserveRow>,
    pub definition_txid: Option<String>,
    pub last_confirmed_height: Option<u64>,
    pub json: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AddressDetail {
    pub address: String,
    pub balance_sats: Option<i64>,
    pub received_sats: Option<i64>,
    pub currency_balances: Vec<CurrencyAmount>,
    pub transaction_count: u64,
    /// Newest first, capped.
    pub recent_txids: Vec<String>,
    /// Set when the source could not list this address's transactions.
    pub activity_error: Option<String>,
    pub json: String,
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
}

/// A resolved name for an identity or currency ID.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NameEntry {
    pub id: String,
    pub name: String,
}

/// One recorded state of an identity, with what changed since the state before it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct IdentityVersion {
    pub height: u64,
    pub time: Option<u64>,
    pub block_hash: Option<String>,
    pub txid: Option<String>,
    pub primary_addresses: Vec<String>,
    pub minimum_signatures: Option<u64>,
    pub revocation_authority: Option<String>,
    pub recovery_authority: Option<String>,
    /// Plain descriptions of the differences from the previous version.
    pub changes: Vec<String>,
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

/// An output of a transaction from which a transfer can be followed.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraceRoot {
    pub output: u32,
    pub label: String,
}

/// One output along a traced transfer.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraceStep {
    pub system_id: String,
    pub txid: String,
    pub output: u32,
    pub label: String,
    pub detail: String,
}

/// One leg of a transfer: an instruction and, when found, what it produced.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraceLeg {
    pub source_txid: String,
    pub source_output: u32,
    /// Carbide's own wording for how far this leg got.
    pub stage: String,
    pub confidence: Confidence,
    pub principal: Vec<CurrencyAmount>,
    pub pool: Option<String>,
    pub conversion_fee: Option<CurrencyAmount>,
    /// "Received", "Forwarded" or "Refunded" when a result output was replayed exactly.
    pub result: Option<String>,
    pub result_amounts: Vec<CurrencyAmount>,
    pub result_txid: Option<String>,
    pub result_output: Option<u32>,
    pub import_height: Option<u64>,
}

/// How much of a chain was searched for a transfer's import.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraceSearch {
    pub system_id: String,
    pub first: u64,
    pub last: u64,
    pub tip: u64,
    pub windows: u32,
    pub more_older: bool,
}

/// A transfer followed from its source output as far as the evidence reaches.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TransferTrace {
    /// One plain statement of the outcome so far.
    pub verdict: String,
    pub confidence: Confidence,
    pub legs: Vec<TraceLeg>,
    pub steps: Vec<TraceStep>,
    pub notes: Vec<String>,
    /// Why the trace stopped, when it did not reach a result.
    pub stop: Option<String>,
    pub search: Option<TraceSearch>,
}

/// One notarization of another system, as this chain recorded it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NotarizationRow {
    pub index: u32,
    pub txid: Option<String>,
    pub output: Option<u32>,
    /// The block on this chain that contains the notarization.
    pub block_hash: Option<String>,
    pub block_height: Option<u64>,
    pub block_time: Option<u64>,
    /// This chain's height that the notarization commits to.
    pub local_height: Option<u64>,
    /// The other system's height that the notarization commits to.
    pub other_height: Option<u64>,
    pub confirmed: bool,
    /// False when the notarization sits on a fork other than the best one.
    pub on_best_chain: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct NotarizationData {
    pub currency_id: Option<String>,
    pub other_system_id: Option<String>,
    pub rows: Vec<NotarizationRow>,
    pub fork_count: u32,
    pub last_confirmed_index: Option<u32>,
    pub json: String,
}

/// A transaction waiting in the local node's memory pool.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MempoolRow {
    pub txid: String,
    pub size: u64,
    pub fee_sats: Option<i64>,
    /// When the node first saw it.
    pub time: Option<u64>,
    /// How many other waiting transactions this one depends on.
    pub depends: u32,
    pub tags: Vec<String>,
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

/// An open marketplace offer.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MarketOffer {
    pub txid: String,
    pub kind: OfferKind,
    /// The identity's name, or the currency ID being offered.
    pub subject: String,
    pub subject_id: Option<String>,
    pub price: f64,
    pub price_currency: String,
    pub expiry_height: Option<u64>,
}

/// Where a search query leads.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Route {
    Block { id: String },
    Transaction { txid: String },
    Identity { name: String },
    Currency { name: String },
    Address { address: String },
}

// -------------------------------------------------------------- explorer

#[derive(uniffi::Object)]
pub struct Explorer {
    client: RpcClient,
    config: Config,
    api_endpoint: Option<String>,
    config_found: bool,
    /// Names already resolved for identity and currency IDs.
    names: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

const RECENT_ADDRESS_TXIDS: usize = 200;
const MAX_RECENT_BLOCKS: u32 = 100;
const MAX_NAME_LOOKUPS: usize = 48;
const MAX_OFFERS: usize = 4000;
const MAX_HISTORY_TIMES: usize = 60;
const MAX_HISTORY_SAMPLES: u32 = 200;
const MAX_BLOCK_TX_BODIES: usize = 150;

#[uniffi::export(async_runtime = "tokio")]
impl Explorer {
    /// Reads the daemon configuration. Does not contact the daemon.
    #[uniffi::constructor]
    pub fn new(options: ConnectionOptions) -> Outcome<Arc<Self>> {
        Ok(Arc::new(Self::open(options)?))
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

    pub async fn chain_status(&self) -> Outcome<ChainStatus> {
        let output = self.settle(self.client.get_info().await)?;
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
        })
    }

    /// Sync progress, mempool size and peers. Peers come only from the local daemon.
    pub async fn node_status(&self) -> Outcome<NodeStatus> {
        let output = self.settle(self.client.get_info().await)?;
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
        })
    }

    /// The `count` blocks ending at `before` (or at the tip), newest first.
    pub async fn recent_blocks(&self, before: Option<u64>, count: u32) -> Outcome<Vec<BlockRow>> {
        let tip = match before {
            Some(height) => height,
            None => self.tip_height().await?,
        };
        let count = u64::from(count.clamp(1, MAX_RECENT_BLOCKS));
        let first = tip.saturating_sub(count - 1);
        let mut tasks = tokio::task::JoinSet::new();
        for height in first..=tip {
            let client = self.client.clone();
            tasks.spawn(async move { fetch_block(&client, BlockId::Height(height)).await });
        }
        let mut rows = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            let output = self.settle(joined.context("block request stopped")?)?;
            rows.push(block_row(&output.result));
        }
        rows.sort_by(|a, b| b.height.cmp(&a.height));
        Ok(rows)
    }

    /// Classifies a search query with Carbide's resolver and confirms what it names.
    pub async fn resolve(&self, query: String) -> Outcome<Route> {
        let query = query.trim();
        Ok(match crate::resolver::classify_start_object(Some(query)) {
            ObjectRoute::Tip => Route::Block {
                id: self.tip_height().await?.to_string(),
            },
            ObjectRoute::BlockHeight(height) => Route::Block {
                id: height.to_string(),
            },
            ObjectRoute::Hash(hash) => {
                let block = fetch_block(&self.client, BlockId::Hash(hash.clone())).await;
                if self.settle(block).is_ok() {
                    Route::Block { id: hash }
                } else if self
                    .settle(self.client.get_raw_transaction(&hash).await)
                    .is_ok()
                {
                    Route::Transaction { txid: hash }
                } else {
                    return Err(fail("No block or transaction has that hash."));
                }
            }
            ObjectRoute::BlockTime(time) => Route::Block {
                id: self.block_near(&time).await?.to_string(),
            },
            ObjectRoute::Estimate(_) => {
                return Err(fail("Future block estimates are not available here yet."));
            }
            ObjectRoute::Address(address) => Route::Address { address },
            ObjectRoute::Identity(name) => Route::Identity { name },
            ObjectRoute::Unknown(value) => match crate::resolver::currency_query(&value) {
                Some(name) => Route::Currency { name: name.into() },
                None => {
                    return Err(fail(
                        "Not recognised. Enter a block height or hash, a transaction ID, a \
                             date, an address, an identity (name@) or a currency (name@@).",
                    ));
                }
            },
        })
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

    /// Every recorded version of an identity, oldest first, with what changed each time.
    pub async fn identity_history(&self, name: String) -> Outcome<Vec<IdentityVersion>> {
        let output = self.settle(self.client.get_identity_history(name.trim()).await)?;
        let entries = output.result["history"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut versions: Vec<IdentityVersion> = Vec::new();
        let mut previous: Option<&Value> = None;
        for entry in &entries {
            let identity = &entry["identity"];
            versions.push(IdentityVersion {
                height: entry["height"].as_u64().unwrap_or(0),
                time: None,
                block_hash: text(&entry["blockhash"]),
                txid: text(&entry["output"]["txid"]),
                primary_addresses: strings(&identity["primaryaddresses"]),
                minimum_signatures: identity["minimumsignatures"].as_u64(),
                revocation_authority: text(&identity["revocationauthority"]),
                recovery_authority: text(&identity["recoveryauthority"]),
                changes: identity_changes(previous, identity),
            });
            previous = Some(identity);
        }
        // Block times, for the most recent versions if there are very many.
        let mut tasks = tokio::task::JoinSet::new();
        let skip = versions.len().saturating_sub(MAX_HISTORY_TIMES);
        for (index, version) in versions.iter().enumerate().skip(skip) {
            let Some(hash) = version.block_hash.clone() else {
                continue;
            };
            let client = self.client.clone();
            tasks.spawn(async move { (index, fetch_block(&client, BlockId::Hash(hash)).await) });
        }
        while let Some(Ok((index, block))) = tasks.join_next().await {
            if let Ok(block) = self.settle(block) {
                versions[index].time = Some(block.result.time);
            }
        }
        Ok(versions)
    }

    /// A currency's state sampled evenly across a range of heights, oldest first.
    pub async fn currency_history(
        &self,
        name: String,
        from_height: u64,
        to_height: u64,
        samples: u32,
    ) -> Outcome<Vec<CurrencyPoint>> {
        let name = name.trim().trim_end_matches('@');
        if to_height <= from_height {
            return Err(fail("The range is empty."));
        }
        let samples = u64::from(samples.clamp(2, MAX_HISTORY_SAMPLES));
        let step = ((to_height - from_height) / (samples - 1)).max(1);
        let range = format!("{from_height},{to_height},{step}");
        let output = self.settle(
            self.client
                .call_with_raw::<Vec<Value>>(
                    "getcurrencystate",
                    &[Value::from(name), Value::from(range)],
                )
                .await,
        )?;
        Ok(output
            .result
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
            .collect())
    }

    /// Follows a transfer from one output through export, import and its result, using
    /// Carbide's DeFi trail. `scan_older_from` searches older import windows from that height.
    pub async fn trace_transfer(
        &self,
        txid: String,
        output: u32,
        scan_older_from: Option<u64>,
    ) -> Outcome<TransferTrace> {
        let seed = self
            .settle(self.client.get_raw_transaction(txid.trim()).await)?
            .result;
        let system_id = self
            .config
            .chain
            .system_id
            .get()
            .cloned()
            .ok_or_else(|| fail("The chain's system ID is not known yet."))?;
        let root = crate::world::OutpointRef {
            system_id,
            txid: txid.trim().to_string(),
            vout: output,
        };
        let plan = crate::defi::SearchPlan {
            request: match scan_older_from {
                Some(height) => crate::defi::ImportSearch::BackwardFrom(height),
                None => crate::defi::ImportSearch::Initial,
            },
            target: None,
            known_imports: Vec::new(),
        };
        let trail =
            crate::defi::investigate(vec![self.config.clone()], root, seed, plan, None).await;
        for endpoint in &self.config.daemon.api_pool.endpoints {
            *endpoint
                .retry_after
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = None;
        }
        Ok(transfer_trace(trail))
    }

    /// Notarizations of a currency's system as this chain sees them, oldest first.
    /// Returns no rows for a currency that is not notarized.
    pub async fn notarizations(&self, currency: String) -> Outcome<NotarizationData> {
        let name = currency.trim().trim_end_matches('@');
        let output = self.settle(
            self.client
                .call_nullable_with_raw::<Value>("getnotarizationdata", &[Value::from(name)])
                .await,
        )?;
        let Some(raw) = output.result else {
            return Ok(NotarizationData {
                currency_id: None,
                other_system_id: None,
                rows: Vec::new(),
                fork_count: 0,
                last_confirmed_index: None,
                json: String::new(),
            });
        };
        let local = self.config.chain.system_id.get().map(|id| id.to_string());
        let mut data = notarization_data(&raw, local.as_deref());
        let mut tasks = tokio::task::JoinSet::new();
        for (position, row) in data.rows.iter().enumerate() {
            let Some(hash) = row.block_hash.clone() else {
                continue;
            };
            let client = self.client.clone();
            tasks.spawn(async move { (position, fetch_block(&client, BlockId::Hash(hash)).await) });
        }
        while let Some(Ok((position, block))) = tasks.join_next().await {
            if let Ok(block) = self.settle(block) {
                data.rows[position].block_height = Some(block.result.height);
                data.rows[position].block_time = Some(block.result.time);
            }
        }
        Ok(data)
    }

    /// Transactions waiting in the local node's memory pool. Only a local daemon has one.
    pub async fn mempool(&self) -> Outcome<Vec<MempoolRow>> {
        let output = self.settle(self.client.get_raw_mempool_verbose(true).await)?;
        let mut rows: Vec<MempoolRow> = output
            .result
            .iter()
            .map(|(txid, entry)| mempool_row(txid, entry))
            .collect();
        rows.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.txid.cmp(&b.txid)));
        Ok(rows)
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

    /// Open offers involving this chain's currency: identities for sale or wanted, and
    /// currency swaps.
    pub async fn offers(&self) -> Outcome<Vec<MarketOffer>> {
        let output = self.settle(self.client.get_offers(&self.config.chain.name, true).await)?;
        let mut offers = Vec::new();
        for entries in output
            .result
            .as_object()
            .into_iter()
            .flatten()
            .map(|(_, list)| list)
        {
            for entry in entries.as_array().into_iter().flatten() {
                if let Some(offer) = market_offer(entry) {
                    offers.push(offer);
                }
                if offers.len() >= MAX_OFFERS {
                    return Ok(offers);
                }
            }
        }
        Ok(offers)
    }

    /// `id` is a height or a block hash.
    pub async fn block(&self, id: String) -> Outcome<BlockDetail> {
        let selector = match id.trim().parse::<u64>() {
            Ok(height) => BlockId::Height(height),
            Err(_) => BlockId::Hash(id.trim().to_string()),
        };
        let mut output = self.settle(fetch_block(&self.client, selector).await)?;
        let mut raw = result_of(&output.response_json);

        // The block lists transaction IDs. Fetch the bodies, which carry the
        // tags and totals, for as many as is reasonable in one view.
        let mut tasks = tokio::task::JoinSet::new();
        for (index, tx) in output
            .result
            .tx
            .iter()
            .take(MAX_BLOCK_TX_BODIES)
            .enumerate()
        {
            let client = self.client.clone();
            let txid = tx.txid.clone();
            tasks.spawn(async move { (index, client.get_raw_transaction(&txid).await) });
        }
        while let Some(joined) = tasks.join_next().await {
            let (index, body) = joined.context("transaction request stopped")?;
            if let Ok(body) = self.settle(body) {
                output.result.tx[index].raw = Some(body.result);
            }
        }
        let block = &output.result;
        let transactions = block
            .tx
            .iter()
            .map(|tx| TransactionRow {
                txid: tx.txid.clone(),
                tags: labels(tx),
                input_count: count(tx.raw.as_ref().map(|raw| &raw["vin"])),
                output_count: count(tx.raw.as_ref().map(|raw| &raw["vout"])),
                output_sats: tx.raw.as_ref().and_then(|raw| {
                    raw["vout"]
                        .as_array()
                        .map(|outputs| outputs.iter().filter_map(sats).sum())
                }),
            })
            .collect();
        let mut fields = Vec::new();
        for (label, key) in [
            ("Merkle root", "merkleroot"),
            ("Final Sapling root", "finalsaplingroot"),
            ("Difficulty", "difficulty"),
            ("Bits", "bits"),
            ("Nonce", "nonce"),
            ("Version", "version"),
            ("Chain work", "chainwork"),
            ("Chain stake", "chainstake"),
            ("Segment ID", "segid"),
        ] {
            if let Some(value) = display(&raw[key]) {
                fields.push(Field {
                    label: label.into(),
                    value,
                });
            }
        }
        if let Some(list) = raw.get_mut("tx").and_then(Value::as_array_mut) {
            for tx in list {
                if let Some(txid) = tx["txid"].as_str() {
                    *tx = Value::from(txid);
                }
            }
        }
        Ok(BlockDetail {
            row: block_row(block),
            previous_hash: block.previousblockhash.clone(),
            next_hash: block.nextblockhash.clone(),
            confirmations: raw["confirmations"].as_i64(),
            fields,
            transactions,
            json: pretty(&raw),
        })
    }

    pub async fn transaction(&self, txid: String) -> Outcome<TransactionDetail> {
        let output = self.settle(self.client.get_raw_transaction(txid.trim()).await)?;
        let raw = output.result;
        let summary = TransactionSummary {
            txid: text(&raw["txid"]).unwrap_or_else(|| txid.trim().to_string()),
            raw: Some(raw.clone()),
        };
        let inputs = raw["vin"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, input)| TransactionInput {
                index: index as u32,
                coinbase: input.get("coinbase").is_some(),
                previous_txid: text(&input["txid"]),
                previous_output: input["vout"].as_u64().map(|n| n as u32),
                addresses: addresses(input),
                value_sats: sats(input),
            })
            .collect();
        let outputs = raw["vout"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(index, output)| TransactionOutput {
                index: output["n"].as_u64().unwrap_or(index as u64) as u32,
                value_sats: sats(output),
                addresses: addresses(&output["scriptPubKey"]),
                script_type: text(&output["scriptPubKey"]["type"]),
                tags: crate::domain::output_tags(output)
                    .into_iter()
                    .map(|tag| tag.kind.label().to_string())
                    .collect(),
                spent_txid: text(&output["spentTxId"]),
                currency_values: amounts(&output["scriptPubKey"]["reserve_balance"]),
                json: pretty(output),
            })
            .collect();
        Ok(TransactionDetail {
            block_hash: text(&raw["blockhash"]),
            height: raw["height"].as_u64(),
            confirmations: raw["confirmations"].as_i64(),
            time: raw["blocktime"].as_u64().or_else(|| raw["time"].as_u64()),
            size: raw["size"]
                .as_u64()
                .or_else(|| raw["hex"].as_str().map(|hex| hex.len() as u64 / 2)),
            version: raw["version"].as_i64(),
            expiry_height: raw["expiryheight"].as_u64(),
            tags: labels(&summary),
            inputs,
            outputs,
            private_inputs: crate::domain::private_input_count(&raw) as u32,
            private_outputs: crate::domain::private_output_count(&raw) as u32,
            trace_roots: crate::defi::roots(&raw)
                .into_iter()
                .map(|(output, label)| TraceRoot { output, label })
                .collect(),
            json: pretty(&without(&raw, "hex")),
            txid: summary.txid,
        })
    }

    pub async fn identity(&self, name: String) -> Outcome<IdentityDetail> {
        let output = self.settle(self.client.get_identity(name.trim(), None).await)?;
        let raw = output.result;
        let identity = &raw["identity"];
        let flags = identity["flags"].as_u64();
        Ok(IdentityDetail {
            name: text(&raw["friendlyname"])
                .or_else(|| text(&identity["name"]).map(|name| format!("{name}@")))
                .unwrap_or_else(|| name.trim().to_string()),
            fully_qualified_name: text(&raw["fullyqualifiedname"]),
            identity_address: text(&identity["identityaddress"]),
            parent: text(&identity["parent"]),
            system_id: text(&identity["systemid"]),
            status: text(&raw["status"]),
            primary_addresses: strings(&identity["primaryaddresses"]),
            minimum_signatures: identity["minimumsignatures"].as_u64(),
            revocation_authority: text(&identity["revocationauthority"]),
            recovery_authority: text(&identity["recoveryauthority"]),
            private_address: text(&identity["privateaddress"]),
            timelock: identity["timelock"].as_u64(),
            flags,
            // CIdentity::FLAG_ACTIVECURRENCY
            defines_currency: flags.is_some_and(|flags| flags & 1 != 0),
            block_height: raw["blockheight"].as_u64(),
            txid: text(&raw["txid"]),
            json: pretty(&raw),
        })
    }

    pub async fn currency(&self, name: String) -> Outcome<CurrencyDetail> {
        let name = name.trim().trim_end_matches('@');
        let output = self.settle(self.client.get_currency(name).await)?;
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
            options: raw["options"].as_u64(),
            proof_protocol: raw["proofprotocol"].as_u64(),
            start_block: raw["startblock"].as_u64(),
            end_block: raw["endblock"].as_u64(),
            supply: state["supply"].as_f64(),
            reserves,
            definition_txid: text(&raw["definitiontxid"]),
            last_confirmed_height: raw["lastconfirmedheight"].as_u64(),
            json: pretty(&raw),
        })
    }

    pub async fn address(&self, address: String) -> Outcome<AddressDetail> {
        let address = address.trim().to_string();
        let balance = self
            .settle(self.client.get_address_balance(&address).await)?
            .result;
        let (transaction_count, recent_txids, activity_error) =
            match self.settle(self.client.get_address_txids(&address).await) {
                Ok(output) => (
                    output.result.len() as u64,
                    output
                        .result
                        .into_iter()
                        .rev()
                        .take(RECENT_ADDRESS_TXIDS)
                        .collect(),
                    None,
                ),
                Err(error) => (0, Vec::new(), Some(readable(&error))),
            };
        Ok(AddressDetail {
            balance_sats: balance["balance"].as_i64(),
            received_sats: balance["received"].as_i64(),
            currency_balances: amounts(&balance["currencybalance"])
                .into_iter()
                .map(|mut entry| {
                    // Balances are keyed by currency ID; name the chain's own coin.
                    if self.config.chain.system_id.get().map(AsRef::as_ref)
                        == Some(entry.currency.as_str())
                    {
                        entry.currency = self.config.chain.name.clone();
                    }
                    entry
                })
                .collect(),
            transaction_count,
            recent_txids,
            activity_error,
            json: pretty(&balance),
            address,
        })
    }
}

impl Explorer {
    fn open(options: ConnectionOptions) -> Result<Self> {
        let chain = options.chain.trim();
        let chain = if chain.is_empty() { "VRSC" } else { chain };
        let network =
            Network::from_testnet(options.testnet || chain.eq_ignore_ascii_case("vrsctest"));
        let config_file = match options.config_file.as_deref().map(str::trim) {
            Some(path) if !path.is_empty() => Some(PathBuf::from(path)),
            _ => platform_config_path(chain, network),
        };

        let mut args = vec!["carbide".to_string(), "--chain".into(), chain.to_string()];
        if network == Network::Testnet {
            args.push("--testnet".into());
        }
        if let Some(path) = &config_file {
            anyhow::ensure!(
                path.is_absolute(),
                "The daemon config path must be absolute."
            );
            args.push("--conf".into());
            args.push(path.display().to_string());
        }
        let cli = Cli::try_parse_from(args).context("invalid connection options")?;
        let mut config = Config::load(&cli).context("failed to load daemon configuration")?;
        let config_found = config
            .daemon
            .config_file
            .as_deref()
            .is_some_and(|path| path.is_file());

        let api_endpoint = options
            .api_endpoint
            .map(|url| url.trim().to_string())
            .filter(|url| !url.is_empty());
        let mut endpoints = Vec::new();
        if let Some(url) = &api_endpoint {
            let parsed = reqwest::Url::parse(url).context("The API address is not a valid URL.")?;
            anyhow::ensure!(
                matches!(parsed.scheme(), "http" | "https")
                    && parsed.username().is_empty()
                    && parsed.password().is_none(),
                "API endpoints require HTTP(S) and no embedded credentials"
            );
            // A new `Control` is enabled; configuring the endpoint is the consent.
            let control = Arc::new(Control::default());
            if let Some(id) = config.chain.system_id.get() {
                let _ = control.expected.set((id.clone(), network));
            }
            endpoints.push(ApiEndpoint {
                url: url.clone(),
                control,
                retry_after: Default::default(),
            });
        }
        config.daemon.api_pool = Arc::new(ApiPool::new(config.chain.name.clone(), endpoints, None));

        // No cache storage: the desktop app does not share the terminal's SQLite cache yet.
        let client = RpcClient::new(config.daemon.clone())?.noninteractive();
        Ok(Self {
            client,
            config,
            api_endpoint,
            config_found,
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
        changes.push("Updated, with no change to control".into());
    }
    changes
}

fn coin_amount(amount: &crate::defi::Amount) -> CurrencyAmount {
    CurrencyAmount {
        currency: amount.currency.clone(),
        amount: amount.satoshis as f64 / 100_000_000.0,
    }
}

/// Carbide reports a stop with the failing RPC attached. Say the common cause plainly.
fn plain_stop(message: &str) -> String {
    if message.contains("-32601") || message.contains("Method not found") {
        return "This data source doesn't allow the lookup needed to go further. A fully synced \
                local daemon can complete the trace."
            .into();
    }
    message.to_string()
}

/// Reduces Carbide's trail to what the app draws, keeping its stage wording.
fn transfer_trace(trail: crate::defi::Trail) -> TransferTrace {
    use crate::defi::{ReceiptKind, Stage};
    let legs: Vec<TraceLeg> = trail
        .legs
        .iter()
        .map(|leg| {
            let confidence = match leg.stage {
                Stage::Received | Stage::Forwarded | Stage::Refunded | Stage::Burned => {
                    Confidence::Verified
                }
                Stage::Unspent | Stage::BurnUnverified => Confidence::Unknown,
                _ => Confidence::Observed,
            };
            let receipt = leg.receipt.as_ref();
            TraceLeg {
                source_txid: leg.source.txid.clone(),
                source_output: leg.source.vout,
                stage: leg.stage.label().to_string(),
                confidence,
                principal: leg.principal.iter().flatten().map(coin_amount).collect(),
                pool: leg.pool.clone(),
                conversion_fee: leg.conversion_fee.as_ref().map(coin_amount),
                result: receipt.map(|receipt| {
                    match receipt.kind {
                        ReceiptKind::Received => "Received",
                        ReceiptKind::Forwarded => "Forwarded",
                        ReceiptKind::Refunded => "Refunded",
                    }
                    .to_string()
                }),
                result_amounts: receipt
                    .map(|receipt| receipt.amounts.iter().map(coin_amount).collect())
                    .unwrap_or_default(),
                result_txid: receipt.map(|receipt| receipt.output.txid.clone()),
                result_output: receipt.map(|receipt| receipt.output.vout),
                import_height: leg
                    .execution
                    .as_ref()
                    .and_then(|execution| execution.height),
            }
        })
        .collect();
    let (verdict, confidence) = match trail.legs.last().map(|leg| leg.stage) {
        Some(Stage::Received) => (
            "Received. The result output was replayed exactly.",
            Confidence::Verified,
        ),
        Some(Stage::Refunded) => (
            "Refunded. The full amount went back to the sender.",
            Confidence::Verified,
        ),
        Some(Stage::Forwarded) => ("Forwarded to the next system.", Confidence::Verified),
        Some(Stage::Burned) => (
            "Burned. The supply change was verified.",
            Confidence::Verified,
        ),
        Some(Stage::Imported | Stage::BurnUnverified) => (
            "Imported, but the result output has not been matched.",
            Confidence::Observed,
        ),
        Some(Stage::Exported) => (
            "Exported. Its arrival has not been found in the blocks searched.",
            Confidence::Observed,
        ),
        Some(Stage::Pending) => ("Waiting to be exported.", Confidence::Observed),
        Some(Stage::Unconfirmed) => ("Not yet confirmed in a block.", Confidence::Observed),
        Some(Stage::Unspent) => ("Not yet picked up for export.", Confidence::Unknown),
        Some(Stage::Observed) => (
            "Seen at its source. Nothing further is established.",
            Confidence::Observed,
        ),
        None => (
            "Nothing could be established for this output.",
            Confidence::Unknown,
        ),
    };
    TransferTrace {
        verdict: verdict.to_string(),
        confidence,
        legs,
        steps: trail
            .entries
            .into_iter()
            .map(|entry| TraceStep {
                system_id: entry.output.system_id.to_string(),
                txid: entry.output.txid,
                output: entry.output.vout,
                label: entry.label,
                detail: entry.detail,
            })
            .collect(),
        notes: trail.notes,
        stop: trail.stop.map(|stop| plain_stop(&stop.message)),
        search: trail.search.map(|search| TraceSearch {
            system_id: search.system,
            first: search.first,
            last: search.last,
            tip: search.tip,
            windows: search.windows,
            more_older: search.more_older,
        }),
    }
}

/// Reads `getnotarizationdata`. Confirmed notarizations are those on the best fork at or
/// before the last confirmed index.
fn notarization_data(raw: &Value, local_system: Option<&str>) -> NotarizationData {
    let forks: Vec<Vec<u64>> = raw["forks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|fork| {
            fork.as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .collect()
        })
        .collect();
    let best = raw["bestchain"].as_u64().unwrap_or(0) as usize;
    let best_fork = forks.get(best).cloned().unwrap_or_default();
    let last_confirmed = raw["lastconfirmed"].as_i64().filter(|index| *index >= 0);
    let confirmed_position = last_confirmed
        .and_then(|index| best_fork.iter().position(|member| *member as i64 == index));
    let mut other_system = None;
    let mut currency_id = None;
    let rows = raw["notarizations"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            let index = entry["index"].as_u64().unwrap_or(0);
            let notarization = &entry["notarization"];
            currency_id = currency_id.take().or_else(|| text(&notarization["currencyid"]));
            let mut local_height = None;
            let mut other_height = None;
            for root in notarization["proofroots"].as_array().into_iter().flatten() {
                let system = root["systemid"].as_str();
                if system.is_some() && system == local_system {
                    local_height = root["height"].as_u64();
                } else {
                    other_height = root["height"].as_u64();
                    other_system = other_system.take().or_else(|| system.map(str::to_owned));
                }
            }
            let position = best_fork.iter().position(|member| *member == index);
            NotarizationRow {
                index: index as u32,
                txid: text(&entry["txid"]),
                output: entry["vout"].as_u64().map(|n| n as u32),
                block_hash: text(&entry["blockhash"]),
                block_height: None,
                block_time: None,
                local_height,
                other_height: other_height.or_else(|| notarization["notarizationheight"].as_u64()),
                confirmed: matches!((position, confirmed_position), (Some(at), Some(limit)) if at <= limit),
                on_best_chain: position.is_some(),
            }
        })
        .collect();
    NotarizationData {
        currency_id,
        other_system_id: other_system,
        rows,
        fork_count: forks.len() as u32,
        last_confirmed_index: last_confirmed.map(|index| index as u32),
        json: pretty(&without(raw, "notarizations")),
    }
}

fn mempool_row(txid: &str, entry: &Value) -> MempoolRow {
    // With full transactions requested, the entry carries the transaction's own fields.
    let tags = if entry.get("vout").is_some() {
        labels(&TransactionSummary {
            txid: txid.to_string(),
            raw: Some(entry.clone()),
        })
    } else {
        Vec::new()
    };
    MempoolRow {
        txid: txid.to_string(),
        size: entry["size"].as_u64().unwrap_or(0),
        fee_sats: entry["fee"]
            .as_f64()
            .map(|coins| (coins * 100_000_000.0).round() as i64),
        time: entry["time"].as_u64(),
        depends: count(Some(&entry["depends"])),
        tags,
    }
}

/// Reads one `getoffers` entry. Each has what is offered and what is accepted in return;
/// either side can be an identity or a map of currency amounts.
fn market_offer(entry: &Value) -> Option<MarketOffer> {
    let body = &entry["offer"];
    let txid = text(&body["txid"])?;
    let offered = &body["offer"];
    let accepted = &body["accept"];
    let first_amount = |side: &Value| {
        side.as_object()?
            .iter()
            .find_map(|(currency, amount)| Some((currency.clone(), amount.as_f64()?)))
    };
    let identity = |side: &Value| {
        Some((
            text(&side["name"]).or_else(|| text(&side["identityid"]))?,
            text(&side["identityid"]),
        ))
    };
    let expiry_height = body["blockexpiry"].as_u64();
    if offered.get("identityid").is_some() {
        let (subject, subject_id) = identity(offered)?;
        let (price_currency, price) = first_amount(accepted)?;
        return Some(MarketOffer {
            txid,
            kind: OfferKind::IdentityForSale,
            subject,
            subject_id,
            price,
            price_currency,
            expiry_height,
        });
    }
    if accepted.get("identityid").is_some() {
        let (subject, subject_id) = identity(accepted)?;
        let (price_currency, price) = first_amount(offered)?;
        return Some(MarketOffer {
            txid,
            kind: OfferKind::IdentityWanted,
            subject,
            subject_id,
            price,
            price_currency,
            expiry_height,
        });
    }
    // A swap: the largest offered amount for the first accepted one.
    let (subject, _) = offered
        .as_object()?
        .iter()
        .filter_map(|(currency, amount)| Some((currency.clone(), amount.as_f64()?)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let (price_currency, price) = first_amount(accepted)?;
    Some(MarketOffer {
        txid,
        kind: OfferKind::CurrencySwap,
        subject_id: Some(subject.clone()),
        subject,
        price,
        price_currency,
        expiry_height,
    })
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

#[cfg(test)]
#[path = "ffi_tests.rs"]
mod tests;
