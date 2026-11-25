//! The dashboard, mempool, peers and other app-wide screens.
//!
//! Part of the bridge's Swift-facing API. See `ffi.rs` for the shared types and helpers.

#[allow(unused_imports)]
use super::*;

use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::geoip::{GeoIpDatabase, GeoIpStatus};

/// The terminal's dashboard keeps this many recent confirmed transactions.
const DASHBOARD_TRANSACTIONS: usize = 100;
/// Individual requests for new mempool entries, per refresh.
const MEMPOOL_CLASSIFY_BATCH: usize = 40;
const MAX_HEIGHT_DATES: usize = 128;
const CALIBRATION_LIFETIME: Duration = Duration::from_secs(600);

// --------------------------------------------------------------- records

/// One transaction in the local node's mempool, as the terminal's list shows it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MempoolEntry {
    pub txid: String,
    pub size: u64,
    pub fee_sats: Option<i64>,
    /// When the node first saw it. `None` when the node did not say.
    pub time: Option<u64>,
    /// The chain height when it entered the mempool.
    pub height: Option<u64>,
    /// Unconfirmed transactions that this one spends from.
    pub parents: Vec<String>,
    /// False until the transaction body is loaded. Tags are empty until then.
    pub classified: bool,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct MempoolSnapshot {
    /// Newest first.
    pub entries: Vec<MempoolEntry>,
    pub total_bytes: u64,
    /// How many entries came with their decoded transaction in this response.
    pub decoded: u32,
    /// The source that answered. The mempool is never served from stored data: an old
    /// mempool says nothing about the mempool now.
    pub provenance: Provenance,
}

/// Tags for a mempool entry loaded by its own request.
#[derive(Debug, Clone, uniffi::Record)]
pub struct MempoolClassification {
    pub txid: String,
    /// False when the node no longer has the transaction or the request failed.
    pub found: bool,
    pub tags: Vec<String>,
}

/// A confirmed transaction from a recent block, for the dashboard.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RecentTransaction {
    pub txid: String,
    pub block_hash: String,
    pub block_height: u64,
    pub block_time: u64,
    /// The transaction's position in its block. The coinbase is 0.
    pub position: u32,
    /// False when the body did not load. Tags and counts are then empty.
    pub loaded: bool,
    pub tags: Vec<String>,
    pub input_count: u32,
    pub output_count: u32,
    pub output_sats: Option<i64>,
}

