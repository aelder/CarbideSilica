//! Charts: block series over any range, moving averages and the fee pool.
//!
//! Part of the bridge's Swift-facing API. See `ffi.rs` for the shared types and helpers.
//!
//! The pure parts are ported from Carbide's terminal charts (`src/app/chart_ui.rs`,
//! `chart_ui/fee_pool.rs` and `chart_ui/seek.rs`), which are tied to the terminal UI and
//! cannot be called from here. Each port says where it came from. Time parsing, time
//! lookup and the forecast use Carbide's `block_time` and `block_forecast`, through a
//! `TimeSource` that a public API also answers (see `SourceTime`).
//!
//! Loading is split into steps so that the app can show progress and stop: the app calls
//! `ChartLoad::step` until it reports `done`, and stops calling when the view goes away.
//! A UniFFI future is not cancelled when its Swift task is, so one long call would keep
//! the node busy after the view closed.

use std::collections::{BTreeMap, VecDeque};

use chrono::{DateTime, Utc};

#[allow(unused_imports)]
use super::*;

/// Carbide's bound on the headers one chart request may read.
pub(super) const MAX_HEADERS: usize = 20_000;
/// The moving-average window, in chain heights.
pub(super) const AVERAGE_WINDOW: usize = 100;
/// The two staked-share windows of the PoW ratio chart.
pub(super) const BALANCE_WINDOWS: [usize; 2] = [100, 1000];
/// Fee pool checkpoints: the default and the detail limit.
pub(super) const FEE_POOL_POINTS: u32 = 100;
pub(super) const FEE_POOL_DETAIL_POINTS: u32 = 2000;
const DEFAULT_CHUNK: u32 = 500;
const MAX_CHUNK: u32 = 2000;

// chainparams.cpp, ASSETCHAINS_LWMAPOS profile, Verus source 72dc29a5f (as cited by Carbide).
// This is the fixed initial posLimit, not GetPoSCheckParams' moving limit.
const POS_REFERENCE: &str = "000000000f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f";
const UNIT: i128 = 100_000_000;

// --------------------------------------------------------------- records

/// How a block was produced, from the node's `validationtype` or `blocktype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BlockProduction {
    Mined,
    Staked,
    Unknown,
}

/// A moving average and the number of blocks that contributed to it.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct ChartAverage {
    pub value: f64,
    pub samples: u32,
}

/// Mined, staked and unknown blocks in a trailing window that ends at a sample.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, uniffi::Record)]
pub struct ProductionCounts {
    pub mined: u32,
    pub staked: u32,
    pub unknown: u32,
    /// The window's full size: 100 or 1,000 blocks.
    pub window: u32,
}

impl ProductionCounts {
    fn total(self) -> u32 {
        self.mined + self.staked + self.unknown
    }

    /// The staked share only for a complete window with no unknown block.
    fn stake_share(self) -> Option<f64> {
        (self.total() == self.window && self.unknown == 0)
            .then(|| f64::from(self.staked) / f64::from(self.window))
    }
}

/// One block of a chart, with the figures derived from the blocks before it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ChartSample {
    pub height: u64,
    pub hash: String,
    pub time: u64,
    pub production: BlockProduction,
    /// True when this block's parent hash is the block before it in the loaded range.
    /// Lines do not join across an unlinked sample.
    pub linked: bool,
    /// True when this block's time is later than the time of the block before it.
    pub time_increases: bool,
    /// Signed seconds from the linked parent's timestamp. `None` without a linked parent.
    pub interval: Option<i64>,
    /// The node's difficulty at a mined block.
    pub mining: Option<f64>,
    /// Staking difficulty at a staked block: the fixed reference over the block's target.
    pub staking: Option<f64>,
    /// The staked block's expanded proof-of-stake target, as hex.
    pub pos_target: Option<String>,
    /// Why a staked block has no staking difficulty, in Carbide's words.
    pub problem: Option<String>,
    pub mining_average: Option<ChartAverage>,
    pub staking_average: Option<ChartAverage>,
    pub window100: ProductionCounts,
    pub window1000: ProductionCounts,
    pub share100: Option<f64>,
    pub share1000: Option<f64>,
}

/// A loaded chart range. `samples` holds only the visible range, oldest first.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ChartPage {
    pub first: u64,
    pub last: u64,
    pub tip: u64,
    pub samples: Vec<ChartSample>,
    /// False while loading, or when the header limit or genesis stopped the range early.
    pub complete: bool,
    /// True when the 20,000-header limit stopped the range.
    pub limited: bool,
    /// Preceding blocks loaded for averages and intervals.
    pub context: u32,
    pub headers: u32,
    /// Visible intervals below zero. The plot does not draw them.
    pub negative_intervals: u32,
    /// Visible staked blocks without a staking difficulty.
    pub unavailable_stake: u32,
    pub status: String,
    /// Headers read from stored observations instead of a source.
    pub stored: u32,
    /// The source of the live headers, or, when no source answered, the stored
    /// observations the page shows (`retained` is then true).
    pub provenance: Option<Provenance>,
    /// True when the range follows parent hashes on a side branch.
    pub side_branch: bool,
}

/// What a chart range loads.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChartRequest {
    /// 7,200, 21,600, 86,400 or 604,800 seconds of header time.
    pub span_seconds: u64,
    /// The newest block of the range, or, when `newer` is true, the last block of the range
    /// before it. `None` is the chain tip.
    pub end_height: Option<u64>,
    /// When known, the hash the block at `end_height` must have.
    pub anchor_hash: Option<String>,
    /// Load the range after `end_height` instead of the range ending at it.
    pub newer: bool,
    /// Trailing heights a sample needs: 2 for intervals, 100 for averages, 1,000 for the
    /// PoW ratio's longer window.
    pub context: u32,
    /// A sought block that must stay inside the range.
    pub focus_height: Option<u64>,
    pub focus_hash: Option<String>,
    /// Headers read in one step. Smaller steps show progress sooner through a slow API.
    pub chunk: u32,
}


/// The result of one loading step.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ChartProgress {
    pub done: bool,
    pub headers: u32,
    /// Set when the step stopped the range early. The page keeps what loaded.
    pub stopped: Option<String>,
}

/// A block named by height and hash, with its time when known.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ChartBlock {
    pub height: u64,
    pub hash: String,
    pub time: u64,
}

/// A seek target typed by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SeekTarget {
    /// A bare number: a height up to the tip, otherwise Unix seconds.
    Number { value: u64 },
    Height { value: u64 },
    Time { unix: i64 },
    Latest,
}

/// What a seek found: the block to select and the newest block of a range centred on it.
#[derive(Debug, Clone, uniffi::Record)]
pub struct SeekOutcome {
    pub block: ChartBlock,
    pub end: ChartBlock,
    pub tip: u64,
    pub note: String,
    /// The target is later than the tip. No block exists for it yet.
    pub future: bool,
}

/// The two currencies a coinbase fee pool can hold.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FeePoolCurrencies {
    pub native_id: String,
    pub native_label: String,
    /// The network's Verus currency on a PBaaS chain. `None` on Verus itself.
    pub reserve_id: Option<String>,
    pub reserve_label: String,
}

/// The fee pool checkpoint in one block's coinbase.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FeePoolCheckpoint {
    pub height: u64,
    pub hash: String,
    pub time: u64,
    /// False when the coinbase has no checkpoint, such as before activation.
    pub known: bool,
    pub native_sats: Option<i64>,
    pub reserve_sats: Option<i64>,
    /// Why the checkpoint could not be read.
    pub problem: Option<String>,
}

/// The fee pool at the latest observed block, fetched apart from any chart range.
#[derive(Debug, Clone, uniffi::Record)]
pub struct FeePoolCurrent {
    pub currencies: FeePoolCurrencies,
    pub checkpoint: FeePoolCheckpoint,
}

/// A future block estimate from Carbide's calibrated rate.
#[derive(Debug, Clone, uniffi::Record)]
pub struct Forecast {
    /// True when the target was a height, false when it was a date.
    pub target_is_height: bool,
    pub height: u64,
    pub time: i64,
    pub seconds_per_block: f64,
    pub start: ChartBlock,
    pub tip: ChartBlock,
    /// When the tip was observed, in Unix seconds.
    pub observed_at: i64,
    pub system: String,
    pub source_label: String,
}

// ---------------------------------------------------------- loading object

