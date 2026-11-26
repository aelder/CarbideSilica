//! Currencies, the marketplace, notarizations and bookmarks.
//!
//! Part of the bridge's Swift-facing API. See `ffi.rs` for the shared types and helpers.
//!
//! Much of the logic here is the pure part of the terminal's views, ported from
//! `src/app/currency_ui.rs`, `src/app/currency_browser.rs`, `src/app/notarization_ui.rs`,
//! `src/app/bookmark_ui.rs` and the marketplace code in `src/app.rs`. Each port names
//! its source. The core modules (`currency_activity`, `currency_status`, `bookmarks`,
//! `block_time`) are called directly.

#[allow(unused_imports)]
use super::*;

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::json;

use crate::block_time::RpcTimeSource;
use crate::currency_activity::{self, Range};
use crate::currency_status::{self, LaunchStatus};
use crate::world::{OutpointRef, SystemId};

const MAX_MARKET_OFFERS: usize = 5000;
const MAX_FUNDING_LOOKUPS: usize = 64;
const MAX_NOTARIZATION_LOOKUPS: usize = 160;
const LOOKUP_CONCURRENCY: usize = 8;
/// The record of earlier notarizations has at most this many entries.
const MAX_RECORD_POINTS: usize = 60;
const MAX_RECORD_LOOKUPS: usize = 120;
const MAX_RECORD_INPUTS: usize = 6;
const STARTUP_TIMEOUT_SECONDS: u64 = 70;
const STARTUP_LIMIT_CAP: usize = 100;
/// The vETH gateway. A standard Ethereum endpoint cannot answer the Verus header request.
const VETH_SYSTEM_ID: &str = "i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X";

// ======================================================================= records

// ---------------------------------------------------------------- currency browser

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LaunchState {
    Launched,
    Prelaunch,
    /// Failed launch: the currency is refunding.
    Failed,
    Unknown,
}

impl From<LaunchStatus> for LaunchState {
    fn from(status: LaunchStatus) -> Self {
        match status {
            LaunchStatus::Launched => Self::Launched,
            LaunchStatus::Prelaunch => Self::Prelaunch,
            LaunchStatus::Failed => Self::Failed,
            LaunchStatus::Unknown => Self::Unknown,
        }
    }
}

impl LaunchState {
    fn core(self) -> LaunchStatus {
        match self {
            Self::Launched => LaunchStatus::Launched,
            Self::Prelaunch => LaunchStatus::Prelaunch,
            Self::Failed => LaunchStatus::Failed,
            Self::Unknown => LaunchStatus::Unknown,
        }
    }
}

/// One currency in the browser: what the terminal's summary tile shows.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CurrencyListing {
    pub name: String,
    pub currency_id: String,
    /// `None` when the definition has no options field.
    pub options: Option<u64>,
    /// Carbide's option tags, including "Unknown options 0x…" and "No option flags".
    pub option_tags: Vec<String>,
    pub launch: LaunchState,
    pub launch_label: String,
    pub start_block: Option<u64>,
    pub supply: Option<f64>,
    pub reserve_ids: Vec<String>,
    /// The definition's own names for the reserves, in the same order. Empty when unnamed.
    pub reserve_names: Vec<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct CurrencyList {
    pub rows: Vec<CurrencyListing>,
    /// The source that answered, as a Carbide source ID.
    pub source: String,
    pub data_source: DataSource,
    /// Live, or the stored list from the last answer.
    pub provenance: Provenance,
}

/// Option flags and launch states that a currency must match.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CurrencyFilter {
    /// Show: the currency must have each of these option bits.
    pub required: u64,
    /// Hide: the currency must have none of these option bits.
    pub excluded: u64,
    /// Launch states to include, as `launch_state_bit` values.
    pub launch_mask: u8,
    /// Matches names, IDs and option tags, ignoring case.
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FlagState {
    Any,
    Show,
    Hide,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct OptionFlag {
    pub bit: u64,
    pub name: String,
}

// ------------------------------------------------------------- currency state seek

/// How a date or time request found its block.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TimeCoverage {
    /// The requested time, Unix seconds UTC.
    pub target_time: i64,
    pub block_time: i64,
    /// Block time minus requested time.
    pub difference_seconds: i64,
    pub exact: bool,
    /// True when the search checked the whole chain, so the block is the nearest one.
    pub whole_chain: bool,
    pub checked_first: u64,
    pub checked_last: u64,
    pub probes: u32,
    /// Carbide's own one-line description of the result.
    pub description: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct StateReserve {
    pub currency_id: String,
    pub weight: Option<f64>,
    pub reserves: Option<f64>,
    pub price_in_reserve: Option<f64>,
    pub reserve_in: f64,
    pub reserve_out: f64,
    pub conversion_fees: f64,
}

/// A currency's recorded state at one exact block, with where it came from.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CurrencyStateAt {
    pub currency_id: String,
    pub height: u64,
    pub block_hash: String,
    pub block_time: Option<u64>,
    pub source: String,
    pub data_source: DataSource,
    /// The tip when the state was read. `None` for a stored state.
    pub tip_height: Option<u64>,
    /// How the request was read: a block height or a time.
    pub interpreted: String,
    pub coverage: Option<TimeCoverage>,
    pub supply: Option<f64>,
    pub launch_label: String,
    pub reserves: Vec<StateReserve>,
    pub json: String,
    /// Live, or a stored state served because no source answered.
    pub provenance: Provenance,
}

// ---------------------------------------------------------------- currency activity

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ActivityKind {
    Import,
    Export,
}

