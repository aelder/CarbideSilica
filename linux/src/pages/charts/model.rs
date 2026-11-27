//! The pure parts of the charts: the spans, the fee pool's options and checkpoints
//! (`FeePoolState` in ChartsView.swift), and the figures of one range (`StackModel` in
//! ChartStack.swift), worked out once for each load and not at each pointer move.

use std::collections::{HashMap, HashSet};

use crate::bridge::api::{ChartSample, FeePoolCheckpoint, FeePoolCurrent};
use crate::format;

use super::format as chart_format;

// MARK: - Span

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    TwoHours,
    SixHours,
    Day,
    Week,
}

impl Span {
    pub const ALL: [Span; 4] = [Span::TwoHours, Span::SixHours, Span::Day, Span::Week];

    pub fn seconds(self) -> u64 {
        match self {
            Span::TwoHours => 7_200,
            Span::SixHours => 21_600,
            Span::Day => 86_400,
            Span::Week => 604_800,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Span::TwoHours => "2 hours",
            Span::SixHours => "6 hours",
            Span::Day => "1 day",
            Span::Week => "7 days",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(2)
    }

    /// The span `delta` steps away, kept to the list.
    pub fn stepped(self, delta: i32) -> Span {
        let index = (self.index() as i32 + delta).clamp(0, Self::ALL.len() as i32 - 1);
        Self::ALL[index as usize]
    }
}

// MARK: - Fee pool state

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Currency {
    Both,
    Native,
    Reserve,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Zero,
    Fit,
    Log,
}

/// The fee pool's checkpoints and display options.
#[derive(Clone)]
pub struct FeePoolState {
    pub current: Option<FeePoolCurrent>,
    pub current_error: Option<String>,
    pub checkpoints: HashMap<String, FeePoolCheckpoint>,
    pub scheduled: HashSet<String>,
    pub requested: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub currency: Currency,
    pub scale: Scale,
    pub detail: bool,
    /// Changed to load the checkpoints again.
    pub generation: u64,
}

impl Default for FeePoolState {
    fn default() -> Self {
        Self {
            current: None,
            current_error: None,
            checkpoints: HashMap::new(),
            scheduled: HashSet::new(),
            requested: 0,
            loading: false,
            error: None,
            currency: Currency::Both,
            scale: Scale::Zero,
            detail: false,
            generation: 0,
        }
    }
}

impl FeePoolState {
    pub fn has_reserve(&self) -> bool {
        self.current.as_ref().is_some_and(|c| c.currencies.reserve_id.is_some())
    }

    pub fn cycle_currency(&mut self) {
        if !self.has_reserve() {
            self.currency = Currency::Native;
            return;
        }
        self.currency = match self.currency {
            Currency::Both => Currency::Native,
            Currency::Native => Currency::Reserve,
            Currency::Reserve => Currency::Both,
        };
    }

    pub fn cycle_scale(&mut self) {
        self.scale = match self.scale {
            Scale::Zero => Scale::Fit,
            Scale::Fit => Scale::Log,
            Scale::Log => Scale::Zero,
        };
    }

    pub fn reset(&mut self, keep_checkpoints: bool) {
        self.current = None;
        self.current_error = None;
        if !keep_checkpoints {
            self.checkpoints.clear();
        }
        self.generation += 1;
    }
}

// MARK: - StackModel

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Rate,
    Mining,
    Staking,
    Share,
    Pool,
}

