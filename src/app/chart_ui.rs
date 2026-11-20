//! Bounded header charts. Samples follow parent hashes, never height aliases.
use super::*;
use ratatui::widgets::{Axis, Chart, Dataset, GraphType};
mod fee_pool;
pub(super) mod seek;
pub(super) use seek::Outcome as SeekOutcome;
pub(super) use seek::{
    apply as apply_seek, paste_prompt, progress as seek_progress, prompt_active, prompt_key,
};

#[cfg(test)]
const WINDOW: u64 = 144;
const MAX_HEADERS: usize = 20_000;
const AVERAGE_WINDOW: usize = 100;
const BALANCE_WINDOWS: [usize; 2] = [100, 1000];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum TimeSpan {
    Hours2,
    Hours6,
    #[default]
    Day1,
    Days7,
}
impl TimeSpan {
    fn seconds(self) -> u64 {
        match self {
            Self::Hours2 => 7200,
            Self::Hours6 => 21600,
            Self::Day1 => 86400,
            Self::Days7 => 604800,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Hours2 => "2h",
            Self::Hours6 => "6h",
            Self::Day1 => "1d",
            Self::Days7 => "7d",
        }
    }
    fn step(self, delta: isize) -> Self {
        let all = [Self::Hours2, Self::Hours6, Self::Day1, Self::Days7];
        let i = all.iter().position(|s| *s == self).unwrap();
        all[i.saturating_add_signed(delta).min(3)]
    }
}
// chainparams.cpp, ASSETCHAINS_LWMAPOS profile, Verus source 72dc29a5f.
// This is the fixed initial posLimit, NOT GetPoSCheckParams' moving limit.
const POS_REFERENCE: &str = "000000000f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f0f";
static NEXT_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Metric {
    Interval,
    Mining,
    Staking,
    Balance,
    FeePool,
}
impl Metric {
    fn label(self) -> &'static str {
        match self {
            Self::Interval => "Block intervals (seconds)",
            Self::Mining => "Mining difficulty",
            Self::Staking => "Staking difficulty",
            Self::Balance => "PoW Ratio",
            Self::FeePool => "Fee pool balances",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Average {
    value: f64,
    samples: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ProductionCounts {
    mined: usize,
    staked: usize,
    unknown: usize,
}
impl ProductionCounts {
    fn total(self) -> usize {
        self.mined + self.staked + self.unknown
    }
    fn stake_share(self, window: usize) -> Option<f64> {
        (self.total() == window && self.unknown == 0).then(|| self.staked as f64 / window as f64)
    }
    fn change(&mut self, production: &str, add: bool) {
        let count = match production {
            "work" => &mut self.mined,
            "stake" => &mut self.staked,
            _ => &mut self.unknown,
        };
        if add {
            *count += 1;
        } else {
            *count -= 1;
        }
    }
}

struct ProductionWindow {
    limit: usize,
    entries: VecDeque<&'static str>,
    counts: ProductionCounts,
}
impl ProductionWindow {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            entries: VecDeque::new(),
            counts: ProductionCounts::default(),
        }
    }
    fn push(&mut self, production: &'static str) -> ProductionCounts {
        if self.entries.len() == self.limit {
            self.counts.change(self.entries.pop_front().unwrap(), false);
        }
        self.entries.push_back(production);
        self.counts.change(production, true);
        self.counts
    }
}

#[derive(Debug, Clone)]
pub(super) struct Sample {
    pub height: u64,
    pub hash: String,
    parent: Option<String>,
    next: Option<String>,
    time: u64,
    production: &'static str,
    interval: Option<f64>,
    mining: Option<f64>,
    staking: Option<f64>,
    target: Option<String>,
    problem: Option<String>,
    averages: [Option<Average>; 2],
    balance: [ProductionCounts; 2],
    pool: Option<[i128; 2]>,
    pool_scheduled: bool,
    pool_requested: bool,
}
impl Sample {
    fn value(&self, metric: Metric) -> Option<f64> {
        match metric {
            Metric::Interval => self.interval,
            Metric::Mining => self.mining,
            Metric::Staking => self.staking,
            Metric::FeePool => self.pool.map(|amounts| amounts[0] as f64 / 100_000_000.0),
            Metric::Balance => match self.production {
                "work" => Some(0.0),
                "stake" => Some(1.0),
                _ => None,
            },
        }
    }
    fn average(&self, metric: Metric) -> Option<Average> {
        match metric {
            Metric::Interval | Metric::Balance | Metric::FeePool => None,
            Metric::Mining => self.averages[0],
            Metric::Staking => self.averages[1],
        }
    }
    fn decode(raw: &Value) -> Result<Self> {
        let height = raw["height"].as_u64().context("Header has no height")?;
        let hash = raw["hash"]
            .as_str()
            .filter(|h| valid_hash(h))
            .context("Header has no valid hash")?
            .to_owned();
        let time = raw["time"].as_u64().context("Header has no timestamp")?;
        let production = match raw["validationtype"].as_str() {
            Some("stake") => "stake",
            Some("work") => "work",
            _ => match raw["blocktype"].as_str() {
                Some("staked" | "minted" | "stake") => "stake",
                Some("mined" | "work") => "work",
                _ => "unknown",
            },
        };
        let (staking, target, problem) = if production == "stake" {
            match stake_difficulty(raw) {
                Ok((value, target)) => (Some(value), Some(target), None),
                Err(error) => (None, None, Some(error.to_string())),
            }
        } else {
            (None, None, None)
        };
        Ok(Self {
            height,
            hash,
            parent: raw["previousblockhash"].as_str().map(str::to_owned),
            next: raw["nextblockhash"].as_str().map(str::to_owned),
            time,
            production,
            interval: None,
            mining: (production == "work")
                .then(|| raw["difficulty"].as_f64())
                .flatten()
                .filter(|v| v.is_finite() && *v > 0.0),
            staking,
            target,
            problem,
            averages: [None; 2],
            balance: [ProductionCounts::default(); 2],
            pool: None,
            pool_scheduled: false,
            pool_requested: false,
        })
    }
}

#[derive(Debug, Clone)]
pub(super) struct Page {
    pub first: u64,
    pub last: u64,
    pub tip: u64,
    pub samples: Vec<Sample>,
    source: String,
    pub status: String,
    span: TimeSpan,
    complete: bool,
    pool: Option<fee_pool::Series>,
}
#[derive(Debug, Clone)]
pub(super) struct View {
    pub metric: Metric,
    pool_currency: usize,
    pool_log: bool,
    pool_fit: bool,
    pool_detail: bool,
    pub selected: Option<String>,
    pub page: Option<Page>,
    pub loading: bool,
    pub error: Option<String>,
    request: u64,
    end: Option<u64>,
    anchor: Option<String>,
    refresh: bool,
    tip: Option<u64>,
    context: Option<(u64, u64)>,
    span: TimeSpan,
    forward: bool,
    seek: Option<seek::Prompt>,
    seek_action: Option<seek::Target>,
    seek_note: Option<String>,
    focus: Option<crate::block_time::TimeBlock>,
    seek_source: Option<String>,
}

pub(super) fn screen(state: &AppState) -> Screen {
    let (end, anchor) = match &state.screen {
        Screen::Block { target, .. } => match state.store.block_for_target(target) {
            Some(block) => (Some(block.summary.height), Some(block.summary.hash.clone())),
            None => match target {
                BlockTarget::Hash(hash) => (None, Some(hash.clone())),
                BlockTarget::Height(height) => (Some(*height), None),
            },
        },
        Screen::Transaction { txid, .. }
            if state
                .store
                .transaction(txid)
                .is_some_and(|tx| tx.block_hash.is_some()) =>
        {
            let tx = state.store.transaction(txid).expect("known transaction");
            (tx.block_height, tx.block_hash.clone())
        }
        _ => state
            .dashboard
            .recent_blocks
            .first()
            .and_then(|hash| state.store.blocks_by_hash.get(hash))
            .map_or((None, None), |block| {
                (Some(block.summary.height), Some(block.summary.hash.clone()))
            }),
    };
    Screen::Charts(Box::new(View {
        metric: Metric::Interval,
        pool_currency: 0,
        pool_log: false,
        pool_fit: false,
        pool_detail: false,
        selected: None,
        page: None,
        loading: true,
        error: None,
        request: next_request(),
        end,
        anchor,
        refresh: false,
        tip: state.tip.height.or(state.store.chain.height),
        context: None,
        span: TimeSpan::default(),
        forward: false,
        seek: None,
        seek_action: None,
        seek_note: None,
        focus: None,
        seek_source: None,
    }))
}
fn next_request() -> u64 {
    NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
}
impl View {
    fn selected_sample(&self) -> Option<&Sample> {
        let samples = &self.page.as_ref()?.samples;
        if let Some(hash) = &self.selected {
            let selected = samples.iter().find(|s| &s.hash == hash);
            if selected.is_some() || self.loading {
                return selected;
            }
        }
        samples
            .iter()
            .rev()
            .find(|s| s.value(self.metric).is_some())
    }
    fn select(&mut self, delta: isize, boundary: Option<bool>) {
        self.seek_note = None;
        let Some(page) = &self.page else { return };
        let samples: Vec<_> = page
            .samples
            .iter()
            .filter(|s| s.value(self.metric).is_some())
            .collect();
        let Some(last) = samples.len().checked_sub(1) else {
            self.selected = None;
            return;
        };
        let index = self
            .selected_sample()
            .and_then(|sample| samples.iter().position(|s| s.hash == sample.hash))
            .unwrap_or(last);
        let next = boundary.map_or_else(
            || index.saturating_add_signed(delta).min(last),
            |end| if end { last } else { 0 },
        );
        self.selected = Some(samples[next].hash.clone());
    }
    fn reload(&mut self, end: u64, anchor: Option<String>, refresh: bool) {
        self.focus = None;
        self.seek_source = None;
        self.seek_note = None;
        self.end = Some(end);
        self.anchor = anchor;
        self.refresh = refresh;
        self.request = next_request();
        self.loading = true;
        self.error = None;
        self.forward = false;
    }
}

pub(super) fn handle_key(
    state: &mut AppState,
    code: KeyCode,
    config: &Config,
    tx: &UnboundedSender<AppEvent>,
) -> bool {
    let Screen::Charts(view) = &mut state.screen else {
        return false;
    };
    match code {
        KeyCode::Char('v') if view.metric == Metric::FeePool => {
            view.pool_currency = if view
                .page
                .as_ref()
                .and_then(|p| p.pool.as_ref())
                .is_some_and(|p| p.reserve)
            {
                (view.pool_currency + 1) % 3
            } else {
                0
            };
        }
        KeyCode::Char('z') if view.metric == Metric::FeePool => {
            if view.pool_log {
                view.pool_log = false;
                view.pool_fit = false;
            } else if view.pool_fit {
                view.pool_fit = false;
                view.pool_log = true;
            } else {
                view.pool_fit = true;
            }
        }
        KeyCode::Char('t') if view.metric == Metric::FeePool => {
            view.pool_detail = !view.pool_detail;
            view.forward = false;
            view.request = next_request();
            view.loading = true;
            view.error = None;
        }
        KeyCode::Char('g') => {
            view.seek = Some(seek::Prompt::new(view.loading));
            view.request = next_request();
            view.loading = false;
            view.seek_action = None;
        }
        KeyCode::Char('1' | '2' | '3' | '4' | '5') => {
            let old_metric = view.metric;
            view.metric = match code {
                KeyCode::Char('2') => Metric::Mining,
                KeyCode::Char('3') => Metric::Staking,
                KeyCode::Char('4') => Metric::Balance,
                KeyCode::Char('5') => Metric::FeePool,
                _ => Metric::Interval,
            };
            if (view.metric == Metric::Balance && old_metric != Metric::Balance)
                || (view.metric == Metric::FeePool && old_metric != Metric::FeePool)
                || (old_metric == Metric::FeePool && view.metric != Metric::FeePool && view.loading)
            {
                // Extend retained context without discarding the range or selection.
                view.request = next_request();
                view.loading = true;
                view.refresh = false;
                view.error = None;
                view.forward = false;
            }
            if view
                .selected_sample()
                .is_none_or(|s| s.value(view.metric).is_none())
            {
                view.selected = None;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => view.select(-1, None),
        KeyCode::Up | KeyCode::Char('k') => view.select(1, None),
        KeyCode::Home => view.select(0, Some(false)),
        KeyCode::End => view.select(0, Some(true)),
        KeyCode::Char('-' | '+' | '=') => {
            let span = view
                .span
                .step(if code == KeyCode::Char('-') { -1 } else { 1 });
            if span != view.span {
                view.span = span;
                view.focus = None;
                view.seek_source = None;
                view.seek_note = None;
                view.request = next_request();
                view.loading = true;
                view.refresh = false;
                view.error = None;
                view.forward = false;
            }
        }
        KeyCode::Char('[') | KeyCode::PageUp if !view.loading => {
            if let Some(page) = &view.page
                && page.first > 0
            {
                let end = page.first - 1;
                let parent = page
                    .samples
                    .first()
                    .filter(|s| s.height == page.first)
                    .and_then(|s| s.parent.clone());
                view.reload(end, parent, false);
            }
        }
        KeyCode::Char(']') | KeyCode::PageDown if !view.loading => {
            if let Some(page) = &view.page
                && page.last < page.tip
            {
                let end = page.last;
                let anchor = page.samples.last().map(|s| s.hash.clone());
                view.reload(end, anchor, false);
                view.forward = true;
            }
        }
        KeyCode::F(5) => {
            if let Some(page) = &view.page {
                view.reload(page.last, view.anchor.clone(), true);
            } else {
                view.request = next_request();
                view.refresh = true;
                view.loading = true;
                view.error = None;
            }
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if let Some(sample) = view.selected_sample() {
                let target = BlockTarget::Hash(sample.hash.clone());
                push_screen(state, block_screen(target.clone()));
                request_block_detail(state, config, tx, target);
            }
        }
        _ => return false,
    }
    true
}

pub(super) struct Task {
    request: u64,
    handle: tokio::task::JoinHandle<()>,
}
impl Drop for Task {
    fn drop(&mut self) {
        self.handle.abort();
    }
}
pub(super) fn drive(
    state: &mut AppState,
    config: &Config,
    tx: &UnboundedSender<AppEvent>,
    task: &mut Option<Task>,
) {
    let Screen::Charts(view) = &mut state.screen else {
        *task = None;
        return;
    };
    let epoch = config.daemon.control.branch_epoch.load(Ordering::Acquire);
    let generation = config.daemon.control.generation();
    if view
        .context
        .is_some_and(|context| context != (epoch, generation))
    {
        view.loading = false;
        view.error = Some("Connection or chain context changed; F5 refreshes".into());
        if let Some(page) = &mut view.page {
            page.status = "Membership unverified".into();
        }
        view.request = next_request();
        view.seek_action = None;
        if let Some(prompt) = &mut view.seek {
            prompt.error = view.error.clone();
        }
        view.context = Some((epoch, generation));
        *task = None;
        return;
    }
    view.context = Some((epoch, generation));
    if !view.loading {
        *task = None;
        return;
    }
    if task.as_ref().is_some_and(|t| t.request == view.request) {
        return;
    }
    let view = view.as_ref().clone();
    let request = view.request;
    let config = config.noninteractive();
    let tx = revision_sender(&config, tx.clone());
    let handle = tokio::spawn(async move {
        if let Some(target) = view.seek_action {
            let result = seek::resolve(&config, target, view.span, |message| {
                let _ = tx.send(AppEvent::ChartSeekProgress {
                    request,
                    message: message.into(),
                });
            })
            .await
            .map_err(|e| e.to_string());
            let _ = tx.send(AppEvent::ChartSeeked { request, result });
        } else {
            load(config, view, tx).await;
        }
    });
    *task = Some(Task { request, handle });
}

pub(super) fn apply(
    state: &mut AppState,
    request: u64,
    page: Option<Page>,
    done: bool,
    error: Option<String>,
) {
    if let Screen::Charts(view) = &mut state.screen
        && view.request == request
    {
        if let Some(page) = page {
            // A context reload walks backwards from the same anchor. Its early
            // progress can temporarily omit the block the user is inspecting.
            if !done
                && let (Some(previous), Some(selected)) = (&view.page, &view.selected)
                && previous.span == page.span
                && previous.source == page.source
                && previous.samples.last().map(|s| &s.hash) == page.samples.last().map(|s| &s.hash)
                && previous
                    .samples
                    .iter()
                    .any(|s| &s.hash == selected && s.height < page.first)
            {
                return;
            }
            view.tip = Some(page.tip);
            view.end = Some(page.last);
            view.anchor = page
                .samples
                .last()
                .map(|s| s.hash.clone())
                .or_else(|| view.anchor.clone());
            view.page = Some(page);
        }
        view.loading = !done;
        view.error = error;
    }
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}
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
fn expand_compact(compact: u32) -> Option<[u8; 32]> {
    let size = (compact >> 24) as usize;
    let mut word = compact & 0x007fffff;
    if compact & 0x00800000 != 0 || word == 0 {
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
fn stake_difficulty(raw: &Value) -> Result<(f64, String)> {
    let nonce = raw["nonce"].as_str().context("PoS nonce unavailable")?;
    anyhow::ensure!(valid_hash(nonce), "Invalid PoS nonce");
    let compact = u32::from_str_radix(&nonce[56..], 16)?;
    let target = expand_compact(compact).context("Invalid compact PoS target")?;
    if let Some(value) = raw.get("postarget") {
        let hex = value.as_str().context("Invalid postarget representation")?;
        anyhow::ensure!(
            hex_target(hex) == Some(target),
            "Nonce and postarget disagree"
        );
    }
    let reference = hex_target(POS_REFERENCE).expect("fixed Verus PoS reference");
    let value = target_float(&reference) / target_float(&target);
    anyhow::ensure!(
        value.is_finite() && value > 0.0,
        "Invalid staking difficulty"
    );
    let hex = target.iter().rev().map(|b| format!("{b:02x}")).collect();
    Ok((value, hex))
}

#[cfg(test)]
fn page(headers: &[Sample], end: u64, tip: u64, source: &str, status: &str) -> Page {
    make_page(
        headers,
        (end.saturating_sub(WINDOW - 1), end),
        tip,
        source,
        status,
        TimeSpan::default(),
        true,
    )
}
fn make_page(
    headers: &[Sample],
    bounds: (u64, u64),
    tip: u64,
    source: &str,
    status: &str,
    span: TimeSpan,
    complete: bool,
) -> Page {
    let (first, end) = bounds;
    let mut samples = Vec::new();
    let mut window = VecDeque::<&Sample>::new();
    let mut sums = [0.0; 2];
    let mut counts = [0usize; 2];
    let mut invalid = [0usize; 2];
    let mut production_windows = BALANCE_WINDOWS.map(ProductionWindow::new);
    let contributions = |sample: &Sample| {
        let values = [sample.mining, sample.staking];
        let bad = [
            sample.production == "unknown"
                || (sample.production == "work" && sample.mining.is_none()),
            sample.production == "unknown"
                || (sample.production == "stake" && sample.staking.is_none()),
        ];
        (values, bad)
    };
    for header in headers.iter().rev() {
        let linked = window.back().is_none_or(|previous| {
            previous.height.checked_add(1) == Some(header.height)
                && header.parent.as_deref() == Some(&previous.hash)
        });
        if !linked {
            window.clear();
            sums = [0.0; 2];
            counts = [0; 2];
            invalid = [0; 2];
            production_windows = BALANCE_WINDOWS.map(ProductionWindow::new);
        }
        let mut sample = header.clone();
        sample.averages = [None; 2];
        sample.interval = None;
        sample.balance = production_windows
            .each_mut()
            .map(|window| window.push(header.production));
        if let Some(parent) = window.back() {
            sample.interval = Some((i128::from(sample.time) - i128::from(parent.time)) as f64);
        }
        if window.len() == AVERAGE_WINDOW {
            let (values, bad) = contributions(window.pop_front().unwrap());
            for i in 0..2 {
                if let Some(value) = values[i] {
                    sums[i] -= value / AVERAGE_WINDOW as f64;
                    counts[i] -= 1;
                }
                invalid[i] -= usize::from(bad[i]);
            }
        }
        let (values, bad) = contributions(header);
        for i in 0..2 {
            if let Some(value) = values[i] {
                sums[i] += value / AVERAGE_WINDOW as f64;
                counts[i] += 1;
            }
            invalid[i] += usize::from(bad[i]);
        }
        window.push_back(header);
        if window.len() == AVERAGE_WINDOW {
            for i in 0..2 {
                if invalid[i] == 0 && counts[i] > 0 {
                    let value = sums[i] / (counts[i] as f64 / AVERAGE_WINDOW as f64);
                    if value.is_finite() && value > 0.0 {
                        sample.averages[i] = Some(Average {
                            value,
                            samples: counts[i],
                        });
                    }
                }
            }
        }
        if (first..=end).contains(&sample.height) {
            samples.push(sample);
        }
    }
    Page {
        first,
        last: end,
        tip,
        samples,
        source: source.into(),
        status: status.into(),
        span,
        complete,
        pool: None,
    }
}
async fn cached_header(
    cache: &Cache,
    config: &Config,
    source: &str,
    hash: &str,
) -> Result<Option<Value>> {
    for (method, params) in [
        ("getblockheader", json!([hash, true])),
        ("getblock", json!([hash, 1])),
    ] {
        if let Some(raw) = cache
            .raw_rpc_response_async(
                source,
                &config.chain_cache_key(),
                method,
                &params.to_string(),
            )
            .await?
        {
            return Ok(Some(rpc_result_value(&raw)?));
        }
    }
    Ok(None)
}
fn check_header(raw: &Value, hash: &str, expected: u64) -> Result<Sample> {
    let sample = Sample::decode(raw)?;
    anyhow::ensure!(
        sample.hash == hash && sample.height == expected,
        "Header hash/height does not match requested chain"
    );
    if expected > 0 {
        anyhow::ensure!(
            sample.parent.as_deref().is_some_and(valid_hash),
            "Header parent unavailable"
        );
    }
    Ok(sample)
}

async fn load(config: Config, view: View, tx: UnboundedSender<AppEvent>) {
    let request = view.request;
    let mut latest = None;
    let mut result = load_inner(&config, &view, &tx, &mut latest).await;
    if view.metric == Metric::FeePool
        && let Some(page) = &mut latest
    {
        let pool_result = fee_pool::load(&config, &view, page, &tx).await;
        if result.is_ok() {
            result = pool_result;
        }
    }
    let _ = tx.send(AppEvent::ChartUpdated {
        request,
        page: latest,
        done: true,
        error: result
            .err()
            .map(|e| format!("Chart history unavailable: {e:#}. F5 retries.")),
    });
}
// A timestamp span follows contiguous headers, with preceding context for
// intervals and MA100; timestamps need not increase monotonically.
struct History {
    span: TimeSpan,
    forward: bool,
    origin: u64,
    tip: u64,
    anchor: String,
    headers: Vec<Sample>,
    context: Vec<Sample>,
    boundary: Option<u64>,
    complete: bool,
    context_window: usize,
    focus: Option<crate::block_time::TimeBlock>,
}
impl History {
    fn new(span: TimeSpan, forward: bool, origin: u64, anchor: String, tip: u64) -> Self {
        Self {
            span,
            forward,
            origin,
            tip,
            anchor,
            headers: Vec::new(),
            context: Vec::new(),
            boundary: None,
            complete: false,
            context_window: AVERAGE_WINDOW,
            focus: None,
        }
    }
    fn for_metric(mut self, metric: Metric) -> Self {
        if metric == Metric::Balance {
            self.context_window = BALANCE_WINDOWS[1];
        } else if metric == Metric::FeePool {
            self.context_window = 2;
        }
        self
    }
    fn with_focus(mut self, focus: Option<crate::block_time::TimeBlock>) -> Self {
        self.focus = focus;
        self
    }
    fn observed_count(&self) -> usize {
        self.headers.len() + self.context.len()
    }
    fn next(&self) -> Option<(u64, Option<String>)> {
        if self.complete || self.observed_count() >= MAX_HEADERS {
            return None;
        }
        let Some(last) = self.headers.last() else {
            return Some((self.origin, Some(self.anchor.clone())));
        };
        if self.forward && self.headers.len() == 1 {
            let parent = self.context.last().unwrap_or(last);
            if parent.height > self.origin.saturating_sub((self.context_window - 2) as u64) {
                return parent
                    .height
                    .checked_sub(1)
                    .map(|height| (height, parent.parent.clone()));
            }
        }
        if self.forward {
            last.height
                .checked_add(1)
                .filter(|h| *h <= self.tip)
                .map(|h| (h, last.next.clone()))
        } else {
            last.height.checked_sub(1).map(|h| (h, last.parent.clone()))
        }
    }
    fn push(&mut self, sample: Sample) -> Result<()> {
        if let Some(focus) = &self.focus
            && sample.height == focus.height
        {
            anyhow::ensure!(
                sample.hash == focus.hash,
                "Seek target belongs to a different branch; retry"
            );
        }
        if self.forward && self.headers.len() == 1 && sample.height < self.origin {
            let previous = self.context.last().unwrap_or(&self.headers[0]);
            anyhow::ensure!(
                sample.height.checked_add(1) == Some(previous.height)
                    && previous.parent.as_deref() == Some(&sample.hash),
                "Moving-average context is discontinuous"
            );
            self.context.push(sample);
            return Ok(());
        }
        if let Some(previous) = self.headers.last() {
            let linked = if self.forward {
                sample.height == previous.height + 1
                    && sample.parent.as_deref() == Some(&previous.hash)
            } else {
                sample.height.checked_add(1) == Some(previous.height)
                    && previous.parent.as_deref() == Some(&sample.hash)
            };
            anyhow::ensure!(linked, "Chart header chain is discontinuous");
        }
        if self.forward {
            // The origin is a lookback parent, not a second copy of the old window.
            if sample.height > self.origin {
                let first = self.headers.get(1).unwrap_or(&sample);
                if sample.time.saturating_sub(first.time) >= self.span.seconds()
                    || sample.height == self.tip
                {
                    self.complete = true;
                }
            }
        } else {
            if self.boundary.is_some_and(|h| {
                sample.height <= h.saturating_sub((self.context_window - 1) as u64)
            }) {
                self.complete = true;
            }
            if self.boundary.is_none() {
                let end_time = self.headers.first().map_or(sample.time, |h| h.time);
                if sample.time <= end_time.saturating_sub(self.span.seconds())
                    && self
                        .focus
                        .as_ref()
                        .is_none_or(|focus| sample.height <= focus.height)
                {
                    self.boundary = Some(sample.height);
                }
            }
            if sample.height == 0 {
                self.boundary.get_or_insert(0);
                self.complete = true;
            }
        }
        self.headers.push(sample);
        Ok(())
    }
    fn page(&self, source: &str, status: &str) -> Option<Page> {
        if self.headers.is_empty() || (self.forward && self.headers.len() < 2) {
            return None;
        }
        let (headers, first, last) = if self.forward {
            let mut headers = self.headers.clone();
            headers.reverse();
            headers.extend(self.context.iter().cloned());
            let last = headers[0].height;
            (headers, self.origin + 1, last)
        } else {
            (
                self.headers.clone(),
                self.boundary
                    .unwrap_or_else(|| self.headers.last().unwrap().height),
                self.origin,
            )
        };
        Some(make_page(
            &headers,
            (first, last),
            self.tip,
            source,
            status,
            self.span,
            self.complete,
        ))
    }
}

async fn load_inner(
    config: &Config,
    view: &View,
    tx: &UnboundedSender<AppEvent>,
    latest: &mut Option<Page>,
) -> Result<()> {
    let cache = Cache::open(&config.cache_path)?;
    let send = |page: Page, latest: &mut Option<Page>| {
        *latest = Some(page.clone());
        let _ = tx.send(AppEvent::ChartUpdated {
            request: view.request,
            page: Some(page),
            done: false,
            error: None,
        });
    };
    let mut known_view = view.clone();
    let mut sources = vec![config.source_cache_key("local-daemon")];
    sources.extend(
        config
            .daemon
            .api_pool
            .endpoints
            .iter()
            .map(|e| crate::rpc::source_id("public-api", &e.url)),
    );
    if let Some(source) = &view.seek_source {
        sources.retain(|candidate| candidate == source);
    }
    if known_view.end.is_none()
        && let Some(hash) = known_view.anchor.as_ref()
    {
        for source in &sources {
            if let Some(raw) = cached_header(&cache, config, source, hash).await?
                && let Ok(sample) = Sample::decode(&raw)
            {
                known_view.end = Some(sample.height);
                break;
            }
        }
    }
    let view = &known_view;
    if let (Some(end), Some(anchor)) = (view.end, view.anchor.as_ref()) {
        for source in sources {
            let mut history = History::new(
                view.span,
                view.forward,
                end,
                anchor.clone(),
                view.tip
                    .unwrap_or_else(|| view.page.as_ref().map_or(end, |page| page.tip)),
            )
            .for_metric(view.metric)
            .with_focus(view.focus.clone());
            while let Some((height, Some(hash))) = history.next() {
                let Some(raw) = cached_header(&cache, config, &source, &hash).await? else {
                    break;
                };
                let Ok(sample) = check_header(&raw, &hash, height) else {
                    break;
                };
                if history.push(sample).is_err() {
                    break;
                }
            }
            if let Some(page) =
                history.page(&source, "Retained observations; membership unverified")
            {
                send(page, latest);
                if history.complete && !view.refresh {
                    return Ok(());
                }
                anyhow::ensure!(
                    history.complete || history.observed_count() < MAX_HEADERS,
                    "Reached the {MAX_HEADERS}-header limit; use a shorter span or page to another range"
                );
                break;
            }
        }
    }
    let client = RpcClient::for_config(config)?;
    let before = client.get_blockchain_info().await?;
    let source = before.source_id;
    let tip = before.result["blocks"]
        .as_u64()
        .context("Source omitted tip height")?;
    anyhow::ensure!(
        view.seek_source
            .as_ref()
            .is_none_or(|expected| expected == &source),
        "Source changed after seeking; retry"
    );
    let end = if let Some(end) = view.end {
        end
    } else if let Some(hash) = &view.anchor {
        let response = client
            .call_with_raw::<Value>("getblockheader", &[json!(hash), json!(true)])
            .await?;
        anyhow::ensure!(
            response.source_id == source,
            "Source changed while locating selected block"
        );
        let sample = Sample::decode(&response.result)?;
        anyhow::ensure!(&sample.hash == hash, "Selected header hash does not match");
        sample.height
    } else {
        tip
    };
    let anchor = if let Some(hash) = &view.anchor {
        hash.clone()
    } else if end == tip {
        before.result["bestblockhash"]
            .as_str()
            .context("Source omitted tip hash")?
            .into()
    } else {
        anyhow::ensure!(end <= tip, "Requested height is beyond the source tip");
        let response = client
            .call_with_raw::<String>("getblockhash", &[json!(end)])
            .await?;
        anyhow::ensure!(
            response.source_id == source,
            "Source changed while locating chart window"
        );
        response.result
    };
    anyhow::ensure!(valid_hash(&anchor), "Source returned invalid anchor hash");
    if view.forward {
        let canonical = client
            .call_with_raw::<String>("getblockhash", &[json!(end)])
            .await?;
        anyhow::ensure!(
            canonical.source_id == source && canonical.result == anchor,
            "Cannot page forward from an orphan anchor; open an active-chain block"
        );
    }
    let mut history = History::new(view.span, view.forward, end, anchor, tip)
        .for_metric(view.metric)
        .with_focus(view.focus.clone());
    while let Some((height, hint)) = history.next() {
        let hash = if let Some(hash) = hint {
            hash
        } else {
            let response = client
                .call_with_raw::<String>("getblockhash", &[json!(height)])
                .await?;
            anyhow::ensure!(
                response.source_id == source,
                "Source changed while locating next header"
            );
            response.result
        };
        let cached = cached_header(&cache, config, &source, &hash)
            .await?
            .filter(|raw| {
                check_header(raw, &hash, height)
                    .is_ok_and(|sample| !view.refresh || sample.problem.is_none())
            });
        let raw = if let Some(raw) = cached {
            raw
        } else {
            let response = client
                .call_with_raw::<Value>("getblockheader", &[json!(hash), json!(true)])
                .await?;
            anyhow::ensure!(
                response.source_id == source,
                "Source changed while loading chart headers"
            );
            response.result
        };
        history.push(check_header(&raw, &hash, height)?)?;
        if (history.observed_count().is_multiple_of(32) || history.complete)
            && let Some(page) = history.page(&source, "Loading; membership unverified")
        {
            send(page, latest);
        }
    }
    if !history.complete {
        if let Some(page) = history.page(&source, "Partial range; header limit reached") {
            send(page, latest);
        }
        anyhow::bail!(
            "Reached the {MAX_HEADERS}-header limit; use a shorter span or page to another range"
        );
    }
    let last = history
        .page(&source, "")
        .context("No newer blocks in this range")?
        .samples
        .last()
        .unwrap()
        .clone();
    let after = client.get_blockchain_info().await?;
    anyhow::ensure!(
        after.source_id == source,
        "Source changed while checking chart context"
    );
    let canonical = client
        .call_with_raw::<String>("getblockhash", &[json!(last.height)])
        .await?;
    anyhow::ensure!(
        canonical.source_id == source,
        "Source changed while checking anchor membership"
    );
    history.tip = after.result["blocks"].as_u64().unwrap_or(tip);
    let status = if canonical.result == last.hash {
        "Observed active chain"
    } else {
        "Orphan anchor; historical branch"
    };
    if let Some(page) = history.page(&source, status) {
        send(page, latest);
    }
    Ok(())
}

fn number(value: f64) -> String {
    if value != 0.0 && !(0.001..1_000_000.0).contains(&value.abs()) {
        format!("{value:.3e}")
    } else {
        format!("{value:.3}")
    }
}
// Uniform, rounded tick values keep labels and grid lines on the same scale.
fn y_ticks(data: &[(f64, f64)], metric: Metric, count: usize) -> Vec<f64> {
    if metric == Metric::Balance {
        return vec![0.0, 0.5, 1.0];
    }
    let low = if matches!(metric, Metric::Interval | Metric::FeePool) {
        0.0
    } else {
        data.iter().map(|(_, v)| *v).reduce(f64::min).unwrap_or(0.0)
    };
    let high = data
        .iter()
        .map(|(_, v)| *v)
        .reduce(f64::max)
        .unwrap_or(1.0)
        .max(0.0);
    let (low, high) = if high > low {
        (low, high)
    } else if high > 0.0 {
        (low * 0.95, (high * 1.05).min(f64::MAX))
    } else {
        (0.0, 1.0)
    };
    let low = if matches!(metric, Metric::Interval | Metric::FeePool) {
        0.0
    } else {
        low
    };
    let rough = ((high - low) / (count.saturating_sub(1).max(1) as f64)).max(f64::MIN_POSITIVE);
    let base = 10.0_f64.powf(rough.log10().floor());
    let factor = [1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .find(|f| *f * base >= rough)
        .unwrap_or(10.0);
    let step = base * factor;
    let start = if matches!(metric, Metric::Interval | Metric::FeePool) {
        0.0
    } else {
        (low / step).floor() * step
    };
    let intervals = ((high - start) / step).ceil().clamp(1.0, 8.0) as usize;
    (0..=intervals).map(|i| start + step * i as f64).collect()
}
fn tick_label(value: f64, step: f64) -> String {
    if value == 0.0 {
        return "0".into();
    }
    if !(0.001..1_000_000.0).contains(&value.abs()) {
        let precision = (value.abs().log10() - step.log10() + 1.0)
            .ceil()
            .clamp(3.0, 12.0) as usize;
        format!("{value:.precision$e}")
    } else {
        let precision = ((-step.log10()).ceil() + 1.0).clamp(0.0, 8.0) as usize;
        format!("{value:.precision$}")
    }
}
fn x_ticks(first: u64, last: u64, count: usize) -> Vec<u64> {
    let intervals = (count.saturating_sub(1) as u64).min(last.saturating_sub(first));
    if intervals == 0 {
        return vec![first];
    }
    (0..=intervals)
        .map(|i| {
            first + ((u128::from(last - first) * u128::from(i)) / u128::from(intervals)) as u64
        })
        .collect()
}
fn average_segments(page: &Page, metric: Metric) -> Vec<Vec<(f64, f64)>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut previous = None;
    for sample in &page.samples {
        let value = sample.average(metric);
        if (value.is_none()
            || previous.is_some_and(|h: u64| h.checked_add(1) != Some(sample.height)))
            && !current.is_empty()
        {
            segments.push(std::mem::take(&mut current));
        }
        if let Some(mean) = value {
            current.push((sample.height as f64, mean.value));
        }
        previous = Some(sample.height);
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn balance_segments(page: &Page, index: usize) -> Vec<Vec<(f64, f64)>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    let mut previous: Option<&Sample> = None;
    for sample in &page.samples {
        let value = sample.balance[index].stake_share(BALANCE_WINDOWS[index]);
        if (value.is_none()
            || previous.is_some_and(|prev| {
                prev.height.checked_add(1) != Some(sample.height)
                    || sample.parent.as_deref() != Some(&prev.hash)
                    || sample.time <= prev.time
            }))
            && !current.is_empty()
        {
            segments.push(std::mem::take(&mut current));
        }
        if let Some(value) = value {
            current.push((sample.time as f64, value));
        }
        previous = Some(sample);
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

fn plot_x(sample: &Sample, metric: Metric) -> f64 {
    if matches!(metric, Metric::Balance | Metric::FeePool) {
        sample.time as f64
    } else {
        sample.height as f64
    }
}

fn plot_bounds(page: &Page, metric: Metric) -> (u64, u64) {
    if matches!(metric, Metric::Balance | Metric::FeePool) {
        let first = page.samples.iter().map(|s| s.time).min().unwrap_or(0);
        let last = page.samples.iter().map(|s| s.time).max().unwrap_or(first);
        (first, last.max(first.saturating_add(1)))
    } else {
        (page.first, page.last.max(page.first + 1))
    }
}

fn balance_label(value: f64) -> String {
    if value == 0.0 {
        "Mined".into()
    } else if value == 1.0 {
        "Staked".into()
    } else {
        "50/50".into()
    }
}

fn balance_counts_label(sample: &Sample, index: usize) -> String {
    let counts = sample.balance[index];
    let window = BALANCE_WINDOWS[index];
    let status = if counts.total() < window {
        format!(" | partial {}/{}", counts.total(), window)
    } else if counts.unknown > 0 {
        format!(" | {} unknown; trend unavailable", counts.unknown)
    } else {
        format!(
            " | {:.1}% staked",
            counts.staked as f64 / window as f64 * 100.0
        )
    };
    format!(
        "{window} blocks: {} mined / {} staked{status}",
        counts.mined, counts.staked
    )
}

fn grid_color(theme: &Theme) -> ratatui::style::Color {
    use ratatui::style::Color;
    match (theme.background, theme.muted) {
        (Color::Rgb(r, g, b), Color::Rgb(mr, mg, mb)) => Color::Rgb(
            ((u16::from(r) * 4 + u16::from(mr)) / 5) as u8,
            ((u16::from(g) * 4 + u16::from(mg)) / 5) as u8,
            ((u16::from(b) * 4 + u16::from(mb)) / 5) as u8,
        ),
        _ => Color::DarkGray,
    }
}
pub(super) fn render(frame: &mut Frame<'_>, theme: &Theme, view: &View, area: Rect) {
    render_chart(frame, theme, view, area);
    seek::render(frame, theme, view, area);
}

fn render_chart(frame: &mut Frame<'_>, theme: &Theme, view: &View, area: Rect) {
    let compact = area.height < 22;
    let detail_height = if view.metric == Metric::FeePool {
        6
    } else if view.metric == Metric::Balance {
        4
    } else if compact && view.metric != Metric::Staking {
        2
    } else if view.selected_sample().is_none() {
        1
    } else {
        4
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(if compact { 1 } else { 2 }),
            Constraint::Min(3),
            Constraint::Length(2),
            Constraint::Length(detail_height),
            Constraint::Length(if compact { 1 } else { 2 }),
        ])
        .split(area);
    frame.render_widget(
        Paragraph::new(format!(" {} ", view.metric.label())).style(
            Style::default()
                .fg(theme.background)
                .bg(theme.link)
                .add_modifier(Modifier::BOLD),
        ),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Charts  ", theme.focused()),
            Span::styled(
                "1 Intervals  2 Mining  3 Staking  4 PoW Ratio  5 Fee pool   ",
                theme.link(),
            ),
            Span::raw(if view.loading {
                format!(" loading {}...", view.span.label())
            } else {
                format!(" Span {} (-/+)", view.span.label())
            }),
        ])),
        rows[1],
    );
    let Some(page) = &view.page else {
        frame.render_widget(
            Paragraph::new(
                view.error
                    .as_deref()
                    .unwrap_or("Loading bounded header history; Esc cancels."),
            )
            .style(theme.muted())
            .wrap(Wrap { trim: false }),
            rows[2],
        );
        return;
    };
    let elapsed = page
        .samples
        .first()
        .zip(page.samples.last())
        .map(|(first, last)| last.time.saturating_sub(first.time))
        .unwrap_or(0);
    let source = if page.source.starts_with("local-daemon:") {
        "local daemon"
    } else {
        "enabled API"
    };
    let elapsed = human_duration(elapsed.min(i64::MAX as u64) as i64);
    let metadata = if compact {
        format!(
            "heights {}..{} | {} headers | {}{}",
            page.first,
            page.last,
            page.samples.len(),
            elapsed,
            if page.complete { "" } else { " | partial" }
        )
    } else {
        format!(
            "heights {}..{} | {} headers | actual {}\n{} | {}{}",
            page.first,
            page.last,
            page.samples.len(),
            elapsed,
            page.status,
            source,
            if page.complete { "" } else { " | partial" }
        )
    };
    let metadata = if page.span != view.span {
        format!("{metadata} | showing {}", page.span.label())
    } else {
        metadata
    };
    frame.render_widget(
        Paragraph::new(metadata)
            .style(theme.muted())
            .wrap(Wrap { trim: false }),
        rows[2],
    );
    // Negative timestamp differences remain inspectable; the interval plot starts
    // at zero and never invents zero-valued points for those negative intervals.
    let data: Vec<_> = page
        .samples
        .iter()
        .filter_map(|s| {
            (if view.metric == Metric::FeePool && view.pool_currency == 2 {
                s.pool.map(|p| p[1] as f64 / 100_000_000.0)
            } else {
                s.value(view.metric)
            })
            .map(|v| {
                if view.metric == Metric::FeePool {
                    fee_pool::scale(v, view.pool_log)
                } else {
                    v
                }
            })
            .filter(|v| view.metric != Metric::Interval || *v >= 0.0)
            .map(|v| (plot_x(s, view.metric), v))
        })
        .collect();
    let selected = view.selected_sample();
    let focus: Vec<_> = selected
        .and_then(|s| {
            (if view.metric == Metric::FeePool && view.pool_currency == 2 {
                s.pool.map(|p| p[1] as f64 / 100_000_000.0)
            } else {
                s.value(view.metric)
            })
            .map(|v| {
                if view.metric == Metric::FeePool {
                    fee_pool::scale(v, view.pool_log)
                } else {
                    v
                }
            })
            .filter(|v| view.metric != Metric::Interval || *v >= 0.0)
            .map(|v| (plot_x(s, view.metric), v))
        })
        .into_iter()
        .collect();
    let averages = if view.metric == Metric::Balance {
        balance_segments(page, 0)
    } else {
        average_segments(page, view.metric)
    };
    let long_trend = if view.metric == Metric::Balance {
        balance_segments(page, 1)
    } else {
        Vec::new()
    };
    let (x_first, x_last) = plot_bounds(page, view.metric);
    let x_labels = x_ticks(x_first, x_last, if area.width >= 100 { 5 } else { 3 })
        .into_iter()
        .map(|x| {
            if matches!(view.metric, Metric::Balance | Metric::FeePool) {
                let time = theme.format_time(x);
                time.get(5..16).unwrap_or(&time).replace('T', " ")
            } else {
                x.to_string()
            }
        })
        .collect::<Vec<_>>();
    let pool_lines: [Vec<Vec<(f64, f64)>>; 2] = if view.metric == Metric::FeePool {
        [0, 1].map(|index| {
            if (view.pool_currency == 1 && index == 1)
                || (view.pool_currency == 2 && index == 0)
                || (index == 1 && !page.pool.as_ref().is_some_and(|p| p.reserve))
            {
                Vec::new()
            } else {
                fee_pool::segments(page, index)
                    .into_iter()
                    .map(|line| {
                        line.into_iter()
                            .map(|(x, y)| (x, fee_pool::scale(y, view.pool_log)))
                            .collect()
                    })
                    .collect()
            }
        })
    } else {
        [Vec::new(), Vec::new()]
    };
    let mut scale_data = data.clone();
    scale_data.extend(pool_lines.iter().flatten().flatten().copied());
    scale_data.extend(averages.iter().flatten().copied());
    let ticks = if view.metric == Metric::FeePool && view.pool_log {
        fee_pool::log_ticks(&scale_data, if rows[3].height >= 12 { 5 } else { 3 })
    } else {
        y_ticks(
            &scale_data,
            if view.metric == Metric::FeePool && view.pool_fit {
                Metric::Mining
            } else {
                view.metric
            },
            if rows[3].height >= 12 { 5 } else { 3 },
        )
    };
    // Pad binary production levels so mined/staked sit at 1/3 and 2/3 height.
    // Trend shares keep the same scale, with 50/50 at the vertical center.
    let min = if view.metric == Metric::Balance {
        -1.0
    } else {
        ticks[0]
    };
    let max = if view.metric == Metric::Balance {
        2.0
    } else {
        *ticks.last().unwrap()
    };
    let step = ticks[1] - ticks[0];
    let grid: Vec<_> = ticks
        .iter()
        .map(|y| vec![(x_first as f64, *y), (x_last as f64, *y)])
        .collect();
    let mut datasets: Vec<_> = grid
        .iter()
        .map(|line| {
            Dataset::default()
                .marker(Marker::Dot)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(grid_color(theme)))
                .data(line)
        })
        .collect();
    for segment in &averages {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(theme.positive))
                .data(segment),
        );
    }
    for segment in &long_trend {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(theme.warning))
                .data(segment),
        );
    }
    for (index, lines) in pool_lines.iter().enumerate() {
        if index == 1 && !page.pool.as_ref().is_some_and(|p| p.reserve) {
            continue;
        }
        for line in lines {
            datasets.push(
                Dataset::default()
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(if index == 0 {
                        theme.link
                    } else {
                        theme.positive
                    }))
                    .data(line),
            );
        }
    }
    // Fee-pool line datasets already render a dot for each point, including
    // single-point segments that have no loaded neighbor yet.
    if view.metric != Metric::FeePool {
        datasets.push(
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Scatter)
                .style(Style::default().fg(theme.link))
                .data(&data),
        );
    }
    datasets.push(
        Dataset::default()
            .marker(Marker::Block)
            .style(Style::default().fg(theme.accent))
            .data(&focus),
    );
    frame.render_widget(
        Chart::new(datasets)
            .legend_position(None)
            .style(theme.base())
            .block(Block::default().borders(Borders::ALL))
            .x_axis(
                Axis::default()
                    .title(
                        if matches!(view.metric, Metric::Balance | Metric::FeePool) {
                            "Time"
                        } else {
                            "Block height"
                        },
                    )
                    .style(theme.muted())
                    .bounds([x_first as f64, x_last as f64])
                    .labels(x_labels.clone()),
            )
            .y_axis(
                Axis::default()
                    .title(if view.metric == Metric::Balance {
                        "Block type"
                    } else if view.metric == Metric::FeePool {
                        if view.pool_log {
                            "Log-like coins"
                        } else if view.pool_fit {
                            "Coins (fit)"
                        } else {
                            "Coins"
                        }
                    } else if view.metric == Metric::Interval {
                        "Seconds"
                    } else {
                        "Difficulty"
                    })
                    .style(theme.muted())
                    .bounds([min, max])
                    .labels(
                        ticks
                            .iter()
                            .map(|value| {
                                if view.metric == Metric::Balance {
                                    balance_label(*value)
                                } else {
                                    if view.metric == Metric::FeePool && view.pool_log {
                                        fee_pool::axis_label(*value)
                                    } else {
                                        tick_label(*value, step)
                                    }
                                }
                            })
                            .collect::<Vec<_>>(),
                    ),
            ),
        rows[3],
    );
    // Chart's native labels use integer floor spacing, while Canvas rounds
    // pixel positions. Align labels to the actual grid projection at every size.
    if rows[3].height > 4 && rows[3].width > 5 {
        let inner = Block::default().borders(Borders::ALL).inner(rows[3]);
        let labels = ticks
            .iter()
            .map(|v| {
                if view.metric == Metric::Balance {
                    balance_label(*v)
                } else {
                    if view.metric == Metric::FeePool && view.pool_log {
                        fee_pool::axis_label(*v)
                    } else {
                        tick_label(*v, step)
                    }
                }
            })
            .collect::<Vec<_>>();
        let label_width = (labels.iter().map(String::len).max().unwrap_or(0) as u16)
            .max(
                x_labels
                    .first()
                    .map_or(0, |label| label.len().saturating_sub(1)) as u16,
            )
            .min(inner.width / 3);
        let height = inner.height.saturating_sub(2);
        let labels_area = Rect::new(inner.x, inner.y, label_width, height);
        frame.render_widget(Clear, labels_area);
        frame.render_widget(Paragraph::new("").style(theme.base()), labels_area);
        for (tick, label) in ticks.iter().zip(labels) {
            let y = inner.y
                + (((max - tick) / (max - min)) * f64::from(height.saturating_sub(1))).round()
                    as u16;
            frame.render_widget(
                Paragraph::new(label)
                    .alignment(Alignment::Right)
                    .style(theme.muted()),
                Rect::new(inner.x, y, label_width, 1),
            );
            frame.buffer_mut()[(inner.x + label_width, y)]
                .set_symbol("├")
                .set_style(theme.muted());
        }
    }
    let dates = page
        .samples
        .first()
        .zip(page.samples.last())
        .map(|(first, last)| {
            format!(
                "Start #{}  {}\nEnd   #{}  {}",
                first.height,
                theme.format_time(first.time),
                last.height,
                theme.format_time(last.time)
            )
        })
        .unwrap_or_else(|| "Endpoint times unavailable".into());
    frame.render_widget(
        Paragraph::new(dates)
            .style(theme.muted())
            .wrap(Wrap { trim: false }),
        rows[4],
    );
    let value_label = |sample: &Sample| {
        let value = sample
            .value(view.metric)
            .map(number)
            .unwrap_or_else(|| "Unavailable".into());
        if view.metric == Metric::Interval {
            value
        } else {
            let average = sample
                .average(view.metric)
                .map(|mean| format!("{} ({} samples)", number(mean.value), mean.samples))
                .unwrap_or_else(|| "unavailable".into());
            format!("{value} | MA100 {average}")
        }
    };
    let detail = if let Some(sample) = selected {
        format!(
            "Selected block {} | {} | {}\nValue {}\n{}\n{}",
            sample.height,
            theme.format_time(sample.time),
            sample.production,
            value_label(sample),
            sample.hash,
            sample
                .problem
                .clone()
                .unwrap_or_else(|| if view.metric == Metric::Staking {
                    format!(
                        "PoS target {}",
                        sample.target.as_deref().unwrap_or("unavailable")
                    )
                } else {
                    String::new()
                })
        )
    } else {
        "No applicable samples in this window; missing values are not zero.".into()
    };
    let detail = if view.metric == Metric::FeePool {
        if let Some(series) = &page.pool {
            let selected = selected
                .and_then(|sample| {
                    sample.pool.map(|values| {
                        format!(
                            "Selected block {} | {}\n{}",
                            sample.height,
                            theme.format_time(sample.time),
                            fee_pool::balances(values, series)
                        )
                    })
                })
                .unwrap_or_else(|| "No fee-pool checkpoints available in this range.".into());
            let current = series
                .current
                .map(|(height, time, values)| {
                    format!(
                        "Current fee pool (observed #{height})\n{}\n{}",
                        fee_pool::balances(values, series),
                        theme.format_time(time)
                    )
                })
                .unwrap_or_else(|| {
                    "Current fee pool unavailable; no zero balance is assumed.".into()
                });
            format!("{selected}\n{current}")
        } else {
            "Loading fee-pool checkpoints and current balance…".into()
        }
    } else if view.metric == Metric::Balance {
        selected
            .map(|sample| {
                format!(
                    "Selected block {} | {} | {}\n{}\n{}\n{}",
                    sample.height,
                    theme.format_time(sample.time),
                    sample.production,
                    balance_counts_label(sample, 0),
                    balance_counts_label(sample, 1),
                    sample.hash,
                )
            })
            .unwrap_or_else(|| "No known mined/staked blocks in this window.".into())
    } else if compact && view.metric != Metric::Staking {
        selected
            .map(|sample| {
                format!(
                    "Selected block {} | {} | {}\nValue {}{}",
                    sample.height,
                    theme.format_time(sample.time),
                    sample.production,
                    value_label(sample),
                    if view.metric == Metric::Interval {
                        format!(" | {}", short_id(&sample.hash))
                    } else {
                        String::new()
                    }
                )
            })
            .unwrap_or(detail)
    } else {
        detail
    };
    frame.render_widget(
        Paragraph::new(detail)
            .style(theme.base())
            .wrap(Wrap { trim: false }),
        rows[5],
    );
    let rejected = page
        .samples
        .iter()
        .filter_map(|s| s.problem.as_deref())
        .collect::<Vec<_>>();
    let negative = page
        .samples
        .iter()
        .filter(|s| s.interval.is_some_and(|v| v < 0.0))
        .count();
    let note = if let Some(error) = &view.error {
        error.clone()
    } else if let Some(note) = &view.seek_note {
        note.clone()
    } else if view.metric == Metric::Staking && !rejected.is_empty() {
        format!(
            "{} unavailable PoS sample(s): {}",
            rejected.len(),
            rejected[0]
        )
    } else if view.metric == Metric::Interval && negative > 0 {
        format!(
            "{negative} negative timestamp interval(s) below the zero baseline; select a block to inspect its signed value."
        )
    } else if view.metric == Metric::Staking {
        "Difficulty = fixed initial Verus LWMA posLimit / observed block PoS target.".into()
    } else {
        "j/Down older point, k/Up newer point | -/+ span | PageUp/[ older range, PageDown/] newer range".into()
    };
    let footer = if view.seek_note.is_some() {
        Paragraph::new(note)
    } else if view.metric == Metric::FeePool && view.error.is_none() {
        let mut spans = Vec::new();
        if let Some(series) = &page.pool {
            spans.push(Span::styled(
                format!("{}  ", series.labels[0]),
                theme.link(),
            ));
            if series.reserve {
                spans.push(Span::styled(
                    format!("{}  ", series.labels[1]),
                    theme.positive(),
                ));
            }
            spans.push(Span::styled(
                format!(
                    "{}/{} sampled checkpoints | F5 refresh",
                    series.loaded, series.requested
                ),
                theme.muted(),
            ));
        }
        Paragraph::new(vec![
            Line::from(spans),
            Line::from(format!(
                "v currency: {} | z scale: {} | t detail: {}",
                match view.pool_currency {
                    1 => "native",
                    2 => "reserve",
                    _ => "both",
                },
                if view.pool_log {
                    "log-like (linear near zero)"
                } else if view.pool_fit {
                    "fit range"
                } else {
                    "linear from zero"
                },
                if view.pool_detail {
                    "up to 2000 points"
                } else {
                    "up to 100 points"
                },
            )),
        ])
    } else if view.metric == Metric::Balance && view.error.is_none() {
        Paragraph::new(Line::from(vec![
            Span::styled("Dots: mined / staked  ", theme.link()),
            Span::styled("100 blocks  ", theme.positive()),
            Span::styled("1000 blocks  ", theme.warning()),
            Span::styled("Center: 50/50", theme.muted()),
        ]))
    } else if view.metric != Metric::Interval && view.error.is_none() && rejected.is_empty() {
        Paragraph::new(Line::from(vec![
            Span::styled("Dots: blocks  ", theme.link()),
            Span::styled("Line: MA100", theme.positive()),
            Span::styled(" (trailing 100 chain heights)", theme.muted()),
        ]))
    } else {
        Paragraph::new(note)
    };
    frame.render_widget(
        footer
            .style(if view.error.is_some() {
                theme.warning()
            } else {
                theme.muted()
            })
            .wrap(Wrap { trim: false }),
        rows[6],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn header(height: u64, time: u64, stake: bool) -> Value {
        json!({"hash":format!("{height:064x}"),"previousblockhash":format!("{:064x}",height.saturating_sub(1)),
            "height":height,"time":time,"validationtype":if stake {"stake"} else {"work"},
            "difficulty":123.5,"nonce":format!("{:064x}",0x1c0f0f0fu32)})
    }
    fn view() -> View {
        let mut state = super::super::tests::regression_state();
        state.dashboard.recent_blocks.clear();
        let Screen::Charts(view) = screen(&state) else {
            unreachable!()
        };
        *view
    }
    #[test]
    fn fee_pool_lines_break_at_unknown_checkpoints_and_render_current_separately() {
        let headers = (0..5)
            .rev()
            .map(|h| Sample::decode(&header(h, h * 60, false)).unwrap())
            .collect::<Vec<_>>();
        let mut view = view();
        let mut page = make_page(
            &headers,
            (0, 4),
            10,
            "local",
            "observed",
            TimeSpan::Day1,
            true,
        );
        for sample in &mut page.samples {
            sample.pool_scheduled = true;
            sample.pool_requested = true;
            sample.pool =
                (sample.height != 2).then_some([sample.height as i128 * 100_000_000, 200_000_000]);
        }
        page.pool = Some(fee_pool::Series {
            labels: ["vDEX".into(), "VRSC".into()],
            reserve: true,
            current: Some((10, 600, [999_000_000, 100_000_001])),
            requested: 5,
            loaded: 4,
        });
        let segments = fee_pool::segments(&page, 0);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].len(), 2);
        assert_eq!(segments[1].len(), 2);
        // Pending scheduled samples and confirmed gaps both interrupt lines.
        page.samples[2].pool_requested = false;
        assert_eq!(fee_pool::segments(&page, 0), segments);
        assert_eq!(fee_pool::segments(&page, 1).len(), 2);
        assert_eq!(plot_bounds(&page, Metric::FeePool), (0, 240));
        assert_eq!(y_ticks(&[(0.0, 999.0)], Metric::FeePool, 5)[0], 0.0);
        view.metric = Metric::FeePool;
        view.selected = Some(page.samples[1].hash.clone());
        view.page = Some(page);
        view.loading = false;
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        for (width, height) in [(160, 34), (80, 24), (40, 10)] {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &theme, &view, frame.area()))
                .unwrap();
            if width >= 80 {
                let text = terminal
                    .backend()
                    .buffer()
                    .content
                    .chunks(width as usize)
                    .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                for needle in [
                    "Fee pool balances",
                    "Selected block 1",
                    "Current fee pool (observed #10)",
                    "9.99000000 vDEX",
                    "1.00000001 VRSC",
                    "4/5 sampled checkpoints",
                ] {
                    assert!(text.contains(needle), "missing {needle}: {text}");
                }
            }
        }
    }

    #[test]
    fn early_vdex_real_balances_render_on_linear_fitted_and_log_axes() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/vdex-fee-pool-1-1000.json"
        ))
        .unwrap();
        let mut headers = Vec::new();
        for row in fixture["samples"].as_array().unwrap().iter().rev() {
            let mut raw = header(
                row["height"].as_u64().unwrap(),
                row["time"].as_u64().unwrap(),
                false,
            );
            raw["hash"] = row["hash"].clone();
            let mut sample = Sample::decode(&raw).unwrap();
            sample.pool_scheduled = true;
            sample.pool_requested = true;
            sample.pool = Some([
                decimal_value_to_zat(&row["balances"][0]).unwrap(),
                decimal_value_to_zat(&row["balances"][1]).unwrap(),
            ]);
            headers.push(sample);
        }
        let mut page = make_page(
            &headers,
            (1, 1000),
            1000,
            "local-daemon:snapshot",
            "Captured historical observations",
            TimeSpan::Day1,
            true,
        );
        assert_eq!(page.samples.len(), 1000);
        assert_eq!(page.samples[0].pool, Some([4_220_099_012, 493_020_000_000]));
        assert_eq!(page.samples[1].pool.unwrap()[0], 6_090_138_283);
        let values = page.samples.last().unwrap().pool.unwrap();
        assert_eq!(values, [10_415_805, 21_653_915]);
        page.pool = Some(fee_pool::Series {
            labels: ["vDEX".into(), "VRSC".into()],
            reserve: true,
            current: Some((1000, page.samples.last().unwrap().time, values)),
            loaded: 1000,
            requested: 1000,
        });
        let mut view = view();
        view.metric = Metric::FeePool;
        view.page = Some(page);
        view.loading = false;
        view.pool_detail = true;
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        for (name, log, fit, currency) in [
            ("linear", false, false, 0),
            ("native", false, true, 1),
            ("log", true, false, 0),
        ] {
            view.pool_log = log;
            view.pool_fit = fit;
            view.pool_currency = currency;
            let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(160, 40)).unwrap();
            terminal
                .draw(|frame| render(frame, &theme, &view, frame.area()))
                .unwrap();
            let text = terminal
                .backend()
                .buffer()
                .content
                .chunks(160)
                .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(text.contains("0.10415805 vDEX | 0.21653915 VRSC"));
            assert!(text.contains("Current fee pool (observed #1000)"));
            if log {
                assert!(text.contains("Log-like coins"));
            }
            if let Ok(dir) = std::env::var("VERUS_EXPLORER_CHART_SNAPSHOT") {
                std::fs::write(
                    std::path::Path::new(&dir).join(format!("vdex-fee-pool-{name}.txt")),
                    text,
                )
                .unwrap();
            }
        }
        assert!(fee_pool::scale(0.10415805, true) > 4.0);
        assert_eq!(fee_pool::scale(0.0, true), 0.0);
        assert_eq!(
            fee_pool::axis_label(fee_pool::scale(0.0001, true)),
            "0.0001"
        );
        assert_eq!(fee_pool::axis_label(fee_pool::scale(100.0, true)), "100");
    }

    #[test]
    fn compact_targets_are_integer_expanded_and_checked_before_plotting() {
        assert_eq!(
            expand_compact(0x03012345).unwrap()[..4],
            [0x45, 0x23, 0x01, 0]
        );
        assert_eq!(expand_compact(0x02012345).unwrap()[..4], [0x23, 0x01, 0, 0]);
        assert_eq!(expand_compact(0x01012345).unwrap()[..4], [0x01, 0, 0, 0]);
        for compact in [0, 0x01000001, 0x03812345, 0x23000001, 0xff123456] {
            assert!(expand_compact(compact).is_none(), "{compact:x}");
        }
        assert!(expand_compact(0x210000ff).is_some());
        let mut raw = header(10, 100, true);
        let (difficulty, target) = stake_difficulty(&raw).unwrap();
        assert!((difficulty - 1.0).abs() < 0.000001);
        raw["postarget"] = json!(target);
        assert_eq!(stake_difficulty(&raw).unwrap().0, difficulty);
        raw["postarget"] = json!("0".repeat(63) + "1");
        assert!(stake_difficulty(&raw).is_err());
        raw["postarget"] = json!(123);
        assert!(stake_difficulty(&raw).is_err());
        raw["nonce"] = json!("invalid");
        assert!(stake_difficulty(&raw).is_err());
    }
    #[test]
    fn intervals_are_signed_and_require_adjacent_parent_hashes() {
        let headers = vec![
            Sample::decode(&header(2, 90, false)).unwrap(),
            Sample::decode(&header(1, 100, true)).unwrap(),
            Sample::decode(&header(0, 100, false)).unwrap(),
        ];
        let page = page(&headers, 2, 2, "local", "retained");
        assert_eq!(
            page.samples.iter().map(|s| s.interval).collect::<Vec<_>>(),
            vec![None, Some(0.0), Some(-10.0)]
        );
        assert_eq!(page.samples[2].mining, Some(123.5));
        assert!(page.samples[2].staking.is_none());
        assert!(page.samples[1].mining.is_none());
        assert!(page.samples[1].staking.is_some());
        let mut broken = headers;
        broken[0].parent = Some("f".repeat(64));
        assert!(page_from_test(&broken).samples[2].interval.is_none());
        assert!(page_from_test(&broken[..1]).samples[0].interval.is_none());
    }
    fn page_from_test(headers: &[Sample]) -> Page {
        page(headers, 2, 2, "local", "retained")
    }
    #[test]
    fn windows_have_144_samples_and_an_extra_parent_without_genesis_underflow() {
        let headers = (56..=200)
            .rev()
            .map(|h| Sample::decode(&header(h, h * 60, false)).unwrap())
            .collect::<Vec<_>>();
        let page = page(&headers, 200, 300, "local", "retained");
        assert_eq!((page.first, page.last, page.samples.len()), (57, 200, 144));
        assert_eq!(page.samples[0].interval, Some(60.0));
        assert_eq!(page.samples[143].height, 200);
        assert!(check_header(&header(2, 0, false), &format!("{:064x}", 3), 2).is_err());
        assert!(check_header(&header(2, 0, false), &format!("{:064x}", 2), 3).is_err());
    }
    #[test]
    fn partial_context_reload_keeps_the_selected_block_openable() {
        let headers: Vec<_> = (1..=5)
            .rev()
            .map(|h| Sample::decode(&header(h, h * 60, false)).unwrap())
            .collect();
        let mut view = view();
        view.metric = Metric::Mining;
        view.page = Some(page(&headers, 5, 5, "local", "retained"));
        view.select(0, Some(false));
        let selected = view.selected.clone().unwrap();
        let request = view.request;
        let mut state = super::super::tests::regression_state();
        state.screen = Screen::Charts(Box::new(view));
        let partial = make_page(
            &headers[..2],
            (4, 5),
            5,
            "local",
            "loading",
            TimeSpan::default(),
            false,
        );
        apply(&mut state, request, Some(partial), false, None);
        let Screen::Charts(view) = &state.screen else {
            unreachable!()
        };
        assert_eq!(view.selected_sample().unwrap().hash, selected);
    }
    #[test]
    fn metric_selection_and_obsolete_results_preserve_exact_samples() {
        let mut view = view();
        view.page = Some(page(
            &[
                Sample::decode(&header(2, 120, false)).unwrap(),
                Sample::decode(&header(1, 60, true)).unwrap(),
            ],
            2,
            2,
            "local",
            "retained",
        ));
        view.metric = Metric::Mining;
        view.select(0, Some(false));
        assert_eq!(view.selected_sample().unwrap().height, 2);
        let mut state = super::super::tests::regression_state();
        let request = view.request;
        state.screen = Screen::Charts(Box::new(view));
        apply(&mut state, request + 1, None, true, Some("obsolete".into()));
        let Screen::Charts(view) = &state.screen else {
            unreachable!()
        };
        assert!(view.error.is_none());
        assert!(view.loading);
        apply(&mut state, request, None, true, None);
        let Screen::Charts(view) = &state.screen else {
            unreachable!()
        };
        assert!(!view.loading);
    }
    #[test]
    fn pending_block_targets_anchor_charts_without_falling_back_to_the_tip() {
        let mut state = super::super::tests::regression_state();
        for target in [BlockTarget::Height(10), BlockTarget::Hash("f".repeat(64))] {
            state.screen = block_screen(target.clone());
            let Screen::Charts(view) = screen(&state) else {
                unreachable!()
            };
            match target {
                BlockTarget::Height(height) => {
                    assert_eq!(view.end, Some(height));
                    assert!(view.anchor.is_none());
                }
                BlockTarget::Hash(hash) => {
                    assert_eq!(view.anchor, Some(hash));
                    assert!(view.end.is_none());
                }
            }
        }
        let mut raw = header(1, 1, true);
        raw["validationtype"] = json!("unknown");
        let sample = Sample::decode(&raw).unwrap();
        assert!(sample.staking.is_none());
        assert!(sample.mining.is_none());
    }

    #[tokio::test]
    async fn retained_headers_reopen_offline_without_contacting_disabled_sources() {
        let mut config = super::super::tests::regression_config();
        config.cache_path = std::env::temp_dir().join(format!(
            "verus-charts-offline-{}-{}.sqlite3",
            std::process::id(),
            next_request()
        ));
        config
            .daemon
            .control
            .set(crate::sources::ConnectionState::Disabled);
        let source = config.source_cache_key("local-daemon");
        let cache = Cache::open(&config.cache_path).unwrap();
        for height in 0..=1 {
            let raw = header(height, height * 60, height == 1);
            cache
                .put_raw_rpc(
                    &source,
                    &config.chain_cache_key(),
                    "getblockheader",
                    &json!([raw["hash"], true]).to_string(),
                    &json!({"result":raw}).to_string(),
                )
                .unwrap();
        }
        let mut view = view();
        view.end = Some(1);
        view.anchor = Some(format!("{:064x}", 1));
        view.tip = Some(20);
        let (tx, mut rx) = unbounded_channel();
        let mut latest = None;
        load_inner(&config, &view, &tx, &mut latest).await.unwrap();
        let page = latest.unwrap();
        assert_eq!(page.samples.len(), 2);
        assert_eq!(page.tip, 20);
        assert!(page.status.contains("Retained"));
        assert!(rx.try_recv().is_ok());
        assert!(
            cached_header(&cache, &config, "another-source", &format!("{:064x}", 1))
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn connection_revision_invalidates_completed_charts_and_rejects_late_updates() {
        let mut state = super::super::tests::regression_state();
        let config = super::super::tests::regression_config();
        let mut view = view();
        view.loading = false;
        view.context = Some((
            config.daemon.control.branch_epoch.load(Ordering::Acquire),
            config.daemon.control.generation(),
        ));
        view.page = Some(page(
            &[Sample::decode(&header(2, 120, false)).unwrap()],
            2,
            2,
            "local",
            "Observed active chain",
        ));
        let old_request = view.request;
        state.screen = Screen::Charts(Box::new(view));
        config
            .daemon
            .control
            .branch_epoch
            .fetch_add(1, Ordering::AcqRel);
        let (tx, _) = unbounded_channel();
        drive(&mut state, &config, &tx, &mut None);
        apply(&mut state, old_request, None, true, Some("late".into()));
        let Screen::Charts(view) = &state.screen else {
            unreachable!()
        };
        assert_ne!(view.request, old_request);
        assert_eq!(view.page.as_ref().unwrap().status, "Membership unverified");
        assert!(view.error.as_deref().unwrap().contains("context changed"));
        assert!(!view.loading);
    }

    #[test]
    fn means_use_100_chain_heights_and_only_the_matching_production_samples() {
        let headers = (0..=249)
            .rev()
            .map(|h| {
                let mut sample = Sample::decode(&header(h, h * 60, h % 2 == 1)).unwrap();
                if h % 2 == 0 {
                    sample.mining = Some((h + 1) as f64);
                }
                sample
            })
            .collect::<Vec<_>>();
        let page = make_page(
            &headers,
            (0, 249),
            249,
            "local",
            "observed",
            TimeSpan::Day1,
            true,
        );
        assert!(page.samples[98].average(Metric::Mining).is_none());
        let mean = page.samples[199].average(Metric::Mining).unwrap();
        assert_eq!(mean.samples, 50);
        assert!((mean.value - 150.0).abs() < 1e-9);
        assert_eq!(
            page.samples[199].average(Metric::Staking).unwrap().samples,
            50
        );
        let mut bad = headers;
        bad.iter_mut().find(|s| s.height == 120).unwrap().mining = None;
        let page = make_page(
            &bad,
            (100, 249),
            249,
            "local",
            "observed",
            TimeSpan::Day1,
            true,
        );
        assert!(
            page.samples
                .iter()
                .find(|s| s.height == 219)
                .unwrap()
                .average(Metric::Mining)
                .is_none()
        );
        assert!(
            page.samples
                .iter()
                .find(|s| s.height == 220)
                .unwrap()
                .average(Metric::Mining)
                .is_some()
        );
        assert!(
            page.samples
                .iter()
                .find(|s| s.height == 150)
                .unwrap()
                .average(Metric::Staking)
                .is_some()
        );
        let segments = average_segments(&page, Metric::Mining);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].last().unwrap().0, 119.0);
        assert_eq!(segments[1].first().unwrap().0, 220.0);
    }

    #[test]
    fn both_paging_directions_load_mean_context_before_the_first_visible_sample() {
        for forward in [false, true] {
            let mut history = History::new(
                TimeSpan::Hours2,
                forward,
                1000,
                format!("{:064x}", 1000),
                5000,
            );
            while let Some((height, _)) = history.next() {
                history
                    .push(Sample::decode(&header(height, height * 60, height % 2 == 1)).unwrap())
                    .unwrap();
            }
            let page = history.page("local", "observed").unwrap();
            assert_eq!(page.samples.len(), 121);
            assert_eq!(history.observed_count(), 220);
            assert!(page.samples[0].average(Metric::Mining).is_some());
            assert!(page.samples[0].average(Metric::Staking).is_some());
        }
    }

    #[test]
    fn heading_is_above_controls_and_means_render_in_a_distinct_color() {
        let headers = (0..=249)
            .rev()
            .map(|h| {
                let mut sample = Sample::decode(&header(h, h * 60, h % 2 == 1)).unwrap();
                if h % 2 == 0 {
                    sample.mining = Some((h + 1) as f64);
                }
                sample
            })
            .collect::<Vec<_>>();
        let mut view = view();
        view.loading = false;
        view.metric = Metric::Mining;
        view.page = Some(make_page(
            &headers,
            (150, 249),
            249,
            "local",
            "observed",
            TimeSpan::Day1,
            true,
        ));
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(160, 34)).unwrap();
        terminal
            .draw(|frame| render(frame, &theme, &view, frame.area()))
            .unwrap();
        let rows = terminal
            .backend()
            .buffer()
            .content
            .chunks(160)
            .collect::<Vec<_>>();
        let line = |i: usize| rows[i].iter().map(|cell| cell.symbol()).collect::<String>();
        assert!(line(0).contains("Mining difficulty"));
        assert!(line(1).contains("Charts"));
        assert!(rows[0].iter().all(|cell| cell.bg == theme.link));
        assert!(
            rows.iter()
                .skip(5)
                .take(16)
                .flat_map(|row| row.iter())
                .any(|cell| cell.fg == theme.positive && cell.symbol() != " ")
        );
        assert!(
            rows.iter()
                .skip(5)
                .take(16)
                .flat_map(|row| row.iter())
                .any(|cell| cell.fg == theme.link && cell.symbol() != " ")
        );
        assert!(rows.iter().any(|row| {
            row.iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
                .contains("Line: MA100")
        }));
    }

    #[test]
    fn balance_windows_include_selected_block_and_preserve_unknown_or_partial_history() {
        let mut headers = (0..1500)
            .rev()
            .map(|height| Sample::decode(&header(height, height * 60, height % 4 != 0)).unwrap())
            .collect::<Vec<_>>();
        let page = make_page(
            &headers,
            (0, 1499),
            1499,
            "local",
            "observed",
            TimeSpan::Day1,
            true,
        );
        assert_eq!(page.samples[98].balance[0].total(), 99);
        assert_eq!(page.samples[98].balance[0].stake_share(100), None);
        assert_eq!(
            page.samples[99].balance[0],
            ProductionCounts {
                mined: 25,
                staked: 75,
                unknown: 0
            }
        );
        assert_eq!(page.samples[99].balance[0].stake_share(100), Some(0.75));
        assert_eq!(page.samples[998].balance[1].stake_share(1000), None);
        assert_eq!(
            page.samples[999].balance[1],
            ProductionCounts {
                mined: 250,
                staked: 750,
                unknown: 0
            }
        );
        assert_eq!(page.samples[999].balance[1].stake_share(1000), Some(0.75));
        assert!(balance_counts_label(&page.samples[98], 0).contains("partial 99/100"));
        assert_eq!(page.samples[100].value(Metric::Balance), Some(0.0));
        assert_eq!(page.samples[101].value(Metric::Balance), Some(1.0));
        let mut drift = headers.clone();
        for sample in drift.iter_mut().filter(|sample| sample.height >= 1400) {
            sample.production = "work";
        }
        let drift = make_page(
            &drift,
            (1499, 1499),
            1499,
            "local",
            "observed",
            TimeSpan::Day1,
            true,
        );
        assert_eq!(drift.samples[0].balance[0].stake_share(100), Some(0.0));
        assert_eq!(drift.samples[0].balance[1].stake_share(1000), Some(0.675));
        headers
            .iter_mut()
            .find(|s| s.height == 1200)
            .unwrap()
            .production = "unknown";
        let page = make_page(
            &headers,
            (0, 1499),
            1499,
            "local",
            "observed",
            TimeSpan::Day1,
            true,
        );
        assert!(page.samples[1299].balance[0].stake_share(100).is_none());
        assert_eq!(page.samples[1300].balance[0].stake_share(100), Some(0.75));
        assert!(page.samples[1499].balance[1].stake_share(1000).is_none());
        assert!(balance_counts_label(&page.samples[1299], 0).contains("1 unknown"));
        let segments = balance_segments(&page, 0);
        assert_eq!(segments[0].last().unwrap().0, 1199.0 * 60.0);
        assert_eq!(segments[1].first().unwrap().0, 1300.0 * 60.0);
        headers.retain(|s| s.height != 1200);
        let page = make_page(
            &headers,
            (0, 1499),
            1499,
            "local",
            "observed",
            TimeSpan::Day1,
            true,
        );
        assert_eq!(page.samples.last().unwrap().balance[1].total(), 299);
    }

    #[test]
    fn balance_loads_999_context_headers_in_both_directions() {
        for forward in [false, true] {
            let mut history = History::new(
                TimeSpan::Hours2,
                forward,
                2000,
                format!("{:064x}", 2000),
                5000,
            )
            .for_metric(Metric::Balance);
            while let Some((height, _)) = history.next() {
                history
                    .push(Sample::decode(&header(height, height * 60, height % 2 != 0)).unwrap())
                    .unwrap();
            }
            let page = history.page("local", "observed").unwrap();
            assert_eq!(history.observed_count(), 1120);
            assert_eq!(page.samples.len(), 121);
            let first = &page.samples[0];
            assert_eq!(
                first.balance[1],
                ProductionCounts {
                    mined: 500,
                    staked: 500,
                    unknown: 0
                }
            );
            assert_eq!(first.balance[0].stake_share(100), Some(0.5));
            assert_eq!(first.balance[1].stake_share(1000), Some(0.5));
        }
    }

    #[test]
    fn balance_selection_requests_deeper_context_once_and_keeps_selected_hash() {
        let mut state = super::super::tests::regression_state();
        let config = super::super::tests::regression_config();
        let (tx, _) = unbounded_channel();
        let mut view = view();
        view.loading = false;
        view.selected = Some(format!("{:064x}", 1000));
        view.page = Some(make_page(
            &[Sample::decode(&header(1000, 60_000, true)).unwrap()],
            (1000, 1000),
            1000,
            "local",
            "observed",
            TimeSpan::Day1,
            true,
        ));
        let selected = view.selected.clone();
        let request = view.request;
        state.screen = Screen::Charts(Box::new(view));
        assert!(handle_key(&mut state, KeyCode::Char('4'), &config, &tx));
        let Screen::Charts(view) = &state.screen else {
            unreachable!()
        };
        assert_eq!(view.metric, Metric::Balance);
        assert!(view.loading);
        assert_ne!(view.request, request);
        assert_eq!(view.selected, selected);
        let request = view.request;
        assert!(handle_key(&mut state, KeyCode::Char('4'), &config, &tx));
        let Screen::Charts(view) = &state.screen else {
            unreachable!()
        };
        assert_eq!(view.request, request);
        assert_eq!(view.selected, selected);
    }

    #[test]
    fn balance_render_has_fixed_binary_axis_time_coordinates_and_both_counts() {
        let headers = (0..1500)
            .rev()
            .map(|height| {
                Sample::decode(&header(
                    height,
                    1_700_000_000 + height * 60,
                    height % 4 != 0,
                ))
                .unwrap()
            })
            .collect::<Vec<_>>();
        let mut view = view();
        view.metric = Metric::Balance;
        view.loading = false;
        view.selected = Some(format!("{:064x}", 1499));
        view.page = Some(make_page(
            &headers,
            (1000, 1499),
            1499,
            "local",
            "observed",
            TimeSpan::Day1,
            true,
        ));
        let page = view.page.as_ref().unwrap();
        assert_eq!(
            plot_bounds(page, Metric::Balance),
            (1_700_060_000, 1_700_089_940)
        );
        assert_eq!(plot_x(&page.samples[1], Metric::Balance), 1_700_060_060.0);
        assert_eq!(y_ticks(&[], Metric::Balance, 5), vec![0.0, 0.5, 1.0]);
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        for (width, height) in [(160, 40), (100, 28), (80, 24)] {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &theme, &view, frame.area()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            let text = buffer
                .content
                .chunks(width as usize)
                .map(|row| row.iter().map(|c| c.symbol()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            for label in [
                "Staked",
                "Mined",
                "50/50",
                "Time",
                "100 blocks: 25 mined / 75 staked",
                "1000 blocks: 250 mined / 750 staked",
            ] {
                assert!(
                    text.contains(label),
                    "{width}x{height} missing {label}:\n{text}"
                );
            }
            let lines = text.lines().collect::<Vec<_>>();
            let staked_row = lines
                .iter()
                .position(|line| line.contains("Staked├"))
                .unwrap();
            let mined_row = lines
                .iter()
                .position(|line| line.contains("Mined├"))
                .unwrap();
            let chart_top = lines.iter().position(|line| line.starts_with('┌')).unwrap();
            let chart_bottom = lines.iter().position(|line| line.starts_with('└')).unwrap();
            assert!(staked_row > chart_top + 1, "staked dots need headroom");
            assert!(mined_row + 2 < chart_bottom, "mined dots need footroom");
            assert!(
                buffer
                    .content
                    .iter()
                    .any(|c| c.fg == theme.positive && c.symbol() != " ")
            );
            assert!(
                buffer
                    .content
                    .iter()
                    .any(|c| c.fg == theme.warning && c.symbol() != " ")
            );
        }
    }

    #[test]
    fn graduated_axes_have_zero_interval_baseline_and_uniform_intermediate_ticks() {
        for metric in [Metric::Interval, Metric::Mining, Metric::Staking] {
            let data = vec![(1.0, 0.0), (2.0, 63.0), (3.0, 120.0)];
            let ticks = y_ticks(&data, metric, 5);
            assert!(ticks.len() >= 3);
            assert!(ticks[0] >= 0.0);
            if matches!(metric, Metric::Interval | Metric::FeePool) {
                assert_eq!(ticks[0], 0.0);
            }
            let step = ticks[1] - ticks[0];
            assert!(
                ticks
                    .windows(2)
                    .all(|v| ((v[1] - v[0]) - step).abs() < 1e-8)
            );
            assert!(*ticks.last().unwrap() >= 120.0);
        }
        let negative = vec![(1.0, -10.0), (2.0, 0.0)];
        assert_eq!(y_ticks(&negative, Metric::Interval, 5)[0], 0.0);
        assert_eq!(x_ticks(1000, 2000, 5), vec![1000, 1250, 1500, 1750, 2000]);
        assert_eq!(x_ticks(0, 1, 5), vec![0, 1]);
        assert_ne!(tick_label(1.00001, 0.00001), tick_label(1.00002, 0.00001));
    }

    #[test]
    fn j_and_down_move_older_and_k_and_up_move_newer() {
        let mut state = super::super::tests::regression_state();
        let config = super::super::tests::regression_config();
        let (tx, _) = unbounded_channel();
        let mut view = view();
        view.loading = false;
        view.page = Some(page(
            &[
                Sample::decode(&header(2, 120, false)).unwrap(),
                Sample::decode(&header(1, 60, false)).unwrap(),
                Sample::decode(&header(0, 0, false)).unwrap(),
            ],
            2,
            2,
            "local",
            "retained",
        ));
        view.metric = Metric::Mining;
        state.screen = Screen::Charts(Box::new(view));
        for (key, height) in [
            (KeyCode::Char('j'), 1),
            (KeyCode::Char('k'), 2),
            (KeyCode::Down, 1),
            (KeyCode::Up, 2),
        ] {
            assert!(handle_key(&mut state, key, &config, &tx));
            let Screen::Charts(view) = &state.screen else {
                unreachable!()
            };
            assert_eq!(view.selected_sample().unwrap().height, height);
        }
        assert!(handle_key(&mut state, KeyCode::Char('-'), &config, &tx));
        let Screen::Charts(view) = &state.screen else {
            unreachable!()
        };
        assert_eq!(view.span, TimeSpan::Hours6);
        assert!(view.loading);
    }

    #[test]
    fn time_spans_measure_observed_timestamps_and_page_without_overlap() {
        let mut history =
            History::new(TimeSpan::Day1, false, 5000, format!("{:064x}", 5000), 10000);
        while let Some((height, _)) = history.next() {
            history
                .push(Sample::decode(&header(height, 1_700_000_000 + height * 30, false)).unwrap())
                .unwrap();
        }
        assert!(history.complete);
        let older = history.page("local", "observed").unwrap();
        assert_eq!(
            (older.first, older.last, older.samples.len()),
            (2120, 5000, 2881)
        );
        assert_eq!(
            older.samples.last().unwrap().time - older.samples.first().unwrap().time,
            86400
        );
        assert_eq!(older.samples.first().unwrap().interval, Some(30.0));
        let mut next = History::new(TimeSpan::Day1, true, 5000, format!("{:064x}", 5000), 10000);
        while let Some((height, _)) = next.next() {
            next.push(Sample::decode(&header(height, 1_700_000_000 + height * 30, false)).unwrap())
                .unwrap();
        }
        let newer = next.page("local", "observed").unwrap();
        assert_eq!(newer.first, older.last + 1);
        assert_eq!(
            newer.samples.last().unwrap().time - newer.samples.first().unwrap().time,
            86400
        );
        assert_eq!(newer.samples.first().unwrap().interval, Some(30.0));
        assert_eq!(TimeSpan::Hours2.step(-1), TimeSpan::Hours2);
        assert_eq!(TimeSpan::Days7.step(1), TimeSpan::Days7);
    }

    #[test]
    fn history_limits_and_discontinuous_forward_headers_do_not_claim_complete_coverage() {
        let mut history = History::new(
            TimeSpan::Days7,
            false,
            30000,
            format!("{:064x}", 30000),
            30000,
        );
        while let Some((height, _)) = history.next() {
            history
                .push(Sample::decode(&header(height, 1_700_000_000, false)).unwrap())
                .unwrap();
        }
        assert_eq!(history.headers.len(), MAX_HEADERS);
        assert!(!history.complete);
        assert!(!history.page("local", "partial").unwrap().complete);
        let mut forward = History::new(TimeSpan::Hours2, true, 2, format!("{:064x}", 2), 10);
        forward
            .push(Sample::decode(&header(2, 120, false)).unwrap())
            .unwrap();
        let mut wrong = Sample::decode(&header(3, 180, false)).unwrap();
        wrong.parent = Some("f".repeat(64));
        assert!(forward.push(wrong).is_err());
    }

    #[test]
    fn charts_render_at_narrow_and_wide_sizes_with_missing_stake_data() {
        let mut view = view();
        let mut bad = header(2, 120, true);
        bad["postarget"] = json!("0".repeat(64));
        view.page = Some(page(
            &[
                Sample::decode(&bad).unwrap(),
                Sample::decode(&header(1, 60, false)).unwrap(),
            ],
            2,
            2,
            "local",
            "retained",
        ));
        for (width, height) in [(80, 18), (160, 34), (40, 10)] {
            for metric in [Metric::Interval, Metric::Mining, Metric::Staking] {
                view.metric = metric;
                let backend = ratatui::backend::TestBackend::new(width, height);
                let mut terminal = Terminal::new(backend).unwrap();
                let theme = Theme::desert(crate::cli::EmojiMode::Off);
                terminal
                    .draw(|frame| render(frame, &theme, &view, frame.area()))
                    .unwrap();
                let text = terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>();
                assert!(text.contains("Charts"));
                if width >= 80 {
                    assert!(text.contains("Block height"), "{width}x{height}: {text}");
                    assert!(text.contains("Start #"));
                    assert!(text.contains("End   #"));
                    assert!(text.contains("1970-01-01"));
                    let grid = grid_color(&theme);
                    assert!(
                        terminal
                            .backend()
                            .buffer()
                            .content
                            .iter()
                            .any(|cell| cell.fg == grid && cell.symbol() != " ")
                    );
                    let tick_rows = terminal
                        .backend()
                        .buffer()
                        .content
                        .chunks(width as usize)
                        .filter(|row| row.iter().any(|cell| cell.symbol() == "├"))
                        .collect::<Vec<_>>();
                    assert!(tick_rows.len() >= 3);
                    for row in tick_rows {
                        assert!(
                            row.iter()
                                .any(|cell| cell.symbol().chars().any(|c| c.is_ascii_digit()))
                        );
                        assert!(
                            row.iter()
                                .any(|cell| cell.fg == grid && cell.symbol() != " ")
                        );
                    }
                    if metric == Metric::Staking {
                        assert!(text.contains("unavailable PoS sample"));
                    } else {
                        assert!(text.contains("Selected block"));
                    }
                }
            }
        }
    }
}