/// A transaction output on a named system.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct OutputRef {
    pub system_id: String,
    pub txid: String,
    pub output: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ActivityRecord {
    pub height: u64,
    pub kind: ActivityKind,
    pub output: OutputRef,
    /// The export this batch came from. For an export, the export itself.
    pub export: Option<OutputRef>,
    pub transfer_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct BlockWindow {
    pub first: u64,
    pub last: u64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ActivityPage {
    pub currency_id: String,
    pub window: BlockWindow,
    pub anchor_height: u64,
    pub anchor_hash: String,
    pub source: String,
    pub records: Vec<ActivityRecord>,
    /// True when the window holds more records than Carbide keeps.
    pub truncated: bool,
    pub older: Option<BlockWindow>,
    /// The next window toward the anchor. `None` at the anchor.
    pub newer: Option<BlockWindow>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ExportInputs {
    pub export: OutputRef,
    /// The outputs the export spent, from its declared input range only.
    pub inputs: Vec<OutputRef>,
    /// True when the range holds more inputs than Carbide lists.
    pub truncated: bool,
    pub source: String,
}

// ---------------------------------------------------------------------- marketplace

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum OfferSort {
    Expiry,
    Name,
    AskAmount,
    ListingAge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum OfferLinkKind {
    Funding,
    Identity,
    Currency,
    Address,
}

/// An object an offer refers to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, uniffi::Record)]
pub struct OfferLink {
    pub kind: OfferLinkKind,
    pub value: String,
}

/// One side of an offer: an identity, or a set of currency amounts.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OfferAsset {
    pub identity_id: Option<String>,
    pub identity_name: Option<String>,
    pub amounts: Vec<CurrencyAmount>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Offer {
    pub txid: String,
    /// The daemon's group for the offer, such as `ids_for_currency_…`.
    pub category: String,
    pub kind: OfferKind,
    pub subject_id: Option<String>,
    /// The exact asking amount, when the offer asks for one currency amount.
    pub ask_currency: Option<String>,
    pub ask_sats: Option<i64>,
    pub offered: OfferAsset,
    pub accepted: OfferAsset,
    /// Zero when the offer has no expiry.
    pub expiry_height: u64,
    /// The funding transaction, identities and currencies. Addresses come from
    /// `offer_funding`.
    pub links: Vec<OfferLink>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct Marketplace {
    /// The currency the offers involve: the chain's own coin, as in Carbide.
    pub subject: String,
    pub offers: Vec<Offer>,
    pub source: String,
    pub truncated: bool,
    /// Live, or the stored offers from the last answer.
    pub provenance: Provenance,
}

/// What the funding transaction of an offer says: when it was listed and its addresses.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OfferFunding {
    pub txid: String,
    pub block_time: Option<u64>,
    pub block_height: Option<u64>,
    pub addresses: Vec<String>,
    pub error: Option<String>,
}

/// What to read for one offer: its funding transaction, and the identity for sale.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OfferKey {
    pub txid: String,
    /// The identity that the offer sells. None for an offer of currencies.
    pub identity_id: Option<String>,
}

/// When an offer was made, and who controls the identity that it sells.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OfferDetail {
    pub txid: String,
    pub block_height: Option<u64>,
    pub block_time: Option<u64>,
    /// The first primary address of the offered identity now. The app calls this the seller.
    pub holder: Option<String>,
    pub error: Option<String>,
}

/// The most offers that one `offer_details` request reads.
const MAX_DETAIL_LOOKUPS: usize = 256;

// ------------------------------------------------------------------- notarizations

/// One system's view of another: `observer`'s node asked about `subject`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NotarizationPerspective {
    pub observer_id: String,
    pub observer_name: String,
    pub subject_id: String,
    pub subject_name: String,
    /// True when the observer is the chain this app is connected to.
    pub local: bool,
    /// True when a node configuration for the observer exists on this Mac.
    pub configured: bool,
}

/// The result of checking a foreign proof root on the owning chain's source.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum RootCheck {
    /// The block is on the source's active chain.
    Active,
    /// The source knows the block, but it is not on its active chain.
    KnownOffChain,
    /// The source has the block at a different height.
    HeightMismatch { found: u64 },
    /// The source has the block but did not say if it is active.
    PresentMembershipUnknown,
    /// Not found, and the source's tip is below the committed height.
    SourceBehind { tip: u64 },
    /// The source explicitly does not know the block.
    NotFound,
    NoSourceConfigured,
    NoSourceEnabled,
    ChainQueryFailed,
    BlockLookupFailed,
    InvalidHeader,
    /// The proof root has no block hash.
    MissingHash,
    /// Not requested, or past the request limit.
    NotChecked,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ProofRootRow {
    pub system_id: String,
    pub system_name: Option<String>,
    pub height: Option<u64>,
    pub block_hash: Option<String>,
    /// True when the root belongs to a system other than the observer.
    pub foreign: bool,
    /// `None` for the observer's own root, and for Ethereum roots, which a Verus node
    /// cannot check.
    pub check: Option<RootCheck>,
    /// Carbide's status text for the check.
    pub status: Option<String>,
    /// The source that answered the check.
    pub checked_on: Option<String>,
    /// True when the check can indicate a fork: an off-chain block or an explicit miss.
    pub possible_fork: bool,
    /// True when the block was found at the committed height, so it can be opened.
    pub resolved: bool,
}

/// The previous notarization a mirror notarization refers to, on its own system.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PreviousCheck {
    pub output: OutputRef,
    pub system_name: Option<String>,
    pub found: bool,
    pub status: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BrowserNotarization {
    pub index: u32,
    pub txid: Option<String>,
    pub output: Option<u32>,
    /// The observer's block that includes the notarization.
    pub block_hash: Option<String>,
    pub included_height: Option<u64>,
    pub included_time: Option<u64>,
    /// Carbide's text for the inclusion check.
    pub inclusion: String,
    pub confirmed: bool,
    /// A placeholder row with no transaction and no block.
    pub synthetic: bool,
    /// False when the row is not on the daemon's best fork.
    pub on_best_chain: bool,
    pub roots: Vec<ProofRootRow>,
    pub previous: Option<PreviousCheck>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum BrowserLoad {
    Ready,
    NoData,
    Unavailable,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct NotarizationBrowser {
    pub perspective: NotarizationPerspective,
    pub load: BrowserLoad,
    /// Why the data is unavailable.
    pub message: Option<String>,
    pub rows: Vec<BrowserNotarization>,
    pub last_confirmed: Option<u32>,
    /// Distinct unconfirmed notarizations in the returned view.
    pub unconfirmed: u32,
    pub fork_count: u32,
    /// The daemon's forks: each is a list of row indexes from the oldest to the newest.
    pub forks: Vec<Vec<u32>>,
    /// The fork that the daemon gives as the best chain.
    pub best_fork: Option<u32>,
    pub source: Option<String>,
    /// True when some checks were not made because of the request limit.
    pub checks_limited: bool,
    pub json: String,
    /// Where the notarization data came from. `None` when no source answered and nothing
    /// is stored.
    pub provenance: Option<Provenance>,
}

/// One earlier notarization in the record that leads to the last confirmed one.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecordPoint {
    pub txid: String,
    /// The block of this chain that includes the notarization.
    pub height: u64,
    pub time: Option<u64>,
}

// ----------------------------------------------------------------------- bookmarks

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum BookmarkCache {
    /// Carbide's cache holds an observation of the object.
    Cached { source: String, tags: Vec<String> },
    Uncached,
    /// The last request for the object failed.
    Failed { message: String },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BookmarkEntry {
    pub index: u32,
    /// auto, block, transaction, address, identity, currency or notarizations.
    pub kind: String,
    pub system_id: String,
    pub system_name: Option<String>,
    /// True when the bookmark belongs to the chain this app is connected to.
    pub this_chain: bool,
    pub target: String,
    pub description: String,
    pub created_at: u64,
    pub cache: BookmarkCache,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BookmarkFile {
    pub entries: Vec<BookmarkEntry>,
    pub path: String,
    /// Lines that are not valid bookmarks. Carbide keeps them unchanged.
    pub issues: Vec<String>,
    pub startup_enabled: bool,
    pub startup_limit: u32,
    /// A problem with Carbide's `config.toml`, if any.
    pub settings_issue: Option<String>,
    pub settings_path: String,
    /// The result of start-up population, once it has run.
    pub startup_report: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PopulationReport {
    pub enabled: bool,
    pub limit: u32,
    pub attempted: u32,
    pub fetched: u32,
    pub message: String,
}

// ============================================================ pure logic (ported)

// CCurrencyDefinition::EOptions, as in `src/app/currency_ui.rs` (`OPTION_TAGS`).
const OPTION_FLAGS: &[(u64, &str)] = &[
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

/// Ported from `currency_ui::option_tags`: unknown bits and an empty set stay visible.
fn listing_tags(options: Option<u64>) -> Vec<String> {
    let Some(options) = options else {
        return vec!["Options unavailable".into()];
    };
    let mut tags: Vec<String> = OPTION_FLAGS
        .iter()
        .filter(|(bit, _)| options & bit != 0)
        .map(|(_, name)| (*name).into())
        .collect();
    let unknown = options & !0x1fff;
    if unknown != 0 {
        tags.push(format!("Unknown options 0x{unknown:x}"));
    }
    if tags.is_empty() {
        tags.push("No option flags".into());
    }
    tags
}

/// Ported from `currency_browser::decode`: one entry per currency ID, sorted by name.
fn currency_listings(list: &[Value]) -> Vec<CurrencyListing> {
    let mut entries = std::collections::BTreeMap::new();
    for record in list {
        let definition = record.get("currencydefinition").unwrap_or(record);
        let Some(id) = text(&definition["currencyid"]) else {
            continue;
        };
        let name = text(&definition["fullyqualifiedname"])
            .or_else(|| text(&definition["name"]))
            .unwrap_or_else(|| id.clone());
        let state = currency_status::reported_state(record);
        let status = currency_status::classify(state);
        let options = definition["options"].as_u64();
        let reserve_ids = strings(&definition["currencies"]);
        let reserve_names = reserve_ids
            .iter()
            .map(|reserve| {
                definition["currencynames"][reserve]
                    .as_str()
                    .filter(|name| !name.is_empty() && name != reserve)
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        entries.insert(
            id.clone(),
            CurrencyListing {
                name,
                currency_id: id,
                options,
                option_tags: listing_tags(options),
                launch: status.into(),
                launch_label: status.label().into(),
                start_block: definition["startblock"].as_u64(),
                supply: state["supply"].as_f64(),
                reserve_ids,
                reserve_names,
            },
        );
    }
    let mut rows: Vec<CurrencyListing> = entries.into_values().collect();
    rows.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.currency_id.cmp(&b.currency_id))
    });
    rows
}

/// Ported from `currency_browser::View::filtered`.
fn currency_matches(row: &CurrencyListing, filter: &CurrencyFilter) -> bool {
    if filter.launch_mask & row.launch.core().bit() == 0 {
        return false;
    }
    let flags = match row.options {
        Some(options) => {
            options & filter.required == filter.required && options & filter.excluded == 0
        }
        None => filter.required == 0 && filter.excluded == 0,
    };
    if !flags {
        return false;
    }
    let needle = filter.text.trim().to_lowercase();
    needle.is_empty()
        || row.name.to_lowercase().contains(&needle)
        || row.currency_id.to_lowercase().contains(&needle)
        || row.option_tags.join(" ").to_lowercase().contains(&needle)
}

/// A date, time or block request for a currency state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SeekTarget {
    /// A bare number: a height at or below the tip, otherwise Unix seconds.
    Number(u64),
    Height(u64),
    Time(i64),
}

/// Ported from the terminal's chart seek (`src/app/chart_ui/seek.rs`, `parse`).
/// `height:` and `#` force a height; `unix:` forces a time.
fn parse_seek(input: &str, now: chrono::DateTime<chrono::Utc>) -> Result<SeekTarget> {
    let input = input.trim();
    anyhow::ensure!(
        !input.is_empty(),
        "Enter a block height, Unix seconds or a date and time."
    );
    if let Some(height) = input
        .strip_prefix("height:")
        .or_else(|| input.strip_prefix('#'))
    {
        return Ok(SeekTarget::Height(
            height
                .trim()
                .parse()
                .context("Enter a block height of zero or more.")?,
        ));
    }
    if let Some(time) = input.strip_prefix("unix:") {
        return Ok(SeekTarget::Time(
            crate::block_time::parse_at(time, now)?.timestamp(),
        ));
    }
    if input.bytes().all(|byte| byte.is_ascii_digit()) {
        let number = input.parse::<u64>().context("The number is out of range.")?;
        anyhow::ensure!(
            number <= 99_999_999_999,
            "Use Unix seconds, or height: followed by a block height."
        );
        return Ok(SeekTarget::Number(number));
    }
    Ok(SeekTarget::Time(
        crate::block_time::parse_at(input, now)?.timestamp(),
    ))
}

/// Ported from `currency_ui::spawn_history`: a bare number at or below the tip is a
/// height, a larger one is Unix seconds. A future height is refused.
fn settle_seek(target: SeekTarget, tip_height: u64) -> Result<SeekTarget> {
    let target = match target {
        SeekTarget::Number(number) if number <= tip_height => SeekTarget::Height(number),
        SeekTarget::Number(number) => {
            SeekTarget::Time(i64::try_from(number).context("The number is out of range.")?)
        }
        other => other,
    };
    if let SeekTarget::Height(height) = target {
        anyhow::ensure!(
            height <= tip_height,
            "Block {height} is in the future. A future currency state is not available. The tip is {tip_height}."
        );
    }
    Ok(target)
}

fn state_reserves(state: &Value) -> Vec<StateReserve> {
    state["reservecurrencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|reserve| {
            let currency_id = text(&reserve["currencyid"])?;
            let flow = &state["currencies"][&currency_id];
            Some(StateReserve {
                weight: reserve["weight"].as_f64(),
                reserves: reserve["reserves"].as_f64(),
                price_in_reserve: reserve["priceinreserve"].as_f64(),
                reserve_in: flow["reservein"].as_f64().unwrap_or(0.0),
                reserve_out: flow["reserveout"].as_f64().unwrap_or(0.0),
                conversion_fees: flow["conversionfees"].as_f64().unwrap_or(0.0),
                currency_id,
            })
        })
        .collect()
}

fn output_ref(outpoint: &OutpointRef) -> OutputRef {
    OutputRef {
        system_id: outpoint.system_id.to_string(),
        txid: outpoint.txid.clone(),
        output: outpoint.vout,
    }
}

fn core_outpoint(output: &OutputRef) -> Result<OutpointRef> {
    Ok(OutpointRef {
        system_id: SystemId::try_from(output.system_id.clone())?,
        txid: output.txid.clone(),
        vout: output.output,
    })
}

fn activity_record(record: &currency_activity::Record) -> ActivityRecord {
    ActivityRecord {
        height: record.height,
        kind: match record.kind {
            currency_activity::Kind::Import => ActivityKind::Import,
            currency_activity::Kind::Export => ActivityKind::Export,
        },
        output: output_ref(&record.output),
        export: record.export.as_ref().map(output_ref),
        transfer_hash: record.transfer_hash.clone(),
    }
}

fn core_record(record: &ActivityRecord) -> Result<currency_activity::Record> {
    Ok(currency_activity::Record {
        height: record.height,
        kind: match record.kind {
            ActivityKind::Import => currency_activity::Kind::Import,
            ActivityKind::Export => currency_activity::Kind::Export,
        },
        output: core_outpoint(&record.output)?,
        export: record.export.as_ref().map(core_outpoint).transpose()?,
        transfer_hash: record.transfer_hash.clone(),
    })
}

fn window(range: Range) -> BlockWindow {
    BlockWindow {
        first: range.first,
        last: range.last,
    }
}

/// Ported from `decimal_string_to_zat` in `src/app.rs`: exact satoshis, or `None` when
/// the value has more precision than a satoshi.
fn decimal_sats(value: &Value) -> Option<i128> {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        _ => {
            return value
                .get("valueZat")
                .and_then(Value::as_i64)
                .map(i128::from)
                .or_else(|| value.get("value").and_then(decimal_sats))
                .or_else(|| value.get("amount").and_then(decimal_sats));
        }
    };
    let value = text.trim();
    let (mantissa, exponent) = value
        .split_once(['e', 'E'])
        .map_or(Some((value, 0)), |(m, e)| Some((m, e.parse::<i32>().ok()?)))?;
    let (negative, unsigned) = mantissa
        .strip_prefix('-')
        .map_or((false, mantissa), |v| (true, v));
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty()
        || !whole.chars().all(|ch| ch.is_ascii_digit())
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}").parse::<i128>().ok()?;
    let scale = 8_i32
        .checked_add(exponent)?
        .checked_sub(i32::try_from(fraction.len()).ok()?)?;
    let amount = if scale >= 0 {
        digits.checked_mul(10_i128.checked_pow(u32::try_from(scale).ok()?)?)?
    } else {
        let divisor = 10_i128.checked_pow(u32::try_from(scale.checked_neg()?).ok()?)?;
        if digits % divisor != 0 {
            return None;
        }
        digits / divisor
    };
    Some(if negative { -amount } else { amount })
}

/// Ported from `marketplace_asking_price`: an ask is exactly one currency amount.
fn asking_price(accepted: &Value) -> Option<(String, i64)> {
    let fields = accepted.as_object()?;
    if fields.len() != 1 || fields.contains_key("name") || fields.contains_key("identityid") {
        return None;
    }
    let (currency, value) = fields.iter().next()?;
    let amount = decimal_sats(value)?;
    (amount > 0).then(|| (currency.clone(), i64::try_from(amount).unwrap_or(i64::MAX)))
}

fn offer_asset(value: &Value) -> OfferAsset {
    let identity_id = text(&value["identityid"]);
    let identity_name = text(&value["name"]);
    let amounts = if identity_id.is_some() || identity_name.is_some() {
        Vec::new()
    } else {
        amounts(value)
    };
    OfferAsset {
        identity_id,
        identity_name,
        amounts,
    }
}

/// Ported from `marketplace_links`: the funding transaction, then each identity and
/// currency on either side, then the addresses of the funding transaction's outputs.
fn links_for(offered: &Value, accepted: &Value, txid: &str, addresses: &[String]) -> Vec<OfferLink> {
    let mut links = vec![OfferLink {
        kind: OfferLinkKind::Funding,
        value: txid.to_string(),
    }];
    let mut push = |link: OfferLink| {
        if !links.contains(&link) {
            links.push(link);
        }
    };
    for asset in [offered, accepted] {
        let Some(fields) = asset.as_object() else {
            continue;
        };
        if let Some(identity) = fields.get("identityid").and_then(Value::as_str) {
            push(OfferLink {
                kind: OfferLinkKind::Identity,
                value: identity.to_string(),
            });
        }
        // An identity side has fields of its own with numbers, such as `original`. They
        // are not currencies. Only a currency side lists currencies as its keys.
        if fields.contains_key("identityid") || fields.contains_key("name") {
            continue;
        }
        for (currency, amount) in fields {
            if amount.as_f64().is_some() {
                push(OfferLink {
                    kind: OfferLinkKind::Currency,
                    value: currency.clone(),
                });
            }
        }
    }
    for address in addresses {
        push(OfferLink {
            kind: OfferLinkKind::Address,
            value: address.clone(),
        });
    }
    links
}

/// The first primary address in a `getidentity` answer.
fn identity_holder(answer: &Value) -> Option<String> {
    text(&answer["identity"]["primaryaddresses"][0])
}

fn offer_kind(offered: &Value, accepted: &Value) -> OfferKind {
    if offered.get("identityid").is_some() || offered.get("name").is_some() {
        OfferKind::IdentityForSale
    } else if accepted.get("identityid").is_some() || accepted.get("name").is_some() {
        OfferKind::IdentityWanted
    } else {
        OfferKind::CurrencySwap
    }
}

/// Ported from `parse_marketplace_offers`: offer records can be nested at any depth,
/// and each takes the name of the group that holds it. Sorted by expiry, without repeats.
fn parse_offers(value: &Value) -> Vec<Offer> {
    fn visit(value: &Value, category: &str, offers: &mut Vec<Offer>) {
        match value {
            Value::Array(values) => {
                for value in values {
                    visit(value, category, offers);
                }
            }
            Value::Object(fields) => {
                if let Some(details) = fields.get("offer").and_then(Value::as_object)
                    && let Some(txid) = details.get("txid").and_then(Value::as_str)
                    && let (Some(offered), Some(accepted)) =
                        (details.get("offer"), details.get("accept"))
                {
                    let ask = asking_price(accepted);
                    offers.push(Offer {
                        txid: txid.to_string(),
                        category: category.to_string(),
                        kind: offer_kind(offered, accepted),
                        subject_id: fields
                            .get("identityid")
                            .or_else(|| fields.get("currencyid"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        ask_currency: ask.as_ref().map(|(currency, _)| currency.clone()),
                        ask_sats: ask.map(|(_, amount)| amount),
                        offered: offer_asset(offered),
                        accepted: offer_asset(accepted),
                        expiry_height: details
                            .get("blockexpiry")
                            .and_then(Value::as_u64)
                            .unwrap_or_default(),
                        links: links_for(offered, accepted, txid, &[]),
                    });
                    return;
                }
                for (key, value) in fields {
                    visit(value, key, offers);
                }
            }
            _ => {}
        }
    }
    let mut offers = Vec::new();
    visit(value, "offers", &mut offers);
    offers.sort_by(|left, right| {
        left.expiry_height
            .cmp(&right.expiry_height)
            .then_with(|| left.txid.cmp(&right.txid))
    });
    offers.dedup_by(|left, right| left.txid == right.txid);
    offers
}

/// Ported from `marketplace_sort_name`.
fn offer_sort_name(offer: &Offer, names: &HashMap<String, String>) -> String {
    let id = offer.subject_id.as_deref();
    for asset in [&offer.offered, &offer.accepted] {
        if asset.identity_id.as_deref() == id
            && let Some(name) = &asset.identity_name
        {
            return name.to_lowercase();
        }
    }
    id.map(|id| names.get(id).cloned().unwrap_or_else(|| id.to_string()))
        .unwrap_or_else(|| offer.category.clone())
        .to_lowercase()
}

/// Ported from `update_marketplace_sort`. Ask amounts group by currency. Listing age
/// puts the newest first, and offers with no known age last, in either direction.
fn sort_offer_list(
    offers: &mut [Offer],
    sort: OfferSort,
    descending: bool,
    names: &HashMap<String, String>,
    times: &HashMap<String, u64>,
) {
    use std::cmp::Ordering;
    offers.sort_by(|a, b| {
        let order = match sort {
            OfferSort::Name => {
                let order = offer_sort_name(a, names).cmp(&offer_sort_name(b, names));
                if descending { order.reverse() } else { order }
            }
            OfferSort::AskAmount => match (
                a.ask_currency.as_ref().zip(a.ask_sats),
                b.ask_currency.as_ref().zip(b.ask_sats),
            ) {
                (Some(a), Some(b)) => a.0.cmp(b.0).then_with(|| {
                    if descending {
                        b.1.cmp(&a.1)
                    } else {
                        a.1.cmp(&b.1)
                    }
                }),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                _ => Ordering::Equal,
            },
            OfferSort::ListingAge => match (times.get(&a.txid), times.get(&b.txid)) {
                (Some(a), Some(b)) => {
                    if descending {
                        a.cmp(b)
                    } else {
                        b.cmp(a)
                    }
                }
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                _ => Ordering::Equal,
            },
            OfferSort::Expiry => {
                let order = a.expiry_height.cmp(&b.expiry_height);
                if descending { order.reverse() } else { order }
            }
        };
        order.then_with(|| a.txid.cmp(&b.txid))
    });
}

// ------------------------------------------------------------ notarization logic

/// A proof root as the daemon reports it. Ported from `notarization_ui::Root`.
#[derive(Debug, Clone)]
struct Root {
    system: String,
    height: Option<u64>,
    hash: Option<String>,
}

#[derive(Debug, Clone)]
struct GraphRow {
    index: usize,
    txid: Option<String>,
    vout: Option<u32>,
    block: Option<String>,
    roots: Vec<Root>,
    foreign_previous: Option<OutpointRef>,
    synthetic: bool,
}

#[derive(Debug)]
struct Graph {
    rows: Vec<GraphRow>,
    confirmed: Option<usize>,
    best: Option<Vec<u64>>,
    forks: Vec<Vec<u64>>,
    best_fork: Option<usize>,
}

impl Graph {
    /// Ported from `notarization_ui::Graph::pending`: distinct unconfirmed outputs.
    fn pending(&self) -> usize {
        let confirmed = self.confirmed.and_then(|i| self.rows.get(i));
        self.rows
            .iter()
            .filter(|row| Some(row.index) != self.confirmed && !row.synthetic)
            .filter_map(|row| Some((row.txid.as_deref()?, row.vout?)))
            .filter(|(txid, vout)| {
                !confirmed.is_some_and(|row| {
                    row.txid.as_deref() == Some(*txid) && row.vout == Some(*vout)
                })
            })
            .collect::<HashSet<_>>()
            .len()
    }
}

/// A 64-digit hash that is not all zeros. Ported from `notarization_ui::hash`.
fn block_hash(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| {
            s.len() == 64
                && s.bytes().all(|c| c.is_ascii_hexdigit())
                && s.bytes().any(|b| b != b'0')
        })
        .map(str::to_ascii_lowercase)
}

/// Ported from `notarization_ui::decode`, with the daemon's forks kept for the lanes.
fn decode_graph(value: &Value, observer: &str) -> Result<Graph> {
    let entries = value["notarizations"]
        .as_array()
        .context("The notarization data has no list of notarizations.")?;
    let confirmed = match value["lastconfirmed"].as_i64() {
        Some(-1) => None,
        None if entries.is_empty() && value["lastconfirmed"].is_null() => None,
        Some(index) if index >= 0 && (index as usize) < entries.len() => Some(index as usize),
        _ => anyhow::bail!("The last confirmed notarization index is not valid."),
    };
    let forks: Vec<Vec<u64>> = value["forks"]
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
    let best_fork = value["bestchain"]
        .as_u64()
        .map(|best| best as usize)
        .filter(|best| *best < forks.len());
    let best = best_fork.map(|best| forks[best].clone());
    let rows = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let notarization = &entry["notarization"];
            let block = block_hash(&entry["blockhash"]);
            let synthetic = entry["txid"]
                .as_str()
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b == b'0'))
                && block.is_none();
            let roots = notarization["proofroots"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|root| {
                    Some(Root {
                        system: root["systemid"].as_str()?.to_owned(),
                        height: root["height"].as_u64(),
                        hash: block_hash(&root["blockhash"]),
                    })
                })
                .collect();
            // In mirror form the previous output belongs to the notarized system.
            let foreign_previous = (|| {
                let system = notarization["currencyid"].as_str()?;
                if notarization["ismirror"].as_bool() != Some(true) || system == observer {
                    return None;
                }
                Some(OutpointRef {
                    system_id: SystemId::try_from(system.to_owned()).ok()?,
                    txid: block_hash(&notarization["prevnotarizationtxid"])?,
                    vout: notarization["prevnotarizationout"]
                        .as_u64()?
                        .try_into()
                        .ok()?,
                })
            })();
            GraphRow {
                index,
                txid: block_hash(&entry["txid"]),
                vout: entry["vout"].as_u64().and_then(|v| v.try_into().ok()),
                block,
                roots,
                foreign_previous,
                synthetic,
            }
        })
        .collect();
    Ok(Graph {
        rows,
        confirmed,
        best,
        forks,
        best_fork,
    })
}