/// One chart range, loaded in steps. See the module comment.
#[derive(uniffi::Object)]
pub struct ChartLoad {
    explorer: Arc<Explorer>,
    state: tokio::sync::Mutex<Walk>,
}

#[uniffi::export(async_runtime = "tokio")]
impl Explorer {
    /// Starts loading a chart range. Call `step` on the result until it is done.
    pub fn chart_load(self: Arc<Self>, request: ChartRequest) -> Arc<ChartLoad> {
        let walk = Walk::new(request);
        Arc::new(ChartLoad {
            explorer: self,
            state: tokio::sync::Mutex::new(walk),
        })
    }

    /// Finds the block for a seek target and the newest block of a range of
    /// `span_seconds` centred on it. Ported from Carbide's `chart_ui/seek.rs`.
    pub async fn chart_seek(&self, target: SeekTarget, span_seconds: u64) -> Outcome<SeekOutcome> {
        // A date search reads up to approximately 150 blocks. They are not recorded.
        let result = seek(&self.unrecorded_client(), target, span_seconds).await;
        Ok(self.settle(result)?)
    }

    /// The fee pool at the latest block, and the currencies it can hold.
    pub async fn fee_pool_current(&self) -> Outcome<FeePoolCurrent> {
        let currencies = self.settle(self.fee_pool_currencies().await)?;
        let client = self.unrecorded_client();
        let info = self.settle(client.get_blockchain_info().await)?.result;
        let height = info["blocks"]
            .as_u64()
            .ok_or_else(|| fail("The source did not report a block height."))?;
        let hash = info["bestblockhash"]
            .as_str()
            .filter(|hash| valid_hash(hash))
            .ok_or_else(|| fail("The source did not report the latest block hash."))?
            .to_string();
        let checkpoint = self.settle(fee_pool_at(&client, height, &hash, &currencies).await)?;
        Ok(FeePoolCurrent {
            currencies,
            checkpoint,
        })
    }

    /// Fee pool checkpoints at the given blocks, in the same order. A block that cannot be
    /// read gives a checkpoint with a problem, not an error, so one gap does not stop a chart.
    /// The reads are not recorded in Carbide's cache.
    pub async fn fee_pool_checkpoints(&self, blocks: Vec<ChartBlock>) -> Outcome<Vec<FeePoolCheckpoint>> {
        let currencies = Arc::new(self.settle(self.fee_pool_currencies().await)?);
        let quiet = self.unrecorded_client();
        let mut tasks = tokio::task::JoinSet::new();
        for (index, block) in blocks.into_iter().enumerate() {
            let client = quiet.clone();
            let currencies = currencies.clone();
            tasks.spawn(async move {
                let result = fee_pool_at(&client, block.height, &block.hash, &currencies).await;
                (index, block, result)
            });
        }
        let mut found = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            let (index, block, result) = joined.map_err(|_| fail("A fee pool request stopped."))?;
            let checkpoint = match self.settle(result) {
                Ok(checkpoint) => checkpoint,
                Err(error) => FeePoolCheckpoint {
                    height: block.height,
                    hash: block.hash,
                    time: block.time,
                    known: false,
                    native_sats: None,
                    reserve_sats: None,
                    problem: Some(readable(&error)),
                },
            };
            found.push((index, checkpoint));
        }
        found.sort_by_key(|(index, _)| *index);
        Ok(found.into_iter().map(|(_, checkpoint)| checkpoint).collect())
    }

    /// A future block's date, or a future date's height, from Carbide's calibrated rate.
    /// A source that is catching up, stale or inconsistent gives Carbide's refusal.
    pub async fn forecast(&self, query: String) -> Outcome<Forecast> {
        use crate::block_forecast::{Estimate, Target, parse_target};
        let target = parse_target(query.trim().strip_prefix("estimate:").unwrap_or(query.trim()))?;
        let calibration = calibrate(&self.config, &self.client).await;
        let estimate = Estimate::new(self.settle(calibration)?, target)?;
        let calibration = &estimate.calibration;
        Ok(Forecast {
            target_is_height: matches!(estimate.target, Target::Height(_)),
            height: estimate.height,
            time: estimate.timestamp,
            seconds_per_block: calibration.seconds_per_block(),
            start: time_block(&calibration.start),
            tip: time_block(&calibration.tip),
            observed_at: calibration.observed_at,
            system: calibration.system.clone(),
            source_label: calibration.source_label.clone(),
        })
    }
}

impl Explorer {
    async fn fee_pool_currencies(&self) -> Result<FeePoolCurrencies> {
        let info = self.client.get_info().await?.result;
        let native = info["chainid"]
            .as_str()
            .map(str::to_string)
            .or_else(|| self.config.chain.system_id.get().map(|id| id.to_string()))
            .context("The native currency ID is unavailable")?;
        if let Some(expected) = self.config.chain.system_id.get() {
            anyhow::ensure!(native == expected.as_ref(), "The source reports another chain");
        }
        let network = Network::from_testnet(self.config.chain.testnet);
        // Coinbase fee pools allow the local currency and the network's Verus currency.
        let reserve = crate::world::system_id("VRSC", network)?.to_string();
        let reserve_label = if self.config.chain.testnet { "VRSCTEST" } else { "VRSC" };
        let native_label = if native == reserve {
            reserve_label.to_string()
        } else {
            info["name"]
                .as_str()
                .unwrap_or(&self.config.chain.name)
                .to_string()
        };
        Ok(FeePoolCurrencies {
            reserve_id: (native != reserve).then_some(reserve),
            native_id: native,
            native_label,
            reserve_label: reserve_label.into(),
        })
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl ChartLoad {
    /// Reads the next group of headers. Returns `done` when the range is complete or stopped.
    pub async fn step(&self) -> Outcome<ChartProgress> {
        let explorer = &self.explorer;
        let mut walk = self.state.lock().await;
        if walk.loader.done {
            return Ok(walk.loader.progress());
        }
        let mut node = NodeHeaders {
            explorer,
            client: explorer.unrecorded_client(),
        };
        let result = walk.step(&mut node).await;
        Ok(explorer.settle(result)?)
    }

    /// The range as loaded so far, or `None` before the first headers arrive.
    pub async fn page(&self) -> Option<ChartPage> {
        let walk = self.state.lock().await;
        let mut page = walk.loader.page()?;
        page.stored = walk.stored;
        page.side_branch = walk.branch_seen;
        page.provenance = match (&walk.offline, &walk.source) {
            (Some(failure), _) => walk.oldest_stored().map(|(source, observed_at)| {
                let mut provenance = self.explorer.provenance(&source, None);
                provenance.retained = true;
                provenance.observed_at = observed_at;
                provenance.failure = Some(failure.clone());
                provenance.caution = Some(
                    "The chart did not check these blocks against the active chain.".into(),
                );
                provenance
            }),
            (None, Some(source)) => Some(self.explorer.provenance(source, None)),
            (None, None) => None,
        };
        Some(page)
    }
}

/// Live headers from the node or API, through a client that does not record answers.
struct NodeHeaders<'a> {
    explorer: &'a Explorer,
    client: RpcClient,
}

impl HeaderSource for NodeHeaders<'_> {
    async fn tip(&mut self) -> Result<(u64, String, String)> {
        let info = self.client.get_blockchain_info().await?;
        let height = info.result["blocks"].as_u64().context("Source omitted tip height")?;
        let hash = info.result["bestblockhash"]
            .as_str()
            .filter(|hash| valid_hash(hash))
            .context("Source omitted tip hash")?
            .to_string();
        Ok((height, hash, info.source_id))
    }

    async fn active_hash(&mut self, height: u64) -> Result<(String, String)> {
        let output = self
            .client
            .call_with_raw::<String>("getblockhash", &[Value::from(height)])
            .await?;
        Ok((output.result, output.source_id))
    }

