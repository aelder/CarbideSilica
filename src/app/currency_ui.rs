use super::*;
use crate::cache::CurrencySnapshot;
use crate::currency_activity::{self, Range, Record, StoredPage};

static NEXT_REQUEST: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
pub(super) fn request_id() -> u64 {
    NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Clone)]
pub(super) struct View {
    pub query: String,
    pub system: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub definition: Option<Value>,
    pub snapshots: Vec<CurrencySnapshot>,
    pub historical: Option<CurrencySnapshot>,
    pub selected: usize,
    pub scroll: u16,
    pub mode: ViewMode,
    pub json: JsonState,
    pub document: Value,
    pub loading: bool,
    pub error: Option<String>,
    pub height_input: Option<String>,
    pub history_request: Option<u64>,
    pub history_note: Option<String>,
    pub activity: Option<StoredPage>,
    pub retained_activity: Vec<StoredPage>,
    pub activity_retained: bool,
    pub activity_request: Option<u64>,
    pub transfer_request: Option<u64>,
    pub transfer_inputs: Vec<crate::world::OutpointRef>,
    pub transfer_export: Option<crate::world::OutpointRef>,
    pub transfer_truncated: bool,
    pub notarizations: notarization_ui::Feed,
}

#[derive(Debug, Clone)]
pub(super) struct Loaded {
    pub query: String,
    pub id: Option<String>,
    pub name: Option<String>,
    pub definition: Option<Value>,
    pub snapshots: Vec<CurrencySnapshot>,
    pub error: Option<String>,
    pub activity: Vec<StoredPage>,
}

#[derive(Debug, Clone)]
pub(super) enum Link {
    Currency(String),
    Transaction(String),
    Historical(u64),
    Block(String),
    Activity(Record),
    ActivityWindow(usize),
    Trail(crate::world::OutpointRef),
    Notarizations,
}

pub(super) fn screen(query: String, system: String) -> Screen {
    Screen::Currency(Box::new(View {
        query,
        system,
        id: None,
        name: None,
        definition: None,
        snapshots: Vec::new(),
        historical: None,
        selected: 0,
        scroll: 0,
        mode: ViewMode::Overview,
        json: JsonState::new(),
        document: Value::Null,
        loading: true,
        error: None,
        height_input: None,
        history_request: None,
        history_note: None,
        activity: None,
        retained_activity: Vec::new(),
        activity_retained: false,
        activity_request: None,
        transfer_request: None,
        transfer_inputs: Vec::new(),
        transfer_export: None,
        transfer_truncated: false,
        notarizations: Default::default(),
    }))
}

fn cache_data(config: &Config, query: &str) -> Loaded {
    let system = config.chain_cache_key();
    let mut loaded = Loaded {
        query: query.into(),
        id: None,
        name: None,
        definition: None,
        snapshots: Vec::new(),
        error: None,
        activity: Vec::new(),
    };
    match Cache::open(&config.cache_path) {
        Ok(cache) => {
            let resolved = cache
                .currency_definition(&system, query)
                .ok()
                .flatten()
                .or_else(|| {
                    cache
                        .currency_definitions(&system)
                        .ok()?
                        .into_iter()
                        .find(|(_, name, _)| name.eq_ignore_ascii_case(query))
                });
            if let Some((id, name, definition)) = resolved {
                loaded.snapshots = cache.currency_snapshots(&system, &id).unwrap_or_default();
                loaded.activity = cache.currency_activity(&system, &id).unwrap_or_default();
                loaded.id = Some(id);
                loaded.name = Some(name);
                loaded.definition = Some(definition);
            }
        }
        Err(error) => loaded.error = Some(error.to_string()),
    }
    loaded
}

pub(super) fn spawn_load(
    config: Config,
    tx: UnboundedSender<AppEvent>,
    query: String,
    refresh: bool,
) {
    tokio::spawn(load(config, tx, query, refresh));
}

pub(super) fn spawn_cached(config: Config, tx: UnboundedSender<AppEvent>, query: String) {
    let tx = revision_sender(&config, tx);
    tokio::spawn(async move {
        if let Ok(loaded) = tokio::task::spawn_blocking(move || cache_data(&config, &query)).await {
            let _ = tx.send(AppEvent::CurrencyViewLoaded(loaded));
        }
    });
}

/// Explicit navigation from a replayed pool leg loads its definition first,
/// then selects the executing system's historical state at the import height.
pub(super) fn spawn_at_height(
    config: Config,
    tx: UnboundedSender<AppEvent>,
    currency_id: String,
    height: u64,
    request_id: u64,
) {
    let tx = revision_sender(&config, tx);
    tokio::spawn(async move {
        load(config.clone(), tx.clone(), currency_id.clone(), false).await;
        let cached_config = config.clone();
        let cached_id = currency_id.clone();
        let cached = tokio::task::spawn_blocking(move || cache_data(&cached_config, &cached_id))
            .await
            .ok()
            .and_then(|loaded| {
                loaded.snapshots.into_iter().find(|snapshot| {
                    snapshot.kind == "historical" && snapshot.reported_height == Some(height)
                })
            });
        if let Some(snapshot) = cached {
            let _ = tx.send(AppEvent::CurrencyHistoryLoaded {
                query: currency_id,
                request_id,
                note: None,
                result: Ok(snapshot),
            });
        } else {
            spawn_history(
                config,
                tx,
                currency_id.clone(),
                currency_id,
                chart_ui::seek::Target::Height(height),
                request_id,
            );
        }
    });
}