/// The notarization of `currency` in a transaction: its output number and its data.
fn notarization_output<'a>(raw: &'a Value, currency: &str) -> Option<(u32, &'a Value)> {
    raw["vout"].as_array()?.iter().find_map(|output| {
        let script = &output["scriptPubKey"];
        ["acceptednotarization", "earnednotarization"]
            .iter()
            .map(|key| &script[*key])
            .find(|data| data["currencyid"].as_str() == Some(currency))
            .and_then(|data| Some((output["n"].as_u64()?.try_into().ok()?, data)))
    })
}

/// Where the notarization before this one can be. A notarization that this chain made
/// names the one before it. A notarization that came from the other system names an
/// output of that system, and then the one before it is an output that this transaction
/// spends. The caller reads each candidate and keeps the first that is a notarization of
/// the same currency.
fn previous_notarization_candidates(raw: &Value, currency: &str) -> Vec<(String, u32)> {
    let mut candidates = Vec::new();
    if let Some((_, data)) = notarization_output(raw, currency)
        && data["ismirror"].as_bool() != Some(true)
        && let Some(txid) = block_hash(&data["prevnotarizationtxid"])
        && let Some(output) = data["prevnotarizationout"].as_u64()
    {
        candidates.push((txid, output as u32));
    }
    for input in raw["vin"].as_array().into_iter().flatten().take(MAX_RECORD_INPUTS) {
        if let Some(txid) = block_hash(&input["txid"])
            && let Some(output) = input["vout"].as_u64()
            && !candidates.iter().any(|(known, _)| *known == txid)
        {
            candidates.push((txid, output as u32));
        }
    }
    candidates
}

