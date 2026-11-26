//! Display formatting shared by the pages (`Format.swift`). The forms follow the macOS app
//! in the en_US locale: grouping with commas, dates as "Nov 26, 2026 at 11:49:03 AM".

use chrono::{DateTime, Local, TimeZone, Utc};

use crate::settings;

/// Groups the digits of a whole number with commas: 4339576 -> "4,339,576".
pub fn group(digits: &str) -> String {
    let (sign, digits) = digits.strip_prefix('-').map_or(("", digits), |rest| ("-", rest));
    let mut out = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    format!("{sign}{out}")
}

pub fn integer(value: u64) -> String {
    group(&value.to_string())
}

pub fn signed(value: i64) -> String {
    group(&value.to_string())
}

/// A number with up to `places` decimals and no trailing zeros, with grouping.
pub fn decimal(value: f64, places: usize) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let text = format!("{:.*}", places, value);
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let fraction = fraction.trim_end_matches('0');
    let whole = if whole == "-0" && fraction.is_empty() { "0" } else { whole };
    if fraction.is_empty() { group(whole) } else { format!("{}.{}", group(whole), fraction) }
}

/// Satoshis as a coin amount with up to eight decimals, e.g. `3.00046564`. Exact: no float.
pub fn coins(sats: i64) -> String {
    let negative = sats < 0;
    let magnitude = sats.unsigned_abs();
    let whole = magnitude / 100_000_000;
    let fraction = format!("{:08}", magnitude % 100_000_000);
    let fraction = fraction.trim_end_matches('0');
    let mut text = group(&whole.to_string());
    if !fraction.is_empty() {
        text = format!("{text}.{fraction}");
    }
    if negative { format!("-{text}") } else { text }
}

pub fn amount(value: f64) -> String {
    decimal(value, 8)
}

fn uses_utc() -> bool {
    settings::bool(settings::key::USE_UTC, false)
}

/// Dates follow the "Show times in UTC" setting.
pub fn date(unix: u64) -> String {
    let style = "%b %-d, %Y at %-I:%M:%S %p";
    if uses_utc() {
        let time: DateTime<Utc> = Utc.timestamp_opt(unix as i64, 0).single().unwrap_or_default();
        format!("{} UTC", time.format(style))
    } else {
        let time: DateTime<Local> =
            Local.timestamp_opt(unix as i64, 0).single().unwrap_or_else(|| Local.timestamp_opt(0, 0).unwrap());
        time.format(style).to_string()
    }
}

/// A time of day, as `Date.formatted(date: .omitted, time: .standard)`: "11:49:03 AM".
pub fn time_of_day(unix: f64) -> String {
    let style = "%-I:%M:%S %p";
    if uses_utc() {
        let time: DateTime<Utc> = Utc.timestamp_opt(unix as i64, 0).single().unwrap_or_default();
        format!("{} UTC", time.format(style))
    } else {
        Local.timestamp_opt(unix as i64, 0).single().map(|t| t.format(style).to_string()).unwrap_or_default()
    }
}

/// "November 2026", for a sentence about the date of the data.
pub fn month_year(unix: u64) -> String {
    Local
        .timestamp_opt(unix as i64, 0)
        .single()
        .map(|time| time.format("%B %Y").to_string())
        .unwrap_or_default()
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn age(unix: u64, now: f64) -> String {
    let seconds = now as i64 - unix as i64;
    match seconds {
        s if s < 0 => "just now".into(),
        s if s < 60 => format!("{s}s ago"),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h {}m ago", s / 3600, s % 3600 / 60),
        s => format!("{}d ago", s / 86_400),
    }
}

/// A byte count in binary units, as `ByteCountFormatter` with `.binary`.
pub fn bytes(count: u64) -> String {
    const UNITS: [&str; 5] = ["KB", "MB", "GB", "TB", "PB"];
    if count < 1024 {
        return format!("{count} bytes");
    }
    let mut value = count as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    let places = match unit {
        0 => 0,
        1 => 1,
        _ => 2,
    };
    format!("{} {}", decimal(value, places), UNITS[unit])
}

/// A long hash or address shortened in the middle.
pub fn short(text: &str, head: usize, tail: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= head + tail + 1 {
        return text.to_string();
    }
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

pub fn difficulty(value: f64) -> String {
    let units = ["", "K", "M", "G", "T", "P", "E"];
    let mut value = value;
    let mut unit = 0;
    while value >= 1000.0 && unit < units.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{} {}", decimal(value, 2), units[unit])
}

/// A share such as `0.25` as `25%`.
pub fn percent(value: f64) -> String {
    format!("{}%", decimal(value * 100.0, 2))
}

/// A share with one decimal: `99.5%`.
pub fn percent1(value: f64) -> String {
    format!("{:.1}%", value * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_and_coins() {
        assert_eq!(integer(4_339_576), "4,339,576");
        assert_eq!(integer(576), "576");
        assert_eq!(coins(300_044_031), "3.00044031");
        assert_eq!(coins(5_000_000_000_000), "50,000");
        assert_eq!(coins(-150_000_000), "-1.5");
        assert_eq!(difficulty(2.02e12), "2.02 T");
        assert_eq!(short("0123456789abcdef0123456789", 8, 6), "01234567…456789");
        assert_eq!(percent(0.25), "25%");
        assert_eq!(age(100, 100.0 + 3725.0), "1h 2m ago");
    }
}