/// How to show a peer field's value. The raw value is always kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PeerFieldKind {
    Text,
    /// Unix seconds.
    Time,
    Bytes,
    /// A duration in seconds, such as a ping time.
    Seconds,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PeerField {
    pub key: String,
    pub label: String,
    /// The node's value as compact text.
    pub value: String,
    pub kind: PeerFieldKind,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PeerFieldGroup {
    pub title: String,
    pub fields: Vec<PeerField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PeerPlace {
    /// The location database has a position for the address.
    Located,
    /// A private, local or loopback address. It has no public location.
    Private,
    /// A public address that the database did not place, or no database is installed.
    Unresolved,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PeerLocation {
    pub address: String,
    pub place: PeerPlace,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub city: Option<String>,
    pub country: Option<String>,
    pub country_code: Option<String>,
    pub asn: Option<u32>,
    pub organization: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LocationDataState {
    NotInstalled,
    Current,
    UpdateAvailable,
    Unavailable,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PeerLocations {
    pub state: LocationDataState,
    /// The installed release, such as `2026-11`.
    pub release: Option<String>,
    /// Why the data cannot be used, when it cannot.
    pub problem: Option<String>,
    /// Where the location database is kept.
    pub directory: String,
    /// Bytes of location data installed in `directory` now. Zero when none is installed.
    pub installed_bytes: u64,
    /// True when `directory` is in the default cache folder. A cache folder chosen in
    /// Settings moves the location data with it.
    pub default_folder: bool,
    pub locations: Vec<PeerLocation>,
}

/// A point on a coast of the world, in degrees.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct WorldPoint {
    pub longitude: f64,
    pub latitude: f64,
}

/// The grid that the coast points are read from: one dot is a quarter of a degree.
const OUTLINE_COLUMNS: u16 = 720;
const OUTLINE_ROWS: u16 = 180;

/// The coasts of the world as the terminal's peer map draws them: the high resolution
/// world of the terminal's drawing library. The library does not give its points to
/// other code, thus the bridge draws the map on a fine grid and reads the dots back.
fn world_points() -> Vec<WorldPoint> {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui::symbols::Marker;
    use ratatui::widgets::Widget;
    use ratatui::widgets::canvas::{Canvas, Map, MapResolution};

    let area = Rect::new(0, 0, OUTLINE_COLUMNS, OUTLINE_ROWS);
    let mut buffer = Buffer::empty(area);
    Canvas::default()
        .marker(Marker::Braille)
        .x_bounds([-180.0, 180.0])
        .y_bounds([-90.0, 90.0])
        .paint(|context| {
            context.draw(&Map {
                resolution: MapResolution::High,
                color: Color::White,
            });
        })
        .render(area, &mut buffer);

    // A Braille cell has 2 columns and 4 rows of dots. The bit of each dot:
    const DOTS: [[u32; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];
    let across = f64::from(OUTLINE_COLUMNS) * 2.0 - 1.0;
    let down = f64::from(OUTLINE_ROWS) * 4.0 - 1.0;
    let mut points = Vec::new();
    for row in 0..OUTLINE_ROWS {
        for column in 0..OUTLINE_COLUMNS {
            let Some(symbol) = buffer[(column, row)].symbol().chars().next() else {
                continue;
            };
            let code = u32::from(symbol);
            if !(0x2801..=0x28FF).contains(&code) {
                continue;
            }
            for (dot_row, bits) in DOTS.iter().enumerate() {
                for (dot_column, bit) in bits.iter().enumerate() {
                    if (code - 0x2800) & bit != 0 {
                        let x = f64::from(column) * 2.0 + dot_column as f64;
                        let y = f64::from(row) * 4.0 + dot_row as f64;
                        points.push(WorldPoint {
                            longitude: -180.0 + x / across * 360.0,
                            latitude: 90.0 - y / down * 180.0,
                        });
                    }
                }
            }
        }
    }
    points
}

static WORLD: std::sync::LazyLock<Vec<WorldPoint>> = std::sync::LazyLock::new(world_points);

/// The coast points of the world, for a map that the app draws without a map service.
#[uniffi::export]
pub fn world_outline() -> Vec<WorldPoint> {
    WORLD.clone()
}

/// One entry in the node's ban list.
#[derive(Debug, Clone, uniffi::Record)]
pub struct BannedPeer {
    pub address: String,
    pub banned_until: Option<i64>,
    pub created: Option<i64>,
    pub reason: Option<String>,
    pub json: String,
}

/// The node's network traffic since it started, read once.
#[derive(Debug, Clone, uniffi::Record)]
pub struct NetworkTotals {
    pub received: u64,
    pub sent: u64,
    /// When the node took the reading, in Unix milliseconds.
    pub time_millis: u64,
}

/// The date of a block height. Future heights are estimated.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HeightDate {
    pub height: u64,
    /// Unix seconds. `None` when the date is not available.
    pub time: Option<i64>,
    pub estimated: bool,
}

/// Start-up arguments in Carbide's form, with `-flag=value` (Verus style) or `--flag value`.
#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct StartArguments {
    /// Set only when the arguments name a chain.
    pub chain: Option<String>,
    pub testnet: bool,
    pub config_file: Option<String>,
    pub data_dir: Option<String>,
    pub rpc_host: Option<String>,
    pub rpc_port: Option<u16>,
    pub rpc_user: Option<String>,
    pub rpc_password: Option<String>,
    pub poll_interval_seconds: Option<u64>,
    /// The object to open: `-open=<object>`, `--open <object>`, `--object` or a bare argument.
    pub open_object: Option<String>,
    /// Why the arguments could not be read. The other fields are then empty.
    pub problem: Option<String>,
}

// ----------------------------------------------------------------- pulse

/// Blocks no more than this many seconds after the block before are one burst.
pub(super) const BURST_SECONDS: u64 = 10;
/// The most blocks one record reads.
const MAX_PULSE_BLOCKS: u32 = 2_000;

/// The chain's own record of how blocks arrive, for the dashboard's chain pulse. Verus
/// blocks arrive in bursts, so the record is of bursts, not of single blocks.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct PulseHistory {
    pub first_height: u64,
    pub last_height: u64,
    /// The blocks that the record holds.
    pub blocks: u32,
    /// Timestamp of the last block minus timestamp of the first.
    pub covered_seconds: u64,
    /// Blocks within this many seconds of the block before are one burst.
    pub burst_seconds: u32,
    /// The waits between bursts in seconds, shortest first.
    pub waits: Vec<u32>,
    /// The length of one counting window.
    pub window_seconds: u32,
    /// The number of blocks in each window, fewest first. The windows overlap: each
    /// starts one fifth of a window after the one before.
    pub window_counts: Vec<u32>,
}

/// The waits between bursts, shortest first. `times` are block timestamps in chain order.
/// A wait runs from the last block of one burst to the first block of the next. A
/// timestamp earlier than the block before stays in the same burst.
pub(super) fn waits_between_bursts(times: &[u64], burst: u64) -> Vec<u32> {
    let mut waits: Vec<u32> = times
        .windows(2)
        .filter_map(|pair| pair[1].checked_sub(pair[0]))
        .filter(|gap| *gap > burst)
        .map(|gap| gap.min(u64::from(u32::MAX)) as u32)
        .collect();
    waits.sort_unstable();
    waits
}

/// The number of blocks in each window of `window` seconds, fewest first. Each window
/// starts `window / 5` seconds after the one before. Only windows that the record covers
/// completely are counted.
pub(super) fn window_counts(times: &[u64], window: u64) -> Vec<u32> {
    let (Some(first), Some(last)) = (times.iter().min().copied(), times.iter().max().copied())
    else {
        return Vec::new();
    };
    let step = (window / 5).max(1);
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    let mut counts = Vec::new();
    let mut start = first;
    while window > 0 && start.saturating_add(window) <= last {
        let from = sorted.partition_point(|time| *time < start);
        let to = sorted.partition_point(|time| *time < start + window);
        counts.push((to - from) as u32);
        start += step;
    }
    counts.sort_unstable();
    counts
}

// -------------------------------------------------------------- explorer

#[uniffi::export(async_runtime = "tokio")]
impl Explorer {
    /// How blocks arrived in the `count` blocks that end at `tip`. A bulk read: it never
    /// asks for permission and adds nothing to the cache.
    pub async fn pulse_history(
        &self,
        tip: u64,
        count: u32,
        window_seconds: u32,
    ) -> Outcome<PulseHistory> {
        let count = u64::from(count.clamp(2, MAX_PULSE_BLOCKS));
        let first = tip.saturating_sub(count - 1).max(1);
        let client = self.unrecorded_client();
        let mut tasks = tokio::task::JoinSet::new();
        for height in first..=tip {
            let client = client.clone();
            tasks.spawn(async move {
                (height, fetch_block(&client, BlockId::Height(height)).await)
            });
        }
        let mut found = Vec::new();
        let mut failure = None;
        while let Some(joined) = tasks.join_next().await {
            let (height, block) = joined.context("block request stopped")?;
            match self.settle(block) {
                Ok(block) => found.push((height, block.result.time)),
                Err(error) => failure = Some(error),
            }
        }
        if found.len() < 2 {
            return Err(failure
                .unwrap_or_else(|| anyhow::anyhow!("The source returned no blocks."))
                .into());
        }
        found.sort_unstable();
        let times: Vec<u64> = found.iter().map(|(_, time)| *time).collect();
        Ok(PulseHistory {
            first_height: found[0].0,
            last_height: found[found.len() - 1].0,
            blocks: found.len() as u32,
            covered_seconds: times[times.len() - 1].saturating_sub(times[0]),
            burst_seconds: BURST_SECONDS as u32,
            waits: waits_between_bursts(&times, BURST_SECONDS),
            window_seconds,
            window_counts: window_counts(&times, u64::from(window_seconds)),
        })
    }

    /// The mempool with the terminal's loading pattern. With `full`, the node is asked for
    /// decoded transactions in bulk; an older node that refuses gets the plain request.
    /// Without `full`, only membership and metadata are read.
    pub async fn mempool_snapshot(&self, full: bool) -> Outcome<MempoolSnapshot> {
        let output = self.settle(self.client.get_raw_mempool_verbose(full).await)?;
        let mut snapshot = mempool_snapshot_from(output.result);
        snapshot.provenance = self.provenance(&output.source_id, None);
        Ok(snapshot)
    }

    /// Tags for mempool entries that arrived after the bulk load, one request each.
    pub async fn classify_mempool(&self, txids: Vec<String>) -> Vec<MempoolClassification> {
        let mut tasks = tokio::task::JoinSet::new();
        for txid in txids.into_iter().take(MEMPOOL_CLASSIFY_BATCH) {
            let client = self.client.clone();
            tasks.spawn(async move {
                let body = client.get_raw_transaction(&txid).await;
                (txid, body)
            });
        }
        let mut found = Vec::new();
        while let Some(Ok((txid, body))) = tasks.join_next().await {
            let body = self.settle(body).ok().map(|output| output.result);
            found.push(MempoolClassification {
                tags: body.as_ref().map_or_else(Vec::new, |raw| {
                    labels(&TransactionSummary {
                        txid: txid.clone(),
                        raw: Some(raw.clone()),
                    })
                }),
                found: body.is_some(),
                txid,
            });
        }
        found
    }

    /// Confirmed transactions in the given blocks, newest block first and, within a block,
    /// last position first, as the terminal's dashboard lists them. At most `limit` (100).
    pub async fn recent_transactions(
        &self,
        block_hashes: Vec<String>,
        limit: u32,
    ) -> Outcome<Vec<RecentTransaction>> {
        let mut tasks = tokio::task::JoinSet::new();
        for (index, hash) in block_hashes
            .into_iter()
            .take(MAX_RECENT_BLOCKS as usize)
            .enumerate()
        {
            let client = self.client.clone();
            tasks.spawn(async move {
                let block = fetch_block(&client, BlockId::Hash(hash.clone())).await;
                (index, hash, block)
            });
        }
        let mut blocks = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            let (index, hash, block) = joined.context("block request stopped")?;
            // A block named by hash does not change, so a stored observation is sound.
            let params = [block_selector(&BlockId::Hash(hash))];
            blocks.push((
                index,
                self.observed("getblock", &params, block).await?.result,
            ));
        }
        blocks.sort_by_key(|(index, _)| *index);
        let blocks: Vec<BlockSummary> = blocks.into_iter().map(|(_, block)| block).collect();
        let mut rows =
            dashboard_transactions(&blocks, (limit as usize).min(DASHBOARD_TRANSACTIONS));

        let mut tasks = tokio::task::JoinSet::new();
        for (index, row) in rows.iter().enumerate() {
            let client = self.client.clone();
            let txid = row.txid.clone();
            tasks.spawn(async move { (index, client.get_raw_transaction(&txid).await) });
        }
        while let Some(Ok((index, body))) = tasks.join_next().await {
            let params = [Value::from(rows[index].txid.as_str()), Value::from(1)];
            if let Ok(body) = self
                .observed::<Value>("getrawtransaction", &params, body)
                .await
            {
                let raw = body.result;
                let row = &mut rows[index];
                row.tags = labels(&TransactionSummary {
                    txid: row.txid.clone(),
                    raw: Some(raw.clone()),
                });
                row.input_count = count(Some(&raw["vin"]));
                row.output_count = count(Some(&raw["vout"]));
                row.output_sats = raw["vout"]
                    .as_array()
                    .map(|outputs| outputs.iter().filter_map(sats).sum());
                row.loaded = true;
            }
        }
        Ok(rows)
    }

    /// The hash of the block before or after a block, by the node's own links. `id` is a
    /// height or a hash. `None` at the first block and at the tip.
    pub async fn adjacent_block(&self, id: String, next: bool) -> Outcome<Option<String>> {
        let selector = match id.trim().parse::<u64>() {
            Ok(height) => BlockId::Height(height),
            Err(_) => BlockId::Hash(id.trim().to_string()),
        };
        let block = self
            .settle(fetch_block(&self.client, selector).await)?
            .result;
        Ok(if next {
            block.nextblockhash
        } else {
            block.previousblockhash
        })
    }

    /// The node's ban list. Local nodes only.
    pub async fn ban_list(&self) -> Outcome<Vec<BannedPeer>> {
        let list = self.client.call_local("listbanned", &[]).await?;
        Ok(banned_peers_from(&list))
    }

    /// The node's network traffic totals. The terminal reads these once per visit.
    pub async fn network_totals(&self) -> Outcome<NetworkTotals> {
        let value = self.client.call_local("getnettotals", &[]).await?;
        Ok(NetworkTotals {
            received: value["totalbytesrecv"].as_u64().unwrap_or(0),
            sent: value["totalbytessent"].as_u64().unwrap_or(0),
            time_millis: value["timemillis"].as_u64().unwrap_or(0),
        })
    }

    /// Where each peer is, from Carbide's location database (DB-IP City Lite and ASN Lite).
    /// Reads local files only. It never downloads anything.
    pub async fn peer_locations(&self, addresses: Vec<String>) -> PeerLocations {
        let directory = crate::geoip::database_directory(&self.config.cache_path);
        // Carbide's own default, from `default_cache_path` in `src/config.rs` (private there).
        let default_folder = directories::BaseDirs::new().is_some_and(|dirs| {
            self.config.cache_path
                == dirs
                    .cache_dir()
                    .join("verus-carbide")
                    .join(Network::from_testnet(self.config.chain.testnet).database_name())
        });
        tokio::task::spawn_blocking(move || {
            let mut found = locate_peers(&directory, &addresses);
            found.default_folder = default_folder;
            found
        })
        .await
        .unwrap_or_else(|error| PeerLocations {
            state: LocationDataState::Unavailable,
            release: None,
            problem: Some(error.to_string()),
            directory: String::new(),
            installed_bytes: 0,
            default_folder: true,
            locations: Vec::new(),
        })
    }

    /// Downloads this month's DB-IP City Lite and ASN Lite databases from db-ip.com and
    /// installs them beside Carbide's cache. Only an explicit request from the person calls
    /// this. Returns the installed release.
    pub async fn install_location_database(&self) -> Outcome<String> {
        let directory = crate::geoip::database_directory(&self.config.cache_path);
        let release = crate::geoip::download(directory).await?;
        *LOCATION_DATABASE.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(release)
    }

    /// Dates for block heights. Past heights use the block's own time. Heights above the tip
    /// are estimated with Carbide's calibrated rate. Heights without a date are returned with
    /// no time. At most 128 heights per request.
    pub async fn height_dates(&self, heights: Vec<u64>) -> Vec<HeightDate> {
        let mut heights: Vec<u64> = heights
            .into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        heights.sort_unstable();
        heights.truncate(MAX_HEIGHT_DATES);
        let Ok(tip) = self.tip_height().await else {
            return heights
                .into_iter()
                .map(|height| HeightDate {
                    height,
                    time: None,
                    estimated: false,
                })
                .collect();
        };
        let mut dates = Vec::new();
        let future: Vec<u64> = heights
            .iter()
            .copied()
            .filter(|height| *height > tip)
            .collect();
        if !future.is_empty() {
            let calibration = self.calibration().await;
            for height in future {
                let time = calibration
                    .as_ref()
                    .and_then(|calibration| calibration.time_at_height(height).ok());
                dates.push(HeightDate {
                    height,
                    time,
                    estimated: true,
                });
            }
        }
        let mut tasks = tokio::task::JoinSet::new();
        for height in heights.into_iter().filter(|height| *height <= tip) {
            let client = self.client.clone();
            tasks.spawn(
                async move { (height, fetch_block(&client, BlockId::Height(height)).await) },
            );
        }
        while let Some(Ok((height, block))) = tasks.join_next().await {
            let time = self
                .settle(block)
                .ok()
                .map(|output| output.result.time as i64);
            dates.push(HeightDate {
                height,
                time,
                estimated: false,
            });
        }
        dates.sort_by_key(|date| date.height);
        dates
    }
}

impl Explorer {
    /// Carbide's calibrated block rate, kept for ten minutes per chain.
    async fn calibration(&self) -> Option<crate::block_forecast::Calibration> {
        let key = self.config.chain_cache_key();
        if let Some((taken, calibration)) = CALIBRATIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            && taken.elapsed() < CALIBRATION_LIFETIME
        {
            return Some(calibration.clone());
        }
        let calibration = crate::block_forecast::calibrate(&self.config).await;
        self.forgive();
        let calibration = calibration.ok()?;
        CALIBRATIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, (Instant::now(), calibration.clone()));
        Some(calibration)
    }

    /// A 64-digit hash names a block or a transaction. As in the terminal's `resolve_hash`,
    /// both are tried quietly first, because one of the two misses is expected. Only when no
    /// source answered does the search make its explicit request, which can ask the person
    /// for API permission and which opens a stored observation when nothing answers.
    async fn resolve_hash(&self, hash: String) -> Outcome<Route> {
        const MISSING: &str = "No block or transaction has this hash.";
        let block = self.settle(fetch_block(&self.client, BlockId::Hash(hash.clone())).await);
        let transaction = self.settle(self.client.get_raw_transaction(&hash).await);
        match (block, transaction) {
            (Ok(_), _) => return Ok(Route::Block { id: hash }),
            (_, Ok(_)) => return Ok(Route::Transaction { txid: hash }),
            (Err(block), Err(transaction))
                if source_answered(&block) && source_answered(&transaction) =>
            {
                return Err(fail(MISSING));
            }
            _ => {}
        }
        let transaction_params = [Value::from(hash.as_str()), Value::from(1)];
        let live = self.primary.get_raw_transaction(&hash).await;
        let transaction_error = match self
            .observed::<Value>("getrawtransaction", &transaction_params, live)
            .await
        {
            Ok(_) => return Ok(Route::Transaction { txid: hash }),
            Err(error) => error,
        };
        let block_params = [block_selector(&BlockId::Hash(hash.clone()))];
        if source_answered(&transaction_error) {
            // A source answers now, so the block request does not ask again.
            let live = fetch_block(&self.primary, BlockId::Hash(hash.clone())).await;
            return match self
                .observed::<BlockSummary>("getblock", &block_params, live)
                .await
            {
                Ok(_) => Ok(Route::Block { id: hash }),
                Err(error) if source_answered(&error) => Err(fail(MISSING)),
                Err(error) => Err(error.into()),
            };
        }
        // No source answered, and the person was asked once. A stored block can still open.
        match self
            .retained::<BlockSummary>("getblock", &block_params, Some(&transaction_error))
            .await
        {
            Some(_) => Ok(Route::Block { id: hash }),
            None => Err(transaction_error.into()),
        }
    }

    /// Search, with the terminal's forms: `currencies`, `estimate:`, `time:` and its
    /// treatment of numbers above the tip. A leading `/` (the terminal's search key) is
    /// ignored.
    pub(super) async fn resolve_query(&self, query: &str) -> Outcome<Route> {
        let query = query.trim();
        let query = query.strip_prefix('/').unwrap_or(query).trim();
        let plan = plan_search(query, Utc::now().timestamp())?;
        Ok(match plan {
            SearchPlan::Route(route) => route,
            // The tip comes from the automatic status request, which never asks, and falls
            // back to the stored status. The block page makes the explicit request.
            SearchPlan::Tip => Route::Block {
                id: self.chain_status().await?.blocks.to_string(),
            },
            SearchPlan::Height(height) => {
                // Without a tip, the height opens as a block, as in the terminal.
                let Ok(tip) = self.tip_height().await else {
                    return Ok(Route::Block {
                        id: height.to_string(),
                    });
                };
                if height <= tip {
                    Route::Block {
                        id: height.to_string(),
                    }
                } else {
                    let genesis = self
                        .settle(fetch_block(&self.client, BlockId::Height(0)).await)?
                        .result
                        .time;
                    match above_tip(height, genesis, Utc::now().timestamp()) {
                        AboveTip::FutureHeight => Route::Estimate {
                            query: height.to_string(),
                        },
                        AboveTip::FutureTime => Route::Estimate {
                            query: format!("unix:{height}"),
                        },
                        AboveTip::PastTime => Route::Block {
                            id: self
                                .block_near(&format!("unix:{height}"))
                                .await?
                                .to_string(),
                        },
                    }
                }
            }
            SearchPlan::Time(time) => Route::Block {
                id: self.block_near(&format!("unix:{time}")).await?.to_string(),
            },
            SearchPlan::Hash(hash) => self.resolve_hash(hash).await?,
        })
    }
}

