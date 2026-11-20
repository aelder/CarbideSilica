//! Reusable time parsing and bounded navigation by recorded block-header time.
//!
//! Header times are not sorted. A binary bracket plus a contiguous neighborhood
//! gives useful navigation without scanning the entire chain. Callers receive
//! the coverage explicitly: a non-exact match is not a global nearest-time proof.
use std::collections::BTreeMap;
use std::future::Future;

use anyhow::{Context, Result, bail, ensure};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use serde_json::{Value, json};

use crate::{cache::Cache, config::Config, rpc::RpcClient};

pub fn parse(input: &str) -> Result<DateTime<Utc>> {
    parse_at(input, Utc::now())
}

/// UTC is the default for both date-only and timezone-free date/time input.
/// Relative expressions are anchored once to the supplied UTC reference time.
pub fn parse_at(input: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let input = input.trim();
    ensure!(
        !input.is_empty(),
        "Enter a date/time or Unix timestamp after time:"
    );
    if let Ok(seconds) = input.parse::<i64>() {
        ensure!(
            input.trim_start_matches(['-', '+']).len() <= 11,
            "Use Unix seconds, not milliseconds"
        );
        return DateTime::from_timestamp(seconds, 0)
            .context("Unix timestamp is out of range (use seconds)");
    }
    if let Ok(value) = DateTime::parse_from_rfc3339(input) {
        ensure!(
            value.timestamp_subsec_nanos() == 0,
            "Use whole seconds for block-time lookup"
        );
        return Ok(value.with_timezone(&Utc));
    }
    if let Ok(value) = DateTime::parse_from_rfc2822(input) {
        return Ok(value.with_timezone(&Utc));
    }
    let mut text = input
        .to_ascii_lowercase()
        .replace(',', "")
        .replace(" at ", " ");
    text = text.strip_prefix("on ").unwrap_or(&text).to_string();
    let parts = text
        .split_whitespace()
        .map(|word| {
            for suffix in ["st", "nd", "rd", "th"] {
                if let Some(number) = word.strip_suffix(suffix)
                    && !number.is_empty()
                    && number.chars().all(|c| c.is_ascii_digit())
                {
                    return number.to_string();
                }
            }
            word.to_string()
        })
        .collect::<Vec<_>>();
    text = parts.join(" ");
    if text.as_bytes().get(10) == Some(&b't') && text.as_bytes().get(4) == Some(&b'-') {
        text.replace_range(10..11, " ");
    }
    if text == "now" {
        return DateTime::from_timestamp(now.timestamp(), 0)
            .context("Reference time is out of range");
    }
    let midnight = |date: NaiveDate| date.and_hms_opt(0, 0, 0).unwrap().and_utc();
    for (word, days) in [("today", 0), ("yesterday", -1), ("tomorrow", 1)] {
        if text == word || text.starts_with(&format!("{word} ")) {
            let date = now
                .date_naive()
                .checked_add_signed(Duration::days(days))
                .context("Date is out of range")?;
            if text == word {
                return Ok(midnight(date));
            }
            text = format!("{} {}", date, text[word.len()..].trim());
            break;
        }
    }
    let words = text.split_whitespace().collect::<Vec<_>>();
    let future = match words.as_slice() {
        ["in", amount, unit] => Some((*amount, *unit)),
        [amount, unit, "from", "now"] => Some((*amount, *unit)),
        _ => None,
    };
    if let Some((amount, unit)) = future {
        let amount: u32 = amount
            .parse()
            .context("Use a nonnegative numeric future duration")?;
        let unit = unit.trim_end_matches('s');
        if matches!(unit, "month" | "year") {
            let months = amount
                .checked_mul(if unit == "year" { 12 } else { 1 })
                .context("Duration is out of range")?;
            return now
                .date_naive()
                .checked_add_months(chrono::Months::new(months))
                .map(midnight)
                .context("Date is out of range");
        }
        let multiplier = match unit {
            "second" => 1,
            "minute" => 60,
            "hour" => 3600,
            "day" => 86400,
            "week" => 604800,
            _ => bail!("Use seconds, minutes, hours, days, weeks, months or years"),
        };
        let reference = if matches!(unit, "day" | "week") {
            midnight(now.date_naive())
        } else {
            now
        };
        return reference
            .checked_add_signed(Duration::seconds(i64::from(amount) * multiplier))
            .and_then(|t| DateTime::from_timestamp(t.timestamp(), 0))
            .context("Date is out of range");
    }
    if let [amount, unit, "ago"] = words.as_slice() {
        let amount: i64 = amount
            .parse()
            .context("Use a numeric relative duration, e.g. 2 hours ago")?;
        ensure!(amount >= 0, "Relative durations must be nonnegative");
        let multiplier = match unit.trim_end_matches('s') {
            "second" => 1,
            "minute" => 60,
            "hour" => 3600,
            "day" => 86400,
            "week" => 604800,
            _ => bail!("Use seconds, minutes, hours, days or weeks before ago"),
        };
        let seconds = amount
            .checked_mul(multiplier)
            .context("Duration is out of range")?;
        let reference = if matches!(unit.trim_end_matches('s'), "day" | "week") {
            midnight(now.date_naive())
        } else {
            now
        };
        return reference
            .checked_sub_signed(Duration::try_seconds(seconds).context("Duration is out of range")?)
            .and_then(|time| DateTime::from_timestamp(time.timestamp(), 0))
            .context("Date is out of range");
    }
    // Explicit numeric offsets are unambiguous; timezone abbreviations other than
    // UTC/GMT are deliberately not interpreted as the machine's local timezone.
    for format in [
        "%Y-%m-%d %H:%M:%S %:z",
        "%Y-%m-%d %H:%M %:z",
        "%Y-%m-%d %H:%M:%S %z",
        "%Y-%m-%d %H:%M %z",
    ] {
        if let Ok(value) = DateTime::parse_from_str(&text, format) {
            return Ok(value.with_timezone(&Utc));
        }
    }
    let mut text = text
        .strip_suffix(" utc")
        .or_else(|| text.strip_suffix(" gmt"))
        .unwrap_or(&text)
        .to_string();
    for suffix in ["am", "pm"] {
        if let Some(before) = text.strip_suffix(suffix)
            && let Some((date, hour)) = before.trim_end().rsplit_once(' ')
            && !hour.is_empty()
            && hour.chars().all(|c| c.is_ascii_digit())
        {
            text = format!("{date} {hour}:00 {suffix}");
            break;
        }
    }
    for date in [
        "%Y-%m-%d", "%Y/%m/%d", "%B %d %Y", "%b %d %Y", "%d %B %Y", "%d %b %Y",
    ] {
        if let Ok(value) = NaiveDate::parse_from_str(&text, date) {
            return Ok(midnight(value));
        }
        for time in [
            "%H:%M:%S",
            "%H:%M",
            "%I:%M:%S %p",
            "%I:%M %p",
            "%I %p",
            "%I:%M%p",
            "%I%p",
        ] {
            if let Ok(value) = NaiveDateTime::parse_from_str(&text, &format!("{date} {time}")) {
                return Ok(value.and_utc());
            }
        }
    }
    bail!(
        "Cannot interpret date/time; numeric dates require year first. Try 2026-11-07 or Nov 7 2026 2pm UTC."
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeBlock {
    pub height: u64,
    pub hash: String,
    pub parent: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone)]
pub struct LocatedBlock {
    pub block: TimeBlock,
    pub target: i64,
    pub searched: (u64, u64),
    pub observed_tip: TimeBlock,
    pub exact_timestamp: bool,
    pub whole_chain: bool,
    pub probes: usize,
    pub source: String,
}
impl LocatedBlock {
    pub fn description(&self) -> String {
        let target = DateTime::from_timestamp(self.target, 0)
            .map(|time| time.to_rfc3339())
            .unwrap_or_else(|| self.target.to_string());
        let delta = i128::from(self.block.timestamp) - i128::from(self.target);
        let quality = if self.exact_timestamp {
            "exact timestamp".to_string()
        } else if self.whole_chain {
            "nearest block".to_string()
        } else {
            format!("nearest of {} checked blocks", self.probes)
        };
        format!(
            "{target} -> #{} ({delta:+}s; {quality}; window #{}-#{})",
            self.block.height, self.searched.0, self.searched.1
        )
    }
}

/// Adapter seam for other explorers, retained indexes and deterministic tests.
pub trait TimeSource {
    fn block_at(&mut self, height: u64) -> impl Future<Output = Result<TimeBlock>> + Send;
}

/// Serial binary bracketing followed by a contiguous +/-64-height scan. Ties
/// prefer the earlier timestamp, then the lower height. Exact timestamp matches
/// are globally minimal in distance; ties outside coverage are not enumerated.
pub async fn locate(
    source: &mut impl TimeSource,
    target: i64,
    tip: TimeBlock,
) -> Result<LocatedBlock> {
    const RADIUS: u64 = 64;
    let mut seen = BTreeMap::new();
    seen.insert(tip.height, tip.clone());
    let mut low = 0;
    let mut high = tip.height;
    while low < high {
        let mid = low + (high - low) / 2;
        let block = observe(source, &mut seen, mid).await?;
        if block.timestamp < target {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let first = low.saturating_sub(RADIUS);
    let last = low.saturating_add(RADIUS).min(tip.height);
    for height in first..=last {
        let block = observe(source, &mut seen, height).await?;
        if height > first {
            ensure!(
                block.parent.as_deref() == Some(&seen[&(height - 1)].hash),
                "Chain changed during time lookup; retry"
            );
        }
    }
    // Include binary probes too: they may contain closer out-of-order times.
    let block = seen
        .values()
        .min_by_key(|block| {
            (
                (i128::from(block.timestamp) - i128::from(target)).abs(),
                block.timestamp,
                block.height,
            )
        })
        .context("No block times available")?
        .clone();
    Ok(LocatedBlock {
        exact_timestamp: block.timestamp == target,
        whole_chain: seen.len() as u64 == tip.height.saturating_add(1),
        probes: seen.len(),
        block,
        target,
        searched: (first, last),
        observed_tip: tip,
        source: String::new(),
    })
}

async fn observe(
    source: &mut impl TimeSource,
    seen: &mut BTreeMap<u64, TimeBlock>,
    height: u64,
) -> Result<TimeBlock> {
    if let Some(block) = seen.get(&height) {
        return Ok(block.clone());
    }
    let block = source.block_at(height).await?;
    ensure!(
        block.height == height,
        "Source returned the wrong height during time lookup"
    );
    seen.insert(height, block.clone());
    Ok(block)
}

pub(crate) struct RpcTimeSource<'a> {
    client: RpcClient,
    cache: Cache,
    config: &'a Config,
    source: String,
}
impl<'a> RpcTimeSource<'a> {
    pub(crate) async fn open(config: &'a Config) -> Result<(Self, TimeBlock, Value)> {
        let client = RpcClient::for_config(config)?;
        let info = client.get_blockchain_info().await?;
        let height = info.result["blocks"]
            .as_u64()
            .context("Source omitted tip height")?;
        let hash = info.result["bestblockhash"]
            .as_str()
            .filter(|h| valid_hash(h))
            .context("Source omitted tip hash")?
            .to_string();
        let mut source = Self {
            client,
            cache: Cache::open(&config.cache_path)?,
            config,
            source: info.source_id,
        };
        let tip = source.block_at(height).await?;
        ensure!(tip.hash == hash, "Chain changed while locating tip; retry");
        Ok((source, tip, info.result))
    }

    pub(crate) fn source_id(&self) -> &str {
        &self.source
    }

    pub(crate) async fn forecast_ready(&self, tip: &TimeBlock) -> Result<()> {
        let info = self.client.get_info().await?;
        ensure!(
            info.source_id == self.source,
            "Source changed while calibrating; retry"
        );
        let blocks = info.result["blocks"].as_u64().unwrap_or(tip.height);
        ensure!(
            info.result["longestchain"]
                .as_u64()
                .is_none_or(|h| h <= blocks),
            "Source is catching up; wait for a current tip before estimating"
        );
        ensure!(
            blocks == tip.height,
            "Tip advanced during calibration; retry the estimate"
        );
        Ok(())
    }

    pub(crate) async fn check(&self, block: &TimeBlock) -> Result<()> {
        let checked = self
            .client
            .call_with_raw::<String>("getblockhash", &[json!(block.height)])
            .await?;
        ensure!(
            checked.source_id == self.source && checked.result == block.hash,
            "Chain or source changed during time lookup; retry"
        );
        Ok(())
    }
}
impl TimeSource for RpcTimeSource<'_> {
    async fn block_at(&mut self, height: u64) -> Result<TimeBlock> {
        let hash = self
            .client
            .call_with_raw::<String>("getblockhash", &[json!(height)])
            .await?;
        ensure!(
            hash.source_id == self.source,
            "Source changed during time lookup; retry"
        );
        ensure!(
            valid_hash(&hash.result),
            "Source returned an invalid block hash"
        );
        let params = json!([hash.result, true]);
        let raw = if let Some(raw) = self
            .cache
            .raw_rpc_response_async(
                &self.source,
                &self.config.chain_cache_key(),
                "getblockheader",
                &params.to_string(),
            )
            .await?
        {
            serde_json::from_str::<Value>(&raw)?["result"].clone()
        } else {
            let header = self
                .client
                .call_with_raw::<Value>("getblockheader", &[json!(hash.result), json!(true)])
                .await?;
            ensure!(
                header.source_id == self.source,
                "Source changed during time lookup; retry"
            );
            header.result
        };
        decode(&raw, height, &hash.result)
    }
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}
fn decode(raw: &Value, height: u64, hash: &str) -> Result<TimeBlock> {
    ensure!(
        raw["height"].as_u64() == Some(height) && raw["hash"].as_str() == Some(hash),
        "Header does not match requested block"
    );
    let parent = raw["previousblockhash"].as_str().map(str::to_string);
    ensure!(
        height == 0 || parent.as_deref().is_some_and(valid_hash),
        "Header parent is unavailable"
    );
    Ok(TimeBlock {
        height,
        hash: hash.to_string(),
        parent,
        timestamp: raw["time"]
            .as_i64()
            .filter(|time| *time >= 0)
            .context("Header timestamp is unavailable")?,
    })
}

/// Look up a timestamp against an already-observed tip and recheck both the
/// tip and chosen block before returning.
pub(crate) async fn find(
    source: &mut RpcTimeSource<'_>,
    target: i64,
    tip: TimeBlock,
) -> Result<LocatedBlock> {
    let mut result = locate(source, target, tip).await?;
    for block in [&result.observed_tip, &result.block] {
        source.check(block).await?;
    }
    result.source = source.source_id().to_string();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-07T18:45:30Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn future_relative_dates_freeze_utc_and_handle_calendar_years() {
        assert_eq!(
            parse_at("in 2 hours", now()).unwrap(),
            now() + Duration::hours(2)
        );
        assert_eq!(
            parse_at("2 hours from now", now()).unwrap(),
            now() + Duration::hours(2)
        );
        assert_eq!(
            parse_at("in 2 days", now()).unwrap().to_rfc3339(),
            "2026-11-09T00:00:00+00:00"
        );
        assert_eq!(
            parse_at("in 2 years", now()).unwrap().to_rfc3339(),
            "2028-11-07T00:00:00+00:00"
        );
        assert!(parse_at("in 4294967295 years", now()).is_err());
        assert!(parse_at("in -1 hours", now()).is_err());
    }

    #[test]
    fn parses_absolute_dates_times_offsets_and_unix_seconds() {
        for input in [
            "2026-11-07",
            "2026/11/07",
            "November 7th, 2026",
            "Nov 7 2026",
            "7 November 2026",
            "on November 7 2026 UTC",
        ] {
            assert_eq!(
                parse_at(input, now()).unwrap().to_rfc3339(),
                "2026-11-07T00:00:00+00:00",
                "{input}"
            );
        }
        for input in [
            "2026-11-07 14:30",
            "2026-11-07T14:30",
            "Nov 7, 2026 at 2:30 pm",
            "7 November 2026 2:30PM UTC",
            "2026-11-07T16:30:00+02:00",
            "2026-11-07 16:30 +02:00",
        ] {
            assert_eq!(
                parse_at(input, now()).unwrap().to_rfc3339(),
                "2026-11-07T14:30:00+00:00",
                "{input}"
            );
        }
        assert_eq!(parse_at("0", now()).unwrap().timestamp(), 0);
        assert_eq!(parse_at("-1", now()).unwrap().timestamp(), -1);
        assert_eq!(
            parse_at("1700000000", now()).unwrap().to_rfc3339(),
            "2023-11-14T22:13:20+00:00"
        );
        assert_eq!(parse_at("2024-02-29", now()).unwrap().day(), 29);
    }

    #[test]
    fn relative_dates_use_utc_midnight_and_clock_durations_use_submission_time() {
        for (input, expected) in [
            ("today", "2026-11-07T00:00:00+00:00"),
            ("yesterday", "2026-11-06T00:00:00+00:00"),
            ("tomorrow", "2026-11-08T00:00:00+00:00"),
            ("2 days ago", "2026-11-05T00:00:00+00:00"),
            ("yesterday at 3 pm", "2026-11-06T15:00:00+00:00"),
            ("2 hours ago", "2026-11-07T16:45:30+00:00"),
            ("now", "2026-11-07T18:45:30+00:00"),
        ] {
            assert_eq!(
                parse_at(input, now()).unwrap().to_rfc3339(),
                expected,
                "{input}"
            );
        }
    }

    #[test]
    fn rejects_ambiguous_invalid_or_overflowing_dates_without_guessing() {
        for input in [
            "",
            "11/07/2026",
            "2026-02-29",
            "2026-11-07 25:00",
            "Nov 7 2026 2pm PST",
            "next flursday",
            "1700000000000",
            "999999999999999999999 days ago",
            "-2 days ago",
            "2026-11-07T00:00:00.5Z",
        ] {
            assert!(parse_at(input, now()).is_err(), "{input}");
        }
    }

    struct FakeSource {
        calls: Vec<u64>,
        overrides: BTreeMap<u64, i64>,
        broken: Option<u64>,
    }
    impl FakeSource {
        fn new() -> Self {
            Self {
                calls: Vec::new(),
                overrides: BTreeMap::new(),
                broken: None,
            }
        }
        fn block(&self, height: u64) -> TimeBlock {
            TimeBlock {
                height,
                hash: format!("{height:064x}"),
                parent: height.checked_sub(1).map(|h| format!("{h:064x}")),
                timestamp: *self.overrides.get(&height).unwrap_or(&(height as i64 * 60)),
            }
        }
    }
    impl TimeSource for FakeSource {
        async fn block_at(&mut self, height: u64) -> Result<TimeBlock> {
            self.calls.push(height);
            let mut block = self.block(height);
            if self.broken == Some(height) {
                block.parent = Some("f".repeat(64));
            }
            Ok(block)
        }
    }

    #[tokio::test]
    async fn finds_nearest_ties_and_reversed_neighbor_times_with_bounded_calls() {
        let mut source = FakeSource::new();
        let tip = source.block(4_000_000);
        let found = locate(&mut source, 2_000_000 * 60 + 30, tip).await.unwrap();
        assert_eq!(found.block.height, 2_000_000);
        assert!(!found.exact_timestamp && !found.whole_chain);
        assert!(source.calls.len() < 160);
        let unique = source
            .calls
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            unique.len(),
            source.calls.len(),
            "no duplicate height probes"
        );
        let mut source = FakeSource::new();
        source.overrides.insert(503, 30_001);
        let tip = source.block(1000);
        let found = locate(&mut source, 30_002, tip).await.unwrap();
        assert_eq!(
            found.block.height, 503,
            "a backward block time beats binary neighbors"
        );
        assert!(found.description().contains("nearest of"));
        assert_eq!(found.searched.1 - found.searched.0, 128);
    }

    #[tokio::test]
    async fn handles_exact_matches_genesis_future_times_and_single_block_chains() {
        for (tip_height, target, expected) in [
            (1000, 30000, 500),
            (1000, -1, 0),
            (1000, 90000, 1000),
            (0, i64::MIN, 0),
            (0, i64::MAX, 0),
        ] {
            let mut source = FakeSource::new();
            let tip = source.block(tip_height);
            let found = locate(&mut source, target, tip).await.unwrap();
            assert_eq!(found.block.height, expected);
            assert_eq!(found.exact_timestamp, target == found.block.timestamp);
            assert_eq!(found.whole_chain, tip_height == 0);
        }
    }

    #[tokio::test]
    async fn rejects_broken_continuity_and_mismatched_headers() {
        let mut source = FakeSource::new();
        source.broken = Some(510);
        let tip = source.block(1000);
        assert!(
            locate(&mut source, 30000, tip)
                .await
                .unwrap_err()
                .to_string()
                .contains("Chain changed")
        );
        assert!(
            decode(
                &json!({"height":4,"hash":"a".repeat(64),"time":10}),
                5,
                &"a".repeat(64)
            )
            .is_err()
        );
        assert!(
            decode(
                &json!({"height":4,"hash":"a".repeat(64),"time":10}),
                4,
                &"a".repeat(64)
            )
            .is_err()
        );
    }
}