async fn load(config: Config, tx: UnboundedSender<AppEvent>, query: String, refresh: bool) {
    let cached_config = config.clone();
    let cached_query = query.clone();
    let mut cached_found = false;
    let mut cached_states = false;
    if let Ok(cached) =
        tokio::task::spawn_blocking(move || cache_data(&cached_config, &cached_query)).await
    {
        cached_found = cached.definition.is_some();
        cached_states = !cached.snapshots.is_empty();
        let _ = tx.send(AppEvent::CurrencyViewLoaded(cached));
    }
    let source_enabled =
        config.daemon.control.state() != crate::sources::ConnectionState::Disabled
            || config.daemon.api_pool.endpoints.iter().any(|endpoint| {
                endpoint.control.state() != crate::sources::ConnectionState::Disabled
            });
    if cached_found && !refresh && (cached_states || !source_enabled) {
        return;
    }
    let result = async {
        let client = RpcClient::for_config(&config)?;
        let client = if cached_found && !refresh {
            client.noninteractive()
        } else {
            client
        };
        let output = client.get_currency(&query).await?;
        let id = output.result["currencyid"]
            .as_str()
            .context("getcurrency omitted currencyid")?
            .to_string();
        let next_config = config.clone();
        Ok::<Loaded, anyhow::Error>(
            tokio::task::spawn_blocking(move || cache_data(&next_config, &id)).await?,
        )
    }
    .await;
    let loaded = match result {
        Ok(mut loaded) => {
            loaded.query = query;
            loaded
        }
        Err(error) => Loaded {
            query,
            id: None,
            name: None,
            definition: None,
            snapshots: Vec::new(),
            error: Some(error.to_string()),
            activity: Vec::new(),
        },
    };
    let _ = tx.send(AppEvent::CurrencyViewLoaded(loaded));
}

pub(super) fn spawn_history(
    config: Config,
    tx: UnboundedSender<AppEvent>,
    query: String,
    currency_id: String,
    target: chart_ui::seek::Target,
    request_id: u64,
) {
    let tx = revision_sender(&config, tx);
    tokio::spawn(async move {
        let mut note = None;
        let result = async {
            use crate::block_time::{RpcTimeSource, TimeSource};
            use chart_ui::seek::Target;
            let config = config.noninteractive();
            let client = RpcClient::for_config(&config)?;
            let before = client.get_blockchain_info().await?;
            let tip_height = before.result["blocks"]
                .as_u64()
                .context("tip height unavailable")?;
            let target = match target {
                Target::Number(number) if number <= tip_height => Target::Height(number),
                Target::Number(number) => Target::Time(i64::try_from(number)?),
                other => other,
            };
            let height = match target {
                Target::Height(height) => {
                    anyhow::ensure!(height <= tip_height, "Future currency state is unavailable");
                    height
                }
                Target::Time(time) => {
                    let (mut source, tip, _) = RpcTimeSource::open(&config).await?;
                    anyhow::ensure!(
                        source.source_id() == before.source_id
                            && Some(tip.hash.as_str()) == before.result["bestblockhash"].as_str(),
                        "source or tip changed during time lookup"
                    );
                    let genesis = source.block_at(0).await?;
                    anyhow::ensure!(
                        time >= genesis.timestamp && time <= tip.timestamp,
                        "Enter a time between genesis and the current tip"
                    );
                    let found = crate::block_time::locate(&mut source, time, tip).await?;
                    source.check(&found.block).await?;
                    note = Some(found.description());
                    found.block.height
                }
                _ => anyhow::bail!("Enter a block height or date/time"),
            };
            let response = client.get_currency_state(&currency_id, height).await?;
            let block = client.get_block(BlockId::Height(height), 1).await?;
            let after = client.get_blockchain_info().await?;
            anyhow::ensure!(
                before.source_id == response.source_id
                    && response.source_id == block.source_id
                    && block.source_id == after.source_id,
                "source changed during historical lookup"
            );
            anyhow::ensure!(
                before.result["bestblockhash"] == after.result["bestblockhash"]
                    && before.result["blocks"] == after.result["blocks"],
                "tip changed during historical lookup"
            );
            anyhow::ensure!(block.result.height == height, "block height mismatch");
            let record = response
                .result
                .iter()
                .find(|record| record["height"].as_u64() == Some(height))
                .context("requested height absent from getcurrencystate result")?;
            let state = record["currencystate"]
                .as_object()
                .context("currency state absent")?;
            anyhow::ensure!(
                state.get("currencyid").and_then(Value::as_str) == Some(&currency_id),
                "currency ID mismatch"
            );
            let snapshot = CurrencySnapshot {
                source: response.source_id.clone(),
                kind: "historical".into(),
                reported_height: Some(height),
                observed: Some(crate::cache::BlockContext {
                    height,
                    hash: block.result.hash,
                }),
                current: false,
                state: Value::Object(state.clone()),
                response: Value::Array(response.result),
            };
            let receipt = response
                .receipt
                .context("historical RPC lacked storage receipt")?;
            let cache = Cache::open(&config.cache_path)?;
            cache
                .record_historical_currency(
                    config.chain_cache_key(),
                    currency_id,
                    snapshot.source.clone(),
                    snapshot.observed.clone().unwrap(),
                    snapshot.state.clone(),
                    snapshot.response.clone(),
                    receipt,
                )
                .await?;
            Ok::<CurrencySnapshot, anyhow::Error>(snapshot)
        }
        .await
        .map_err(|error| error.to_string());
        let _ = tx.send(AppEvent::CurrencyHistoryLoaded {
            query,
            request_id,
            note,
            result,
        });
    });
}

fn selected_anchor(view: &View) -> Option<(String, crate::cache::BlockContext)> {
    view.historical
        .as_ref()
        .or_else(|| {
            view.snapshots
                .iter()
                .find(|s| s.kind == "bestcurrencystate" && s.observed.is_some())
        })
        .and_then(|anchor| {
            anchor
                .observed
                .clone()
                .map(|block| (anchor.source.clone(), block))
        })
}

fn request_activity(
    view: &mut View,
    config: &Config,
    tx: &UnboundedSender<AppEvent>,
    range: Range,
    source: String,
    anchor: crate::cache::BlockContext,
) {
    if view.activity_request.is_some() || view.loading {
        return;
    }
    let Some(currency) = view.id.clone() else {
        return;
    };
    let id = request_id();
    view.activity_request = Some(id);
    view.error = None;
    let config = config.clone();
    let tx = tx.clone();
    let query = view.query.clone();
    tokio::spawn(async move {
        let result = currency_activity::load(config, currency, range, source, anchor)
            .await
            .map_err(|e| e.to_string());
        let _ = tx.send(AppEvent::CurrencyActivityLoaded {
            query,
            request_id: id,
            result,
        });
    });
}

pub(super) fn start_activity(view: &mut View, config: &Config, tx: &UnboundedSender<AppEvent>) {
    if let Some((source, block)) = selected_anchor(view) {
        request_activity(
            view,
            config,
            tx,
            Range {
                first: block.height,
                last: block.height,
            },
            source,
            block,
        );
    } else {
        view.error = Some("Load an anchored state before checking activity".into());
    }
}