#[derive(Debug, Clone)]
struct Header {
    height: u64,
    time: Option<u64>,
    confirmations: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckFailure {
    NoSourceConfigured,
    NoSourceEnabled,
    ChainQueryFailed,
    BlockLookupFailed,
    InvalidHeader,
    TransactionLookupFailed,
}

impl CheckFailure {
    /// Carbide's labels, from `notarization_ui::CheckFailure::label`. The terminal's
    /// "(see F2)" points to its Sources screen; here the label names the Sources screen.
    fn label(self) -> &'static str {
        match self {
            Self::NoSourceConfigured => "No source configured for this chain",
            Self::NoSourceEnabled => "No source is enabled. Turn one on in Sources.",
            Self::ChainQueryFailed => "Could not query chain status",
            Self::BlockLookupFailed => "Block header lookup failed",
            Self::InvalidHeader => "Source returned an invalid block header",
            Self::TransactionLookupFailed => "Could not verify the referenced outpoint",
        }
    }

    fn root_check(self) -> RootCheck {
        match self {
            Self::NoSourceConfigured => RootCheck::NoSourceConfigured,
            Self::NoSourceEnabled => RootCheck::NoSourceEnabled,
            Self::ChainQueryFailed => RootCheck::ChainQueryFailed,
            Self::InvalidHeader => RootCheck::InvalidHeader,
            Self::BlockLookupFailed | Self::TransactionLookupFailed => RootCheck::BlockLookupFailed,
        }
    }
}

#[derive(Debug, Clone)]
enum Check<T> {
    Found { source: String, value: T },
    NotFound { source: String, tip: Option<u64> },
    Unavailable(CheckFailure),
}

/// Ported from `notarization_ui::root_status`. `None` means the status is not shown:
/// an Ethereum root that no Verus source can check.
fn classify_root(check: Option<&Check<Header>>, root: &Root) -> Option<(RootCheck, String)> {
    if root.system == VETH_SYSTEM_ID && matches!(check, None | Some(Check::Unavailable(_))) {
        return None;
    }
    Some(match check {
        Some(Check::Found { value, .. }) => match (root.height, value.confirmations) {
            (Some(height), _) if height != value.height => (
                RootCheck::HeightMismatch { found: value.height },
                format!("Height mismatch (found #{})", value.height),
            ),
            (_, Some(-1)) => (
                RootCheck::KnownOffChain,
                "Known off-chain • possible fork".into(),
            ),
            (_, Some(n)) if n > 0 => (RootCheck::Active, "Active".into()),
            _ => (
                RootCheck::PresentMembershipUnknown,
                "Present • membership unavailable".into(),
            ),
        },
        Some(Check::NotFound { tip: Some(tip), .. }) if root.height.is_some_and(|h| h > *tip) => (
            RootCheck::SourceBehind { tip: *tip },
            format!("Not found • source tip #{tip} is behind"),
        ),
        Some(Check::NotFound { .. }) => (RootCheck::NotFound, "Not found • possible fork".into()),
        Some(Check::Unavailable(reason)) => (reason.root_check(), reason.label().into()),
        None if root.hash.is_some() => (RootCheck::NotChecked, "Awaiting check".into()),
        None => (RootCheck::MissingHash, "Proof root has no block hash".into()),
    })
}

fn header_of(value: &Value, hash: &str) -> Option<Header> {
    if value["hash"].as_str()? != hash {
        return None;
    }
    Some(Header {
        height: value["height"].as_u64()?,
        time: value["time"].as_u64(),
        confirmations: value["confirmations"].as_i64(),
    })
}

/// Ported from `notarization_ui::browser`: a known PBaaS pair has a second perspective,
/// the other chain's node asked about this one.
fn perspective_pairs(
    observer: &str,
    subject: &str,
    parent: &str,
    options: Option<u64>,
    definition_system: Option<&str>,
) -> Vec<(String, String)> {
    let mut pairs = vec![(observer.to_string(), subject.to_string())];
    let definition_is_child =
        options.is_some_and(|flags| flags & 0x100 != 0) && definition_system == Some(subject);
    if (observer == parent && subject != parent && definition_is_child)
        || (subject == parent && observer != parent)
    {
        pairs.push((subject.to_string(), observer.to_string()));
    }
    pairs
}

// ------------------------------------------------------------- bookmark logic

fn bookmark_kind(kind: &str) -> Option<crate::bookmarks::Kind> {
    use crate::bookmarks::Kind;
    Some(match kind.trim().to_ascii_lowercase().as_str() {
        "auto" | "" => Kind::Auto,
        "block" => Kind::Block,
        "transaction" => Kind::Transaction,
        "address" => Kind::Address,
        "identity" => Kind::Identity,
        "currency" => Kind::Currency,
        "notarizations" | "notarization" => Kind::Notarizations,
        _ => return None,
    })
}

/// Ported from `bookmark_ui::snapshot_tags`: tags derived from a cached observation.
fn snapshot_tags(snapshot: &crate::bookmarks::Snapshot) -> Vec<String> {
    use crate::bookmarks::Kind;
    let transaction_tags = |raw: &Value| {
        labels(&TransactionSummary {
            txid: raw["txid"].as_str().unwrap_or_default().into(),
            raw: Some(raw.clone()),
        })
    };
    match snapshot.kind {
        Kind::Transaction => transaction_tags(&snapshot.raw),
        Kind::Block => snapshot.raw["tx"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|tx| tx.is_object())
            .flat_map(transaction_tags)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        Kind::Identity => vec!["Identity".into()],
        Kind::Address => vec!["Address".into()],
        Kind::Currency => vec!["Currency".into()],
        Kind::Notarizations => vec!["Notarizations".into()],
        Kind::Auto => vec![],
    }
}

/// Reads the `[bookmarks]` table of Carbide's `config.toml`. Other tables are ignored.
fn startup_options(settings: Option<&str>) -> Result<crate::bookmarks::Options> {
    let Some(settings) = settings else {
        return Ok(Default::default());
    };
    let value: toml::Value = toml::from_str(settings).context("Carbide's config.toml is not valid")?;
    match value.get("bookmarks") {
        Some(table) => Ok(table
            .clone()
            .try_into()
            .context("The [bookmarks] table in Carbide's config.toml is not valid")?),
        None => Ok(Default::default()),
    }
}

fn settings_path() -> Option<std::path::PathBuf> {
    Some(
        directories::BaseDirs::new()?
            .config_dir()
            .join("verus-explorer-tui/config.toml"),
    )
}

type BookmarkKey = (String, String, String);

fn bookmark_key(bookmark: &crate::bookmarks::Bookmark) -> BookmarkKey {
    (
        bookmark.system_id.to_string(),
        bookmark.kind.label().to_string(),
        bookmark.target.clone(),
    )
}

/// Start-up population failures stay visible in the list for the session.
fn population_failures() -> &'static std::sync::Mutex<HashMap<BookmarkKey, String>> {
    static FAILURES: std::sync::OnceLock<std::sync::Mutex<HashMap<BookmarkKey, String>>> =
        std::sync::OnceLock::new();
    FAILURES.get_or_init(Default::default)
}

static POPULATION_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
use std::sync::atomic::Ordering;

fn population_report() -> &'static std::sync::Mutex<Option<String>> {
    static REPORT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    &REPORT
}


// ------------------------------------------------------------ stored currency states

/// One historical currency state recorded in Carbide's cache.
#[derive(Debug, Clone, PartialEq)]
struct StoredState {
    source: String,
    height: u64,
    block_hash: String,
    state: Value,
    response: Value,
    observed_at: u64,
}

/// Reads the historical states that Carbide recorded for a currency, newest request first.
/// It uses a read-only connection, as `retained_row` in `ffi.rs` does, so it never writes
/// and works while another Carbide process owns the cache. The query follows
/// `currency_snapshots` in `src/cache/currency.rs`, and adds the observation time.
fn stored_states(path: &std::path::Path, system: &str, currency: &str) -> Result<Vec<StoredState>> {
    use rusqlite::{OpenFlags, params};
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let connection = rusqlite::Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    connection.busy_timeout(std::time::Duration::from_secs(2))?;
    let mut statement = connection.prepare(
        "select s.source,s.reported_height,s.observed_block_hash,state.payload,response.payload,s.observed_at
         from currency_states s
         join raw_blobs state on state.hash=s.state_hash
         join raw_blobs response on response.hash=s.payload_hash
         where s.system_id=?1 and s.currency_id=?2 and s.state_kind='historical'
           and s.reported_height is not null and s.observed_block_hash is not null
         order by s.request_order desc,s.observed_at desc limit 256",
    )?;
    let rows = statement.query_map(params![system, currency], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)?,
        ))
    })?;
    let mut states = Vec::new();
    for row in rows {
        let (source, height, block_hash, state, response, observed_at) = row?;
        states.push(StoredState {
            source,
            height: u64::try_from(height)?,
            block_hash,
            state: serde_json::from_str(&state)?,
            response: serde_json::from_str(&response)?,
            observed_at: observed_at.max(0) as u64,
        });
    }
    Ok(states)
}

