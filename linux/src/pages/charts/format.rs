//! The words and scales of the charts: `ChartFormat` (ChartsView.swift and FeePoolChart.swift),
//! `PlotScale` (ChartPlot.swift) and the scale functions of `FeePoolPlotModel`
//! (FeePoolChart.swift). All of it is pure, and tested below.

use chrono::{DateTime, Local, TimeZone, Utc};

use crate::bridge::api::{BlockProduction, ProductionCounts};
use crate::format;

pub fn production(value: BlockProduction) -> &'static str {
    match value {
        BlockProduction::Mined => "Mined",
        BlockProduction::Staked => "Staked",
        BlockProduction::Unknown => "Unknown",
    }
}

/// "+275 s", "-3 s", "0 s".
pub fn signed_seconds(value: i64) -> String {
    format!("{}{} s", if value > 0 { "+" } else { "" }, value)
}

/// A full-precision figure for the details, not a rounded axis label.
pub fn exact(value: f64) -> String {
    format::decimal(value, 8)
}

/// A number with 0 to `places` decimals and grouping, as `.precision(.fractionLength(0...n))`.
pub fn up_to(value: f64, places: usize) -> String {
    format::decimal(value, places)
}

/// A share with one decimal, as `.percent.precision(.fractionLength(1))`: "53.0%".
pub fn percent_one(value: f64) -> String {
    format!("{}%", grouped_fixed(value * 100.0, 1))
}

/// A number with exactly `places` decimals and grouping, as `.fractionLength(n)`.
pub fn grouped_fixed(value: f64, places: usize) -> String {
    let text = format!("{:.*}", places, value);
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let whole = if whole == "-0" { "0" } else { whole };
    if fraction.is_empty() { format::group(whole) } else { format!("{}.{}", format::group(whole), fraction) }
}

pub fn window(counts: &ProductionCounts, share: Option<f64>) -> String {
    let total = counts.mined + counts.staked + counts.unknown;
    let mut text = format!("{} mined, {} staked", counts.mined, counts.staked);
    if total < counts.window {
        text += &format!(". Partial history: {} of {} blocks. No trend.", total, counts.window);
    } else if counts.unknown > 0 {
        text += &format!(", {} unknown. No trend.", counts.unknown);
    } else if let Some(share) = share {
        text += &format!(". {} staked.", percent_one(share));
    }
    text
}

/// "1 d 0 h 0 min", "3 h 12 min", "4 min 35 s".
pub fn duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let (days, hours, minutes) = (seconds / 86_400, seconds % 86_400 / 3_600, seconds % 3_600 / 60);
    if days > 0 {
        return format!("{days} d {hours} h {minutes} min");
    }
    if hours > 0 {
        return format!("{hours} h {minutes} min");
    }
    format!("{minutes} min {} s", seconds % 60)
}

/// ISO 8601 in UTC, as `Date.formatted(.iso8601)`: "2026-11-26T11:06:32Z".
pub fn iso(unix: i64) -> String {
    Utc.timestamp_opt(unix, 0)
        .single()
        .map(|time| time.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| unix.to_string())
}

/// A short axis time that follows the UTC setting: "1:00 PM", or "Nov 25, 1:00 PM" for a
/// span of more than two days.
pub fn axis_time(unix: u64, span: u64) -> String {
    let style = if span > 2 * 86_400 { "%b %-d, %-I:%M %p" } else { "%-I:%M %p" };
    if crate::display::uses_utc() {
        let time: DateTime<Utc> = Utc.timestamp_opt(unix as i64, 0).single().unwrap_or_default();
        time.format(style).to_string()
    } else {
        Local.timestamp_opt(unix as i64, 0).single().map(|t| t.format(style).to_string()).unwrap_or_default()
    }
}

/// Satoshis as an exact amount with eight decimals. A missing amount says so.
pub fn sats(value: Option<i64>) -> String {
    let Some(value) = value else { return "Unknown".into() };
    let whole = value / 100_000_000;
    let fraction = format!("{:08}", (value % 100_000_000).unsigned_abs());
    let sign = if value < 0 && whole == 0 { "-" } else { "" };
    format!("{sign}{}.{fraction}", format::signed(whole))
}

// MARK: - PlotScale

/// Evenly spaced positions between the ends, both included. Ported from Carbide's
/// `chart_ui::x_ticks`.
pub fn x_ticks(low: f64, high: f64, count: usize, integral: bool) -> Vec<f64> {
    let intervals = if integral { ((count - 1) as f64).min((high - low).floor()) } else { (count - 1) as f64 };
    if intervals <= 0.0 {
        return vec![low];
    }
    (0..=intervals as usize)
        .map(|index| {
            let value = low + (high - low) * index as f64 / intervals;
            if integral { value.floor() } else { value }
        })
        .collect()
}