pub(super) fn start_window(
    view: &mut View,
    config: &Config,
    tx: &UnboundedSender<AppEvent>,
    direction: i8,
) {
    if direction == 0 {
        if let Some((source, block)) = selected_anchor(view) {
            request_activity(
                view,
                config,
                tx,
                Range::ending_at(block.height),
                source,
                block,
            );
        } else {
            view.error = Some("Load an anchored state before checking activity".into());
        }
    } else if let Some(page) = &view.activity {
        let range = if direction < 0 {
            page.page.range.older()
        } else {
            page.page.range.newer(page.page.anchor.height)
        };
        if let Some(range) = range {
            let source = page.page.source.clone();
            let anchor = page.page.anchor.clone();
            request_activity(view, config, tx, range, source, anchor);
        }
    }
}

pub(super) fn clear_activity(view: &mut View) {
    view.activity = None;
    view.activity_request = None;
    view.transfer_request = None;
    view.transfer_inputs.clear();
    view.transfer_export = None;
    rebuild_document(view);
}

pub(super) fn spawn_transfers(
    config: Config,
    tx: UnboundedSender<AppEvent>,
    query: String,
    request_id: u64,
    record: Record,
) {
    tokio::spawn(async move {
        let result = async {
            let target = record
                .export
                .as_ref()
                .context("no source export reference")?;
            let client = RpcClient::for_config(&config.noninteractive())?;
            let response = client.get_raw_transaction(&target.txid).await?;
            currency_activity::export_inputs(&record, &response.result)
        }
        .await
        .map_err(|e: anyhow::Error| e.to_string());
        let _ = tx.send(AppEvent::CurrencyTransfersLoaded {
            query,
            request_id,
            result,
        });
    });
}

pub(super) fn apply(view: &mut View, loaded: Loaded) {
    if view.query != loaded.query {
        return;
    }
    if let Some(id) = loaded.id {
        if view.id.as_ref().is_some_and(|previous| previous != &id) {
            view.notarizations = Default::default();
        }
        view.id = Some(id);
    }
    if let Some(name) = loaded.name {
        view.name = Some(name);
    }
    if let Some(definition) = loaded.definition {
        view.definition = Some(definition);
    }
    if !loaded.snapshots.is_empty() {
        view.snapshots = loaded.snapshots;
    }
    if !loaded.activity.is_empty() {
        if view.activity.is_none() && view.activity_request.is_none() {
            view.activity = loaded.activity.first().cloned();
            view.activity_retained = true;
        }
        view.retained_activity = loaded.activity;
    }
    view.error = loaded.error;
    view.loading = view.history_request.is_some();
    rebuild_document(view);
}

fn preferred<'a>(view: &'a View, kind: &str) -> Option<&'a CurrencySnapshot> {
    view.snapshots
        .iter()
        .find(|s| s.kind == kind && s.current)
        .or_else(|| {
            view.snapshots
                .iter()
                .find(|s| s.kind == kind && s.observed.is_some())
        })
        .or_else(|| view.snapshots.iter().find(|s| s.kind == kind))
}

fn latest_unanchored<'a>(view: &'a View, kind: &str) -> Option<&'a CurrencySnapshot> {
    view.snapshots
        .iter()
        .find(|s| s.kind == kind && s.observed.is_none())
}

pub(super) fn rebuild_document(view: &mut View) {
    let best = preferred(view, "bestcurrencystate").map(snapshot_document);
    let confirmed = preferred(view, "lastconfirmedcurrencystate").map(snapshot_document);
    let unanchored_best = latest_unanchored(view, "bestcurrencystate").map(snapshot_document);
    let unanchored_confirmed =
        latest_unanchored(view, "lastconfirmedcurrencystate").map(snapshot_document);
    view.document = json!({
        "currencyid": view.id, "observing_system": view.system, "definition": view.definition,
        "bestcurrencystate": best,
        "lastconfirmedcurrencystate": confirmed,
        "latest_unanchored_best": unanchored_best,
        "latest_unanchored_confirmed": unanchored_confirmed,
        "historical": view.historical.as_ref().map(snapshot_document),
        "activity": view.activity.as_ref().map(|stored| &stored.page),
        "activity_membership": view.activity.as_ref().map(|stored| &stored.membership),
        "activity_retained": view.activity_retained,
        "export_inputs": {"export":view.transfer_export,"candidates":view.transfer_inputs,"truncated":view.transfer_truncated},
    });
}

fn snapshot_document(snapshot: &CurrencySnapshot) -> Value {
    json!({
        "source": snapshot.source,
        "reported_height": snapshot.reported_height,
        "observed_height": snapshot.observed.as_ref().map(|block| block.height),
        "observed_block_hash": snapshot.observed.as_ref().map(|block| &block.hash),
        "matched_source_tip_when_loaded": snapshot.current,
        "state": snapshot.state,
    })
}

fn field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .filter(|v| !v.is_null())
        .map_or("—".into(), |v| {
            v.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| v.to_string())
        })
}

fn source_result(snapshot: &CurrencySnapshot) -> &Value {
    snapshot
        .response
        .get("result")
        .unwrap_or(&snapshot.response)
}

fn snapshot_label(snapshot: &CurrencySnapshot, height: &impl Fn(u64) -> String) -> String {
    match &snapshot.observed {
        Some(block) if snapshot.current => format!(
            "matched source tip when loaded: {} {} via {} (reported {})",
            height(block.height),
            short_id(&block.hash),
            short_id(&snapshot.source),
            snapshot.reported_height.map_or("?".into(), height)
        ),
        Some(block) => format!(
            "anchored at {} {} via {} (current membership unverified; reported {})",
            height(block.height),
            short_id(&block.hash),
            short_id(&snapshot.source),
            snapshot.reported_height.map_or("?".into(), height)
        ),
        None => format!(
            "unanchored via {} (reported {})",
            short_id(&snapshot.source),
            snapshot.reported_height.map_or("?".into(), height)
        ),
    }
}