/// The height a request names when no source answers. Without a tip a bare number is read
/// as a height when it is too small to be a Unix time after Verus's launch. A time request
/// needs a source to find its block.
fn stored_height(target: SeekTarget) -> Option<u64> {
    match target {
        SeekTarget::Height(height) => Some(height),
        SeekTarget::Number(number) if number < 1_000_000_000 => Some(number),
        _ => None,
    }
}

// ======================================================== exported free functions

/// Liquidity pools, with known failed launches hidden: the browser's default.
#[uniffi::export]
pub fn default_currency_filter() -> CurrencyFilter {
    CurrencyFilter {
        required: 1,
        excluded: 0,
        launch_mask: LaunchStatus::DEFAULT_MASK,
        text: String::new(),
    }
}

#[uniffi::export]
pub fn currency_option_flags() -> Vec<OptionFlag> {
    OPTION_FLAGS
        .iter()
        .map(|(bit, name)| OptionFlag {
            bit: *bit,
            name: (*name).into(),
        })
        .collect()
}

#[uniffi::export]
pub fn currency_flag_state(filter: CurrencyFilter, bit: u64) -> FlagState {
    if filter.required & bit != 0 {
        FlagState::Show
    } else if filter.excluded & bit != 0 {
        FlagState::Hide
    } else {
        FlagState::Any
    }
}

/// Any → Show → Hide → Any, as in `currency_browser::View::edit_key`.
#[uniffi::export]
pub fn cycle_currency_flag(filter: CurrencyFilter, bit: u64) -> CurrencyFilter {
    let mut filter = filter;
    if filter.required & bit != 0 {
        filter.required &= !bit;
        filter.excluded |= bit;
    } else if filter.excluded & bit != 0 {
        filter.excluded &= !bit;
    } else {
        filter.required |= bit;
    }
    filter
}

#[uniffi::export]
pub fn launch_state_bit(state: LaunchState) -> u8 {
    state.core().bit()
}

/// The positions in `rows` of the currencies that match.
#[uniffi::export]
pub fn filter_currencies(rows: Vec<CurrencyListing>, filter: CurrencyFilter) -> Vec<u32> {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| currency_matches(row, &filter))
        .map(|(index, _)| index as u32)
        .collect()
}

/// The 1,000 blocks that end at `height`.
#[uniffi::export]
pub fn activity_window_ending_at(height: u64) -> BlockWindow {
    window(Range::ending_at(height))
}

/// Sorts offers as the terminal does. `names` maps IDs to names for the name sort.
/// `times` maps funding transaction IDs to block times for the listing-age sort.
#[uniffi::export]
pub fn sort_offers(
    offers: Vec<Offer>,
    sort: OfferSort,
    descending: bool,
    names: HashMap<String, String>,
    times: HashMap<String, u64>,
) -> Vec<Offer> {
    let mut offers = offers;
    sort_offer_list(&mut offers, sort, descending, &names, &times);
    offers
}

/// The offer's links with the funding transaction's addresses added.
#[uniffi::export]
pub fn offer_links_with(offer: Offer, addresses: Vec<String>) -> Vec<OfferLink> {
    let mut links = offer.links;
    for address in addresses {
        let link = OfferLink {
            kind: OfferLinkKind::Address,
            value: address,
        };
        if !links.contains(&link) {
            links.push(link);
        }
    }
    links
}

// ===================================================================== explorer

#[uniffi::export(async_runtime = "tokio")]
impl Explorer {
    // ------------------------------------------------------------ currency browser

    /// Every currency the node lists, with what the browser's rows show.
    pub async fn currency_list(&self) -> Outcome<CurrencyList> {
        let output = self
            .observed(
                "listcurrencies",
                &[],
                self.primary.call_with_raw::<Value>("listcurrencies", &[]).await,
            )
            .await?;
        let list = output
            .result
            .as_array()
            .ok_or_else(|| fail("The node did not return a list of currencies."))?;
        Ok(CurrencyList {
            rows: currency_listings(list),
            source: output.source_id.clone(),
            data_source: source_of(&output.source_id),
            provenance: output.provenance,
        })
    }

    // --------------------------------------------------------- currency state seek

    /// The currency's state at an exact block. `query` is a block height, Unix seconds
    /// or a date and time in UTC; `height:`, `#` and `unix:` remove ambiguity. A future
    /// state is refused.
    pub async fn currency_state_at(
        &self,
        currency: String,
        query: String,
    ) -> Outcome<CurrencyStateAt> {
        let currency = currency.trim().trim_end_matches('@').to_string();
        let target = parse_seek(&query, chrono::Utc::now())?;
        // Whether a source answers decides between a live state and a stored one.
        let before = match self.client.get_info().await {
            Ok(output) => output,
            Err(error) => {
                let error = self.settle::<()>(Err(error)).unwrap_err();
                if source_answered(&error) {
                    return Err(error.into());
                }
                return self.stored_state_at(&currency, target, &error).await;
            }
        };
        let tip = before.result["blocks"]
            .as_u64()
            .ok_or_else(|| fail("The source did not report a block height."))?;
        let tip_hash = self.settle(
            self.client
                .call_with_raw::<String>("getblockhash", &[json!(tip)])
                .await,
        )?;
        let target = settle_seek(target, tip)?;
        let (height, coverage, interpreted) = match target {
            SeekTarget::Height(height) => (height, None, format!("Block height {height}")),
            SeekTarget::Time(time) => {
                let config = self.config.noninteractive();
                let (mut source, tip_block, _) = RpcTimeSource::open(&config).await?;
                if source.source_id() != before.source_id {
                    return Err(fail("The source changed during the time request. Try again."));
                }
                if time > tip_block.timestamp {
                    return Err(fail(
                        "The time is after the current tip. A future currency state is not available.",
                    ));
                }
                let genesis = source.block_at(0).await?;
                if time < genesis.timestamp {
                    return Err(fail("The time is before the first block of the chain."));
                }
                let found = crate::block_time::locate(&mut source, time, tip_block).await?;
                source.check(&found.block).await?;
                let interpreted = chrono::DateTime::from_timestamp(time, 0)
                    .map(|date| format!("Time {}", date.format("%Y-%m-%d %H:%M:%S UTC")))
                    .unwrap_or_else(|| format!("Unix time {time}"));
                let coverage = TimeCoverage {
                    target_time: found.target,
                    block_time: found.block.timestamp,
                    difference_seconds: found.block.timestamp - found.target,
                    exact: found.exact_timestamp,
                    whole_chain: found.whole_chain,
                    checked_first: found.searched.0,
                    checked_last: found.searched.1,
                    probes: found.probes as u32,
                    description: found.description(),
                };
                (found.block.height, Some(coverage), interpreted)
            }
            SeekTarget::Number(_) => unreachable!("settle_seek resolves bare numbers"),
        };
        let response = self.settle(self.client.get_currency_state(&currency, height).await)?;
        let block = self.settle(fetch_block(&self.client, BlockId::Height(height)).await)?;
        let tip_after = self.settle(
            self.client
                .call_with_raw::<String>("getblockhash", &[json!(tip)])
                .await,
        )?;
        if [&response.source_id, &block.source_id, &tip_after.source_id, &tip_hash.source_id]
            .iter()
            .any(|source| **source != before.source_id)
        {
            return Err(fail("The source changed during the request. Try again."));
        }
        if tip_after.result != tip_hash.result {
            return Err(fail(
                "The active chain changed during the request. Try again.",
            ));
        }
        if block.result.height != height {
            return Err(fail("The node returned a block at a different height."));
        }
        let record = response
            .result
            .iter()
            .find(|record| record["height"].as_u64() == Some(height))
            .ok_or_else(|| fail("The node did not return a state at that height."))?;
        let state = &record["currencystate"];
        if !state.is_object() {
            return Err(fail("The node returned no currency state at that height."));
        }
        let state_id = text(&state["currencyid"]).unwrap_or_default();
        if currency.starts_with('i') && currency.len() == 34 && state_id != currency {
            return Err(fail("The node returned the state of a different currency."));
        }
        // Record the state as the terminal does, so it can be opened when no source answers.
        if let (Some(cache), Some(receipt)) = (&self.cache, response.receipt.clone()) {
            let recorded = cache
                .record_historical_currency(
                    self.config.chain_cache_key(),
                    state_id.clone(),
                    response.source_id.clone(),
                    crate::cache::BlockContext {
                        height,
                        hash: block.result.hash.clone(),
                    },
                    state.clone(),
                    Value::Array(response.result.clone()),
                    receipt,
                )
                .await;
            if let Err(error) = recorded {
                tracing::debug!("historical currency state not recorded: {error:#}");
            }
        }
        Ok(CurrencyStateAt {
            currency_id: state_id,
            height,
            block_hash: block.result.hash.clone(),
            block_time: Some(block.result.time),
            data_source: source_of(&response.source_id),
            provenance: self.provenance(&response.source_id, Some(height)),
            source: response.source_id,
            tip_height: Some(tip),
            interpreted,
            coverage,
            supply: state["supply"].as_f64(),
            launch_label: currency_status::classify(state).label().into(),
            reserves: state_reserves(state),
            json: pretty(state),
        })
    }

    // ---------------------------------------------------------- currency activity

    /// Imports and exports of the currency in a window of at most 1,000 blocks that ends
    /// at or before the anchor, the block of the chosen state. The anchor must still be on
    /// the active chain before and after the request.
    pub async fn currency_activity(
        &self,
        currency: String,
        first: u64,
        last: u64,
        anchor_height: u64,
        anchor_hash: String,
    ) -> Outcome<ActivityPage> {
        let range = Range { first, last };
        range
            .validate()
            .map_err(|_| fail("A window can hold at most 1,000 blocks."))?;
        if last > anchor_height {
            return Err(fail("The window must end at or before the block of the state."));
        }
        let system = self
            .config
            .chain
            .system_id
            .get()
            .cloned()
            .ok_or_else(|| fail("The system ID of the chain is not known."))?;
        let anchor = |output: &crate::rpc::RpcCallOutput<BlockSummary>| {
            output.result.hash == anchor_hash && output.result.height == anchor_height
        };
        let before = self.settle(fetch_block(&self.client, BlockId::Height(anchor_height)).await)?;
        if !anchor(&before) {
            return Err(fail(
                "The block of the state is no longer on the active chain. Request the state again.",
            ));
        }
        let params = [json!(currency), json!(first), json!(last)];
        let refused = |method: &str, error: anyhow::Error| {
            fail(&format!(
                "The source did not answer the {method} request: {}",
                readable(&error)
            ))
        };
        let imports = self
            .settle(
                self.client
                    .call_nullable_with_raw::<Vec<Value>>("getimports", &params)
                    .await,
            )
            .map_err(|error| refused("getimports", error))?;
        let exports = self
            .settle(
                self.client
                    .call_nullable_with_raw::<Vec<Value>>("getexports", &params)
                    .await,
            )
            .map_err(|error| refused("getexports", error))?;
        let after = self.settle(fetch_block(&self.client, BlockId::Height(anchor_height)).await)?;
        if [&imports.source_id, &exports.source_id, &after.source_id]
            .iter()
            .any(|source| **source != before.source_id)
            || !anchor(&after)
        {
            return Err(fail(
                "The source or the active chain changed during the request. Try again.",
            ));
        }
        let (records, truncated) = currency_activity::records(
            &system,
            range,
            imports.result.as_deref().unwrap_or_default(),
            exports.result.as_deref().unwrap_or_default(),
        );
        Ok(ActivityPage {
            currency_id: currency,
            window: window(range),
            anchor_height,
            anchor_hash,
            source: before.source_id,
            records: records.iter().map(activity_record).collect(),
            truncated,
            older: range.older().map(window),
            newer: range.newer(anchor_height).map(window),
        })
    }