#[uniffi::export]
/// Groups a peer's `getpeerinfo` entry as the terminal's peer overview does: connection,
/// traffic and latency, chain progress, and other fields.
pub fn peer_field_groups(json: String) -> Vec<PeerFieldGroup> {
    serde_json::from_str::<Value>(&json)
        .map(|value| peer_groups(&value))
        .unwrap_or_default()
}

#[uniffi::export]
/// Reads start-up arguments as Carbide's `src/cli.rs` does. `arguments` starts with the
/// program path, as `CommandLine.arguments` does. Arguments that macOS adds (`-psn_…`,
/// `-NS… value`, `-Apple… value`) are ignored.
pub fn start_arguments(arguments: Vec<String>) -> StartArguments {
    use clap::parser::ValueSource;
    use clap::{CommandFactory, FromArgMatches};

    let mut args = vec!["carbide".to_string()];
    args.extend(normalize_arguments(without_system_arguments(
        arguments.into_iter().skip(1).collect(),
    )));
    let problem = |text: String| StartArguments {
        problem: Some(text.trim().to_string()),
        ..Default::default()
    };
    let matches = match Cli::command().try_get_matches_from(&args) {
        Ok(matches) => matches,
        Err(error) => return problem(error.to_string()),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => return problem(error.to_string()),
    };
    let path = |path: &Option<PathBuf>| path.as_ref().map(|path| path.display().to_string());
    StartArguments {
        chain: (matches.value_source("chain") == Some(ValueSource::CommandLine))
            .then(|| cli.chain.clone()),
        testnet: cli.testnet,
        config_file: path(&cli.config_file),
        data_dir: path(&cli.data_dir),
        rpc_host: cli.rpc_host.clone(),
        rpc_port: cli.rpc_port,
        rpc_user: cli.rpc_user.clone(),
        rpc_password: cli.rpc_password.clone(),
        poll_interval_seconds: cli.poll_interval_seconds,
        open_object: cli.requested_object(),
        problem: None,
    }
}