// CCurrencyDefinition::EOptions, src/pbaas/crosschainrpc.h (daemon 1.2.18).
pub(super) const OPTION_FRACTIONAL: u64 = 1;
pub(super) const OPTION_TAGS: &[(u64, &str)] = &[
    (OPTION_FRACTIONAL, "Fractional"),
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

pub(super) fn option_tags(definition: &Value) -> Vec<String> {
    let Some(options) = definition["options"].as_u64() else {
        return vec!["Options unavailable".into()];
    };
    let mut tags: Vec<String> = OPTION_TAGS
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

pub(super) fn launch_badge(
    theme: &Theme,
    status: crate::currency_status::LaunchStatus,
) -> Span<'static> {
    use crate::currency_status::LaunchStatus;
    let color = match status {
        LaunchStatus::Launched => theme.positive,
        LaunchStatus::Prelaunch => theme.warning,
        LaunchStatus::Failed => theme.negative,
        LaunchStatus::Unknown => theme.muted,
    };
    Span::styled(
        format!(" {} ", status.label()),
        theme.status_badge_on(color),
    )
}

pub(super) fn tag_line(theme: &Theme, tags: &[String]) -> Line<'static> {
    Line::from(
        tags.iter()
            .flat_map(|tag| {
                [
                    Span::styled(format!(" {tag} "), theme.status_badge()),
                    Span::raw(" "),
                ]
            })
            .collect::<Vec<_>>(),
    )
}

fn section_heading(label: &str) -> bool {
    matches!(
        label,
        "Notarizations (current)"
            | "Definition"
            | "Basket currencies (definition weights)"
            | "Best state"
            | "Last confirmed state"
            | "Historical state"
    ) || label.starts_with("Currency activity heights")
        || label.starts_with("Export inputs from")
        || label.starts_with("Retained activity")
        || label.starts_with("Retained historical")
}

pub(super) fn rows(view: &View) -> Vec<(String, Option<Link>)> {
    rows_display(view, None, None, false, None)
}