    async fn by_hash(&mut self, hash: &str) -> Result<Option<(Header, String)>> {
        match self.client.call_with_raw::<Value>("getblock", &[Value::from(hash)]).await {
            Ok(output) => Ok(Some((Header::decode(&output.result)?, output.source_id))),
            // The source answered that it does not have the block.
            Err(error) if source_answered(&error) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn by_heights(&mut self, heights: &[u64]) -> Result<Vec<(Header, String)>> {
        fetch_headers(&self.client, heights).await
    }

    async fn recorded(&mut self, hash: &str) -> Option<Stored> {
        // The terminal app and other views record blocks and headers in Carbide's cache.
        let path = self.explorer.config.cache_path.clone();
        let system = self.explorer.config.chain_cache_key();
        let hash = hash.to_string();
        blocking(move || {
            for (method, params) in [
                ("getblockheader", vec![Value::from(hash.clone()), Value::from(true)]),
                ("getblock", vec![Value::from(hash.clone())]),
            ] {
                if let Some(row) = retained_row(&path, &system, method, &params)? {
                    let response: Value = serde_json::from_str(&row.payload)?;
                    let header = Header::decode(&response["result"])?;
                    if header.hash == hash {
                        return Ok(Some(Stored {
                            header,
                            source: row.source,
                            observed_at: row.observed_at,
                        }));
                    }
                }
            }
            Ok(None)
        })
        .await
        .ok()
        .flatten()
    }

    async fn recorded_at_height(&mut self, height: u64) -> Option<Stored> {
        let hash = self.recorded_value("getblockhash", vec![Value::from(height)]).await?;
        self.recorded(hash.as_str()?).await
    }

    async fn recorded_tip(&mut self) -> Option<Stored> {
        let info = self.recorded_value("getblockchaininfo", Vec::new()).await?;
        self.recorded(info["bestblockhash"].as_str()?).await
    }

    fn label(&self, source: &str) -> String {
        self.explorer.sources.label(source).0
    }
}

impl NodeHeaders<'_> {
    /// The result of the latest recorded answer to a request, from any source.
    async fn recorded_value(&self, method: &'static str, params: Vec<Value>) -> Option<Value> {
        let path = self.explorer.config.cache_path.clone();
        let system = self.explorer.config.chain_cache_key();
        blocking(move || {
            let Some(row) = retained_row(&path, &system, method, &params)? else {
                return Ok(None);
            };
            let response: Value = serde_json::from_str(&row.payload)?;
            Ok(response.get("result").cloned())
        })
        .await
        .ok()
        .flatten()
    }
}

/// Runs blocking work, such as SQLite, off the async threads.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.context("A storage task stopped")?
}

// ------------------------------------------------------------------ walk

/// Where chart headers come from while a range loads. `NodeHeaders` is the real one. The
/// tests use samples.
pub(super) trait HeaderSource {
    /// The tip height and hash, and the source that answered.
    fn tip(&mut self) -> impl Future<Output = Result<(u64, String, String)>> + Send;
    /// The active chain's hash at a height, and the source that answered.
    fn active_hash(&mut self, height: u64) -> impl Future<Output = Result<(String, String)>> + Send;
    /// A block by hash. `None` when the source answers that it does not have it.
    fn by_hash(&mut self, hash: &str) -> impl Future<Output = Result<Option<(Header, String)>>> + Send;
    /// Blocks at heights, in the same order.
    fn by_heights(&mut self, heights: &[u64]) -> impl Future<Output = Result<Vec<(Header, String)>>> + Send;
    /// A header already recorded in Carbide's cache.
    fn recorded(&mut self, hash: &str) -> impl Future<Output = Option<Stored>> + Send;
    /// The recorded header that a recorded `getblockhash` names for a height.
    fn recorded_at_height(&mut self, height: u64) -> impl Future<Output = Option<Stored>> + Send;
    /// The recorded header of the latest recorded tip.
    fn recorded_tip(&mut self) -> impl Future<Output = Option<Stored>> + Send;
    /// A source in words.
    fn label(&self, source: &str) -> String;
}

/// One chart range's walk: the loader's state, where its headers came from, and the
/// source it is pinned to. Ported in outline from Carbide's `chart_ui::load_inner`:
/// recorded headers first, then the source, with every live answer from the same source.
/// Nothing the walk reads is recorded.
pub(super) struct Walk {
    pub loader: Loader,
    started: bool,
    /// The source every live answer must come from.
    pub source: Option<String>,
    /// Why no source answered, when the walk uses recorded headers only.
    pub offline: Option<String>,
    /// True while the walk follows parent hashes on a side branch.
    pub branch: bool,
    pub branch_seen: bool,
    pub stored: u32,
    stored_sources: BTreeMap<String, u64>,
}

impl Walk {
    pub(super) fn new(request: ChartRequest) -> Self {
        Self {
            loader: Loader::new(request),
            started: false,
            source: None,
            offline: None,
            branch: false,
            branch_seen: false,
            stored: 0,
            stored_sources: BTreeMap::new(),
        }
    }

    /// The oldest observation among the recorded headers, with its source.
    pub(super) fn oldest_stored(&self) -> Option<(String, u64)> {
        self.stored_sources
            .iter()
            .min_by_key(|(_, at)| **at)
            .map(|(source, at)| (source.clone(), *at))
    }

    fn pin(&self, live: &impl HeaderSource, source: &str) -> Result<()> {
        if let Some(pinned) = &self.source {
            anyhow::ensure!(
                pinned == source,
                "The source changed during loading, from {} to {}. Load the range again.",
                live.label(pinned),
                live.label(source)
            );
        }
        Ok(())
    }

    /// One step: up to `chunk` headers.
    pub(super) async fn step(&mut self, live: &mut impl HeaderSource) -> Result<ChartProgress> {
        if !self.started {
            self.begin(live).await?;
            self.started = true;
        }
        let budget = self.loader.request.chunk.clamp(1, MAX_CHUNK) as usize;
        let mut added = 0;
        while !self.loader.done && added < budget {
            let Some(expect) = self.loader.expect() else {
                self.loader.done = true;
                break;
            };
            let count = self.advance(live, expect, budget - added).await?;
            if count == 0 {
                break;
            }
            added += count;
        }
        if self.loader.done {
            self.finish(live).await?;
        }
        Ok(self.loader.progress())
    }

