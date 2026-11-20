//! Two-endpoint empirical forecasts. No predicted block is an observed block.
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};

use crate::{
    block_time::{RpcTimeSource, TimeBlock, TimeSource},
    config::Config,
};

#[derive(Debug, Clone)]
pub struct Calibration {
    pub start: TimeBlock,
    pub tip: TimeBlock,
    pub observed_at: i64,
    pub source: String,
    pub source_label: String,
    pub system: String,
}

impl Calibration {
    pub fn new(
        start: TimeBlock,
        tip: TimeBlock,
        observed_at: i64,
        source: String,
        system: String,
    ) -> Result<Self> {
        ensure!(
            tip.height > start.height && tip.height - start.height >= 1000,
            "At least 1,000 calibration blocks after the startup/transition guard are required"
        );
        ensure!(
            start.timestamp >= 0 && tip.timestamp > start.timestamp,
            "Calibration timestamps do not increase"
        );
        ensure!(
            DateTime::from_timestamp(observed_at, 0).is_some(),
            "Observation time is out of range"
        );
        ensure!(
            i128::from(observed_at) - i128::from(tip.timestamp) <= 3600
                && i128::from(tip.timestamp) - i128::from(observed_at) <= 900,
            "The observed tip is stale or its clock is too far ahead to estimate"
        );
        let source_label = source_kind(&source).to_string();
        Ok(Self {
            start,
            tip,
            observed_at,
            source,
            source_label,
            system,
        })
    }

    pub fn seconds_per_block(&self) -> f64 {
        (self.tip.timestamp - self.start.timestamp) as f64
            / (self.tip.height - self.start.height) as f64
    }

    /// Rounded second, anchored to a verified tip at observation UTC, not header age.
    pub fn time_at_height(&self, height: u64) -> Result<i64> {
        ensure!(
            height > self.tip.height,
            "Use recorded block dates for existing heights"
        );
        let blocks = i128::from(height - self.tip.height);
        let span = i128::from(self.tip.height - self.start.height);
        let elapsed = i128::from(self.tip.timestamp - self.start.timestamp);
        let seconds = (blocks * elapsed + span / 2) / span;
        let timestamp = i64::try_from(i128::from(self.observed_at) + seconds)
            .context("Estimated date is out of range")?;
        ensure!(
            DateTime::from_timestamp(timestamp, 0).is_some(),
            "Estimated date is out of range"
        );
        Ok(timestamp)
    }

    /// Floor expected production: estimated latest height reached by the target.
    pub fn height_at_time(&self, timestamp: i64) -> Result<u64> {
        ensure!(
            timestamp >= self.observed_at,
            "Use historical lookup for past dates"
        );
        ensure!(
            DateTime::from_timestamp(timestamp, 0).is_some(),
            "Target date is out of range"
        );
        let seconds = i128::from(timestamp) - i128::from(self.observed_at);
        let blocks = seconds * i128::from(self.tip.height - self.start.height)
            / i128::from(self.tip.timestamp - self.start.timestamp);
        u64::try_from(i128::from(self.tip.height) + blocks)
            .context("Estimated height is out of range")
    }
}

fn source_kind(source: &str) -> &'static str {
    if source.starts_with("local-daemon:") {
        "Local daemon"
    } else if source.starts_with("public-api:") {
        "API"
    } else {
        "Unknown source"
    }
}