// --------------------------------------------------------------- helpers

static LOCATION_DATABASE: Mutex<Option<(PathBuf, Arc<GeoIpDatabase>)>> = Mutex::new(None);

static CALIBRATIONS: std::sync::LazyLock<
    Mutex<HashMap<String, (Instant, crate::block_forecast::Calibration)>>,
> = std::sync::LazyLock::new(Default::default);

use chrono::Utc;

/// Reads `getrawmempool true` (with or without decoded transactions). Port of the terminal's
/// `parse_mempool_snapshot` in `src/app.rs`: newest first, then by ID.
fn mempool_snapshot_from(values: HashMap<String, Value>) -> MempoolSnapshot {
    let mut total_bytes = 0_u64;
    let mut decoded = 0;
    let mut entries: Vec<MempoolEntry> = values
        .into_iter()
        .map(|(txid, value)| {
            // `fulltxes` puts the transaction under `fulltx`. Accept the fields at the top
            // level as well.
            let body = match value.get("fulltx") {
                Some(raw) => Some(raw),
                None => value.get("vout").map(|_| &value),
            }
            .filter(|raw| raw["vin"].is_array() && raw["vout"].is_array())
            .filter(|raw| raw.get("txid").is_none() || raw["txid"].as_str() == Some(txid.as_str()));
            let tags = body.map_or_else(Vec::new, |raw| {
                labels(&TransactionSummary {
                    txid: txid.clone(),
                    raw: Some(raw.clone()),
                })
            });
            if body.is_some() {
                decoded += 1;
            }
            let size = value["size"].as_u64().unwrap_or(0);
            total_bytes = total_bytes.saturating_add(size);
            MempoolEntry {
                size,
                fee_sats: value["fee"]
                    .as_f64()
                    .map(|coins| (coins * 100_000_000.0).round() as i64),
                time: value["time"].as_u64().filter(|time| *time > 0),
                height: value["height"].as_u64(),
                parents: strings(&value["depends"]),
                classified: body.is_some(),
                tags,
                txid,
            }
        })
        .collect();
    entries.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| a.txid.cmp(&b.txid)));
    MempoolSnapshot {
        entries,
        total_bytes,
        decoded,
        // Set by the caller, which knows the source.
        provenance: Provenance {
            source_id: String::new(),
            source: String::new(),
            kind: SourceKind::Unknown,
            retained: false,
            observed_at: 0,
            observed_height: None,
            failure: None,
            caution: None,
        },
    }
}