impl Panel {
    pub const ALL: [Panel; 5] = [Panel::Rate, Panel::Mining, Panel::Staking, Panel::Share, Panel::Pool];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn title(self) -> &'static str {
        match self {
            Panel::Rate => "Block rate",
            Panel::Mining => "Mining difficulty",
            Panel::Staking => "Staking difficulty",
            Panel::Share => "Staked share",
            Panel::Pool => "Fee pool",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    None,
    Outside,
    Run,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub time: f64,
    pub value: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bucket {
    pub start: f64,
    pub end: f64,
    pub count: usize,
    pub flag: Flag,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tick {
    pub value: f64,
    pub label: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub mean: f64,
    pub low: f64,
    pub high: f64,
}

/// Three standard deviations of chance for the staked share of 100 blocks at 50%.
pub const CHANCE_LOW: f64 = 0.5 - 3.0 * 0.05;
pub const CHANCE_HIGH: f64 = 0.5 + 3.0 * 0.05;
/// Blocks no more than this many seconds after the block before are one burst.
pub const BURST_SECONDS: i64 = 10;

/// The figures of one range.
pub struct StackModel {
    pub start: f64,
    pub end: f64,
    pub index: HashMap<String, usize>,
    times: Vec<f64>,
    /// Original indices in timestamp order, keeping the first block at each timestamp.
    by_time: Vec<usize>,

    pub bucket_seconds: f64,
    pub bucket_name: &'static str,
    pub rate_target: f64,
    pub buckets: Vec<Bucket>,
    pub rate_limits: Option<Limits>,

    pub mining: Vec<Point>,
    pub staking: Vec<Point>,
    pub mining_average: Vec<Option<Point>>,
    pub staking_average: Vec<Option<Point>>,
    /// The value at the block or at the nearest block of the kind before it.
    pub mining_at: Vec<Option<f64>>,
    pub staking_at: Vec<Option<f64>>,
    pub share100: Vec<Option<Point>>,
    pub share1000: Vec<Option<Point>>,

    pub pool: Vec<Point>,
    pub reserve: Vec<Point>,
    pub pool_rises: Vec<f64>,
    pool_at: Vec<Option<(f64, bool)>>,
    pool_scale: Scale,
    coin: String,
    checkpoints_read: usize,
    checkpoints_wanted: usize,

    pub long_waits: Vec<(f64, f64)>,
    /// The waits between bursts of the range in seconds, shortest first.
    pub waits: Vec<f64>,
    pub time_ticks: Vec<f64>,
}

/// The first position whose value passes the test, in a list sorted so that all values
/// that fail come first.
pub fn partition_index(list: &[f64], test: impl Fn(f64) -> bool) -> usize {
    let (mut low, mut high) = (0, list.len());
    while low < high {
        let middle = (low + high) / 2;
        if test(list[middle]) {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    low
}

impl StackModel {
    pub fn new(samples: &[ChartSample], span_seconds: u64, fee: &FeePoolState, coin: &str) -> Self {
        let times: Vec<f64> = samples.iter().map(|s| s.time as f64).collect();
        let start = times.first().copied().unwrap_or(0.0);
        let end = times.last().copied().unwrap_or(1.0).max(start + 1.0);
        let mut index = HashMap::new();
        for (position, sample) in samples.iter().enumerate() {
            index.entry(sample.hash.clone()).or_insert(position);
        }

        // The block rate: complete periods that end at the newest block.
        let (bucket_seconds, bucket_name) = match span_seconds {
            0..=7_200 => (600.0, "10 minutes"),
            7_201..=21_600 => (1_800.0, "30 minutes"),
            21_601..=86_400 => (3_600.0, "hour"),
            _ => (21_600.0, "6 hours"),
        };
        let rate_target = bucket_seconds / 60.0;
        let mut periods: Vec<Bucket> = Vec::new();
        let mut period_end = end;
        let mut by_time: Vec<_> = (0..samples.len()).collect();
        by_time.sort_by_key(|&i| samples[i].time);
        let sorted: Vec<_> = by_time.iter().map(|&i| times[i]).collect();
        by_time.dedup_by_key(|i| samples[*i].time);
        while period_end - bucket_seconds >= start {
            let low = period_end - bucket_seconds;
            let from = partition_index(&sorted, |t| t > low);
            let to = partition_index(&sorted, |t| t > period_end);
            periods.push(Bucket { start: low, end: period_end, count: to - from, flag: Flag::None });
            period_end = low;
        }
        periods.reverse();
        // A process behaviour chart: the mean, and limits of 2.66 times the mean moving range.
        let mut rate_limits = None;
        if periods.len() >= 10 {
            let counts: Vec<f64> = periods.iter().map(|p| p.count as f64).collect();
            let mean = counts.iter().sum::<f64>() / counts.len() as f64;
            let moving: Vec<f64> = counts.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
            let range = moving.iter().sum::<f64>() / moving.len() as f64;
            let (low, high) = ((mean - 2.66 * range).max(0.0), mean + 2.66 * range);
            rate_limits = Some(Limits { mean, low, high });
            // A run of seven periods on one side of the mean is a signal too.
            let mut run = 0usize;
            let mut side = 0.0;
            for position in 0..periods.len() {
                let now = if counts[position] > mean {
                    1.0
                } else if counts[position] < mean {
                    -1.0
                } else {
                    0.0
                };
                run = if now != 0.0 && now == side { run + 1 } else { 1 };
                side = now;
                if run >= 7 && now != 0.0 {
                    for back in (position + 1 - run)..=position {
                        periods[back].flag = Flag::Run;
                    }
                }
            }
            for position in 0..periods.len() {
                if counts[position] > high || counts[position] < low {
                    periods[position].flag = Flag::Outside;
                }
            }
        }

        let point = |s: &ChartSample, v: f64| Point { time: s.time as f64, value: v };
        let mining = samples.iter().filter_map(|s| s.mining.map(|v| point(s, v))).collect();
        let staking = samples.iter().filter_map(|s| s.staking.map(|v| point(s, v))).collect();
        let mining_average = samples.iter().map(|s| s.mining_average.map(|a| point(s, a.value))).collect();
        let staking_average = samples.iter().map(|s| s.staking_average.map(|a| point(s, a.value))).collect();
        let (mut last_mining, mut last_staking) = (None, None);
        let (mut mining_at, mut staking_at) = (Vec::new(), Vec::new());
        for sample in samples {
            if let Some(value) = sample.mining {
                last_mining = Some(value);
            }
            if let Some(value) = sample.staking {
                last_staking = Some(value);
            }
            mining_at.push(last_mining);
            staking_at.push(last_staking);
        }
        let share100 = samples.iter().map(|s| s.share100.map(|v| point(s, v))).collect();
        let share1000 = samples.iter().map(|s| s.share1000.map(|v| point(s, v))).collect();

        // The fee pool: the checkpoints that are loaded, in block order.
        let mut native = Vec::new();
        let mut second = Vec::new();
        let mut rises = Vec::new();
        let mut carried = Vec::new();
        let mut before: Option<(u64, f64)> = None;
        let mut last: Option<f64> = None;
        let has_reserve = fee.has_reserve();
        for sample in samples {
            let found = fee.checkpoints.get(&sample.hash).filter(|c| c.known).and_then(|c| c.native_sats.map(|s| (c, s)));
            if let Some((checkpoint, sats)) = found {
                let value = sats as f64 / 100_000_000.0;
                if fee.currency != Currency::Reserve {
                    native.push(point(sample, value));
                }
                if let Some((height, previous)) = before
                    && height + 1 == sample.height
                    && value > previous
                {
                    rises.push(sample.time as f64);
                }
                before = Some((sample.height, value));
                last = Some(value);
                carried.push(Some((value, true)));
                if has_reserve
                    && fee.currency != Currency::Native
                    && let Some(reserve) = checkpoint.reserve_sats
                {
                    second.push(point(sample, reserve as f64 / 100_000_000.0));
                }
            } else {
                carried.push(last.map(|v| (v, false)));
            }
        }
        let checkpoints_read = samples.iter().filter(|s| fee.checkpoints.contains_key(&s.hash)).count();
        let checkpoints_wanted = fee.requested.max(checkpoints_read);

        // The longest gaps of the range: the longest hundredth, and 12 at most, so that a
        // long range does not fill with bands.
        let gaps: Vec<f64> = samples.iter().filter_map(|s| s.interval.filter(|i| *i > 0).map(|i| i as f64)).collect();
        let long_waits = if gaps.len() >= 200 {
            let mut ranked = gaps.clone();
            ranked.sort_by(f64::total_cmp);
            let n = gaps.len();
            let limit = ranked[((n as f64 * 0.99) as usize).min(n - 1)].max(ranked[n.saturating_sub(12)]);
            samples
                .iter()
                .filter_map(|s| {
                    let interval = s.interval?;
                    (interval as f64 >= limit).then(|| (s.time as f64 - interval as f64, s.time as f64))
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut waits: Vec<f64> =
            samples.iter().filter_map(|s| s.interval.filter(|i| *i > BURST_SECONDS).map(|i| i as f64)).collect();
        waits.sort_by(f64::total_cmp);

        // Six to eight time labels, on round times.
        let length = end - start;
        let step = [600.0, 1_800.0, 3_600.0, 10_800.0, 21_600.0, 43_200.0, 86_400.0]
            .into_iter()
            .find(|step| length / step <= 9.0)
            .unwrap_or(86_400.0);
        let mut time_ticks = Vec::new();
        let mut tick = (start / step).ceil() * step;
        while tick <= end {
            time_ticks.push(tick);
            tick += step;
        }

        Self {
            start,
            end,
            index,
            times,
            by_time,
            bucket_seconds,
            bucket_name,
            rate_target,
            buckets: periods,
            rate_limits,
            mining,
            staking,
            mining_average,
            staking_average,
            mining_at,
            staking_at,
            share100,
            share1000,
            pool: native,
            reserve: second,
            pool_rises: if fee.currency == Currency::Reserve { Vec::new() } else { rises },
            pool_at: carried,
            pool_scale: fee.scale,
            coin: coin.to_string(),
            checkpoints_read,
            checkpoints_wanted,
            long_waits,
            waits,
            time_ticks,
        }
    }

    // MARK: Positions

    pub fn x(&self, time: f64, left: f32, right: f32) -> f32 {
        left + ((time - self.start) / (self.end - self.start)) as f32 * (right - left)
    }

    /// The block nearest in time to a horizontal position.
    pub fn nearest(&self, x: f32, left: f32, right: f32) -> Option<usize> {
        let share = ((x - left) / (right - left)) as f64;
        let time = self.start + share.clamp(0.0, 1.0) * (self.end - self.start);
        let next = self.by_time.partition_point(|&i| self.times[i] < time);
        let (before, after) = self.by_time.split_at(next);
        match (before.last(), after.first()) {
            (Some(&a), Some(&b)) => Some(match (time - self.times[a]).total_cmp(&(self.times[b] - time)) {
                std::cmp::Ordering::Less => a,
                std::cmp::Ordering::Greater => b,
                std::cmp::Ordering::Equal => a.min(b),
            }),
            (a, b) => a.or(b).copied(),
        }
    }

    /// The fee pool's log-like scale keeps zero defined. The other panels are linear.
    pub fn scaled(&self, value: f64, panel: Panel) -> f64 {
        if panel == Panel::Pool && self.pool_scale == Scale::Log { chart_format::transform(value) } else { value }
    }

    pub fn range(&self, panel: Panel) -> (f64, f64) {
        fn fitted(values: &[f64], floor: f64) -> (f64, f64) {
            let low = values.iter().copied().fold(f64::INFINITY, f64::min);
            let high = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            if !(low.is_finite() && high.is_finite() && high > low) {
                let value = values.first().copied().unwrap_or(1.0);
                return ((value * 0.9).max(floor), value * 1.1 + 1e-9);
            }
            let pad = (high - low) * 0.08;
            ((low - pad).max(floor), high + pad)
        }
        match panel {
            Panel::Rate => {
                let mut values: Vec<f64> = self.buckets.iter().map(|b| b.count as f64).collect();
                values.push(self.rate_target);
                if let Some(limits) = self.rate_limits {
                    values.extend([limits.low, limits.high]);
                }
                fitted(&values, 0.0)
            }
            Panel::Mining => {
                let mut values: Vec<f64> = self.mining.iter().map(|p| p.value).collect();
                values.extend(self.mining_average.iter().flatten().map(|p| p.value));
                fitted(&values, 0.0)
            }
            Panel::Staking => {
                let mut values: Vec<f64> = self.staking.iter().map(|p| p.value).collect();
                values.extend(self.staking_average.iter().flatten().map(|p| p.value));
                fitted(&values, 0.0)
            }
            Panel::Share => {
                let values: Vec<f64> = self.share100.iter().chain(self.share1000.iter()).flatten().map(|p| p.value).collect();
                let min = values.iter().copied().fold(f64::INFINITY, f64::min);
                let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let min = if min.is_finite() { min } else { 0.3 };
                let max = if max.is_finite() { max } else { 0.7 };
                (0.3f64.min(min - 0.03), 0.7f64.max(max + 0.03))
            }
            Panel::Pool => {
                let values: Vec<f64> = self.pool.iter().map(|p| p.value).collect();
                let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let max = if max.is_finite() { max } else { 1.0 };
                match self.pool_scale {
                    Scale::Zero => (0.0, (max * 1.08).max(1e-8)),
                    Scale::Fit => fitted(&values, 0.0),
                    Scale::Log => (0.0, chart_format::transform(max * 1.08).max(1.0)),
                }
            }
        }
    }

    pub fn ticks(&self, panel: Panel) -> Vec<Tick> {
        let (low, high) = self.range(panel);
        if panel == Panel::Pool && self.pool_scale == Scale::Log {
            let mut ticks = Vec::new();
            let mut value = 0.0;
            while value <= high {
                let at = if value <= 1.0 { value * chart_format::KNEE } else { chart_format::KNEE * 10f64.powf(value - 1.0) };
                ticks.push(Tick { value: at, label: chart_format::log_label(value) });
                value += 1.0;
            }
            return ticks;
        }
        if panel == Panel::Share {
            return (0..=10)
                .map(|i| i as f64 / 10.0)
                .filter(|v| *v > low + 0.02 && *v < high - 0.02)
                .map(|v| Tick { value: v, label: format!("{}%", (v * 100.0).round() as i64) })
                .collect();
        }
        let values = chart_format::nice_ticks(&[low, high], false, 5);
        let step = if values.len() > 1 { values[1] - values[0] } else { 1.0 };
        values
            .into_iter()
            .filter(|v| *v >= low && *v <= high)
            .map(|value| {
                let label = match panel {
                    Panel::Rate => format!("{}", value.round() as i64),
                    Panel::Mining | Panel::Staking => format::difficulty(value),
                    Panel::Share => format!("{}%", (value * 100.0).round() as i64),
                    Panel::Pool => chart_format::linear_label(value, step),
                };
                Tick { value, label }
            })
            .collect()
    }

    // MARK: Answers

    /// True when the panel has a signal that a rule found.
    pub fn signal(&self, panel: Panel) -> bool {
        match panel {
            Panel::Rate => self.buckets.iter().any(|b| b.flag == Flag::Outside),
            Panel::Share => self
                .share100
                .last()
                .copied()
                .flatten()
                .is_some_and(|p| p.value < CHANCE_LOW || p.value > CHANCE_HIGH),
            _ => false,
        }
    }

    /// The value now, the change in the range, and a word for what is usual.
    pub fn answer(&self, panel: Panel) -> String {
        match panel {
            Panel::Rate => {
                if self.buckets.is_empty() {
                    return format!("The range is shorter than one period of {}.", self.bucket_name);
                }
                let mean = self.buckets.iter().map(|b| b.count).sum::<usize>() as f64 / self.buckets.len() as f64;
                let mut text = format!(
                    "{} blocks each {}, target {}",
                    chart_format::up_to(mean, 1),
                    self.bucket_name,
                    self.rate_target as i64
                );
                let outside = self.buckets.iter().filter(|b| b.flag == Flag::Outside).count();
                let run = self.buckets.iter().any(|b| b.flag == Flag::Run);
                if self.rate_limits.is_none() {
                    text += ". Limits need 10 periods.";
                } else if outside > 0 {
                    text += &format!(". {} period{} outside the limits.", outside, if outside == 1 { "" } else { "s" });
                } else if run {
                    text += ". 7 periods in sequence are on one side of the mean.";
                } else {
                    text += ". In its usual range.";
                }
                text
            }
            Panel::Mining => Self::change(&self.mining_average, &self.mining),
            Panel::Staking => Self::change(&self.staking_average, &self.staking),
            Panel::Share => {
                let Some(now) = self.share100.last().copied().flatten().map(|p| p.value) else {
                    return "A share needs 100 known blocks.".into();
                };
                let inside = (CHANCE_LOW..=CHANCE_HIGH).contains(&now);
                format!(
                    "{} staked over 100 blocks. {}",
                    format::percent(now),
                    if inside { "Inside the limits of chance." } else { "Outside the limits of chance." }
                )
            }
            Panel::Pool => {
                let Some(now) = self.pool.last().map(|p| p.value) else { return self.coin.clone() };
                let mut text = format!("{} {}", chart_format::up_to(now, 8), self.coin);
                if let Some(first) = self.pool.first().map(|p| p.value).filter(|f| *f > 0.0) {
                    text += &format!(", {} in this range", Self::percent_change(first, now));
                }
                if self.checkpoints_read >= self.times.len() {
                    text += ". Each block of the range is read.";
                } else {
                    text += &format!(
                        ". {} of {} checkpoints read.",
                        format::integer(self.checkpoints_read as u64),
                        format::integer(self.checkpoints_wanted as u64)
                    );
                }
                text
            }
        }
    }

    fn change(average: &[Option<Point>], raw: &[Point]) -> String {
        let values: Vec<f64> = average.iter().flatten().map(|p| p.value).collect();
        match (values.first(), values.last()) {
            (Some(first), Some(last)) if *first > 0.0 => format!(
                "{}, {} in this range. The line is the trailing MA100.",
                format::difficulty(*last),
                Self::percent_change(*first, *last)
            ),
            _ => raw
                .last()
                .map(|p| format!("{} at the newest block of this kind", format::difficulty(p.value)))
                .unwrap_or_else(|| "No block of this kind in the range.".into()),
        }
    }

    pub fn percent_change(first: f64, last: f64) -> String {
        let change = (last / first - 1.0) * 100.0;
        if change.abs() < 0.5 {
            return "no change".into();
        }
        format!("{} {}%", if change > 0.0 { "up" } else { "down" }, chart_format::up_to(change.abs(), 1))
    }

    /// True when the model has figures for the block at this place of the list.
    pub fn has(&self, index: usize) -> bool {
        index < self.times.len() && index < self.mining_at.len() && index < self.staking_at.len() && index < self.pool_at.len()
    }

    pub fn rate_text(&self, index: usize) -> String {
        let Some(time) = self.times.get(index).copied() else { return "–".into() };
        let Some(bucket) = self.buckets.iter().find(|b| time > b.start && time <= b.end) else { return "–".into() };
        format!("{} in its {}", bucket.count, if self.bucket_name == "hour" { "hour" } else { "period" })
    }

    pub fn pool_text(&self, index: usize) -> String {
        let Some(Some((value, exact))) = self.pool_at.get(index).copied() else { return "–".into() };
        let text = format!("{} {}", chart_format::grouped_fixed(value, 8), self.coin);
        if exact { text } else { text + " (nearest earlier checkpoint)" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::api::{BlockProduction, ProductionCounts};

    fn sample(height: u64, time: u64, interval: Option<i64>) -> ChartSample {
        ChartSample {
            height,
            hash: format!("h{height}"),
            time,
            production: if height % 2 == 0 { BlockProduction::Staked } else { BlockProduction::Mined },
            linked: true,
            time_increases: true,
            interval,
            mining: (height % 2 == 1).then_some(1000.0 + height as f64),
            staking: (height % 2 == 0).then_some(30_000.0),
            pos_target: None,
            problem: None,
            mining_average: None,
            staking_average: None,
            window100: ProductionCounts::default(),
            window1000: ProductionCounts::default(),
            share100: Some(0.5),
            share1000: None,
        }
    }

    /// One block a minute for `minutes` minutes.
    fn steady(minutes: u64) -> Vec<ChartSample> {
        (0..minutes).map(|i| sample(i, 1_000_000 + i * 60, Some(60))).collect()
    }

    #[test]
    fn spans() {
        assert_eq!(Span::Day.stepped(1), Span::Week);
        assert_eq!(Span::Week.stepped(1), Span::Week);
        assert_eq!(Span::TwoHours.stepped(-1), Span::TwoHours);
        assert_eq!(Span::SixHours.seconds(), 21_600);
    }

    #[test]
    fn buckets_and_limits() {
        let samples = steady(24 * 60 + 1);
        let model = StackModel::new(&samples, 86_400, &FeePoolState::default(), "VRSC");
        assert_eq!(model.bucket_name, "hour");
        assert_eq!(model.rate_target, 60.0);
        assert_eq!(model.buckets.len(), 24);
        // Each period ends at a block and holds the 60 blocks after its start.
        assert!(model.buckets.iter().all(|b| b.count == 60));
        let limits = model.rate_limits.unwrap();
        assert_eq!(limits.mean, 60.0);
        assert_eq!((limits.low, limits.high), (60.0, 60.0));
        assert!(!model.signal(Panel::Rate));
        assert_eq!(model.answer(Panel::Rate), "60 blocks each hour, target 60. In its usual range.");
        assert_eq!(model.rate_text(samples.len() - 1), "60 in its hour");
    }

    #[test]
    fn outside_and_runs() {
        // Twelve periods of ten minutes; one period has no blocks.
        let mut samples = Vec::new();
        let mut height = 0;
        for period in 0..12u64 {
            let count = if period == 5 { 0 } else { 10 };
            for i in 0..count {
                samples.push(sample(height, 1_000_000 + period * 600 + 1 + i * 59, Some(59)));
                height += 1;
            }
        }
        let model = StackModel::new(&samples, 7_200, &FeePoolState::default(), "VRSC");
        assert_eq!(model.bucket_name, "10 minutes");
        assert!(model.buckets.len() >= 10);
        assert!(model.buckets.iter().any(|b| b.flag == Flag::Outside));
        assert!(model.signal(Panel::Rate));
        assert!(model.answer(Panel::Rate).contains("outside the limits."));
    }

    #[test]
    fn short_range() {
        let samples = steady(5);
        let model = StackModel::new(&samples, 86_400, &FeePoolState::default(), "VRSC");
        assert!(model.buckets.is_empty());
        assert_eq!(model.answer(Panel::Rate), "The range is shorter than one period of hour.");
        assert_eq!(model.answer(Panel::Pool), "VRSC");
    }

    #[test]
    fn carried_values_and_nearest() {
        let samples = steady(10);
        let model = StackModel::new(&samples, 7_200, &FeePoolState::default(), "VRSC");
        assert_eq!(model.mining_at[0], None);
        assert_eq!(model.mining_at[2], Some(1001.0));
        assert_eq!(model.nearest(0.0, 0.0, 100.0), Some(0));
        assert_eq!(model.nearest(100.0, 0.0, 100.0), Some(9));
        assert_eq!(model.nearest(50.0, 0.0, 100.0), Some(4));
        assert_eq!(model.x(model.end, 10.0, 110.0), 110.0);
        assert!(model.has(9));
        assert!(!model.has(10));
    }

    #[test]
    fn nearest_handles_out_of_order_and_equal_block_times() {
        let samples: Vec<_> = [0, 80, 20, 80, 100].into_iter().enumerate()
            .map(|(i, t)| sample(i as u64, t, None)).collect();
        let model = StackModel::new(&samples, 7_200, &FeePoolState::default(), "VRSC");
        assert_eq!(model.nearest(21.0, 0.0, 100.0), Some(2));
        assert_eq!(model.nearest(80.0, 0.0, 100.0), Some(1));
        assert_eq!(model.nearest(50.0, 0.0, 100.0), Some(1));
    }

    #[test]
    fn share_and_waits() {
        let mut samples = steady(300);
        samples[150].interval = Some(3_000);
        samples[150].share100 = Some(0.9);
        let last = samples.len() - 1;
        samples[last].share100 = Some(0.9);
        let model = StackModel::new(&samples, 86_400, &FeePoolState::default(), "VRSC");
        assert!(model.signal(Panel::Share));
        assert_eq!(model.answer(Panel::Share), "90% staked over 100 blocks. Outside the limits of chance.");
        let (low, high) = model.range(Panel::Share);
        assert!(low <= 0.3 && high >= 0.93);
        // The longest hundredth: the 3,000-second gap is among them.
        assert!(model.long_waits.iter().any(|(s, e)| e - s == 3_000.0));
        assert_eq!(model.waits.len(), 300);
        assert_eq!(*model.waits.last().unwrap(), 3_000.0);
        assert_eq!(partition_index(&model.waits, |w| w >= 3_000.0), 299);
    }

    #[test]
    fn answers_of_change() {
        assert_eq!(StackModel::percent_change(100.0, 100.2), "no change");
        assert_eq!(StackModel::percent_change(100.0, 35.3), "down 64.7%");
        assert_eq!(StackModel::percent_change(10.0, 23.45), "up 134.5%");
    }

    #[test]
    fn panel_ticks() {
        let samples = steady(200);
        let model = StackModel::new(&samples, 7_200, &FeePoolState::default(), "VRSC");
        let share: Vec<String> = model.ticks(Panel::Share).into_iter().map(|t| t.label).collect();
        assert_eq!(share, vec!["40%", "50%", "60%"]);
        let pool = model.ticks(Panel::Pool);
        assert_eq!(pool.first().map(|t| t.label.as_str()), Some("0"));
        assert!(!model.time_ticks.is_empty());
    }

    #[test]
    fn fee_cycles() {
        let mut fee = FeePoolState::default();
        fee.cycle_currency();
        assert_eq!(fee.currency, Currency::Native);
        fee.cycle_scale();
        fee.cycle_scale();
        assert_eq!(fee.scale, Scale::Log);
        fee.cycle_scale();
        assert_eq!(fee.scale, Scale::Zero);
        fee.reset(false);
        assert_eq!(fee.generation, 1);
    }
}