    /// Finds the tip, the anchor and whether the anchor is on a side branch. When no source
    /// answers, starts from recorded headers.
    async fn begin(&mut self, live: &mut impl HeaderSource) -> Result<()> {
        let requested_end = self.loader.request.end_height;
        let requested_anchor = self.loader.request.anchor_hash.clone();
        match live.tip().await {
            Ok((tip, best, source)) => {
                self.source = Some(source);
                let mut end = requested_end;
                let mut anchor = requested_anchor;
                if let (None, Some(hash)) = (end, &anchor) {
                    // An anchor by hash alone: find its height.
                    let header = self.header_by_hash(live, hash).await?.with_context(|| {
                        "The node does not have the requested block. It can be on a side branch that the node no longer has."
                    })?;
                    end = Some(header.height);
                }
                let end = end.unwrap_or(tip);
                match &anchor {
                    None if end == tip => anchor = Some(best),
                    None => {
                        anyhow::ensure!(end <= tip, "The requested block is later than the source's tip");
                        let (hash, source) = live.active_hash(end).await?;
                        self.pin(live, &source)?;
                        anchor = Some(hash);
                    }
                    Some(hash) => {
                        let active = if end <= tip {
                            let (active, source) = live.active_hash(end).await?;
                            self.pin(live, &source)?;
                            Some(active)
                        } else {
                            None
                        };
                        if active.as_deref() != Some(hash.as_str()) {
                            self.branch = true;
                            self.branch_seen = true;
                        }
                    }
                }
                let request = &mut self.loader.request;
                request.end_height = Some(end);
                request.anchor_hash = anchor;
                self.loader.start(tip.max(end))?;
                if self.branch && self.loader.request.newer {
                    anyhow::bail!(
                        "This range is on a side branch. The node does not give the newer blocks of a side branch."
                    );
                }
            }
            Err(error) if !source_answered(&error) => {
                self.offline = Some(readable(&error));
                let found = match (&requested_anchor, requested_end) {
                    (Some(hash), _) => live.recorded(hash).await,
                    (None, Some(height)) => live.recorded_at_height(height).await,
                    (None, None) => live.recorded_tip().await,
                };
                let Some(found) = found else {
                    anyhow::bail!(
                        "No source answered, and Carbide's cache holds no recorded block for this range. {}",
                        readable(&error)
                    );
                };
                let request = &mut self.loader.request;
                request.end_height = Some(found.header.height);
                request.anchor_hash = Some(found.header.hash.clone());
                self.loader.start(found.header.height)?;
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    async fn header_by_hash(&mut self, live: &mut impl HeaderSource, hash: &str) -> Result<Option<Header>> {
        if let Some(found) = live.recorded(hash).await {
            return Ok(Some(found.header));
        }
        match live.by_hash(hash).await? {
            Some((header, source)) => {
                self.pin(live, &source)?;
                Ok(Some(header))
            }
            None => Ok(None),
        }
    }

    /// Adds headers for one expectation: recorded ones first, then the source. Returns how
    /// many headers the loader took.
    async fn advance(&mut self, live: &mut impl HeaderSource, expect: Expect, limit: usize) -> Result<usize> {
        // 1. Headers recorded in Carbide's cache, following parent hashes.
        if let Expect::Hash { hash, .. } = &expect {
            let mut found = Vec::new();
            let mut next = Some(hash.clone());
            while let Some(hash) = next.take()
                && found.len() < limit
                && let Some(stored) = live.recorded(&hash).await
            {
                next = (stored.header.height > 0)
                    .then(|| stored.header.parent.clone())
                    .flatten();
                found.push(stored);
            }
            if !found.is_empty() {
                let count = self.take_stored(found)?;
                if count > 0 {
                    return Ok(count);
                }
            }
        }
        // 2. The source.
        if self.offline.is_some() {
            self.loader.done = true;
            self.loader.stopped = Some(format!(
                "No source answered. The chart shows the {} recorded blocks that link to this range.",
                self.loader.headers.len()
            ));
            return Ok(0);
        }
        if self.branch {
            let Expect::Hash { height, hash } = expect else {
                anyhow::bail!(
                    "This range is on a side branch. The node does not give the newer blocks of a side branch."
                );
            };
            let Some((header, source)) = live.by_hash(&hash).await? else {
                anyhow::bail!(
                    "The node no longer has block {hash} at height {height}. The side branch cannot be followed further."
                );
            };
            self.pin(live, &source)?;
            let before = self.loader.headers.len();
            self.loader.accept(vec![header])?;
            // Where the branch meets the active chain, the faster walk by height continues.
            let (active, source) = live.active_hash(height).await?;
            self.pin(live, &source)?;
            if active == hash {
                self.branch = false;
            }
            return Ok(self.loader.headers.len() - before);
        }
        let Some(heights) = self.loader.next_chunk() else {
            self.loader.done = true;
            return Ok(0);
        };
        let heights: Vec<u64> = heights.into_iter().take(limit.max(1)).collect();
        let fetched = live.by_heights(&heights).await?;
        let mut headers = Vec::with_capacity(fetched.len());
        for (header, source) in fetched {
            self.pin(live, &source)?;
            headers.push(header);
        }
        let before = self.loader.headers.len();
        self.loader.accept(headers)?;
        Ok(self.loader.headers.len() - before)
    }

    fn take_stored(&mut self, found: Vec<Stored>) -> Result<usize> {
        let before = self.loader.headers.len();
        let mut origins = Vec::new();
        let headers: Vec<Header> = found
            .into_iter()
            .map(|stored| {
                origins.push((stored.header.height, stored.source, stored.observed_at));
                stored.header
            })
            .collect();
        self.loader.accept(headers)?;
        for (height, source, at) in origins {
            if self.loader.headers.contains_key(&height) {
                let oldest = self.stored_sources.entry(source).or_insert(at);
                *oldest = (*oldest).min(at);
            }
        }
        let count = self.loader.headers.len() - before;
        self.stored += count as u32;
        Ok(count)
    }

    /// Checks the newest block against the active chain, when a source answers.
    async fn finish(&mut self, live: &mut impl HeaderSource) -> Result<()> {
        if self.offline.is_some() {
            self.loader.status = "Stored observations, not checked against a source".into();
            return Ok(());
        }
        if self.loader.limited {
            return Ok(());
        }
        let Some(last) = self
            .loader
            .page_bounds()
            .and_then(|(_, last)| self.loader.headers.get(&last))
            .map(|h| (h.height, h.hash.clone()))
        else {
            return Ok(());
        };
        let (active, source) = live.active_hash(last.0).await?;
        self.pin(live, &source)?;
        self.loader.status = if active == last.1 {
            "Observed active chain".into()
        } else if self.branch_seen {
            "Side branch: this range follows parent hashes from a block that is not on the active chain".into()
        } else {
            "The newest block of this range left the active chain after loading. Load the range again.".into()
        };
        Ok(())
    }
}

/// A header recorded in Carbide's cache, with where and when it was observed.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Stored {
    pub header: Header,
    pub source: String,
    pub observed_at: u64,
}

#[derive(Debug, Clone)]
pub(super) enum Expect {
    /// The block at a height with a known hash: the parent of the lowest loaded block.
    Hash { height: u64, hash: String },
    /// The child at a height of a known parent: the next block of a newer range.
    Child { height: u64, parent: String },
    /// The block at a height, with no hash known.
    Height { height: u64 },
}



// ---------------------------------------------------------------- loader

/// A decoded header. Ported from Carbide's `chart_ui::Sample::decode`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Header {
    pub height: u64,
    pub hash: String,
    pub parent: Option<String>,
    pub time: u64,
    pub production: BlockProduction,
    pub mining: Option<f64>,
    pub staking: Option<f64>,
    pub target: Option<String>,
    pub problem: Option<String>,
}

impl Header {
    pub(super) fn decode(raw: &Value) -> Result<Self> {
        let height = raw["height"].as_u64().context("Header has no height")?;
        let hash = raw["hash"]
            .as_str()
            .filter(|hash| valid_hash(hash))
            .context("Header has no valid hash")?
            .to_owned();
        let time = raw["time"].as_u64().context("Header has no timestamp")?;
        let production = match raw["validationtype"].as_str() {
            Some("stake") => BlockProduction::Staked,
            Some("work") => BlockProduction::Mined,
            _ => match raw["blocktype"].as_str() {
                Some("staked" | "minted" | "stake") => BlockProduction::Staked,
                Some("mined" | "work") => BlockProduction::Mined,
                _ => BlockProduction::Unknown,
            },
        };
        let (staking, target, problem) = if production == BlockProduction::Staked {
            match stake_difficulty(raw) {
                Ok((value, target)) => (Some(value), Some(target), None),
                Err(error) => (None, None, Some(error.to_string())),
            }
        } else {
            (None, None, None)
        };
        let parent = raw["previousblockhash"].as_str().map(str::to_owned);
        anyhow::ensure!(
            height == 0 || parent.as_deref().is_some_and(valid_hash),
            "Header parent unavailable"
        );
        Ok(Self {
            height,
            hash,
            parent,
            time,
            production,
            mining: (production == BlockProduction::Mined)
                .then(|| raw["difficulty"].as_f64())
                .flatten()
                .filter(|value| value.is_finite() && *value > 0.0),
            staking,
            target,
            problem,
        })
    }
}

/// A chart range's loading state. Carbide's `chart_ui::History` follows parent hashes one
/// header at a time. This version reads groups of heights at once, then checks that each
/// header's parent hash is the header below it, so the result is the same linked chain.
pub(super) struct Loader {
    pub request: ChartRequest,
    pub tip: Option<u64>,
    /// The newest block of an older range, or the last block before a newer range.
    pub origin: u64,
    pub headers: BTreeMap<u64, Header>,
    /// Older ranges: the oldest visible height, once the span is covered.
    pub boundary: Option<u64>,
    /// Newer ranges: the newest visible height, once the span is covered.
    pub newest: Option<u64>,
    pub done: bool,
    pub limited: bool,
    pub stopped: Option<String>,
    pub status: String,
}

impl Loader {
    pub(super) fn new(request: ChartRequest) -> Self {
        Self {
            request,
            tip: None,
            origin: 0,
            headers: BTreeMap::new(),
            boundary: None,
            newest: None,
            done: false,
            limited: false,
            stopped: None,
            status: "Loading".into(),
        }
    }

    fn context(&self) -> u64 {
        u64::from(self.request.context.max(2))
    }

    pub(super) fn start(&mut self, tip: u64) -> Result<()> {
        self.tip = Some(tip);
        let end = self.request.end_height.unwrap_or(tip);
        anyhow::ensure!(end <= tip, "The requested block is later than the source's tip");
        self.origin = end;
        if self.request.newer && end >= tip {
            self.done = true;
            self.stopped = Some("No newer blocks exist after this range.".into());
        }
        Ok(())
    }