/// The terminal's dashboard order (`rebuild_dashboard_transactions` in `src/app.rs`): blocks
/// as given, newest first, each block's transactions from last to first, without repeats.
fn dashboard_transactions(blocks: &[BlockSummary], limit: usize) -> Vec<RecentTransaction> {
    let mut seen = HashSet::new();
    blocks
        .iter()
        .flat_map(|block| {
            block
                .tx
                .iter()
                .enumerate()
                .rev()
                .map(move |(position, tx)| (block, position, tx))
        })
        .filter(|(_, _, tx)| seen.insert(tx.txid.clone()))
        .take(limit)
        .map(|(block, position, tx)| RecentTransaction {
            txid: tx.txid.clone(),
            block_hash: block.hash.clone(),
            block_height: block.height,
            block_time: block.time,
            position: position as u32,
            loaded: false,
            tags: Vec::new(),
            input_count: 0,
            output_count: 0,
            output_sats: None,
        })
        .collect()
}

/// Port of the terminal's `peer_detail_lines` grouping in `src/app.rs`.
fn peer_groups(peer: &Value) -> Vec<PeerFieldGroup> {
    let mut groups: [Vec<PeerField>; 4] = Default::default();
    for (key, value) in peer.as_object().into_iter().flatten() {
        let (group, label, kind) = match key.as_str() {
            "id" | "addr" | "inbound" => continue,
            "addrlocal" => (0, "Local address", PeerFieldKind::Text),
            "conntime" => (0, "Connected since", PeerFieldKind::Time),
            "version" => (0, "Protocol version", PeerFieldKind::Text),
            "subver" => (0, "User agent", PeerFieldKind::Text),
            "services" => (0, "Services", PeerFieldKind::Text),
            "tls_established" => (0, "TLS established", PeerFieldKind::Text),
            "tls_verified" => (0, "TLS verified", PeerFieldKind::Text),
            "whitelisted" => (0, "Whitelisted", PeerFieldKind::Text),
            "bytessent" => (1, "Bytes sent", PeerFieldKind::Bytes),
            "bytesrecv" => (1, "Bytes received", PeerFieldKind::Bytes),
            "lastsend" => (1, "Last send", PeerFieldKind::Time),
            "lastrecv" => (1, "Last receive", PeerFieldKind::Time),
            "pingtime" => (1, "Last ping", PeerFieldKind::Seconds),
            "pingwait" => (1, "Pending ping", PeerFieldKind::Seconds),
            "timeoffset" => (1, "Clock offset (seconds)", PeerFieldKind::Text),
            "startingheight" => (2, "Starting height", PeerFieldKind::Text),
            "synced_headers" => (2, "Synced headers", PeerFieldKind::Text),
            "synced_blocks" => (2, "Synced blocks", PeerFieldKind::Text),
            "inflight" => (2, "Blocks in flight", PeerFieldKind::Text),
            _ => (3, key.as_str(), PeerFieldKind::Text),
        };
        // A zero time means "never" in the node's answer; show the number as it is.
        let kind = if kind == PeerFieldKind::Time && value.as_u64().unwrap_or(0) == 0 {
            PeerFieldKind::Text
        } else {
            kind
        };
        groups[group].push(PeerField {
            key: key.clone(),
            label: label.to_string(),
            value: match value {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            },
            kind,
        });
    }
    [
        "Connection",
        "Traffic and latency",
        "Chain progress",
        "Other fields",
    ]
    .into_iter()
    .zip(groups)
    .filter(|(_, fields)| !fields.is_empty())
    .map(|(title, fields)| PeerFieldGroup {
        title: title.into(),
        fields,
    })
    .collect()
}

