//! Source-qualified snapshots of the daemon's notarization graph.
use super::*;
use crate::world::{OutpointRef, SystemId};
use std::sync::Mutex;

const HEADER_HEIGHT: u16 = 4;
const EVIDENCE_HEIGHT: u16 = 6;
const ROW_HEIGHT: u16 = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Perspective {
    observer: String,
    subject: String,
}

#[derive(Debug, Clone)]
struct Root {
    system: String,
    height: Option<u64>,
    hash: Option<String>,
}

#[derive(Debug, Clone)]
struct Row {
    index: usize,
    txid: Option<String>,
    vout: Option<u32>,
    block: Option<String>,
    roots: Vec<Root>,
    foreign_previous: Option<OutpointRef>,
    synthetic: bool,
}

impl Row {
    fn outpoint(&self, system: &str) -> Option<OutpointRef> {
        Some(OutpointRef {
            system_id: SystemId::try_from(system.to_string()).ok()?,
            txid: self.txid.clone()?,
            vout: self.vout?,
        })
    }
}

#[derive(Debug)]
struct Graph {
    rows: Vec<Row>,
    confirmed: Option<usize>,
}

impl Graph {
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

fn hash(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| {
            s.len() == 64
                && s.bytes().all(|c| c.is_ascii_hexdigit())
                && s.bytes().any(|b| b != b'0')
        })
        .map(str::to_ascii_lowercase)
}

fn decode(value: &Value, observer: &str) -> Result<Graph> {
    let entries = value["notarizations"]
        .as_array()
        .context("Missing notarization array")?;
    let confirmed = match value["lastconfirmed"].as_i64() {
        Some(-1) => None,
        None if entries.is_empty() && value["lastconfirmed"].is_null() => None,
        Some(index) if index >= 0 && (index as usize) < entries.len() => Some(index as usize),
        _ => anyhow::bail!("Invalid last-confirmed notarization index"),
    };
    let rows = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let notarization = &entry["notarization"];
            let txid = hash(&entry["txid"]);
            let block = hash(&entry["blockhash"]);
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
                        hash: hash(&root["blockhash"]),
                    })
                })
                .collect();
            // In mirror form the previous outpoint still belongs to the earned
            // notarization's system. See the transaction relationship decoder.
            let foreign_previous = (|| {
                let system = notarization["currencyid"].as_str()?;
                if notarization["ismirror"].as_bool() != Some(true) || system == observer {
                    return None;
                }
                Some(OutpointRef {
                    system_id: SystemId::try_from(system.to_owned()).ok()?,
                    txid: hash(&notarization["prevnotarizationtxid"])?,
                    vout: notarization["prevnotarizationout"]
                        .as_u64()?
                        .try_into()
                        .ok()?,
                })
            })();
            Row {
                index,
                txid,
                vout: entry["vout"].as_u64().and_then(|v| v.try_into().ok()),
                block,
                roots,
                foreign_previous,
                synthetic,
            }
        })
        .collect();
    Ok(Graph { rows, confirmed })
}

#[derive(Debug, Clone)]
struct Stamp {
    control: Arc<crate::sources::Control>,
    generation: u64,
    branch: u64,
}

impl Stamp {
    fn new(control: &Arc<crate::sources::Control>) -> Self {
        Self {
            control: control.clone(),
            generation: control.generation(),
            branch: control.branch_epoch.load(Ordering::Acquire),
        }
    }
    fn current(&self) -> bool {
        self.control.generation() == self.generation
            && self.control.branch_epoch.load(Ordering::Acquire) == self.branch
    }
}

#[derive(Debug, Clone)]
struct Header {
    height: u64,
    time: Option<u64>,
    confirmations: Option<i64>,
}

#[derive(Debug)]
enum Check<T> {
    Found { source: String, value: T },
    NotFound { source: String, tip: Option<u64> },
    Unavailable,
}

#[derive(Debug, Default)]
enum Load {
    #[default]
    Idle,
    Loading,
    NoData,
    Unavailable,
    Ready {
        graph: Arc<Graph>,
        source: String,
    },
}

#[derive(Debug, Default)]
struct Snapshot {
    generation: u64,
    stamps: Vec<Stamp>,
    load: Load,
    headers: HashMap<(String, String), Check<Header>>,
    transactions: HashMap<OutpointRef, Check<()>>,
    busy: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct Feed(Arc<Mutex<Snapshot>>);

impl Feed {
    pub(super) fn refresh(&self) {
        let mut state = self.0.lock().unwrap();
        *state = Snapshot {
            generation: state.generation.wrapping_add(1),
            ..Default::default()
        };
    }