    /// The outputs that an import's or export's source export spent, from the export's
    /// declared input range only. Each can be the start of a transfer trace.
    pub async fn export_inputs(&self, record: ActivityRecord) -> Outcome<ExportInputs> {
        let export = record
            .export
            .clone()
            .ok_or_else(|| fail("This record has no reference to a source export."))?;
        let core = core_record(&record)?;
        let Some(client) = self.client_for_system(&export.system_id).ok() else {
            let name = self
                .system_name(&export.system_id)
                .unwrap_or_else(|| export.system_id.clone());
            return Err(fail(&format!(
                "The source export is on {name}. No node for that system is configured on this Mac."
            )));
        };
        let transaction = self.settle(client.get_raw_transaction(&export.txid).await)?;
        let (inputs, truncated) = currency_activity::export_inputs(&core, &transaction.result)?;
        Ok(ExportInputs {
            export,
            inputs: inputs.iter().map(output_ref).collect(),
            truncated,
            source: transaction.source_id,
        })
    }

    // ------------------------------------------------------------------ marketplace

    /// Open offers involving the chain's own coin, as Carbide requests them: one
    /// `getoffers` request for the native currency.
    pub async fn marketplace(&self) -> Outcome<Marketplace> {
        let subject = self.config.chain.name.clone();
        // The same parameters as `RpcClient::get_offers`, so the stored answer matches.
        let params = [json!(subject), json!(true), json!(false)];
        let output = self
            .observed("getoffers", &params, self.primary.get_offers(&subject, true).await)
            .await?;
        let mut offers = parse_offers(&output.result);
        let truncated = offers.len() > MAX_MARKET_OFFERS;
        offers.truncate(MAX_MARKET_OFFERS);
        Ok(Marketplace {
            subject,
            offers,
            source: output.source_id,
            truncated,
            provenance: output.provenance,
        })
    }