fn banned_peers_from(list: &Value) -> Vec<BannedPeer> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(BannedPeer {
                address: text(&entry["address"])?,
                banned_until: entry["banned_until"].as_i64(),
                created: entry["ban_created"].as_i64(),
                reason: text(&entry["ban_reason"]),
                json: pretty(entry),
            })
        })
        .collect()
}

/// The IP address of a peer address such as `203.0.113.18:27485` or `[2001:db8::1]:27485`.
fn peer_address_ip(address: &str) -> Option<IpAddr> {
    peer_ip(address).parse().ok()
}

fn locate_peers(directory: &std::path::Path, addresses: &[String]) -> PeerLocations {
    let status = crate::geoip::status(directory);
    let (state, release, mut problem) = match &status {
        GeoIpStatus::Missing => (LocationDataState::NotInstalled, None, None),
        GeoIpStatus::Current(release) => (LocationDataState::Current, Some(release.clone()), None),
        GeoIpStatus::UpdateAvailable(release) => (
            LocationDataState::UpdateAvailable,
            Some(release.clone()),
            None,
        ),
        GeoIpStatus::Invalid(error) => (LocationDataState::Unavailable, None, Some(error.clone())),
    };
    let mut state = state;
    let database = if matches!(
        status,
        GeoIpStatus::Current(_) | GeoIpStatus::UpdateAvailable(_)
    ) {
        match open_location_database(directory) {
            Ok(database) => Some(database),
            Err(error) => {
                state = LocationDataState::Unavailable;
                problem = Some(format!("{error:#}"));
                None
            }
        }
    } else {
        None
    };
    let locations = addresses
        .iter()
        .map(|address| place_peer(address, database.as_deref()))
        .collect();
    PeerLocations {
        state,
        release,
        problem,
        directory: directory.display().to_string(),
        installed_bytes: installed_bytes(directory),
        default_folder: false,
        locations,
    }
}