    /// The next block the range needs: below the lowest loaded block (its parent hash is
    /// known), above the highest (its parent is known), or the anchor itself.
    pub(super) fn expect(&self) -> Option<Expect> {
        if self.done || self.headers.len() >= MAX_HEADERS {
            return None;
        }
        let tip = self.tip?;
        let anchor = || match &self.request.anchor_hash {
            Some(hash) => Expect::Hash {
                height: self.origin,
                hash: hash.clone(),
            },
            None => Expect::Height { height: self.origin },
        };
        if !self.headers.contains_key(&self.origin) {
            return Some(anchor());
        }
        let (lowest, highest) = (self.headers.values().next()?, self.headers.values().next_back()?);
        let below = |floor: u64| {
            (lowest.height > floor && lowest.height > 0).then(|| Expect::Hash {
                height: lowest.height - 1,
                hash: lowest.parent.clone().unwrap_or_default(),
            })
        };
        if self.request.newer {
            if let Some(expect) = below(self.origin.saturating_sub(self.context() - 2)) {
                return Some(expect);
            }
            (highest.height < tip).then(|| Expect::Child {
                height: highest.height + 1,
                parent: highest.hash.clone(),
            })
        } else {
            let floor = match self.boundary {
                Some(boundary) => boundary.saturating_sub(self.context() - 1),
                None => 0,
            };
            below(floor)
        }
    }

    /// The heights to read next, or `None` when the range is complete.
    pub(super) fn next_chunk(&self) -> Option<Vec<u64>> {
        if self.done {
            return None;
        }
        let room = MAX_HEADERS.saturating_sub(self.headers.len()) as u64;
        if room == 0 {
            return None;
        }
        let chunk = u64::from(self.request.chunk.clamp(1, MAX_CHUNK)).min(room);
        let tip = self.tip?;
        if self.request.newer {
            // First the origin and the context before it, then forward from the origin.
            let low = self.origin.saturating_sub(self.context() - 2);
            if let Some(missing) = (low..=self.origin).rev().find(|h| !self.headers.contains_key(h)) {
                let from = missing.saturating_sub(chunk - 1).max(low);
                return Some((from..=missing).collect());
            }
            let next = self.headers.keys().next_back().map_or(self.origin + 1, |h| h + 1);
            if next > tip {
                return None;
            }
            Some((next..=(next + chunk - 1).min(tip)).collect())
        } else {
            let next = match self.headers.keys().next() {
                None => self.origin,
                Some(0) => return None,
                Some(lowest) => lowest - 1,
            };
            let floor = match self.boundary {
                Some(boundary) => boundary.saturating_sub(self.context() - 1),
                None => 0,
            };
            if next < floor {
                return None;
            }
            let from = next.saturating_sub(chunk - 1).max(floor);
            Some((from..=next).rev().collect())
        }
    }

    /// Adds a group of headers read by `next_chunk`. Checks heights, the anchor and the
    /// parent links, then updates the boundary and completion.
    pub(super) fn accept(&mut self, headers: Vec<Header>) -> Result<()> {
        for header in headers {
            if self.done {
                break;
            }
            if header.height == self.origin
                && let Some(anchor) = &self.request.anchor_hash
            {
                anyhow::ensure!(
                    &header.hash == anchor,
                    "The block at height {} is not the requested block. It is not on the active chain, or the chain changed. Load the range again.",
                    header.height
                );
            }
            if let (Some(height), Some(hash)) = (self.request.focus_height, &self.request.focus_hash)
                && header.height == height
            {
                anyhow::ensure!(
                    &header.hash == hash,
                    "The sought block belongs to a different branch. Try again."
                );
            }
            if let Some(above) = self.headers.get(&(header.height + 1)) {
                anyhow::ensure!(
                    above.parent.as_deref() == Some(header.hash.as_str()),
                    "The chain changed during loading. Load the range again."
                );
            }
            if let Some(below) = header.height.checked_sub(1).and_then(|h| self.headers.get(&h)) {
                anyhow::ensure!(
                    header.parent.as_deref() == Some(below.hash.as_str()),
                    "The chain changed during loading. Load the range again."
                );
            }
            let (height, time) = (header.height, header.time);
            self.headers.insert(height, header);
            if self.request.newer {
                if height > self.origin && self.newest.is_none() {
                    let first = self.headers.get(&(self.origin + 1)).map_or(time, |h| h.time);
                    if time.saturating_sub(first) >= self.request.span_seconds
                        || Some(height) == self.tip
                    {
                        self.newest = Some(height);
                        self.done = true;
                    }
                }
            } else {
                if self
                    .boundary
                    .is_some_and(|b| height <= b.saturating_sub(self.context() - 1))
                {
                    self.done = true;
                }
                if self.boundary.is_none() {
                    let end_time = self.headers.get(&self.origin).map_or(time, |h| h.time);
                    if time <= end_time.saturating_sub(self.request.span_seconds)
                        && self.request.focus_height.is_none_or(|focus| height <= focus)
                    {
                        self.boundary = Some(height);
                    }
                }
                if height == 0 {
                    self.boundary.get_or_insert(0);
                    self.done = true;
                }
            }
        }
        if !self.done && self.headers.len() >= MAX_HEADERS {
            self.done = true;
            self.limited = true;
            self.status = "Partial range: the 20,000-header limit was reached".into();
            self.stopped = Some(format!(
                "The chart reached the {MAX_HEADERS}-header limit for one request. Choose a shorter span or page onward."
            ));
        }
        if self.request.newer && !self.done && self.headers.keys().next_back() == self.tip.as_ref() {
            self.done = true;
        }
        Ok(())
    }

    /// The visible heights so far.
    pub(super) fn page_bounds(&self) -> Option<(u64, u64)> {
        if self.request.newer {
            let last = self.newest.or_else(|| self.headers.keys().next_back().copied())?;
            (last > self.origin).then_some((self.origin + 1, last))
        } else {
            let lowest = *self.headers.keys().next()?;
            Some((self.boundary.unwrap_or(lowest), self.origin))
        }
    }

    pub(super) fn page(&self) -> Option<ChartPage> {
        let (first, last) = self.page_bounds()?;
        let headers: Vec<&Header> = self.headers.range(..=last).map(|(_, h)| h).collect();
        let mut page = build_page(&headers, first, last);
        page.tip = self.tip.unwrap_or(last);
        page.complete = self.done && !self.limited;
        page.limited = self.limited;
        page.status = self.status.clone();
        Some(page)
    }