/// Rounded, uniform ticks so that labels and grid lines share one scale. Ported from
/// Carbide's `chart_ui::y_ticks`.
pub fn nice_ticks(values: &[f64], from_zero: bool, count: usize) -> Vec<f64> {
    let mut low = if from_zero { 0.0 } else { values.iter().copied().fold(f64::INFINITY, f64::min) };
    if !low.is_finite() {
        low = 0.0;
    }
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut high = (if max.is_finite() { max } else { 1.0 }).max(0.0);
    if high <= low {
        if high > 0.0 {
            low *= 0.95;
            high *= 1.05;
        } else {
            low = 0.0;
            high = 1.0;
        }
    }
    if from_zero {
        low = 0.0;
    }
    let rough = ((high - low) / (count.max(2) - 1) as f64).max(f64::MIN_POSITIVE);
    let base = 10f64.powf(rough.log10().floor());
    let factor = [1.0, 2.0, 2.5, 5.0, 10.0].into_iter().find(|f| f * base >= rough).unwrap_or(10.0);
    let step = base * factor;
    let start = if from_zero { 0.0 } else { (low / step).floor() * step };
    let intervals = ((high - start) / step).ceil().clamp(1.0, 8.0) as usize;
    (0..=intervals).map(|i| start + step * i as f64).collect()
}

// MARK: - The fee pool's scale (FeePoolPlotModel)

/// Carbide's log-like scale (`chart_ui::fee_pool::scale`): linear from zero to the
/// protocol's 0.0001 minimum share, then logarithmic, so that zero stays defined.
pub const KNEE: f64 = 0.0001;

pub fn transform(value: f64) -> f64 {
    if value <= KNEE { value / KNEE } else { 1.0 + (value / KNEE).log10() }
}

pub fn log_label(value: f64) -> String {
    let coins = if value <= 1.0 { value * KNEE } else { KNEE * 10f64.powf(value - 1.0) };
    if coins >= 1_000_000.0 {
        return scientific(coins, 3);
    }
    format::decimal(coins, 8)
}

pub fn linear_label(value: f64, step: f64) -> String {
    let digits = ((-step.log10()).ceil() as i64 + 1).clamp(0, 8) as usize;
    format::decimal(value, digits)
}

/// Scientific notation with `digits` significant digits, as Foundation writes it: "1.23E6".
fn scientific(value: f64, digits: usize) -> String {
    let exponent = value.abs().log10().floor() as i32;
    let mantissa = value / 10f64.powi(exponent);
    format!("{}E{}", format::decimal(mantissa, digits.saturating_sub(1)), exponent)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        assert_eq!(signed_seconds(275), "+275 s");
        assert_eq!(signed_seconds(-3), "-3 s");
        assert_eq!(signed_seconds(0), "0 s");
        assert_eq!(duration(86_400), "1 d 0 h 0 min");
        assert_eq!(duration(275), "4 min 35 s");
        assert_eq!(duration(3 * 3600 + 12 * 60 + 5), "3 h 12 min");
        assert_eq!(duration(-5), "0 min 0 s");
        assert_eq!(exact(2_460_000_000_000.0), "2,460,000,000,000");
        assert_eq!(exact(35_780.123_456_789), "35,780.12345679");
        assert_eq!(iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(sats(Some(4_315_490)), "0.04315490");
        assert_eq!(sats(Some(123_456_789_012)), "1,234.56789012");
        assert_eq!(sats(None), "Unknown");
        assert_eq!(percent_one(0.53), "53.0%");
    }

    #[test]
    fn windows() {
        let full = ProductionCounts { mined: 47, staked: 53, unknown: 0, window: 100 };
        assert_eq!(window(&full, Some(0.53)), "47 mined, 53 staked. 53.0% staked.");
        let partial = ProductionCounts { mined: 10, staked: 5, unknown: 0, window: 100 };
        assert_eq!(window(&partial, None), "10 mined, 5 staked. Partial history: 15 of 100 blocks. No trend.");
        let unknown = ProductionCounts { mined: 40, staked: 50, unknown: 10, window: 100 };
        assert_eq!(window(&unknown, None), "40 mined, 50 staked, 10 unknown. No trend.");
    }

    #[test]
    fn ticks() {
        assert_eq!(x_ticks(0.0, 100.0, 5, false), vec![0.0, 25.0, 50.0, 75.0, 100.0]);
        assert_eq!(x_ticks(10.0, 12.0, 5, true), vec![10.0, 11.0, 12.0]);
        assert_eq!(x_ticks(10.0, 10.0, 5, true), vec![10.0]);
        assert_eq!(nice_ticks(&[0.0, 73.0], true, 5), vec![0.0, 20.0, 40.0, 60.0, 80.0]);
        assert_eq!(nice_ticks(&[31.0, 49.0], false, 5), vec![30.0, 35.0, 40.0, 45.0, 50.0]);
        // One value: a range around it.
        let one = nice_ticks(&[10.0], false, 5);
        assert!(one.first().unwrap() <= &9.5 && one.last().unwrap() >= &10.5);
        // No values: zero to one.
        assert_eq!(nice_ticks(&[], false, 5), vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    }

    #[test]
    fn fee_scale() {
        assert_eq!(transform(0.0), 0.0);
        assert_eq!(transform(KNEE), 1.0);
        assert!((transform(0.01) - 3.0).abs() < 1e-9);
        assert_eq!(log_label(0.0), "0");
        assert_eq!(log_label(1.0), "0.0001");
        assert_eq!(log_label(3.0), "0.01");
        assert_eq!(log_label(12.0), "1E7");
        assert_eq!(linear_label(0.02, 0.02), "0.02");
        assert_eq!(linear_label(0.06, 0.02), "0.06");
        assert_eq!(linear_label(5.0, 5.0), "5");
    }
}