/// The size of the location database files in `directory`.
fn installed_bytes(directory: &std::path::Path) -> u64 {
    std::fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("dbip-"))
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

fn open_location_database(directory: &std::path::Path) -> Result<Arc<GeoIpDatabase>> {
    let mut cached = LOCATION_DATABASE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((path, database)) = cached.as_ref()
        && path == directory
    {
        return Ok(database.clone());
    }
    let database = Arc::new(GeoIpDatabase::open(directory)?);
    *cached = Some((directory.to_path_buf(), database.clone()));
    Ok(database)
}

/// Port of the pin kinds in the terminal's `render_peer_map`.
fn place_peer(address: &str, database: Option<&GeoIpDatabase>) -> PeerLocation {
    let ip = peer_address_ip(address);
    let found = ip.and_then(|ip| database.and_then(|database| database.lookup(ip)));
    let place = match (&found, ip) {
        (Some(_), _) => PeerPlace::Located,
        (None, Some(ip)) if !crate::geoip::is_public_ip(ip) => PeerPlace::Private,
        _ => PeerPlace::Unresolved,
    };
    PeerLocation {
        address: address.to_string(),
        place,
        latitude: found.as_ref().map(|geo| geo.latitude),
        longitude: found.as_ref().map(|geo| geo.longitude),
        city: found.as_ref().and_then(|geo| geo.city.clone()),
        country: found.as_ref().and_then(|geo| geo.country.clone()),
        country_code: found.as_ref().and_then(|geo| geo.country_code.clone()),
        asn: found.as_ref().and_then(|geo| geo.asn),
        organization: found.and_then(|geo| geo.organization),
    }
}