    pub(super) fn progress(&self) -> ChartProgress {
        ChartProgress {
            done: self.done,
            headers: self.headers.len() as u32,
            stopped: self.stopped.clone(),
        }
    }
}

/// Reads headers at the given heights, in the given order. Carbide's client allows four
/// requests at a time. `getblock` without a verbosity argument, because some public APIs
/// refuse one.
async fn fetch_headers(client: &RpcClient, heights: &[u64]) -> Result<Vec<(Header, String)>> {
    let mut tasks = tokio::task::JoinSet::new();
    for (index, height) in heights.iter().copied().enumerate() {
        let client = client.clone();
        tasks.spawn(async move {
            let output = client
                .call_with_raw::<Value>("getblock", &[Value::from(height.to_string())])
                .await?;
            let header = Header::decode(&output.result)?;
            anyhow::ensure!(
                header.height == height,
                "The source returned the wrong block height"
            );
            Ok::<_, anyhow::Error>((index, (header, output.source_id)))
        });
    }
    let mut found = Vec::with_capacity(heights.len());
    while let Some(joined) = tasks.join_next().await {
        found.push(joined.context("A block request stopped")??);
    }
    found.sort_by_key(|(index, _)| *index);
    Ok(found.into_iter().map(|(_, header)| header).collect())
}

// ------------------------------------------------------------ page figures

/// Builds the visible samples from linked headers, oldest first. Ported from Carbide's
/// `chart_ui::make_page`: intervals need a linked parent, each average needs a full,
/// contiguous 100-height window, and an unlinked header restarts every window.
pub(super) fn build_page(headers: &[&Header], first: u64, last: u64) -> ChartPage {
    let mut samples = Vec::new();
    let mut window: VecDeque<&Header> = VecDeque::new();
    let mut production: [VecDeque<BlockProduction>; 2] = Default::default();
    let mut previous: Option<&Header> = None;
    let mut context = 0;
    for header in headers {
        let linked = previous.is_some_and(|before| {
            before.height.checked_add(1) == Some(header.height)
                && header.parent.as_deref() == Some(before.hash.as_str())
        });
        if !linked {
            window.clear();
            production.iter_mut().for_each(VecDeque::clear);
        }
        let interval = linked.then(|| {
            let parent = previous.expect("linked header has a parent");
            (i128::from(header.time) - i128::from(parent.time)) as i64
        });
        let time_increases = previous.is_some_and(|before| header.time > before.time);
        if window.len() == AVERAGE_WINDOW {
            window.pop_front();
        }
        window.push_back(header);
        let counts = [0, 1].map(|index| {
            let entries = &mut production[index];
            if entries.len() == BALANCE_WINDOWS[index] {
                entries.pop_front();
            }
            entries.push_back(header.production);
            let mut counts = ProductionCounts {
                window: BALANCE_WINDOWS[index] as u32,
                ..Default::default()
            };
            for entry in entries.iter() {
                match entry {
                    BlockProduction::Mined => counts.mined += 1,
                    BlockProduction::Staked => counts.staked += 1,
                    BlockProduction::Unknown => counts.unknown += 1,
                }
            }
            counts
        });
        previous = Some(header);
        if !(first..=last).contains(&header.height) {
            if header.height < first {
                context += 1;
            }
            continue;
        }
        samples.push(ChartSample {
            height: header.height,
            hash: header.hash.clone(),
            time: header.time,
            production: header.production,
            linked,
            time_increases,
            interval,
            mining: header.mining,
            staking: header.staking,
            pos_target: header.target.clone(),
            problem: header.problem.clone(),
            mining_average: average(&window, BlockProduction::Mined),
            staking_average: average(&window, BlockProduction::Staked),
            window100: counts[0],
            window1000: counts[1],
            share100: counts[0].stake_share(),
            share1000: counts[1].stake_share(),
        });
    }
    let negative_intervals = samples.iter().filter(|s| s.interval.is_some_and(|i| i < 0)).count();
    let unavailable_stake = samples
        .iter()
        .filter(|s| s.production == BlockProduction::Staked && s.staking.is_none())
        .count();
    ChartPage {
        first,
        last,
        tip: last,
        samples,
        complete: false,
        limited: false,
        context,
        headers: headers.len() as u32,
        negative_intervals: negative_intervals as u32,
        unavailable_stake: unavailable_stake as u32,
        status: String::new(),
        stored: 0,
        provenance: None,
        side_branch: false,
    }
}

/// The mean over a full window of the values of one production type. Blocks of the other
/// type are left out. An unknown block, or a matching block without a value, makes the
/// window unusable: a missing value is never taken as zero.
pub(super) fn average(window: &VecDeque<&Header>, kind: BlockProduction) -> Option<ChartAverage> {
    if window.len() != AVERAGE_WINDOW {
        return None;
    }
    let mut sum = 0.0;
    let mut samples = 0u32;
    for header in window {
        let value = match kind {
            BlockProduction::Mined => header.mining,
            _ => header.staking,
        };
        if header.production == BlockProduction::Unknown
            || (header.production == kind && value.is_none())
        {
            return None;
        }
        if header.production == kind
            && let Some(value) = value
        {
            sum += value;
            samples += 1;
        }
    }
    let value = sum / f64::from(samples);
    (samples > 0 && value.is_finite() && value > 0.0).then_some(ChartAverage { value, samples })
}

// --------------------------------------------------------- staking difficulty

pub(super) fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}

/// A 256-bit hex number as little-endian bytes. Ported from Carbide's `chart_ui::hex_target`.
fn hex_target(hex: &str) -> Option<[u8; 32]> {
    if !valid_hash(hex) {
        return None;
    }
    let mut bytes = [0; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[62 - i * 2..64 - i * 2], 16).ok()?;
    }
    bytes.iter().any(|b| *b != 0).then_some(bytes)
}

/// Bitcoin's compact target expansion, in integers. Ported from `chart_ui::expand_compact`.
pub(super) fn expand_compact(compact: u32) -> Option<[u8; 32]> {
    let size = (compact >> 24) as usize;
    let mut word = compact & 0x007f_ffff;
    if compact & 0x0080_0000 != 0 || word == 0 {
        return None;
    }
    let mut bytes = [0; 32];
    if size <= 3 {
        word >>= 8 * (3 - size);
        bytes[..4].copy_from_slice(&word.to_le_bytes());
    } else {
        for (i, byte) in word.to_le_bytes()[..3].iter().enumerate() {
            let index = size - 3 + i;
            if index >= 32 {
                if *byte != 0 {
                    return None;
                }
            } else {
                bytes[index] = *byte;
            }
        }
    }
    bytes.iter().any(|b| *b != 0).then_some(bytes)
}

fn target_float(bytes: &[u8; 32]) -> f64 {
    bytes
        .iter()
        .rev()
        .fold(0.0, |value, byte| value * 256.0 + f64::from(*byte))
}

/// Staking difficulty at a proof-of-stake block: the fixed initial reference limit over the
/// block's target. The compact target is the nonce's last four bytes, and must equal
/// `postarget` when the node reports it. Ported from Carbide's `chart_ui::stake_difficulty`.
pub(super) fn stake_difficulty(raw: &Value) -> Result<(f64, String)> {
    let nonce = raw["nonce"].as_str().context("PoS nonce unavailable")?;
    anyhow::ensure!(valid_hash(nonce), "Invalid PoS nonce");
    let compact = u32::from_str_radix(&nonce[56..], 16)?;
    let target = expand_compact(compact).context("Invalid compact PoS target")?;
    if let Some(value) = raw.get("postarget") {
        let hex = value.as_str().context("Invalid postarget representation")?;
        anyhow::ensure!(hex_target(hex) == Some(target), "Nonce and postarget disagree");
    }
    let reference = hex_target(POS_REFERENCE).expect("fixed Verus PoS reference");
    let value = target_float(&reference) / target_float(&target);
    anyhow::ensure!(value.is_finite() && value > 0.0, "Invalid staking difficulty");
    let hex = target.iter().rev().map(|b| format!("{b:02x}")).collect();
    Ok((value, hex))
}

// ------------------------------------------------------------------- seek

/// Reads a seek target. Ported from Carbide's `chart_ui::seek::parse`: a bare number is a
/// height up to the tip and Unix seconds after it; `height:`, `#` and `unix:` remove the
/// doubt. Dates follow Carbide's `block_time` parser, in UTC unless an offset is given.
#[uniffi::export]
pub fn chart_seek_parse(input: String, now: i64) -> Outcome<SeekTarget> {
    let now = DateTime::<Utc>::from_timestamp(now, 0).ok_or_else(|| fail("The clock is out of range."))?;
    Ok(parse_seek(&input, now)?)
}

pub(super) fn parse_seek(input: &str, now: DateTime<Utc>) -> Result<SeekTarget> {
    let input = input.trim();
    anyhow::ensure!(!input.is_empty(), "Enter a block height, Unix seconds, or date/time");
    if let Some(height) = input.strip_prefix("height:").or_else(|| input.strip_prefix('#')) {
        return Ok(SeekTarget::Height {
            value: height.trim().parse().context("Enter a nonnegative block height")?,
        });
    }
    if let Some(time) = input.strip_prefix("unix:") {
        return Ok(SeekTarget::Time {
            unix: crate::block_time::parse_at(time, now)?.timestamp(),
        });
    }
    if input.bytes().all(|byte| byte.is_ascii_digit()) {
        let number = input.parse::<u64>().context("Number is out of range")?;
        anyhow::ensure!(
            number <= 99_999_999_999,
            "Use Unix seconds, or height: followed by a block height"
        );
        return Ok(SeekTarget::Number { value: number });
    }
    Ok(SeekTarget::Time {
        unix: crate::block_time::parse_at(input, now)?.timestamp(),
    })
}