    fn update(
        &self,
        generation: u64,
        tx: &UnboundedSender<AppEvent>,
        update: impl FnOnce(&mut Snapshot),
    ) -> bool {
        let mut state = self.0.lock().unwrap();
        if state.generation != generation || !state.stamps.iter().all(Stamp::current) {
            return false;
        }
        update(&mut state);
        let _ = tx.send(AppEvent::NotarizationChanged);
        true
    }
}

#[derive(Debug, Clone)]
pub(super) struct View {
    perspectives: Vec<(Perspective, Feed)>,
    names: HashMap<String, String>,
    active: usize,
    selected: Vec<usize>,
    selected_outpoint: Vec<Option<OutpointRef>>,
    root: usize,
}

impl View {
    pub(super) fn bookmark_target(&self) -> Result<(SystemId, String)> {
        let (perspective, _) = &self.perspectives[self.active];
        Ok((
            SystemId::try_from(perspective.observer.clone())?,
            perspective.subject.clone(),
        ))
    }
}

fn parent_id(testnet: bool) -> Option<String> {
    crate::world::system_id("VRSC", crate::world::Network::from_testnet(testnet))
        .ok()
        .map(String::from)
}

pub(super) fn browser(
    view: &currency_ui::View,
    config: &Config,
    configs: &[Config],
) -> Option<Screen> {
    let subject = view.id.clone()?;
    let observer = view.system.clone();
    let mut perspectives = vec![(
        Perspective {
            observer: observer.clone(),
            subject: subject.clone(),
        },
        view.notarizations.clone(),
    )];
    let parent = parent_id(config.chain.testnet)?;
    let definition_is_child = view.definition.as_ref().is_some_and(|def| {
        def["options"]
            .as_u64()
            .is_some_and(|flags| flags & 0x100 != 0)
            && def["systemid"].as_str() == Some(subject.as_str())
    });
    if (observer == parent && subject != parent && definition_is_child)
        || (subject == parent && observer != parent)
    {
        perspectives.push((
            Perspective {
                observer: subject.clone(),
                subject: observer.clone(),
            },
            Feed::default(),
        ));
    }
    let mut names: HashMap<_, _> = configs
        .iter()
        .filter(|c| c.chain.testnet == config.chain.testnet)
        .map(|c| (c.chain_cache_key(), c.chain.name.clone()))
        .collect();
    names
        .entry(subject)
        .or_insert_with(|| view.name.clone().unwrap_or_else(|| view.query.clone()));
    names.insert(
        parent,
        if config.chain.testnet {
            "VRSCTEST"
        } else {
            "VRSC"
        }
        .into(),
    );
    let count = perspectives.len();
    Some(Screen::Notarizations(Box::new(View {
        perspectives,
        names,
        active: 0,
        selected: vec![0; count],
        selected_outpoint: vec![None; count],
        root: 0,
    })))
}

fn connection(configs: &[Config], network: bool, system: &str) -> Option<Config> {
    configs
        .iter()
        .find(|config| config.chain.testnet == network && config.chain_cache_key() == system)
        .map(Config::noninteractive)
}

fn capacity(height: u16) -> usize {
    usize::from(list_height(height) / ROW_HEIGHT).max(1)
}

fn list_height(height: u16) -> u16 {
    let body = height.saturating_sub(HEADER_HEIGHT);
    body - body.saturating_sub(ROW_HEIGHT).min(EVIDENCE_HEIGHT)
}

fn first_visible(count: usize, selected: usize, page: usize) -> usize {
    selected
        .saturating_sub(page / 2)
        .min(count.saturating_sub(page))
}

enum Lookup {
    Inclusion(String),
    Root(String, String),
    Transaction(OutpointRef),
}

pub(super) fn drive(
    state: &mut AppState,
    config: &Config,
    configs: &[Config],
    tx: &UnboundedSender<AppEvent>,
    height: u16,
) -> bool {
    let (perspective, feed, selection) = match &mut state.screen {
        Screen::Currency(view) if view.mode == ViewMode::Overview => {
            let Some(subject) = view.id.clone() else {
                return false;
            };
            (
                Perspective {
                    observer: view.system.clone(),
                    subject,
                },
                view.notarizations.clone(),
                None,
            )
        }
        Screen::Notarizations(view) => {
            let (p, feed) = &view.perspectives[view.active];
            let snapshot = feed.0.lock().unwrap();
            if let Load::Ready { graph, .. } = &snapshot.load {
                let selected = &mut view.selected[view.active];
                if let Some(outpoint) = &view.selected_outpoint[view.active]
                    && let Some(position) = graph
                        .rows
                        .iter()
                        .position(|row| row.outpoint(&p.observer).as_ref() == Some(outpoint))
                {
                    *selected = position;
                }
                *selected = (*selected).min(graph.rows.len().saturating_sub(1));
            }
            (p.clone(), feed.clone(), Some(view.selected[view.active]))
        }
        _ => return false,
    };
    let network = config.chain.testnet;
    let mut snapshot = feed.0.lock().unwrap();
    if snapshot.stamps.iter().any(|stamp| !stamp.current()) {
        *snapshot = Snapshot {
            generation: snapshot.generation.wrapping_add(1),
            ..Default::default()
        };
    }
    let generation = snapshot.generation;
    if matches!(snapshot.load, Load::Idle) {
        let Some(source) = connection(configs, network, &perspective.observer) else {
            snapshot.load = Load::Unavailable;
            return true;
        };
        snapshot.stamps = configs
            .iter()
            .filter(|c| c.chain.testnet == network)
            .flat_map(|c| {
                std::iter::once(&c.daemon.control)
                    .chain(c.daemon.api_pool.endpoints.iter().map(|e| &e.control))
            })
            .map(Stamp::new)
            .collect();
        snapshot.load = Load::Loading;
        drop(snapshot);
        let tx = tx.clone();
        tokio::spawn(async move {
            let load = match aggregate(&source, &perspective).await {
                Ok(Some((graph, source))) => Load::Ready {
                    graph: Arc::new(graph),
                    source,
                },
                Ok(None) => Load::NoData,
                Err(_) => Load::Unavailable,
            };
            feed.update(generation, &tx, |s| s.load = load);
        });
        return true;
    }
    if snapshot.busy {
        return false;
    }
    let Load::Ready { graph, source } = &snapshot.load else {
        return false;
    };
    let source = source.clone();
    let mut lookups = Vec::new();
    if let Some(block) = graph
        .confirmed
        .and_then(|i| graph.rows.get(i))
        .and_then(|row| row.block.as_ref())
        && !snapshot
            .headers
            .contains_key(&(perspective.observer.clone(), block.clone()))
    {
        lookups.push(Lookup::Inclusion(block.clone()));
    } else if let Some(selection) = selection {
        let page = capacity(height);
        let start = first_visible(graph.rows.len(), selection, page);
        let mut blocks = HashSet::new();
        let mut transactions = HashSet::new();
        for row in graph.rows.iter().skip(start).take(page) {
            if let Some(block) = &row.block {
                let key = (perspective.observer.clone(), block.clone());
                if !snapshot.headers.contains_key(&key) && blocks.insert(key) {
                    lookups.push(Lookup::Inclusion(block.clone()));
                }
            }
            for root in row
                .roots
                .iter()
                .filter(|r| r.system != perspective.observer)
            {
                if let Some(block) = &root.hash {
                    let key = (root.system.clone(), block.clone());
                    if !snapshot.headers.contains_key(&key) && blocks.insert(key) {
                        lookups.push(Lookup::Root(root.system.clone(), block.clone()));
                    }
                }
            }
            if let Some(target) = &row.foreign_previous
                && !snapshot.transactions.contains_key(target)
                && transactions.insert(target.clone())
            {
                lookups.push(Lookup::Transaction(target.clone()));
            }
        }
    }
    if lookups.is_empty() {
        return false;
    }
    // Keep navigation responsive and avoid queueing every header in a large view.
    lookups.truncate(8);
    snapshot.busy = true;
    drop(snapshot);
    let configs = configs.to_vec();
    let tx = tx.clone();
    tokio::spawn(async move {
        let mut peers = HashMap::new();
        for lookup in lookups {
            match lookup {
                Lookup::Inclusion(block) => {
                    let check = inclusion(
                        connection(&configs, network, &perspective.observer),
                        &source,
                        &block,
                    )
                    .await;
                    if !feed.update(generation, &tx, |s| {
                        s.headers
                            .insert((perspective.observer.clone(), block), check);
                    }) {
                        return;
                    }
                }
                Lookup::Root(system, block) => {
                    let peer = match peers.entry(system.clone()) {
                        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(peer_source(connection(&configs, network, &system)).await)
                        }
                    };
                    let check = match peer {
                        Some(peer) => foreign_root(peer, &block).await,
                        None => Check::Unavailable,
                    };
                    if !feed.update(generation, &tx, |s| {
                        s.headers.insert((system, block), check);
                    }) {
                        return;
                    }
                }
                Lookup::Transaction(target) => {
                    let check = foreign_transaction(
                        connection(&configs, network, target.system_id.as_ref()),
                        &target,
                    )
                    .await;
                    if !feed.update(generation, &tx, |s| {
                        s.transactions.insert(target, check);
                    }) {
                        return;
                    }
                }
            }
        }
        feed.update(generation, &tx, |s| s.busy = false);
    });
    false
}