    /// When each offer was listed and the addresses its funding transaction pays.
    /// At most 64 transactions per request.
    pub async fn offer_funding(&self, txids: Vec<String>) -> Vec<OfferFunding> {
        let mut tasks = tokio::task::JoinSet::new();
        let permits = Arc::new(tokio::sync::Semaphore::new(LOOKUP_CONCURRENCY));
        for (position, txid) in txids.into_iter().take(MAX_FUNDING_LOOKUPS).enumerate() {
            let client = self.client.clone();
            let permits = permits.clone();
            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await;
                (position, txid.clone(), client.get_raw_transaction(&txid).await)
            });
        }
        let mut found = Vec::new();
        while let Some(Ok((position, txid, result))) = tasks.join_next().await {
            let funding = match self.settle(result) {
                Ok(output) => {
                    let raw = output.result;
                    let mut seen = Vec::new();
                    for output in raw["vout"].as_array().into_iter().flatten() {
                        for address in addresses(&output["scriptPubKey"]) {
                            if !seen.contains(&address) {
                                seen.push(address);
                            }
                        }
                    }
                    OfferFunding {
                        txid,
                        block_time: raw["blocktime"].as_u64(),
                        block_height: raw["height"].as_u64(),
                        addresses: seen,
                        error: None,
                    }
                }
                Err(error) => OfferFunding {
                    txid,
                    block_time: None,
                    block_height: None,
                    addresses: Vec::new(),
                    error: Some(readable(&error)),
                },
            };
            found.push((position, funding));
        }
        found.sort_by_key(|(position, _)| *position);
        found.into_iter().map(|(_, funding)| funding).collect()
    }

    /// When each offer was made and who holds the identity it sells, for the catalogue's
    /// filters. A bulk read: it never asks for permission and adds nothing to the cache.
    /// At most 256 offers per request.
    pub async fn offer_details(&self, offers: Vec<OfferKey>) -> Vec<OfferDetail> {
        let client = self.unrecorded_client();
        let mut tasks = tokio::task::JoinSet::new();
        let permits = Arc::new(tokio::sync::Semaphore::new(LOOKUP_CONCURRENCY));
        for (position, key) in offers.into_iter().take(MAX_DETAIL_LOOKUPS).enumerate() {
            let client = client.clone();
            let permits = permits.clone();
            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await;
                let funding = client.get_raw_transaction(&key.txid).await;
                let identity = match &key.identity_id {
                    Some(id) => Some(client.call_with_raw::<Value>("getidentity", &[json!(id)]).await),
                    None => None,
                };
                (position, key.txid, funding, identity)
            });
        }
        let mut found = Vec::new();
        while let Some(Ok((position, txid, funding, identity))) = tasks.join_next().await {
            let mut detail = OfferDetail {
                txid,
                block_height: None,
                block_time: None,
                holder: None,
                error: None,
            };
            match self.settle(funding) {
                Ok(output) => {
                    detail.block_height = output.result["height"].as_u64();
                    detail.block_time = output.result["blocktime"].as_u64();
                }
                Err(error) => detail.error = Some(readable(&error)),
            }
            if let Some(identity) = identity {
                match self.settle(identity) {
                    Ok(output) => detail.holder = identity_holder(&output.result),
                    Err(error) => detail.error = Some(readable(&error)),
                }
            }
            found.push((position, detail));
        }
        found.sort_by_key(|(position, _)| *position);
        found.into_iter().map(|(_, detail)| detail).collect()
    }

    // --------------------------------------------------------------- notarizations

    /// The perspectives for a currency's notarizations: this chain's node on the
    /// currency, and for a known PBaaS pair also the other chain's node on this chain.
    pub fn notarization_perspectives(
        &self,
        subject_id: String,
        subject_name: Option<String>,
        options: Option<u64>,
        definition_system: Option<String>,
    ) -> Vec<NotarizationPerspective> {
        let observer = self.config.chain_cache_key();
        let network = Network::from_testnet(self.config.chain.testnet);
        let parent = crate::world::system_id("VRSC", network)
            .map(|id| id.to_string())
            .unwrap_or_default();
        perspective_pairs(
            &observer,
            &subject_id,
            &parent,
            options,
            definition_system.as_deref(),
        )
        .into_iter()
        .map(|(observer_id, subject)| {
            let name = |id: &str| {
                if id == subject_id
                    && let Some(name) = &subject_name
                {
                    return name.clone();
                }
                self.system_name(id).unwrap_or_else(|| id.to_string())
            };
            NotarizationPerspective {
                observer_name: name(&observer_id),
                subject_name: name(&subject),
                local: observer_id == self.config.chain_cache_key(),
                configured: self.config_for_system(&observer_id).is_some(),
                observer_id,
                subject_id: subject,
            }
        })
        .collect()
    }

    /// The notarizations `observer`'s node reports for `subject`, with inclusion and
    /// foreign proof roots checked on each owning chain's source when `check` is true.
    pub async fn notarization_browser(
        &self,
        observer_id: String,
        subject_id: String,
        subject_name: Option<String>,
        check: bool,
    ) -> Outcome<NotarizationBrowser> {
        let perspective = NotarizationPerspective {
            observer_name: self
                .system_name(&observer_id)
                .unwrap_or_else(|| observer_id.clone()),
            subject_name: subject_name
                .or_else(|| self.system_name(&subject_id))
                .unwrap_or_else(|| subject_id.clone()),
            local: observer_id == self.config.chain_cache_key(),
            configured: self.config_for_system(&observer_id).is_some(),
            observer_id: observer_id.clone(),
            subject_id: subject_id.clone(),
        };
        let unavailable = |perspective: NotarizationPerspective, message: String| NotarizationBrowser {
            perspective,
            load: BrowserLoad::Unavailable,
            message: Some(message),
            rows: Vec::new(),
            last_confirmed: None,
            unconfirmed: 0,
            fork_count: 0,
            forks: Vec::new(),
            best_fork: None,
            source: None,
            checks_limited: false,
            json: String::new(),
            provenance: None,
        };
        let client = match self.client_for_system(&observer_id) {
            Ok(client) => client,
            Err(status) => {
                let message = match status {
                    super::sources::ProofRootStatus::NoSourceEnabled => format!(
                        "Each source for {} is off. Turn one on in Sources to ask that chain about {}.",
                        perspective.observer_name, perspective.subject_name
                    ),
                    _ => format!(
                        "No node for {} is configured on this Mac. The app cannot ask that chain about {}.",
                        perspective.observer_name, perspective.subject_name
                    ),
                };
                return Ok(unavailable(perspective, message));
            }
        };
        let params = [json!(subject_id), json!(false), json!(false)];
        let live = client
            .call_with_raw::<Value>("getnotarizationdata", &params)
            .await;
        // Only this chain's answers are stored under this chain, so only the local view
        // can fall back to a stored observation.
        let answer = if perspective.local {
            self.observed("getnotarizationdata", &params, live).await
        } else {
            self.settle(live).map(|output| super::Answer {
                provenance: self.provenance(&output.source_id, None),
                result: output.result,
                response_json: output.response_json,
                source_id: output.source_id,
            })
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err(error) => {
                let message = format!(
                    "The {} node did not answer the notarization request: {}",
                    perspective.observer_name,
                    readable(&error)
                );
                return Ok(unavailable(perspective, message));
            }
        };
        let source = answer.source_id.clone();
        let provenance = answer.provenance;
        // Checks of a stored view would compare old rows with blocks now.
        let check = check && !provenance.retained;
        let raw = answer.result;
        if raw.is_null() {
            return Ok(NotarizationBrowser {
                load: BrowserLoad::NoData,
                message: Some("The node returned no notarization data.".into()),
                source: Some(source),
                provenance: Some(provenance),
                ..unavailable(perspective, String::new())
            });
        }
        let graph = match decode_graph(&raw, &observer_id) {
            Ok(graph) => graph,
            Err(error) => return Ok(unavailable(perspective, readable(&error))),
        };

        // Inclusion on the observer, foreign roots and foreign previous outputs, each once.
        let mut inclusions: HashMap<String, Check<Header>> = HashMap::new();
        let mut roots: HashMap<(String, String), Check<Header>> = HashMap::new();
        let mut previous: HashMap<OutpointRef, Check<()>> = HashMap::new();
        let mut limited = false;
        if check {
            let mut wanted_blocks = Vec::new();
            let mut wanted_roots = Vec::new();
            let mut wanted_previous = Vec::new();
            let mut seen = HashSet::new();
            // The confirmed row first, then the newest rows.
            let mut order: Vec<&GraphRow> = graph.rows.iter().collect();
            order.sort_by_key(|row| (Some(row.index) != graph.confirmed, std::cmp::Reverse(row.index)));
            for row in order {
                if let Some(block) = &row.block
                    && seen.insert(("inclusion".to_string(), block.clone()))
                {
                    wanted_blocks.push(block.clone());
                }
                for root in row.roots.iter().filter(|root| root.system != observer_id) {
                    if let Some(hash) = &root.hash
                        && seen.insert((root.system.clone(), hash.clone()))
                    {
                        wanted_roots.push((root.system.clone(), hash.clone()));
                    }
                }
                if let Some(target) = &row.foreign_previous
                    && seen.insert(("previous".into(), format!("{}:{}", target.txid, target.vout)))
                {
                    wanted_previous.push(target.clone());
                }
            }
            let total = wanted_blocks.len() + wanted_roots.len() + wanted_previous.len();
            limited = total > MAX_NOTARIZATION_LOOKUPS;
            let mut budget = MAX_NOTARIZATION_LOOKUPS;
            wanted_blocks.truncate(budget);
            budget -= wanted_blocks.len();
            wanted_roots.truncate(budget);
            budget -= wanted_roots.len();
            wanted_previous.truncate(budget);

            inclusions = self.inclusion_checks(&client, &source, wanted_blocks).await;
            roots = self.root_checks(wanted_roots).await;
            previous = self.previous_checks(wanted_previous).await;
        }

        let name = |id: &str| self.system_name(id);
        let rows = graph
            .rows
            .iter()
            .map(|row| {
                let inclusion = row.block.as_ref().and_then(|block| inclusions.get(block));
                let (included_height, included_time) = match inclusion {
                    Some(Check::Found { value, .. }) => (Some(value.height), value.time),
                    _ => (None, None),
                };
                // Ported from `notarization_ui::inclusion_text`.
                let inclusion_text = match inclusion {
                    Some(Check::Found { value, .. }) => match value.time {
                        Some(_) => format!("#{}", value.height),
                        None => format!("#{} at time unavailable", value.height),
                    },
                    None if row.block.is_some() && check => "Not checked".into(),
                    None if row.block.is_some() => "awaiting block lookup".into(),
                    _ => "unavailable".into(),
                };
                let proof_roots = row
                    .roots
                    .iter()
                    .map(|root| {
                        let foreign = root.system != observer_id;
                        let found = root
                            .hash
                            .as_ref()
                            .and_then(|hash| roots.get(&(root.system.clone(), hash.clone())));
                        let status = if foreign {
                            classify_root(found, root)
                        } else {
                            None
                        };
                        let status = match status {
                            Some((RootCheck::NotChecked, _)) if check && limited => Some((
                                RootCheck::NotChecked,
                                "Not checked. The app reached the request limit.".into(),
                            )),
                            Some((RootCheck::NotChecked, _)) if !check => {
                                Some((RootCheck::NotChecked, "Not checked".into()))
                            }
                            other => other,
                        };
                        let checked_on = match found {
                            Some(Check::Found { source, .. } | Check::NotFound { source, .. }) => {
                                Some(source.clone())
                            }
                            _ => None,
                        };
                        ProofRootRow {
                            system_id: root.system.clone(),
                            system_name: name(&root.system),
                            height: root.height,
                            block_hash: root.hash.clone(),
                            foreign,
                            possible_fork: matches!(
                                status,
                                Some((RootCheck::KnownOffChain | RootCheck::NotFound, _))
                            ),
                            resolved: matches!(
                                (found, root.height),
                                (Some(Check::Found { value, .. }), Some(height)) if value.height == height
                            ),
                            check: status.as_ref().map(|(check, _)| check.clone()),
                            status: status.map(|(_, text)| text),
                            checked_on,
                        }
                    })
                    .collect();
                let previous = row.foreign_previous.as_ref().map(|target| {
                    let (found, status) = match previous.get(target) {
                        Some(Check::Found { source, .. }) => (true, format!("Present on {source}")),
                        Some(Check::Unavailable(reason)) => (false, reason.label().into()),
                        Some(Check::NotFound { .. }) => {
                            (false, "Referenced outpoint not found".into())
                        }
                        None => (false, "Not checked".into()),
                    };
                    PreviousCheck {
                        output: output_ref(target),
                        system_name: name(target.system_id.as_ref()),
                        found,
                        status,
                    }
                });
                BrowserNotarization {
                    index: row.index as u32,
                    txid: row.txid.clone(),
                    output: row.vout,
                    block_hash: row.block.clone(),
                    included_height,
                    included_time,
                    inclusion: inclusion_text,
                    confirmed: Some(row.index) == graph.confirmed,
                    synthetic: row.synthetic,
                    on_best_chain: graph
                        .best
                        .as_ref()
                        .is_none_or(|best| best.contains(&(row.index as u64))),
                    roots: proof_roots,
                    previous,
                }
            })
            .collect();
        Ok(NotarizationBrowser {
            perspective,
            load: BrowserLoad::Ready,
            message: None,
            rows,
            last_confirmed: graph.confirmed.map(|index| index as u32),
            unconfirmed: graph.pending() as u32,
            fork_count: graph.forks.len() as u32,
            forks: graph
                .forks
                .iter()
                .map(|fork| fork.iter().map(|index| *index as u32).collect())
                .collect(),
            best_fork: graph.best_fork.map(|index| index as u32),
            source: Some(source),
            checks_limited: limited,
            json: pretty(&without(&raw, "notarizations")),
            provenance: Some(provenance),
        })
    }

    /// The notarizations of `subject` on this chain that lead to `txid`, from `txid`
    /// back. Each entry is one transaction that the node has, with its block. The record
    /// stops at the limit, or where the node does not have the transaction before.
    pub async fn notarization_record(&self, subject_id: String, txid: String) -> Vec<RecordPoint> {
        let mut record = Vec::new();
        let mut lookups = 1;
        let Ok(mut raw) = self
            .settle(self.client.get_raw_transaction(&txid).await)
            .map(|output| output.result)
        else {
            return record;
        };
        let mut current = txid;
        loop {
            let Some(height) = raw["height"].as_u64().filter(|_| raw["blockhash"].is_string()) else {
                break;
            };
            record.push(RecordPoint {
                txid: current.clone(),
                height,
                time: raw["blocktime"].as_u64().filter(|time| *time > 0),
            });
            if record.len() >= MAX_RECORD_POINTS {
                break;
            }
            let mut next = None;
            for (candidate, output) in previous_notarization_candidates(&raw, &subject_id) {
                if lookups >= MAX_RECORD_LOOKUPS {
                    break;
                }
                lookups += 1;
                let Ok(found) = self.settle(self.client.get_raw_transaction(&candidate).await) else {
                    continue;
                };
                if notarization_output(&found.result, &subject_id).is_some_and(|(n, _)| n == output) {
                    next = Some((candidate, found.result));
                    break;
                }
            }
            let Some((candidate, found)) = next else { break };
            current = candidate;
            raw = found;
        }
        self.forgive();
        record
    }

    // -------------------------------------------------------------------- bookmarks

    /// The bookmark file, with the tags and observation source of each cached object.
    pub async fn bookmark_file(&self) -> Outcome<BookmarkFile> {
        let book = self.book()?;
        let cache_path = self.config.cache_path.clone();
        let bookmarks: Vec<crate::bookmarks::Bookmark> =
            book.entries.iter().map(|(_, bookmark)| bookmark.clone()).collect();
        let cached = tokio::task::spawn_blocking(move || {
            let cache = crate::cache::Cache::open(&cache_path).map_err(|e| readable(&e));
            bookmarks
                .iter()
                .map(|bookmark| match &cache {
                    Ok(cache) => crate::bookmarks::cached(cache, bookmark).map_err(|e| readable(&e)),
                    Err(error) => Err(error.clone()),
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|_| fail("The cache did not answer."))?;
        let failures = population_failures()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let this_chain = self.config.chain_cache_key();
        let entries = book
            .entries
            .iter()
            .zip(cached)
            .enumerate()
            .map(|(index, ((_, bookmark), cached))| {
                let cache = match cached {
                    Ok(Some(snapshot)) => BookmarkCache::Cached {
                        tags: snapshot_tags(&snapshot),
                        source: snapshot.source,
                    },
                    Ok(None) => match failures.get(&bookmark_key(bookmark)) {
                        Some(message) => BookmarkCache::Failed {
                            message: message.clone(),
                        },
                        None => BookmarkCache::Uncached,
                    },
                    Err(message) => BookmarkCache::Failed { message },
                };
                BookmarkEntry {
                    index: index as u32,
                    kind: bookmark.kind.label().into(),
                    system_id: bookmark.system_id.to_string(),
                    system_name: self.system_name(bookmark.system_id.as_ref()),
                    this_chain: bookmark.system_id.as_ref() == this_chain,
                    target: bookmark.target.clone(),
                    description: bookmark.description.clone(),
                    created_at: bookmark.created_at,
                    cache,
                }
            })
            .collect();
        let path = settings_path();
        let settings = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok());
        let (options, settings_issue) = match startup_options(settings.as_deref()) {
            Ok(options) => (options, None),
            Err(error) => (Default::default(), Some(readable(&error))),
        };
        Ok(BookmarkFile {
            entries,
            path: book.path.display().to_string(),
            issues: book.issues.clone(),
            startup_enabled: options.fetch_missing_on_startup,
            startup_limit: options.startup_limit.min(STARTUP_LIMIT_CAP) as u32,
            settings_issue,
            settings_path: path.map(|p| p.display().to_string()).unwrap_or_default(),
            startup_report: population_report()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        })
    }

    /// Saves a bookmark of any kind, including `notarizations` and `auto`. `system_id`
    /// is the observing chain; `None` uses this chain.
    pub fn save_bookmark(
        &self,
        system_id: Option<String>,
        kind: String,
        target: String,
        description: String,
    ) -> Outcome<()> {
        let kind = bookmark_kind(&kind).ok_or_else(|| fail("The bookmark kind is not known."))?;
        let system_id = match system_id.map(|id| id.trim().to_string()).filter(|id| !id.is_empty()) {
            Some(id) => SystemId::try_from(id).map_err(|_| fail("The system ID is not valid."))?,
            None => self
                .config
                .chain
                .system_id
                .get()
                .cloned()
                .ok_or_else(|| fail("The system ID of the chain is not known."))?,
        };
        let bookmark =
            crate::bookmarks::Bookmark::new(system_id, kind, target.trim().to_string(), description)?;
        Ok(self.book()?.add(bookmark)?)
    }

    /// Carbide's optional start-up population: when `[bookmarks]
    /// fetch_missing_on_startup` is set in Carbide's `config.toml`, requests up to
    /// `startup_limit` (at most 100) uncached bookmarks one at a time, each with a
    /// 70-second limit. It uses only configured sources and never asks for permission.
    pub async fn populate_bookmarks(&self) -> Outcome<PopulationReport> {
        let settings = settings_path().and_then(|path| std::fs::read_to_string(path).ok());
        let options = startup_options(settings.as_deref())?;
        let limit = options.startup_limit.min(STARTUP_LIMIT_CAP);
        // Start-up population runs once per process, as in the terminal. A second request,
        // from another place in the app, does nothing.
        if options.fetch_missing_on_startup && POPULATION_STARTED.swap(true, Ordering::AcqRel) {
            return Ok(PopulationReport {
                enabled: true,
                limit: limit as u32,
                attempted: 0,
                fetched: 0,
                message: population_report()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone()
                    .unwrap_or_else(|| "Start-up population is running.".into()),
            });
        }
        if !options.fetch_missing_on_startup {
            return Ok(PopulationReport {
                enabled: false,
                limit: limit as u32,
                attempted: 0,
                fetched: 0,
                message: "Start-up population is off.".into(),
            });
        }
        let book = self.book()?;
        let cache = crate::cache::Cache::open(&self.config.cache_path)?;
        let mut attempted = 0;
        let mut fetched = 0;
        for (_, bookmark) in &book.entries {
            if !matches!(crate::bookmarks::cached(&cache, bookmark), Ok(None)) {
                continue;
            }
            if attempted >= limit {
                break;
            }
            let Some(mut config) = self.config_for_system(bookmark.system_id.as_ref()) else {
                continue;
            };
            attempted += 1;
            config.daemon.background = true;
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(STARTUP_TIMEOUT_SECONDS),
                crate::bookmarks::fetch(config.noninteractive(), bookmark, false),
            )
            .await
            .map_err(|_| "The start-up request timed out.".to_string())
            .and_then(|result| result.map_err(|error| readable(&error)));
            self.forgive();
            let mut failures = population_failures().lock().unwrap_or_else(|e| e.into_inner());
            match result {
                Ok(_) => {
                    fetched += 1;
                    failures.remove(&bookmark_key(bookmark));
                }
                Err(message) => {
                    failures.insert(bookmark_key(bookmark), message);
                }
            }
        }
        let message =
            format!("Start-up bookmarks: {fetched} fetched, {attempted} attempted (limit {limit})");
        *population_report().lock().unwrap_or_else(|e| e.into_inner()) = Some(message.clone());
        Ok(PopulationReport {
            enabled: true,
            limit: limit as u32,
            attempted: attempted as u32,
            fetched: fetched as u32,
            message,
        })
    }
}