/// A plain description of what a seek input means, shown before the user submits it.
/// Ported from the preview in Carbide's `chart_ui::seek::render`.
#[uniffi::export]
pub fn chart_seek_preview(input: String, now: i64, tip: Option<u64>) -> String {
    let Some(now) = DateTime::<Utc>::from_timestamp(now, 0) else {
        return "The clock is out of range.".into();
    };
    if input.trim().is_empty() {
        return "Enter a block height, Unix seconds, or a date and time (UTC by default).".into();
    }
    match parse_seek(&input, now) {
        Ok(SeekTarget::Height { value }) => format!("Block {value}"),
        Ok(SeekTarget::Number { value }) if tip.is_some_and(|tip| value <= tip) => {
            format!("Block {value}")
        }
        Ok(SeekTarget::Number { value }) => i64::try_from(value)
            .ok()
            .and_then(|time| DateTime::<Utc>::from_timestamp(time, 0))
            .map_or_else(
                || "The number is out of range.".into(),
                |time| {
                    format!(
                        "Unix time {}. The app checks the range when you submit.",
                        time.to_rfc3339()
                    )
                },
            ),
        Ok(SeekTarget::Time { unix }) => DateTime::<Utc>::from_timestamp(unix, 0)
            .map_or_else(|| "The time is out of range.".into(), |time| format!("Date and time {}", time.to_rfc3339())),
        Ok(SeekTarget::Latest) => "The latest block".into(),
        Err(error) => error.to_string(),
    }
}

/// Block times from one pinned source. Carbide's `block_time::RpcTimeSource` reads
/// `getblockheader` with a verbosity argument, which public APIs refuse, so this source
/// reads `getblock` without one. The checks are those of `RpcTimeSource`.
pub(super) struct SourceTime {
    client: RpcClient,
    source: String,
}

impl SourceTime {
    /// Opens a source and reads its tip. Returns the source, the tip and the node's
    /// `getblockchaininfo`.
    async fn open(client: &RpcClient) -> Result<(Self, crate::block_time::TimeBlock, Value)> {
        use crate::block_time::TimeSource;
        let info = client.get_blockchain_info().await?;
        let height = info.result["blocks"].as_u64().context("Source omitted tip height")?;
        let hash = info.result["bestblockhash"]
            .as_str()
            .filter(|hash| valid_hash(hash))
            .context("Source omitted tip hash")?
            .to_string();
        let mut source = Self {
            client: client.clone(),
            source: info.source_id,
        };
        let tip = source.block_at(height).await?;
        anyhow::ensure!(tip.hash == hash, "Chain changed while locating tip; retry");
        Ok((source, tip, info.result))
    }

    /// Checks that a block is still at its height on the same source.
    async fn check(&self, block: &crate::block_time::TimeBlock) -> Result<()> {
        let checked = self
            .client
            .call_with_raw::<String>("getblockhash", &[Value::from(block.height)])
            .await?;
        anyhow::ensure!(
            checked.source_id == self.source && checked.result == block.hash,
            "Chain or source changed during time lookup; retry"
        );
        Ok(())
    }
}

impl crate::block_time::TimeSource for SourceTime {
    async fn block_at(&mut self, height: u64) -> Result<crate::block_time::TimeBlock> {
        let response = self
            .client
            .call_with_raw::<Value>("getblock", &[Value::from(height.to_string())])
            .await?;
        anyhow::ensure!(response.source_id == self.source, "Source changed during time lookup; retry");
        time_header(&response.result, height)
    }
}

/// A block's time fields, checked as Carbide's `block_time::decode` checks them.
pub(super) fn time_header(raw: &Value, height: u64) -> Result<crate::block_time::TimeBlock> {
    anyhow::ensure!(
        raw["height"].as_u64() == Some(height),
        "Header does not match requested block"
    );
    let hash = raw["hash"]
        .as_str()
        .filter(|hash| valid_hash(hash))
        .context("Header does not match requested block")?;
    let parent = raw["previousblockhash"].as_str().map(str::to_string);
    anyhow::ensure!(
        height == 0 || parent.as_deref().is_some_and(valid_hash),
        "Header parent is unavailable"
    );
    Ok(crate::block_time::TimeBlock {
        height,
        hash: hash.to_string(),
        parent,
        timestamp: raw["time"]
            .as_i64()
            .filter(|time| *time >= 0)
            .context("Header timestamp is unavailable")?,
    })
}

/// Carbide's calibration (`block_forecast::from_source`) on a `SourceTime`. The checks,
/// the start heights and the rate are Carbide's: `start_height`, `Calibration::new`.
async fn calibrate(config: &Config, client: &RpcClient) -> Result<crate::block_forecast::Calibration> {
    use crate::block_time::TimeSource;
    let (mut source, tip, info) = SourceTime::open(client).await?;
    anyhow::ensure!(
        info["initialblockdownload"].as_bool() != Some(true)
            && info["headers"].as_u64().is_none_or(|headers| headers <= tip.height),
        "Source is catching up; wait for a current tip before estimating"
    );
    let system = config.chain_cache_key();
    let first = crate::block_forecast::start_height(&system, config.chain.testnet, tip.height)?;
    let start = source.block_at(first).await?;
    source.check(&start).await?;
    source.check(&tip).await?;
    // Carbide's `RpcTimeSource::forecast_ready`.
    let latest = client.get_info().await?;
    anyhow::ensure!(latest.source_id == source.source, "Source changed while calibrating; retry");
    let blocks = latest.result["blocks"].as_u64().unwrap_or(tip.height);
    anyhow::ensure!(
        latest.result["longestchain"].as_u64().is_none_or(|h| h <= blocks),
        "Source is catching up; wait for a current tip before estimating"
    );
    anyhow::ensure!(blocks == tip.height, "Tip advanced during calibration; retry the estimate");
    let mut calibration = crate::block_forecast::Calibration::new(
        start,
        tip,
        Utc::now().timestamp(),
        source.source.clone(),
        system,
    )?;
    calibration.source_label = source_label(config, &source.source);
    Ok(calibration)
}

/// The source as Carbide labels it: host and port of a node, or the origin of an API,
/// never credentials, paths or tokens. Ported from `block_forecast::connection_label`.
pub(super) fn source_label(config: &Config, source: &str) -> String {
    let local = format!("http://{}:{}", config.daemon.host, config.daemon.port);
    if crate::rpc::source_id("local-daemon", &local) == source {
        return endpoint_label("Local daemon", &local);
    }
    if let Some(endpoint) = config
        .daemon
        .api_pool
        .endpoints
        .iter()
        .find(|endpoint| crate::rpc::source_id("public-api", &endpoint.url) == source)
    {
        return endpoint_label("API", &endpoint.url);
    }
    if source.starts_with("local-daemon:") {
        "Local daemon".into()
    } else if source.starts_with("public-api:") {
        "API".into()
    } else {
        "Unknown source".into()
    }
}

/// Ported from Carbide's `block_forecast::endpoint_label`.
pub(super) fn endpoint_label(kind: &str, endpoint: &str) -> String {
    let Ok(url) = reqwest::Url::parse(endpoint) else {
        return kind.into();
    };
    if kind == "Local daemon" {
        let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
            return kind.into();
        };
        let host = if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]")
        } else {
            host.into()
        };
        format!("{kind} — {host}:{port}")
    } else {
        format!("{kind} — {}", url.origin().ascii_serialization())
    }
}

async fn seek(client: &RpcClient, target: SeekTarget, span_seconds: u64) -> Result<SeekOutcome> {
    use crate::block_time::TimeSource;
    let (mut source, tip, _) = SourceTime::open(client).await?;
    let target = match target {
        SeekTarget::Number { value } if value <= tip.height => SeekTarget::Height { value },
        SeekTarget::Number { value } => {
            let genesis = source.block_at(0).await?;
            let time = i64::try_from(value).context("Number is out of range")?;
            anyhow::ensure!(
                time >= genesis.timestamp,
                "Enter a height from 0 to {} or Unix seconds from {} onward",
                tip.height,
                genesis.timestamp
            );
            SeekTarget::Time { unix: time }
        }
        target => target,
    };
    let future = match target {
        SeekTarget::Height { value } => value > tip.height,
        SeekTarget::Time { unix } => unix > tip.timestamp,
        _ => false,
    };
    if future {
        return Ok(SeekOutcome {
            block: time_block(&tip),
            end: time_block(&tip),
            tip: tip.height,
            note: "No block exists yet for this target. You can show the latest block instead.".into(),
            future: true,
        });
    }
    let (block, note) = match target {
        SeekTarget::Height { value } => (source.block_at(value).await?, format!("Seek to block {value}")),
        SeekTarget::Latest => (tip.clone(), format!("Seek to the latest block, {}", tip.height)),
        SeekTarget::Time { unix } => {
            let found = crate::block_time::locate(&mut source, unix, tip.clone()).await?;
            let note = seek_note(unix, found.block.timestamp, &found.description());
            (found.block, note)
        }
        SeekTarget::Number { .. } => unreachable!("numbers are resolved above"),
    };
    let end_time = block.timestamp.saturating_add((span_seconds / 2) as i64);
    let end = if end_time >= tip.timestamp {
        tip.clone()
    } else {
        let found = crate::block_time::locate(&mut source, end_time, tip.clone()).await?;
        if found.block.height >= block.height {
            found.block
        } else {
            block.clone()
        }
    };
    for checked in [&tip, &block, &end] {
        source.check(checked).await?;
    }
    Ok(SeekOutcome {
        block: time_block(&block),
        end: time_block(&end),
        tip: tip.height,
        note,
        future: false,
    })
}