/// A display label never includes URL credentials, paths, query tokens or hashes.
fn endpoint_label(kind: &str, endpoint: &str) -> String {
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

fn connection_label(config: &Config, source: &str) -> String {
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
    source_kind(source).into()
}

/// VRSC's studied normal eras; other chains exclude genesis/startup independently.
/// This does not certify absence of unknown interruptions inside a two-point span.
pub fn start_height(system: &str, testnet: bool, tip: u64) -> Result<u64> {
    let start = if !testnet && system == "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV" {
        match tip {
            0..=909364 => 10000,
            909365..=1576699 => {
                anyhow::bail!("No normal-operation calibration in the staking repair period")
            }
            1576700..=2549419 => 1576700,
            2549420..=2549919 => {
                anyhow::bail!("Calibration is unavailable during the PBaaS transition")
            }
            _ => 2549920,
        }
    } else {
        501
    };
    ensure!(
        tip >= start + 1000,
        "Insufficient history after the startup/transition guard"
    );
    Ok(start)
}

pub(crate) async fn from_source(
    source: &mut RpcTimeSource<'_>,
    config: &Config,
    tip: TimeBlock,
    info: &serde_json::Value,
) -> Result<Calibration> {
    ensure!(
        info["initialblockdownload"].as_bool() != Some(true)
            && info["headers"]
                .as_u64()
                .is_none_or(|headers| headers <= tip.height),
        "Source is catching up; wait for a current tip before estimating"
    );
    let system = config.chain_cache_key();
    let first = start_height(&system, config.chain.testnet, tip.height)?;
    let start = source.block_at(first).await?;
    source.check(&start).await?;
    source.check(&tip).await?;
    source.forecast_ready(&tip).await?;
    let mut calibration = Calibration::new(
        start,
        tip,
        Utc::now().timestamp(),
        source.source_id().into(),
        system,
    )?;
    calibration.source_label = connection_label(config, &calibration.source);
    Ok(calibration)
}

pub async fn calibrate(config: &Config) -> Result<Calibration> {
    let config = config.noninteractive();
    let (mut source, tip, info) = RpcTimeSource::open(&config).await?;
    from_source(&mut source, &config, tip, &info).await
}

#[derive(Debug, Clone)]
pub enum Target {
    Height(u64),
    Time(i64),
}

pub fn parse_target(query: &str) -> Result<Target> {
    let query = query.trim();
    if let Some((prefix, value)) = query.split_once(':') {
        if ["time", "date", "unix"]
            .iter()
            .any(|p| prefix.eq_ignore_ascii_case(p))
        {
            return Ok(Target::Time(crate::block_time::parse(value)?.timestamp()));
        }
        if prefix.eq_ignore_ascii_case("height") {
            return Ok(Target::Height(
                value
                    .trim()
                    .parse()
                    .context("Enter an unsigned block height")?,
            ));
        }
    }
    if let Ok(height) = query.parse() {
        Ok(Target::Height(height))
    } else {
        Ok(Target::Time(crate::block_time::parse(query)?.timestamp()))
    }
}

#[derive(Debug, Clone)]
pub struct Estimate {
    pub calibration: Calibration,
    pub target: Target,
    pub height: u64,
    pub timestamp: i64,
}

impl Estimate {
    pub fn new(calibration: Calibration, target: Target) -> Result<Self> {
        let (height, timestamp) = match target {
            Target::Height(h) => (h, calibration.time_at_height(h)?),
            Target::Time(t) => (calibration.height_at_time(t)?, t),
        };
        Ok(Self {
            calibration,
            target,
            height,
            timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn connection_labels_show_endpoints_without_credentials_or_tokens() {
        assert_eq!(
            endpoint_label("Local daemon", "http://localhost:27486"),
            "Local daemon — localhost:27486"
        );
        assert_eq!(
            endpoint_label(
                "Local daemon",
                "http://user:secret@[::1]:27486/private?token=secret"
            ),
            "Local daemon — [::1]:27486"
        );
        assert_eq!(
            endpoint_label(
                "API",
                "https://user:secret@example.org:8443/private-token?key=secret#token"
            ),
            "API — https://example.org:8443"
        );
    }
    fn block(height: u64, timestamp: i64) -> TimeBlock {
        TimeBlock {
            height,
            timestamp,
            hash: format!("{height:064x}"),
            parent: None,
        }
    }
    fn calibration() -> Calibration {
        Calibration::new(
            block(1000, 100000),
            block(3000, 223800),
            223900,
            "local".into(),
            "chain".into(),
        )
        .unwrap()
    }
    #[test]
    fn shared_rate_and_inverse_use_observation_anchor_and_explicit_rounding() {
        let c = calibration();
        assert_eq!(c.seconds_per_block(), 61.9);
        assert_eq!(c.time_at_height(5000).unwrap(), 347700);
        assert_eq!(c.height_at_time(347700).unwrap(), 5000);
        assert_eq!(c.height_at_time(347699).unwrap(), 4999);
        assert_eq!(c.height_at_time(c.observed_at).unwrap(), 3000);
        assert!(c.time_at_height(3000).is_err());
        assert!(c.height_at_time(223899).is_err());
        assert!(c.time_at_height(u64::MAX).is_err());
        assert!(c.height_at_time(i64::MAX).is_err());
    }
    #[test]
    fn excludes_known_eras_and_genesis_and_rejects_stale_or_invalid_pairs() {
        let vrsc = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        assert_eq!(start_height(vrsc, false, 4314420).unwrap(), 2549920);
        for h in [909365, 1576699, 2549420, 2549919] {
            assert!(start_height(vrsc, false, h).is_err());
        }
        assert_eq!(start_height("another chain", false, 2000).unwrap(), 501);
        assert_eq!(start_height(vrsc, true, 2000).unwrap(), 501);
        assert!(start_height("young", false, 1500).is_err());
        assert!(
            Calibration::new(block(0, 0), block(2000, 2000), 6001, "s".into(), "c".into()).is_err()
        );
        assert!(
            Calibration::new(
                block(0, 2000),
                block(2000, 1000),
                1000,
                "s".into(),
                "c".into()
            )
            .is_err()
        );
    }

    #[test]
    fn explicit_unix_target_is_distinct_from_a_numeric_height() {
        assert!(matches!(
            parse_target("5000000").unwrap(),
            Target::Height(5000000)
        ));
        assert!(matches!(
            parse_target("unix:1800000000").unwrap(),
            Target::Time(1800000000)
        ));
        assert!(matches!(
            parse_target("height:5000000").unwrap(),
            Target::Height(5000000)
        ));
        assert!(parse_target("height:-1").is_err());
    }
}