impl Explorer {
    /// The latest stored state of the currency at a height, served because no source
    /// answered. Carbide records each exact state that a source reports.
    async fn stored_state_at(
        &self,
        currency: &str,
        target: SeekTarget,
        failure: &anyhow::Error,
    ) -> Outcome<CurrencyStateAt> {
        let why = readable(failure);
        let Some(height) = stored_height(target) else {
            return Err(fail(&format!(
                "No source answered: {why} A time request needs a source. A stored state opens by its block height."
            )));
        };
        let path = self.config.cache_path.clone();
        let system = self.config.chain_cache_key();
        let id = currency.to_string();
        let states = tokio::task::spawn_blocking(move || stored_states(&path, &system, &id))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
        let Some(found) = states.iter().find(|stored| stored.height == height) else {
            let mut heights: Vec<u64> = states.iter().map(|stored| stored.height).collect();
            heights.sort_unstable();
            heights.dedup();
            let known = if heights.is_empty() {
                String::new()
            } else {
                format!(
                    " Stored states exist at blocks {}.",
                    heights
                        .iter()
                        .take(20)
                        .map(u64::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return Err(fail(&format!(
                "No source answered: {why} No stored state of this currency exists at block {height}.{known}"
            )));
        };
        let block_time = found
            .response
            .as_array()
            .into_iter()
            .flatten()
            .find(|record| record["height"].as_u64() == Some(height))
            .and_then(|record| record["blocktime"].as_u64());
        let mut provenance = self.provenance(&found.source, Some(height));
        provenance.retained = true;
        provenance.observed_at = found.observed_at;
        provenance.failure = Some(why);
        provenance.caution = Some(
            "The state is the one this source reported for this block. The app cannot check now that the block is still on the active chain."
                .into(),
        );
        let state = &found.state;
        Ok(CurrencyStateAt {
            currency_id: text(&state["currencyid"]).unwrap_or_else(|| currency.to_string()),
            height,
            block_hash: found.block_hash.clone(),
            block_time,
            source: found.source.clone(),
            data_source: source_of(&found.source),
            tip_height: None,
            interpreted: format!("Block height {height}, stored"),
            coverage: None,
            supply: state["supply"].as_f64(),
            launch_label: currency_status::classify(state).label().into(),
            reserves: state_reserves(state),
            json: pretty(state),
            provenance,
        })
    }

    /// The configuration for a system: this chain or another chain configured here.
    fn config_for_system(&self, system: &str) -> Option<Config> {
        self.all_configs()
            .into_iter()
            .find(|config| config.chain_cache_key() == system)
    }

    /// A known name for a system or currency: a resolved name, a configured chain's name,
    /// or the root chain's name.
    fn system_name(&self, id: &str) -> Option<String> {
        if let Some(name) = self.cached_name(id) {
            return Some(name.trim_end_matches('@').to_string());
        }
        let network = Network::from_testnet(self.config.chain.testnet);
        if crate::world::system_id("VRSC", network).is_ok_and(|parent| parent.as_ref() == id) {
            return Some(if self.config.chain.testnet { "VRSCTEST" } else { "VRSC" }.into());
        }
        // A PBaaS chain's folder is named by a hash, which is not a name.
        self.other_chains
            .iter()
            .find(|config| config.chain_cache_key() == id)
            .map(|config| config.chain.name.clone())
            .filter(|name| !(name.len() == 40 && name.bytes().all(|c| c.is_ascii_hexdigit())))
    }

    /// Ported from `notarization_ui::inclusion`: the block header on the observer's source.
    async fn inclusion_checks(
        &self,
        client: &RpcClient,
        source: &str,
        blocks: Vec<String>,
    ) -> HashMap<String, Check<Header>> {
        let mut tasks = tokio::task::JoinSet::new();
        let permits = Arc::new(tokio::sync::Semaphore::new(LOOKUP_CONCURRENCY));
        for block in blocks {
            let client = client.clone();
            let permits = permits.clone();
            let source = source.to_string();
            tasks.spawn(async move {
                let _permit = permits.acquire_owned().await;
                for (method, verbosity) in [("getblockheader", json!(true)), ("getblock", json!(1))] {
                    if let Ok(value) = client.call_on(&source, method, &[json!(block), verbosity]).await
                        && let Some(header) = header_of(&value, &block).filter(|h| h.time.is_some())
                    {
                        return (block, Check::Found { source, value: header });
                    }
                }
                (block, Check::Unavailable(CheckFailure::BlockLookupFailed))
            });
        }
        let mut found = HashMap::new();
        while let Some(Ok((block, check))) = tasks.join_next().await {
            found.insert(block, check);
        }
        self.forgive();
        found
    }

    /// Ported from `notarization_ui::foreign_client`: a client for an enabled source of
    /// the owning chain.
    fn foreign_client(&self, system: &str) -> Result<RpcClient, CheckFailure> {
        // The sources module owns which chains are configured and enabled.
        self.client_for_system(system).map_err(|status| match status {
            super::sources::ProofRootStatus::NoSourceConfigured => {
                CheckFailure::NoSourceConfigured
            }
            super::sources::ProofRootStatus::NoSourceEnabled => CheckFailure::NoSourceEnabled,
            _ => CheckFailure::ChainQueryFailed,
        })
    }

    /// Ported from `notarization_ui::peer_source`: the lookup and its tip are pinned to
    /// the same source.
    async fn peer_source(&self, system: &str) -> Result<(RpcClient, String, Option<u64>), CheckFailure> {
        let client = self.foreign_client(system)?;
        let tip = client
            .get_blockchain_info()
            .await
            .map_err(|_| CheckFailure::ChainQueryFailed)?;
        Ok((client, tip.source_id, tip.result["blocks"].as_u64()))
    }

    /// Ported from `notarization_ui::foreign_root`: the exact block hash on an enabled
    /// source for the owning chain.
    async fn root_checks(
        &self,
        roots: Vec<(String, String)>,
    ) -> HashMap<(String, String), Check<Header>> {
        let mut by_system: HashMap<String, Vec<String>> = HashMap::new();
        for (system, hash) in roots {
            by_system.entry(system).or_default().push(hash);
        }
        let mut found = HashMap::new();
        for (system, hashes) in by_system {
            let peer = self.peer_source(&system).await;
            let mut tasks = tokio::task::JoinSet::new();
            let permits = Arc::new(tokio::sync::Semaphore::new(LOOKUP_CONCURRENCY));
            for hash in hashes {
                let (client, source, tip) = match &peer {
                    Ok((client, source, tip)) => (client.clone(), source.clone(), *tip),
                    Err(reason) => {
                        found.insert((system.clone(), hash), Check::Unavailable(*reason));
                        continue;
                    }
                };
                let permits = permits.clone();
                tasks.spawn(async move {
                    let _permit = permits.acquire_owned().await;
                    let check = match client
                        .call_on(&source, "getblockheader", &[json!(hash), json!(true)])
                        .await
                    {
                        Ok(value) => header_of(&value, &hash).map_or(
                            Check::Unavailable(CheckFailure::InvalidHeader),
                            |value| Check::Found {
                                source: source.clone(),
                                value,
                            },
                        ),
                        Err(error)
                            if error
                                .downcast_ref::<crate::rpc::ApplicationError>()
                                .is_some_and(|e| e.is_block_not_found()) =>
                        {
                            Check::NotFound { source, tip }
                        }
                        Err(_) => Check::Unavailable(CheckFailure::BlockLookupFailed),
                    };
                    (hash, check)
                });
            }
            while let Some(Ok((hash, check))) = tasks.join_next().await {
                found.insert((system.clone(), hash), check);
            }
        }
        self.forgive();
        found
    }

    /// Ported from `notarization_ui::foreign_transaction`.
    async fn previous_checks(&self, targets: Vec<OutpointRef>) -> HashMap<OutpointRef, Check<()>> {
        let mut found = HashMap::new();
        for target in targets {
            let check = match self.foreign_client(target.system_id.as_ref()) {
                Err(reason) => Check::Unavailable(reason),
                Ok(client) => match client.get_raw_transaction(&target.txid).await {
                        Ok(result)
                            if result.result["txid"].as_str() == Some(&target.txid)
                                && result.result["vout"].as_array().is_some_and(|outs| {
                                    outs.iter().any(|out| {
                                        out["n"].as_u64() == Some(u64::from(target.vout))
                                    })
                                }) =>
                        {
                            Check::Found {
                                source: result.source_id,
                                value: (),
                            }
                        }
                        _ => Check::Unavailable(CheckFailure::TransactionLookupFailed),
                },
            };
            found.insert(target, check);
        }
        self.forgive();
        found
    }
}

#[cfg(test)]
#[path = "ffi_currency_tests.rs"]
mod tests;