pub(super) fn seek_note(requested: i64, matched: i64, description: &str) -> String {
    let format = |time: i64| {
        DateTime::<Utc>::from_timestamp(time, 0).map_or_else(|| time.to_string(), |time| time.to_rfc3339())
    };
    format!(
        "Requested {}. Matched {}, a difference of {:+} s. {description}",
        format(requested),
        format(matched),
        matched - requested
    )
}

fn time_block(block: &crate::block_time::TimeBlock) -> ChartBlock {
    ChartBlock {
        height: block.height,
        hash: block.hash.clone(),
        time: block.timestamp.max(0) as u64,
    }
}

// --------------------------------------------------------------- fee pool

/// The fee pool amounts in a coinbase: native, then reserve, in satoshis. `Ok(None)` when
/// the coinbase has no checkpoint, such as before activation. A valid checkpoint may omit a
/// currency to mean zero. Ported from Carbide's `chart_ui::fee_pool::checkpoint`.
pub(super) fn fee_pool_amounts(tx: &Value, native: &str, reserve: Option<&str>) -> Result<Option<[i128; 2]>> {
    anyhow::ensure!(
        tx["vin"]
            .as_array()
            .is_some_and(|vin| vin.len() == 1 && vin[0].get("coinbase").is_some()),
        "Expected a coinbase transaction"
    );
    let outputs = tx["vout"].as_array().context("Coinbase outputs unavailable")?;
    let pools: Vec<&Value> = outputs
        .iter()
        .filter_map(|out| out["scriptPubKey"].get("feepool"))
        .collect();
    if pools.is_empty() {
        return Ok(None);
    }
    anyhow::ensure!(pools.len() == 1, "Ambiguous fee-pool checkpoints");
    let pool = pools[0];
    anyhow::ensure!(pool["version"].as_u64() == Some(1), "Unsupported fee-pool version");
    let entries = pool["currencyvalues"]
        .as_object()
        .context("Fee-pool amounts unavailable")?;
    let mut result = [0; 2];
    for (id, value) in entries {
        let value = decimal_sats(value)
            .filter(|v| (0..=i128::from(i64::MAX)).contains(v))
            .context("Invalid fee-pool amount")?;
        anyhow::ensure!(
            id == native || Some(id.as_str()) == reserve || value == 0,
            "Unexpected fee-pool currency"
        );
        if id == native {
            result[0] = value;
        } else if Some(id.as_str()) == reserve {
            result[1] = value;
        }
    }
    Ok(Some(result))
}

/// A JSON decimal as exact satoshis, without a floating-point product. `None` for a value
/// that is negative or finer than one satoshi. Ported from Carbide's
/// `defi::replay::amount_sats`, which is private to its module.
pub(super) fn decimal_sats(value: &Value) -> Option<i128> {
    let raw = value.as_number()?.to_string();
    let (mantissa, exponent) = match raw.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, exponent.parse::<i32>().ok()?),
        None => (raw.as_str(), 0),
    };
    let (whole, fractional) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fractional}").parse::<i128>().ok()?;
    let scale = 8_i32.checked_add(exponent)?.checked_sub(fractional.len() as i32)?;
    let sats = if scale >= 0 {
        digits.checked_mul(10_i128.checked_pow(scale as u32)?)?
    } else {
        let divisor = 10_i128.checked_pow(scale.unsigned_abs())?;
        (digits % divisor == 0).then_some(digits / divisor)?
    };
    (sats >= 0).then_some(sats)
}

async fn fee_pool_at(
    client: &RpcClient,
    height: u64,
    hash: &str,
    currencies: &FeePoolCurrencies,
) -> Result<FeePoolCheckpoint> {
    let block = fetch_block(client, BlockId::Hash(hash.to_string())).await?.result;
    anyhow::ensure!(
        block.hash == hash && block.height == height,
        "Fee-pool block mismatch"
    );
    let txid = block
        .tx
        .first()
        .map(|tx| tx.txid.clone())
        .filter(|id| valid_hash(id))
        .context("Coinbase reference unavailable")?;
    let tx = client.get_raw_transaction(&txid).await?.result;
    anyhow::ensure!(
        tx["txid"].as_str() == Some(txid.as_str()) && tx["blockhash"].as_str() == Some(hash),
        "Fee-pool coinbase/block mismatch"
    );
    let amounts = fee_pool_amounts(&tx, &currencies.native_id, currencies.reserve_id.as_deref())?;
    Ok(FeePoolCheckpoint {
        height,
        hash: hash.to_string(),
        time: block.time,
        known: amounts.is_some(),
        native_sats: amounts.map(|a| a[0] as i64),
        reserve_sats: amounts.and_then(|a| currencies.reserve_id.as_ref().map(|_| a[1] as i64)),
        problem: None,
    })
}

/// The order in which to read fee pool checkpoints for `count` samples: the endpoints
/// first, then the selected sample, then finer midpoint passes in random order within each
/// pass. At most `limit` indices, on a grid that stays stable across paging, with the
/// selected sample put in place of its nearest interior grid point. Ported from Carbide's
/// `chart_ui::fee_pool::{indices, fetch_order}`.
#[uniffi::export]
pub fn fee_pool_order(count: u32, selected: Option<u32>, detail: bool) -> Vec<u32> {
    let limit = if detail { FEE_POOL_DETAIL_POINTS } else { FEE_POOL_POINTS };
    let selected = selected.map(|s| s as usize);
    let grid = fee_pool_indices(count as usize, selected, limit as usize);
    fetch_order(&grid, selected, &mut rand::rng())
        .into_iter()
        .map(|index| index as u32)
        .collect()
}

pub(super) fn fee_pool_indices(count: usize, selected: Option<usize>, limit: usize) -> Vec<usize> {
    if count <= limit {
        return (0..count).collect();
    }
    let mut result: Vec<usize> = (0..limit).map(|i| i * (count - 1) / (limit - 1)).collect();
    if let Some(selected) = selected.filter(|i| *i < count)
        && !result.contains(&selected)
    {
        let nearest = (1..limit - 1)
            .min_by_key(|&i| result[i].abs_diff(selected))
            .expect("the grid has interior points");
        result[nearest] = selected;
        result.sort_unstable();
    }
    result
}

pub(super) fn fetch_order(indices: &[usize], selected: Option<usize>, rng: &mut impl rand::Rng) -> Vec<usize> {
    use rand::seq::SliceRandom;
    if indices.len() < 2 {
        return indices.to_vec();
    }
    let last = indices.len() - 1;
    let mut order = vec![indices[0], indices[last]];
    if let Some(selected) = selected
        && indices[1..last].contains(&selected)
    {
        order.push(selected);
    }
    let mut intervals = vec![(0, last)];
    while !intervals.is_empty() {
        let mut next = Vec::new();
        let start = order.len();
        for (left, right) in intervals {
            if right - left > 1 {
                let middle = left + (right - left) / 2;
                if Some(indices[middle]) != selected {
                    order.push(indices[middle]);
                }
                next.extend([(left, middle), (middle, right)]);
            }
        }
        order[start..].shuffle(rng);
        intervals = next;
    }
    order
}

/// Satoshis as a whole-unit amount with eight decimals. Ported from Carbide's
/// `chart_ui::fee_pool::amount`.
pub(super) fn sats_text(value: i128) -> String {
    format!("{}.{:08}", value / UNIT, value % UNIT)
}

#[cfg(test)]
#[path = "ffi_charts_tests.rs"]
mod tests;