async fn aggregate(config: &Config, perspective: &Perspective) -> Result<Option<(Graph, String)>> {
    let client = RpcClient::for_config(config)?;
    let response = client
        .call_nullable_with_raw::<Value>(
            "getnotarizationdata",
            &[json!(perspective.subject), json!(false), json!(false)],
        )
        .await?;
    response
        .result
        .map(|value| Ok((decode(&value, &perspective.observer)?, response.source_id)))
        .transpose()
}

fn header(value: &Value, hash: &str) -> Option<Header> {
    if value["hash"].as_str()? != hash {
        return None;
    }
    Some(Header {
        height: value["height"].as_u64()?,
        time: value["time"].as_u64(),
        confirmations: value["confirmations"].as_i64(),
    })
}

async fn inclusion(config: Option<Config>, source: &str, block: &str) -> Check<Header> {
    let Ok(Some(value)) = async {
        let Some(config) = config else {
            return Ok::<_, anyhow::Error>(None);
        };
        let cache = Cache::open(&config.cache_path)?;
        for (method, params) in [
            ("getblockheader", json!([block, true])),
            ("getblock", json!([block, 1])),
        ] {
            if let Some(raw) = cache
                .raw_rpc_response_async(
                    source,
                    &config.chain_cache_key(),
                    method,
                    &params.to_string(),
                )
                .await?
                && let Ok(value) = rpc_result_value(&raw)
                && let Some(value) = header(&value, block).filter(|h| h.time.is_some())
            {
                return Ok(Some(value));
            }
        }
        let client = RpcClient::for_config(&config)?;
        for (method, verbosity) in [("getblockheader", json!(true)), ("getblock", json!(1))] {
            if let Ok(value) = client
                .call_on(source, method, &[json!(block), verbosity])
                .await
                && let Some(value) = header(&value, block).filter(|h| h.time.is_some())
            {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }
    .await
    else {
        return Check::Unavailable;
    };
    Check::Found {
        source: source.into(),
        value,
    }
}

struct PeerSource {
    client: RpcClient,
    source: String,
    tip: Option<u64>,
}

async fn peer_source(config: Option<Config>) -> Option<PeerSource> {
    let client = RpcClient::for_config(&config?).ok()?;
    // Qualify not-found responses as well as successful lookups to a specific
    // endpoint. A fallback response would describe a different chain view.
    let tip = client.get_blockchain_info().await.ok()?;
    Some(PeerSource {
        client,
        source: tip.source_id,
        tip: tip.result["blocks"].as_u64(),
    })
}

async fn foreign_root(peer: &PeerSource, block: &str) -> Check<Header> {
    let source = peer.source.clone();
    match peer
        .client
        .call_on(&source, "getblockheader", &[json!(block), json!(true)])
        .await
    {
        Ok(value) => {
            header(&value, block).map_or(Check::Unavailable, |value| Check::Found { source, value })
        }
        Err(error)
            if error
                .downcast_ref::<crate::rpc::ApplicationError>()
                .is_some_and(|e| e.is_block_not_found()) =>
        {
            Check::NotFound {
                source,
                tip: peer.tip,
            }
        }
        Err(_) => Check::Unavailable,
    }
}

async fn foreign_transaction(config: Option<Config>, target: &OutpointRef) -> Check<()> {
    let Some(config) = config else {
        return Check::Unavailable;
    };
    let Ok(client) = RpcClient::for_config(&config) else {
        return Check::Unavailable;
    };
    match client.get_raw_transaction(&target.txid).await {
        Ok(result)
            if result.result["txid"].as_str() == Some(&target.txid)
                && result.result["vout"].as_array().is_some_and(|outs| {
                    outs.iter()
                        .any(|out| out["n"].as_u64() == Some(u64::from(target.vout)))
                }) =>
        {
            Check::Found {
                source: result.source_id,
                value: (),
            }
        }
        _ => Check::Unavailable,
    }
}

pub(super) fn summary(
    feed: &Feed,
    observer: &str,
    theme: Option<&Theme>,
) -> Vec<(String, Option<currency_ui::Link>)> {
    use currency_ui::Link;
    let state = feed.0.lock().unwrap();
    let mut rows = vec![("Notarizations (current)".into(), None)];
    match &state.load {
        Load::Ready { graph, .. } => {
            if let Some(row) = graph.confirmed.and_then(|index| graph.rows.get(index)) {
                if row.synthetic {
                    rows.push(("  Last confirmed: synthetic proof root".into(), None));
                } else if let (Some(txid), Some(vout)) = (&row.txid, row.vout) {
                    rows.push((
                        format!("  Last confirmed: {txid}:{vout}"),
                        Some(Link::Transaction(txid.clone())),
                    ));
                    let included = row
                        .block
                        .as_ref()
                        .and_then(|block| state.headers.get(&(observer.to_owned(), block.clone())));
                    rows.push((
                        format!(
                            "  Included: {}",
                            inclusion_text(included, row.block.is_some(), theme)
                        ),
                        None,
                    ));
                } else {
                    rows.push(("  Last confirmed: transaction unavailable".into(), None));
                }
            } else {
                rows.push(("  Last confirmed: none returned".into(), None));
            }
            rows.push((
                format!("  Unconfirmed in current view: {}", graph.pending()),
                None,
            ));
        }
        load => {
            rows.push((format!("  {}", load_label(load)), None));
            rows.push(("  Unconfirmed in current view: unavailable".into(), None));
        }
    }
    rows.push((
        "  Browse notarizations [n]".into(),
        Some(Link::Notarizations),
    ));
    rows
}

fn load_label(load: &Load) -> &'static str {
    match load {
        Load::Idle | Load::Loading => "Loading notarization data…",
        Load::NoData => "No notarization data returned",
        Load::Unavailable => "Notarization data unavailable",
        Load::Ready { .. } => "No notarizations in current view",
    }
}

fn inclusion_text(check: Option<&Check<Header>>, has_hash: bool, theme: Option<&Theme>) -> String {
    match check {
        Some(Check::Found { value, .. }) => {
            let time = value.time.map_or_else(
                || "time unavailable".into(),
                |time| theme.map_or_else(|| time.to_string(), |theme| theme.format_time(time)),
            );
            format!("#{} at {time}", value.height)
        }
        None if has_hash => "awaiting block lookup".into(),
        _ => "unavailable".into(),
    }
}

fn root_status(check: Option<&Check<Header>>, root: &Root) -> String {
    match check {
        Some(Check::Found { value, .. }) => match (root.height, value.confirmations) {
            (Some(height), _) if height != value.height => {
                format!("Height mismatch (found #{})", value.height)
            }
            (_, Some(-1)) => "Known off-chain • possible fork".into(),
            (_, Some(n)) if n > 0 => "Active".into(),
            _ => "Present • membership unavailable".into(),
        },
        Some(Check::NotFound { tip: Some(tip), .. }) if root.height.is_some_and(|h| h > *tip) => {
            format!("Not found • source tip #{tip} is behind")
        }
        Some(Check::NotFound { .. }) => "Not found • possible fork".into(),
        Some(Check::Unavailable) => "Unavailable".into(),
        None if root.hash.is_some() => "Awaiting check".into(),
        None => "Unavailable".into(),
    }
}

fn source_label(source: &str) -> String {
    source.split_once(':').map_or_else(
        || source.to_string(),
        |(kind, id)| format!("{kind}:{}", short_id(id)),
    )
}

impl View {
    pub(super) fn has_perspectives(&self) -> bool {
        self.perspectives.len() > 1
    }

    fn name<'a>(&'a self, id: &'a str) -> &'a str {
        self.names.get(id).map_or(id, String::as_str)
    }
}

pub(super) enum Navigation {
    Transaction(TransactionContext),
    Block { system: String, hash: String },
}

#[derive(Debug, Clone)]
pub(super) struct TransactionContext {
    pub target: OutpointRef,
    observer: String,
    feed: Feed,
}

impl TransactionContext {
    pub(super) fn status_line(&self) -> String {
        let snapshot = self.feed.0.lock().unwrap();
        let status = if snapshot.stamps.iter().all(Stamp::current)
            && let Load::Ready { graph, .. } = &snapshot.load
        {
            if graph
                .confirmed
                .and_then(|index| graph.rows.get(index))
                .and_then(|row| row.outpoint(self.target.system_id.as_ref()))
                .as_ref()
                == Some(&self.target)
            {
                "confirmed"
            } else if graph.rows.iter().any(|row| {
                row.outpoint(self.target.system_id.as_ref()).as_ref() == Some(&self.target)
            }) {
                "unconfirmed"
            } else {
                "status unavailable"
            }
        } else {
            "status unavailable"
        };
        format!(
            "Notarization output {}: {status} in {} view",
            self.target.vout, self.observer
        )
    }
}

pub(super) fn key(view: &mut View, key: KeyCode, height: u16) -> (bool, Option<Navigation>) {
    let active = view.active;
    let (perspective, feed) = &view.perspectives[active];
    match key {
        KeyCode::Tab | KeyCode::BackTab => {
            view.active = (active + 1) % view.perspectives.len();
            view.root = 0;
        }
        KeyCode::Char('r') | KeyCode::F(5) => {
            if let Load::Ready { graph, .. } = &feed.0.lock().unwrap().load {
                view.selected_outpoint[active] = graph
                    .rows
                    .get(view.selected[active])
                    .and_then(|row| row.outpoint(&perspective.observer));
            }
            view.root = 0;
            feed.refresh();
        }
        KeyCode::Down
        | KeyCode::Up
        | KeyCode::Char('j' | 'k')
        | KeyCode::PageDown
        | KeyCode::PageUp
        | KeyCode::Home
        | KeyCode::End => {
            let snapshot = feed.0.lock().unwrap();
            if let Load::Ready { graph, .. } = &snapshot.load {
                let selected = &mut view.selected[active];
                let last = graph.rows.len().saturating_sub(1);
                *selected = match key {
                    KeyCode::Down | KeyCode::Char('j') => selected.saturating_add(1).min(last),
                    KeyCode::Up | KeyCode::Char('k') => selected.saturating_sub(1),
                    KeyCode::PageDown => selected.saturating_add(capacity(height)).min(last),
                    KeyCode::PageUp => selected.saturating_sub(capacity(height)),
                    KeyCode::End => last,
                    _ => 0,
                };
                view.selected_outpoint[active] = graph
                    .rows
                    .get(*selected)
                    .and_then(|row| row.outpoint(&perspective.observer));
                view.root = 0;
            }
        }
        KeyCode::Char('[' | ']' | 'b') | KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            let snapshot = feed.0.lock().unwrap();
            if let Load::Ready { graph, .. } = &snapshot.load
                && let Some(row) = graph.rows.get(view.selected[active])
            {
                let roots = row
                    .roots
                    .iter()
                    .filter(|root| root.system != perspective.observer);
                let count = roots.clone().count();
                match key {
                    KeyCode::Char(']') if count > 0 => view.root = (view.root + 1) % count,
                    KeyCode::Char('[') if count > 0 => view.root = (view.root + count - 1) % count,
                    KeyCode::Char('b') => {
                        if let Some(root) = roots.into_iter().nth(view.root)
                            && let Some(hash) = &root.hash
                            && let Some(Check::Found { value, .. }) =
                                snapshot.headers.get(&(root.system.clone(), hash.clone()))
                            && root.height == Some(value.height)
                        {
                            return (
                                true,
                                Some(Navigation::Block {
                                    system: root.system.clone(),
                                    hash: hash.clone(),
                                }),
                            );
                        }
                    }
                    KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                        if let Some(target) = row.outpoint(&perspective.observer) {
                            return (
                                true,
                                Some(Navigation::Transaction(TransactionContext {
                                    target,
                                    observer: view.name(&perspective.observer).into(),
                                    feed: feed.clone(),
                                })),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => return (false, None),
    }
    (true, None)
}

fn root_heights(view: &View, row: &Row, perspective: &Perspective) -> String {
    let mut systems = vec![perspective.observer.as_str()];
    if perspective.subject != perspective.observer {
        systems.push(perspective.subject.as_str());
    }
    // Token IDs need not be proof-root systems. Include actual additional roots.
    systems.extend(row.roots.iter().map(|root| root.system.as_str()));
    let mut seen = HashSet::new();
    systems
        .into_iter()
        .filter(|id| seen.insert(*id))
        .map(|id| {
            let height = row
                .roots
                .iter()
                .find(|root| root.system == id)
                .and_then(|root| root.height)
                .map_or_else(|| "—".into(), |height| format!("#{height}"));
            format!("{} {height}", view.name(id))
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

pub(super) fn render(frame: &mut Frame<'_>, theme: &Theme, view: &View, area: Rect) {
    let (perspective, feed) = &view.perspectives[view.active];
    let state = feed.0.lock().unwrap();
    let mut heading = vec![
        Line::styled(
            format!("Notarizations · {}", view.name(&perspective.subject)),
            theme.focused(),
        ),
        Line::from(vec![
            Span::styled(
                format!("{} view", view.name(&perspective.observer)),
                theme.link(),
            ),
            Span::raw(if view.has_perspectives() {
                format!("  ·  {} of {}", view.active + 1, view.perspectives.len())
            } else {
                String::new()
            }),
        ]),
    ];
    let Load::Ready { graph, source } = &state.load else {
        heading.push(Line::styled(load_label(&state.load), theme.muted()));
        frame.render_widget(Paragraph::new(heading).style(theme.base()), area);
        return;
    };

    let page = capacity(area.height);
    let selected = view.selected[view.active];
    let start = first_visible(graph.rows.len(), selected, page);
    let end = (start + page).min(graph.rows.len());
    heading.push(Line::from(vec![
        Span::styled(
            format!(
                "{} rows · {} unconfirmed",
                graph.rows.len(),
                graph.pending()
            ),
            theme.focused(),
        ),
        Span::styled(
            if start < end {
                format!("  ·  Showing {}–{end}", start + 1)
            } else {
                String::new()
            },
            theme.muted(),
        ),
        Span::styled(format!("  ·  {}", source_label(source)), theme.muted()),
    ]));
    if graph.rows.is_empty() {
        heading.push(Line::styled(load_label(&state.load), theme.muted()));
    }
    let visible_height = ((end - start) as u16 * ROW_HEIGHT).min(list_height(area.height));
    let [header, list, evidence] = Layout::vertical([
        Constraint::Length(HEADER_HEIGHT),
        Constraint::Length(visible_height),
        Constraint::Min(0),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(heading).style(theme.base()), header);

    let body = scroll_view::content_area(list, graph.rows.len() > page);
    for (offset, row) in graph.rows[start..end].iter().enumerate() {
        let tile = Rect {
            y: body.y + offset as u16 * ROW_HEIGHT,
            height: body
                .height
                .saturating_sub(offset as u16 * ROW_HEIGHT)
                .min(ROW_HEIGHT),
            ..body
        };
        let lines = notarization_tile_lines(
            theme,
            view,
            perspective,
            &state,
            row,
            Some(row.index) == graph.confirmed,
            tile_content_area(tile).width,
        );
        let focused = row.index == selected;
        let content = render_tile_gutter(frame, theme, focused, tile);
        frame.render_widget(
            Paragraph::new(lines).style(theme.tile_body(focused)),
            content,
        );
    }
    scroll_view::render(frame, theme, list, graph.rows.len(), start, end - start);

    if let Some(row) = graph.rows.get(selected) {
        render_selected_evidence(
            frame,
            theme,
            view,
            perspective,
            &state,
            row,
            Rect {
                height: evidence.height.min(EVIDENCE_HEIGHT),
                ..evidence
            },
        );
    }
}

fn notarization_tile_lines(
    theme: &Theme,
    view: &View,
    perspective: &Perspective,
    state: &Snapshot,
    row: &Row,
    confirmed: bool,
    width: u16,
) -> Vec<Line<'static>> {
    let (status, badge) = if row.synthetic {
        (
            "Synthetic proof root",
            Style::default()
                .fg(theme.muted)
                .add_modifier(Modifier::BOLD),
        )
    } else if confirmed {
        ("Confirmed", theme.status_badge())
    } else {
        (
            "Unconfirmed",
            Style::default()
                .fg(theme.background)
                .bg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )
    };
    let outpoint = match (&row.txid, row.vout) {
        (Some(txid), Some(vout)) => {
            let full = format!("{txid}:{vout}");
            let display = if status.len() + full.len() + 3 <= usize::from(width) {
                full.clone()
            } else {
                format!(
                    "{}:{vout}",
                    truncate_middle(
                        txid,
                        usize::from(width)
                            .saturating_sub(status.len() + 4 + vout.to_string().len())
                            .min(15)
                    )
                )
            };
            theme.value_as(
                "notarization outpoint",
                display,
                full,
                Style::default().fg(theme.link),
            )
        }
        _ => Span::raw("—"),
    };
    let muted = Style::default().fg(theme.muted);
    let check = row.block.as_ref().and_then(|hash| {
        state
            .headers
            .get(&(perspective.observer.clone(), hash.clone()))
    });
    let inclusion = inclusion_text(check, row.block.is_some(), Some(theme));
    let statuses = row
        .roots
        .iter()
        .filter(|root| root.system != perspective.observer)
        .map(|root| {
            let check = root
                .hash
                .as_ref()
                .and_then(|hash| state.headers.get(&(root.system.clone(), hash.clone())));
            format!("{} {}", view.name(&root.system), root_status(check, root))
        })
        .collect::<Vec<_>>()
        .join(" | ");
    vec![
        Line::from(vec![
            Span::styled(format!(" {status} "), badge),
            Span::raw(" "),
            outpoint,
        ]),
        Line::from(vec![
            Span::styled("Proof roots  ", muted),
            theme.value(
                "proof root heights",
                root_heights(view, row, perspective),
                Style::default(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Included ", muted),
            theme.value("inclusion height and time", inclusion, Style::default()),
            Span::raw(if statuses.is_empty() { "" } else { "  ·  " }),
            theme.value("proof root check", statuses, Style::default()),
        ]),
    ]
}

fn render_selected_evidence(
    frame: &mut Frame<'_>,
    theme: &Theme,
    view: &View,
    perspective: &Perspective,
    state: &Snapshot,
    row: &Row,
    area: Rect,
) {
    let foreign_roots: Vec<_> = row
        .roots
        .iter()
        .filter(|root| root.system != perspective.observer)
        .collect();
    let mut lines = Vec::new();
    if let Some(root) = foreign_roots.get(view.root) {
        let hash = root.hash.as_deref().unwrap_or("—");
        let check = state.headers.get(&(root.system.clone(), hash.to_string()));
        lines.push(Line::from(vec![
            Span::styled(
                format!("Proof root {}/{}  ", view.root + 1, foreign_roots.len()),
                Style::default()
                    .fg(theme.focus)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "{} · {}",
                view.name(&root.system),
                root_status(check, root)
            )),
        ]));
        lines.push(Line::from(theme.value(
            "proof root hash",
            hash,
            Style::default().fg(theme.link),
        )));
        if let Some(Check::Found { source, .. } | Check::NotFound { source, .. }) = check {
            lines.push(Line::styled(
                format!("Checked on {}", source_label(source)),
                Style::default().fg(theme.muted),
            ));
        }
    }
    if let Some(target) = &row.foreign_previous {
        let status = match state.transactions.get(target) {
            Some(Check::Found { source, .. }) => format!("Present on {}", source_label(source)),
            Some(_) => "Unavailable".into(),
            None => "Awaiting check".into(),
        };
        let outpoint = format!("{}:{}", target.txid, target.vout);
        let display = if usize::from(area.width) >= 100 {
            outpoint.clone()
        } else {
            format!("{}:{}", short_id(&target.txid), target.vout)
        };
        lines.push(Line::from(vec![
            Span::raw("Previous on "),
            theme.value(
                "system",
                view.name(target.system_id.as_ref()),
                Style::default(),
            ),
            Span::raw("  "),
            theme.value_as(
                "previous outpoint",
                display,
                outpoint,
                Style::default().fg(theme.link),
            ),
            Span::raw(" · "),
            theme.value("previous notarization status", status, Style::default()),
        ]));
    }
    if lines.is_empty() || area.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme.base().bg(theme.panel))
            .block(
                Block::default()
                    .title(" Selected notarization ")
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(theme.muted)),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(number: u64, output: u32) -> Value {
        json!({"txid":format!("{number:064x}"), "vout":output, "blockhash":"b".repeat(64)})
    }

    #[test]
    fn confirmed_anchor_and_pending_count_use_outpoints_across_all_branches() {
        let graph = decode(
            &json!({
                "lastconfirmed":0, "forks":[[0,1,2],[0,1,3]],
                "notarizations":[entry(1,0),entry(2,0),entry(3,0),entry(2,1),entry(2,0),entry(1,0)]
            }),
            "VRSC",
        )
        .unwrap();
        assert_eq!(graph.rows.len(), 6);
        assert_eq!(graph.confirmed, Some(0));
        assert_eq!(graph.pending(), 3);
        assert_eq!(graph.rows[3].vout, Some(1));
    }

    #[test]
    fn empty_view_and_synthetic_root_have_no_transaction_links() {
        let empty = decode(&json!({"lastconfirmed":-1,"notarizations":[]}), "VRSC").unwrap();
        assert_eq!(empty.confirmed, None);
        assert_eq!(empty.pending(), 0);
        assert!(
            decode(
                &json!({"lastconfirmed":1,"notarizations":[entry(1,0)]}),
                "VRSC"
            )
            .is_err()
        );
        let graph = decode(
            &json!({"lastconfirmed":0,"notarizations":[
            {"txid":"0".repeat(64),"blockhash":"0".repeat(64),"vout":-1}]}),
            "VRSC",
        )
        .unwrap();
        assert!(graph.rows[0].synthetic);
        assert!(
            graph.rows[0]
                .outpoint("i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV")
                .is_none()
        );
        let feed = Feed::default();
        feed.0.lock().unwrap().load = Load::NoData;
        let text = summary(&feed, "VRSC", None);
        assert!(
            text.iter()
                .any(|(line, _)| line == "  Unconfirmed in current view: unavailable")
        );
    }

    #[test]
    fn refresh_and_source_revisions_discard_stale_results() {
        let feed = Feed::default();
        let (tx, mut rx) = unbounded_channel();
        let source = Arc::new(crate::sources::Control::default());
        feed.0.lock().unwrap().stamps.push(Stamp::new(&source));
        assert!(feed.update(0, &tx, |s| s.load = Load::Loading));
        assert!(matches!(rx.try_recv(), Ok(AppEvent::NotarizationChanged)));
        source.branch_epoch.fetch_add(1, Ordering::AcqRel);
        assert!(!feed.update(0, &tx, |s| s.load = Load::NoData));
        feed.refresh();
        assert!(!feed.update(0, &tx, |s| s.load = Load::Unavailable));
        feed.0.lock().unwrap().stamps.push(Stamp::new(&source));
        source.set(crate::sources::ConnectionState::Disabled);
        assert!(!feed.update(1, &tx, |s| s.load = Load::NoData));
        assert!(rx.try_recv().is_err());
    }
}