type CopyFields = HashMap<String, Vec<(&'static str, std::ops::Range<usize>)>>;

fn rows_display(
    view: &View,
    theme: Option<&Theme>,
    store: Option<&ObjectStore>,
    raw_identities: bool,
    mut copy_fields: Option<&mut CopyFields>,
) -> Vec<(String, Option<Link>)> {
    let height =
        |h: u64| theme.map_or_else(|| h.to_string(), |t| t.format_height_on(&view.system, h));
    let mut rows = vec![
        (
            format!("Currency {}", view.name.as_deref().unwrap_or(&view.query)),
            None,
        ),
        (view.id.as_deref().unwrap_or("?").to_string(), None),
    ];
    if let Some(input) = &view.height_input {
        rows.push((
            format!("Historical height / date: {input}_  (Enter to load, Esc to cancel)"),
            None,
        ));
    }
    if let Some(note) = &view.history_note {
        rows.push((note.clone(), None));
    }
    if view.loading {
        rows.push(("Loading currency data…".into(), None));
    }
    if let Some(error) = &view.error {
        rows.push((format!("Source: {error}"), None));
    }
    let Some(definition) = &view.definition else {
        rows.push((
            "Definition unavailable; refresh with r when a source is enabled".into(),
            None,
        ));
        return rows;
    };
    let status = preferred(view, "bestcurrencystate")
        .or_else(|| preferred(view, "lastconfirmedcurrencystate"))
        .map_or(crate::currency_status::LaunchStatus::Unknown, |snapshot| {
            crate::currency_status::classify(&snapshot.state)
        });
    rows.push((
        format!("Launch status (observed): {}", status.label()),
        None,
    ));
    rows.extend(notarization_ui::summary(
        &view.notarizations,
        &view.system,
        theme,
    ));
    rows.push(("Definition".into(), None));
    for chunk in option_tags(definition).chunks(3) {
        rows.push((format!("Configuration: {}", chunk.join(" | ")), None));
    }
    let mut properties = Vec::new();
    for (key, label) in [
        ("name", "Name"),
        ("parent", "Parent"),
        ("systemid", "System"),
        ("launchsystemid", "Launch system"),
        ("startblock", "Start block"),
        ("endblock", "End block"),
        ("initialsupply", "Initial supply"),
        ("options", "Options"),
    ] {
        let value = if matches!(key, "startblock" | "endblock") {
            definition[key]
                .as_u64()
                .filter(|h| *h > 0)
                .map(|h| {
                    let system = definition[if key == "startblock" {
                        "launchsystemid"
                    } else {
                        "systemid"
                    }]
                    .as_str()
                    .unwrap_or(&view.system);
                    theme.map_or_else(|| h.to_string(), |t| t.format_height_on(system, h))
                })
                .unwrap_or_else(|| field(definition, key))
        } else {
            field(definition, key)
        };
        let value = if let Some(store) = store.filter(|_| !raw_identities) {
            store
                .currency_name(&value)
                .or_else(|| store.identity_name(&value))
                .map(str::to_string)
                .unwrap_or(value)
        } else {
            value
        };
        properties.push((label, value));
    }
    for pair in properties.chunks(2) {
        let (label, value) = &pair[0];
        let mut row = format!("  {label}: {value}");
        if let Some((label, value)) = pair.get(1) {
            row.push_str(&format!(" | {label}: {value}"));
        }
        rows.push((row, None));
    }
    rows.push((
        format!(
            "  Observing system: {}",
            store
                .and_then(|store| store.currency_name(&view.system))
                .unwrap_or(&view.system)
        ),
        None,
    ));
    let locator = view
        .snapshots
        .iter()
        .find_map(|s| source_result(s)["definitiontxid"].as_str());
    if let Some(txid) = locator {
        rows.push((
            format!("  definition tx: {txid}"),
            Some(Link::Transaction(txid.into())),
        ));
    }
    for (key, label) in [
        ("besttxid", "best state tx"),
        ("lastconfirmedtxid", "last-confirmed state tx"),
    ] {
        if let Some(txid) = view
            .snapshots
            .iter()
            .find_map(|s| source_result(s)[key].as_str())
        {
            rows.push((
                format!("  {label}: {txid}"),
                Some(Link::Transaction(txid.into())),
            ));
        }
    }
    if definition["options"]
        .as_u64()
        .is_some_and(|options| options & OPTION_FRACTIONAL != 0)
        && let Some(currencies) = definition["currencies"].as_array()
        && !currencies.is_empty()
    {
        rows.push(("Basket currencies (definition weights)".into(), None));
        let entries = currencies
            .iter()
            .filter_map(Value::as_str)
            .enumerate()
            .map(|(index, id)| {
                let name = view
                    .snapshots
                    .iter()
                    .find_map(|s| source_result(s)["currencynames"][id].as_str())
                    .or_else(|| store.and_then(|store| store.currency_name(id)))
                    .unwrap_or(id);
                let weight = definition["weights"]
                    .get(index)
                    .map_or("—".into(), Value::to_string);
                (
                    vec![name.into(), weight, id.into()],
                    Some(Link::Currency(id.into())),
                )
            })
            .collect();
        table_rows(
            &mut rows,
            &["Currency", "Weight", "i-address"],
            &[false, true, false],
            entries,
            copy_fields.as_deref_mut(),
        );
    }
    for (label, kind) in [
        ("Best state", "bestcurrencystate"),
        ("Last confirmed state", "lastconfirmedcurrencystate"),
    ] {
        rows.push((label.into(), None));
        if let Some(snapshot) = preferred(view, kind) {
            state_rows(
                &mut rows,
                snapshot,
                &height,
                store,
                copy_fields.as_deref_mut(),
            );
        } else {
            rows.push(("  Unavailable".into(), None));
        }
        if let Some(snapshot) = latest_unanchored(view, kind)
            && preferred(view, kind).is_some_and(|preferred| {
                preferred.observed.is_some()
                    && (preferred.state != snapshot.state
                        || preferred.reported_height != snapshot.reported_height
                        || preferred.source != snapshot.source)
            })
        {
            rows.push((
                "  Newer or separate unanchored observation (not confirmed as current):".into(),
                None,
            ));
            state_rows(
                &mut rows,
                snapshot,
                &height,
                store,
                copy_fields.as_deref_mut(),
            );
        }
    }
    if let Some(snapshot) = &view.historical {
        rows.push(("Historical state".into(), None));
        state_rows(&mut rows, snapshot, &height, store, copy_fields);
    }
    if view.activity_request.is_some() {
        rows.push(("Loading activity window…".into(), None));
    }
    if let Some(stored) = &view.activity {
        let page = &stored.page;
        rows.push((
            format!(
                "Currency activity heights {}..{} (max {}; {})",
                height(page.range.first),
                height(page.range.last),
                currency_activity::MAX_RECORDS,
                if view.activity_retained {
                    "retained observation"
                } else {
                    "anchor checked"
                }
            ),
            None,
        ));
        rows.push((
            format!(
                "  source {} | anchor {}:{} | membership {}",
                short_id(&page.source),
                height(page.anchor.height),
                short_id(&page.anchor.hash),
                stored.membership
            ),
            None,
        ));
        rows.push((
            "  [ older window | ] newer window | F8 inspect selected batch inputs".into(),
            None,
        ));
        if page.records.is_empty() {
            rows.push(("  None found in this window".into(), None));
        }
        for record in &page.records {
            rows.push((
                format!(
                    "  {} {:7} {}:{}",
                    height(record.height),
                    if record.kind == currency_activity::Kind::Import {
                        "Import"
                    } else {
                        "Export"
                    },
                    short_id(&record.output.txid),
                    record.output.vout
                ),
                Some(Link::Activity(record.clone())),
            ));
        }
        if page.truncated {
            rows.push((
                "  Results truncated; use a narrower height window to inspect a busy block".into(),
                None,
            ));
        }
    }
    if view.transfer_request.is_some() {
        rows.push(("Loading selected export inputs…".into(), None));
    }
    if let Some(export) = &view.transfer_export {
        rows.push((
            format!(
                "Export inputs from {}:{} on {} (Enter opens F8)",
                short_id(&export.txid),
                export.vout,
                short_id(export.system_id.as_ref())
            ),
            None,
        ));
        rows.push(("  Candidates from the export input range; selected source must decode as a reserve transfer".into(), None));
        for output in &view.transfer_inputs {
            rows.push((
                format!("  Transfer :{}  {}", output.vout, short_id(&output.txid)),
                Some(Link::Trail(output.clone())),
            ));
        }
        if view.transfer_request.is_none() && view.transfer_inputs.is_empty() {
            rows.push(("  No transfer inputs available".into(), None));
        }
        if view.transfer_truncated {
            rows.push((
                format!(
                    "  First {} inputs shown; open the export for the rest",
                    currency_activity::MAX_INPUTS
                ),
                None,
            ));
        }
    }
    if !view.retained_activity.is_empty() {
        rows.push((
            "Retained activity windows (source/block-qualified observations)".into(),
            None,
        ));
        for (index, stored) in view.retained_activity.iter().enumerate() {
            rows.push((
                format!(
                    "  {}..{} via {} ({})",
                    height(stored.page.range.first),
                    height(stored.page.range.last),
                    stored.page.source,
                    stored.membership
                ),
                Some(Link::ActivityWindow(index)),
            ));
        }
    }
    let mut heights = BTreeSet::new();
    for snapshot in &view.snapshots {
        if snapshot.kind == "historical"
            && let Some(height) = snapshot.reported_height
        {
            heights.insert(height);
        }
    }
    if !heights.is_empty() {
        rows.push(("Retained historical heights".into(), None));
        for h in heights.iter().rev().take(20) {
            rows.push((
                format!("  height {}", height(*h)),
                Some(Link::Historical(*h)),
            ));
        }
    }
    // Keep spacing in the same row model used for rendering and navigation.
    let first_heading = rows.iter().position(|(label, _)| section_heading(label));
    rows.into_iter()
        .enumerate()
        .flat_map(|(index, row)| {
            let spacer = (Some(index) != first_heading && section_heading(&row.0))
                .then(|| (String::new(), None));
            spacer.into_iter().chain(std::iter::once(row))
        })
        .collect()
}

pub(super) fn show_cached_height(view: &mut View, height: u64) -> bool {
    let Some(snapshot) = view
        .snapshots
        .iter()
        .find(|snapshot| snapshot.kind == "historical" && snapshot.reported_height == Some(height))
        .cloned()
    else {
        return false;
    };
    view.historical = Some(snapshot);
    view.history_request = None;
    view.history_note = None;
    view.error = None;
    view.loading = false;
    clear_activity(view);
    rebuild_document(view);
    focus_section(view, "Historical state");
    true
}

pub(super) fn focus_section(view: &mut View, prefix: &str) {
    if let Some(index) = rows(view)
        .iter()
        .position(|(label, _)| label.starts_with(prefix))
    {
        view.selected = index;
        view.scroll = index.saturating_sub(2).min(u16::MAX as usize) as u16;
    }
}

/// Pad by terminal cell width, including wide/emoji currency names.
fn table_rows(
    rows: &mut Vec<(String, Option<Link>)>,
    headers: &[&'static str],
    numeric: &[bool],
    entries: Vec<(Vec<String>, Option<Link>)>,
    mut copy_fields: Option<&mut CopyFields>,
) {
    if entries.is_empty() {
        return;
    }
    let widths: Vec<usize> = headers
        .iter()
        .enumerate()
        .map(|(column, header)| {
            entries
                .iter()
                .map(|(cells, _)| Line::from(cells[column].as_str()).width())
                .chain(std::iter::once(Line::from(*header).width()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let format = |cells: &[String]| {
        let mut row = String::new();
        let mut fields = Vec::new();
        for (column, cell) in cells.iter().enumerate() {
            row.push_str("  ");
            let padding =
                " ".repeat(widths[column].saturating_sub(Line::from(cell.as_str()).width()));
            if numeric[column] {
                row.push_str(&padding);
            }
            let start = row.len();
            row.push_str(cell.trim_end());
            fields.push((headers[column], start..row.len()));
            if !numeric[column] {
                row.push_str(&padding);
            }
        }
        row.truncate(row.trim_end().len());
        (row, fields)
    };
    rows.push((
        format(&headers.iter().map(|s| (*s).to_string()).collect::<Vec<_>>()).0,
        None,
    ));
    rows.push((
        format!(
            "  {}",
            widths
                .iter()
                .map(|width| "─".repeat(*width))
                .collect::<Vec<_>>()
                .join("  ")
        ),
        None,
    ));
    for (cells, link) in entries {
        let (row, fields) = format(&cells);
        if let Some(copy_fields) = copy_fields.as_deref_mut() {
            copy_fields.insert(row.clone(), fields);
        }
        rows.push((row, link));
    }
}

fn state_rows(
    rows: &mut Vec<(String, Option<Link>)>,
    snapshot: &CurrencySnapshot,
    height: &impl Fn(u64) -> String,
    store: Option<&ObjectStore>,
    mut copy_fields: Option<&mut CopyFields>,
) {
    rows.push((
        format!("  {}", snapshot_label(snapshot, height)),
        snapshot
            .observed
            .as_ref()
            .map(|block| Link::Block(block.hash.clone())),
    ));
    let state = &snapshot.state;
    rows.push((
        format!(
            "Launch status: {}",
            crate::currency_status::classify(state).label()
        ),
        None,
    ));
    let mut summary = String::new();
    let mut fields = Vec::new();
    for (label, key) in [
        ("supply", "supply"),
        ("emitted", "emitted"),
        ("primary fees", "primarycurrencyfees"),
        ("conversion fees", "primarycurrencyconversionfees"),
    ] {
        summary.push_str(&format!("  {label} "));
        let start = summary.len();
        summary.push_str(&field(state, key));
        fields.push((label, start..summary.len()));
    }
    if let Some(copy_fields) = copy_fields.as_deref_mut() {
        copy_fields.insert(summary.clone(), fields);
    }
    rows.push((summary, None));
    if let Some(reserves) = state["reservecurrencies"].as_array() {
        let name = |id: &str| {
            source_result(snapshot)["currencynames"][id]
                .as_str()
                .or_else(|| store.and_then(|store| store.currency_name(id)))
                .map(str::to_string)
                .unwrap_or_else(|| short_id(id))
        };
        let main = reserves
            .iter()
            .map(|reserve| {
                let id = reserve["currencyid"].as_str().unwrap_or("?");
                (
                    vec![
                        name(id),
                        field(reserve, "reserves"),
                        field(reserve, "weight"),
                        field(reserve, "priceinreserve"),
                    ],
                    (id != "?").then(|| Link::Currency(id.into())),
                )
            })
            .collect();
        table_rows(
            rows,
            &["Currency", "Reserves", "Weight", "Price in reserve"],
            &[false, true, true, true],
            main,
            copy_fields.as_deref_mut(),
        );
        let details = reserves
            .iter()
            .map(|reserve| {
                let id = reserve["currencyid"].as_str().unwrap_or("?");
                let details = &state["currencies"][id];
                (
                    vec![
                        name(id),
                        field(details, "lastconversionprice"),
                        field(details, "viaconversionprice"),
                        field(details, "reservein"),
                        field(details, "reserveout"),
                        field(details, "fees"),
                        field(details, "conversionfees"),
                    ],
                    None,
                )
            })
            .collect();
        table_rows(
            rows,
            &[
                "Currency",
                "Last conversion",
                "Via conversion",
                "Reserve in",
                "Reserve out",
                "Fees",
                "Conversion fees",
            ],
            &[false, true, true, true, true, true, true],
            details,
            copy_fields,
        );
    }
}

pub(super) fn render(
    frame: &mut Frame<'_>,
    theme: &Theme,
    view: &View,
    store: &ObjectStore,
    raw_identities: bool,
    area: Rect,
) {
    let mut copy_fields = CopyFields::new();
    let capture = theme.copy_values.borrow().is_recording();
    let rows = rows_display(
        view,
        Some(theme),
        Some(store),
        raw_identities,
        capture.then_some(&mut copy_fields),
    );
    let visible = usize::from(area.height.max(1));
    let scroll = usize::from(view.scroll)
        .min(view.selected)
        .max(view.selected.saturating_add(1).saturating_sub(visible))
        .min(rows.len().saturating_sub(visible));
    let body = scroll_view::content_area(area, rows.len() > visible);
    let gutter = if body.width > 2 { 2 } else { 0 };
    let content = Rect::new(body.x + gutter, body.y, body.width - gutter, body.height);
    let lines = rows
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible)
        .map(|(index, (label, link))| {
            let selected = index == view.selected;
            if label.starts_with("Launch status") {
                let status = if label
                    .ends_with(crate::currency_status::LaunchStatus::Failed.label())
                {
                    crate::currency_status::LaunchStatus::Failed
                } else if label.ends_with(crate::currency_status::LaunchStatus::Prelaunch.label()) {
                    crate::currency_status::LaunchStatus::Prelaunch
                } else if label.ends_with(crate::currency_status::LaunchStatus::Launched.label()) {
                    crate::currency_status::LaunchStatus::Launched
                } else {
                    crate::currency_status::LaunchStatus::Unknown
                };
                return Line::from(vec![
                    Span::styled(
                        format!("{}: ", label.split_once(": ").unwrap().0),
                        theme.muted(),
                    ),
                    launch_badge(theme, status),
                ]);
            }
            if label.starts_with("Configuration: ") {
                return tag_line(
                    theme,
                    &label
                        .trim_start_matches("Configuration: ")
                        .split(" | ")
                        .map(str::to_string)
                        .collect::<Vec<_>>(),
                );
            }
            if section_heading(label) {
                return Line::styled(format!(" {} ", label), theme.focused().bg(theme.panel));
            }
            if index == 0 {
                return Line::from(vec![
                    Span::styled("Currency ", theme.focused()),
                    theme.value(
                        "currency name",
                        view.name.as_deref().unwrap_or(&view.query),
                        theme.focused(),
                    ),
                ]);
            }
            let style = if selected {
                theme.focused()
            } else if link.is_some() || index == 1 {
                theme.link()
            } else {
                theme.base()
            };
            if label.starts_with("  ")
                && let Some((left, right)) = label.split_once(" | ")
                && let (Some((left_label, left_value)), Some((right_label, right_value))) =
                    (left.split_once(": "), right.split_once(": "))
            {
                return identity_property_pair(
                    theme,
                    left_label.trim(),
                    left_value.into(),
                    right_label,
                    right_value.into(),
                    content.width,
                );
            }
            if let Some(fields) = copy_fields.get(label) {
                let mut spans = Vec::new();
                let mut offset = 0;
                for (key, range) in fields {
                    if let Some(value) = label.get(range.clone()) {
                        spans.push(Span::styled(label[offset..range.start].to_string(), style));
                        spans.push(theme.value(key, value, style));
                        offset = range.end;
                    }
                }
                spans.push(Span::styled(label[offset..].to_string(), style));
                return Line::from(spans);
            }
            if let Some((key, value)) = label.split_once(": ") {
                Line::from(vec![
                    Span::styled(
                        format!("{key}: "),
                        if link.is_some() { style } else { theme.muted() },
                    ),
                    theme.value(key.trim(), value, style),
                ])
            } else if index == 1 {
                Line::from(theme.value("currency i-address", label.clone(), style))
            } else if let Some(link) = link {
                let value = match link {
                    Link::Block(hash) | Link::Transaction(hash) | Link::Currency(hash) => {
                        hash.as_str()
                    }
                    _ => label.as_str(),
                };
                Line::from(theme.value_as("currency record", label.clone(), value, style))
            } else {
                Line::styled(label.clone(), style)
            }
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).style(theme.base()), content);
    if gutter != 0
        && let Some(offset) = view.selected.checked_sub(scroll)
        && offset < usize::from(area.height)
        && rows
            .get(view.selected)
            .is_some_and(|(label, _)| !label.is_empty())
    {
        render_selection_gutter(
            frame,
            theme,
            Rect::new(area.x, area.y + offset as u16, 1, 1),
        );
    }
    scroll_view::render(frame, theme, area, rows.len(), scroll, visible);
}

pub(super) fn select(view: &mut View, delta: isize, height: u16) {
    let rows = rows(view);
    let last = rows.len().saturating_sub(1);
    view.selected = view.selected.saturating_add_signed(delta).min(last);
    if rows[view.selected].0.is_empty() && delta != 0 {
        view.selected = view
            .selected
            .saturating_add_signed(delta.signum())
            .min(last);
    }
    let visible = usize::from(height.max(1));
    if view.selected < usize::from(view.scroll) {
        view.scroll = view.selected.min(u16::MAX as usize) as u16;
    }
    if view.selected >= usize::from(view.scroll) + visible {
        view.scroll = view
            .selected
            .saturating_sub(visible - 1)
            .min(u16::MAX as usize) as u16;
    }
}

pub(super) fn selected_link(view: &View) -> Option<Link> {
    rows(view).get(view.selected)?.1.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_tags_decode_all_protocol_bits_and_keep_unknown_options_visible() {
        assert_eq!(option_tags(&json!({"options":0})), vec!["No option flags"]);
        assert_eq!(
            option_tags(&json!({"options":33})),
            vec!["Fractional", "Token"]
        );
        for (bit, name) in OPTION_TAGS {
            assert_eq!(option_tags(&json!({"options":bit})), vec![*name]);
        }
        assert_eq!(
            option_tags(&json!({"options":8192})),
            vec!["Unknown options 0x2000"]
        );
        assert_eq!(option_tags(&json!({})), vec!["Options unavailable"]);
    }

    #[test]
    fn currency_header_and_sections_have_distinct_styles() {
        let Screen::Currency(mut view) =
            screen("VRSC".into(), "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV".into())
        else {
            unreachable!()
        };
        view.definition = Some(
            json!({"currencyid":"i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV", "name":"VRSC", "options":256}),
        );
        view.loading = false;
        let store = ObjectStore::default();
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| render(frame, &theme, &view, &store, false, frame.area()))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let line = |y| {
            (0..100)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        };
        let first = (0..30)
            .find(|y| line(*y).contains("Notarizations (current)"))
            .unwrap();
        assert!(!line(first - 1).trim().is_empty());
        let section = (0..30).find(|y| line(*y).contains("Definition")).unwrap();
        assert_eq!(buffer[(3, section)].bg, theme.panel);
        assert!(line(section - 1).trim().is_empty());
        let tags = (0..30).find(|y| line(*y).contains("PBaaS chain")).unwrap();
        assert_eq!(Some(buffer[(3, tags)].bg), theme.status_badge().bg);
        assert_eq!(buffer[(0, view.selected as u16)].symbol(), ">");
        assert_eq!(buffer[(0, view.selected as u16)].bg, theme.accent);
        assert!(line(0).contains("Currency VRSC"));
    }

    #[test]
    fn definition_basket_is_fractional_only_and_section_spacers_are_skipped() {
        let Screen::Currency(mut view) = screen("vETH".into(), "VRSC".into()) else {
            unreachable!()
        };
        let reserve = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        view.definition = Some(json!({"options":0x80, "currencies":[reserve], "weights":[1]}));
        assert!(
            !rows(&view)
                .iter()
                .any(|(label, _)| label.starts_with("Basket currencies"))
        );

        view.definition.as_mut().unwrap()["options"] = json!(OPTION_FRACTIONAL | 0x80);
        let rows = rows(&view);
        let basket = rows
            .iter()
            .position(|(label, _)| label.starts_with("Basket currencies"))
            .unwrap();
        assert!(rows[basket - 1].0.is_empty());
        assert!(matches!(&rows[basket + 3].1, Some(Link::Currency(id)) if id == reserve));
        view.selected = basket - 2;
        select(&mut view, 1, 30);
        assert_eq!(view.selected, basket);
        select(&mut view, -1, 30);
        assert_eq!(view.selected, basket - 2);
    }

    #[test]
    fn reserve_table_aligns_numeric_columns_by_terminal_cells() {
        let mut rows = Vec::new();
        table_rows(
            &mut rows,
            &["Currency", "Reserves", "Weight"],
            &[false, true, true],
            vec![
                (
                    vec!["VRSC".into(), "12345.67".into(), "0.5".into()],
                    Some(Link::Currency("i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV".into())),
                ),
                (vec!["🌱".into(), "1.2".into(), "0.125".into()], None),
            ],
            None,
        );
        let end = |row: &str, value: &str| {
            let offset = row.find(value).unwrap() + value.len();
            Line::from(&row[..offset]).width()
        };
        assert_eq!(end(&rows[0].0, "Reserves"), end(&rows[2].0, "12345.67"));
        assert_eq!(end(&rows[2].0, "12345.67"), end(&rows[3].0, "1.2"));
        assert_eq!(end(&rows[0].0, "Weight"), end(&rows[2].0, "0.5"));
        assert_eq!(end(&rows[2].0, "0.5"), end(&rows[3].0, "0.125"));
        assert!(
            matches!(&rows[2].1, Some(Link::Currency(id)) if id == "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV")
        );
    }

    #[test]
    fn reserve_state_rows_resolve_response_names_and_cached_definitions() {
        let id = "i6j1rzjgrDhSmUYiTtp21J8Msiudv5hgt9";
        let Screen::Currency(mut view) = screen("iPool".into(), "iSystem".into()) else {
            unreachable!()
        };
        view.loading = false;
        view.definition = Some(json!({"currencyid":"iPool"}));
        view.snapshots.push(CurrencySnapshot {
            source: "local".into(),
            kind: "bestcurrencystate".into(),
            reported_height: Some(1),
            observed: None,
            current: false,
            state: json!({"reservecurrencies":[{"currencyid":id,"reserves":1}]}),
            response: json!({"result":{"currencynames":{(id):"Bridge.vDEX"}}}),
        });
        assert!(
            rows(&view)
                .iter()
                .any(|(label, _)| label.contains("Bridge.vDEX") && label.contains("1"))
        );
        view.snapshots[0].response = Value::Null;
        let mut store = ObjectStore::default();
        store.insert_currency_definition(CurrencyDefinitionRecord {
            currency_id: id.into(),
            name: "Bridge.vDEX".into(),
            definition_json: json!({"currencyid":id}),
        });
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(160, 25)).unwrap();
        terminal
            .draw(|frame| render(frame, &theme, &view, &store, false, frame.area()))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            text.contains("Bridge.vDEX") && text.contains("Price in reserve"),
            "{text}"
        );
        assert!(
            rows(&view)
                .iter()
                .any(|(_, link)| matches!(link,Some(Link::Currency(target)) if target==id))
        );
    }

    #[test]
    fn cached_historical_state_can_open_without_a_source_call() {
        let Screen::Currency(mut view) = screen("iPool".into(), "iVRSC".into()) else {
            unreachable!()
        };
        view.definition = Some(json!({"currencyid":"iPool"}));
        view.snapshots.push(CurrencySnapshot {
            source: "local".into(),
            kind: "historical".into(),
            reported_height: Some(3_962_166),
            observed: Some(crate::cache::BlockContext {
                height: 3_962_166,
                hash: "old-block".into(),
            }),
            current: false,
            state: json!({"currencyid":"iPool","supply":16542}),
            response: Value::Null,
        });
        view.history_request = Some(42);
        view.history_note = Some("obsolete time lookup".into());
        view.error = Some("obsolete error".into());
        assert!(show_cached_height(&mut view, 3_962_166));
        assert!(
            view.history_request.is_none() && view.history_note.is_none() && view.error.is_none()
        );
        assert!(!view.loading);
        assert_eq!(view.document["historical"]["state"]["supply"], 16542);
        assert!(
            rows(&view)
                .iter()
                .any(|(label, _)| label.contains("current membership unverified"))
        );
        assert!(!show_cached_height(&mut view, 3_962_167));
    }

    #[test]
    fn definition_links_and_names_come_from_rpc_result_envelope() {
        let Screen::Currency(mut view) = screen("iPool".into(), "iVRSC".into()) else {
            unreachable!()
        };
        view.definition = Some(json!({
            "currencyid":"iPool","options":OPTION_FRACTIONAL,"currencies":["iReserve"],"weights":[0.25]
        }));
        view.snapshots.push(CurrencySnapshot {
            source: "local".into(),
            kind: "bestcurrencystate".into(),
            reported_height: Some(10),
            observed: None,
            current: false,
            state: json!({"currencyid":"iPool"}),
            response: json!({"result":{"definitiontxid":"definition-tx",
                "currencynames":{"iReserve":"Reserve"}}}),
        });
        let rows = rows(&view);
        assert!(
            rows.iter()
                .any(|(label, link)| label.contains("definition-tx")
                    && matches!(link, Some(Link::Transaction(txid)) if txid == "definition-tx"))
        );
        assert!(
            rows.iter()
                .any(|(label, _)| label.contains("Reserve") && label.contains("0.25"))
        );
    }
}