/// What a search names before any request is made.
#[derive(Debug, PartialEq)]
enum SearchPlan {
    Route(Route),
    Tip,
    /// A number that is a height up to the tip. Above the tip it can be a time.
    Height(u64),
    /// A past moment, in Unix seconds.
    Time(i64),
    Hash(String),
}

#[derive(Debug, PartialEq)]
enum AboveTip {
    FutureHeight,
    FutureTime,
    PastTime,
}

/// Port of the terminal's `submit_search` and `resolve_search_time` rules in `src/app.rs`.
/// Relative and future targets are frozen at `now`, as the terminal does on submission.
fn plan_search(query: &str, now: i64) -> Outcome<SearchPlan> {
    use crate::block_forecast::{Target, parse_target};
    if query.eq_ignore_ascii_case("currencies") || query == "currency:" {
        return Ok(SearchPlan::Route(Route::Currencies));
    }
    if let Some(name) = crate::resolver::currency_query(query) {
        return Ok(SearchPlan::Route(Route::Currency { name: name.into() }));
    }
    let now_time = chrono::DateTime::from_timestamp(now, 0).unwrap_or_else(Utc::now);
    Ok(match crate::resolver::classify_start_object(Some(query)) {
        ObjectRoute::Tip => SearchPlan::Tip,
        ObjectRoute::Estimate(target) => {
            let frozen = match target.trim().split_once(':') {
                // Relative input is anchored to the moment of the search.
                Some((prefix, value))
                    if ["time", "date", "unix"]
                        .iter()
                        .any(|name| prefix.eq_ignore_ascii_case(name)) =>
                {
                    Target::Time(crate::block_time::parse_at(value, now_time)?.timestamp())
                }
                _ => match target.trim().parse::<u64>() {
                    Ok(height) => Target::Height(height),
                    Err(_) => match parse_target(&target)? {
                        Target::Height(height) => Target::Height(height),
                        Target::Time(_) => Target::Time(
                            crate::block_time::parse_at(&target, now_time)?.timestamp(),
                        ),
                    },
                },
            };
            SearchPlan::Route(Route::Estimate {
                query: match frozen {
                    Target::Height(height) => height.to_string(),
                    Target::Time(time) => format!("unix:{time}"),
                },
            })
        }
        ObjectRoute::BlockHeight(height) => SearchPlan::Height(height),
        ObjectRoute::BlockTime(text) => {
            let time = crate::block_time::parse_at(&text, now_time)?.timestamp();
            if time > now {
                SearchPlan::Route(Route::Estimate {
                    query: format!("unix:{time}"),
                })
            } else {
                SearchPlan::Time(time)
            }
        }
        ObjectRoute::Hash(hash) => SearchPlan::Hash(hash),
        ObjectRoute::Address(address) => SearchPlan::Route(Route::Address { address }),
        ObjectRoute::Identity(name) => SearchPlan::Route(Route::Identity { name }),
        ObjectRoute::Unknown(_) => {
            return Err(fail(
                "The app does not recognize this text. Enter a block height, a hash, a \
                 transaction ID, a date, an address, an identity (name@), a currency (name@@), \
                 currencies, or estimate: and a future height or date.",
            ));
        }
    })
}

/// The terminal's rule for a number above the tip: at or after the genesis time it is Unix
/// seconds, and below that it is a future height.
fn above_tip(number: u64, genesis_time: u64, now: i64) -> AboveTip {
    if number < genesis_time {
        AboveTip::FutureHeight
    } else if number as i64 > now {
        AboveTip::FutureTime
    } else {
        AboveTip::PastTime
    }
}

/// Leaves out the arguments that macOS and Xcode add to an app's command line.
fn without_system_arguments(arguments: Vec<String>) -> Vec<String> {
    let mut kept = Vec::new();
    let mut iter = arguments.into_iter().peekable();
    while let Some(argument) = iter.next() {
        if argument.starts_with("-psn_") {
            continue;
        }
        if argument.starts_with("-NS") || argument.starts_with("-Apple") {
            if iter.peek().is_some_and(|next| !next.starts_with('-')) {
                iter.next();
            }
            continue;
        }
        kept.push(argument);
    }
    kept
}

/// Port of `normalize_verus_style_args` in `src/cli.rs`, which is private there.
fn normalize_arguments(arguments: Vec<String>) -> Vec<String> {
    const FLAGS: [(&str, &str); 12] = [
        ("testnet", "testnet"),
        ("chain", "chain"),
        ("datadir", "datadir"),
        ("conf", "conf"),
        ("rpcconnect", "rpcconnect"),
        ("rpcport", "rpcport"),
        ("rpcuser", "rpcuser"),
        ("rpcpassword", "rpcpassword"),
        ("probe", "probe"),
        ("open", "open"),
        ("pollinterval", "poll-interval"),
        ("poll-interval", "poll-interval"),
    ];
    arguments
        .into_iter()
        .map(|argument| {
            let Some(rest) = argument
                .strip_prefix('-')
                .filter(|rest| !rest.starts_with('-'))
            else {
                return argument;
            };
            let (name, value) = match rest.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (rest, None),
            };
            match FLAGS.iter().find(|(verus, _)| *verus == name) {
                Some((_, long)) => match value {
                    Some(value) => format!("--{long}={value}"),
                    None => format!("--{long}"),
                },
                None => argument,
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "ffi_shell_tests.rs"]
mod tests;
