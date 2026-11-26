//! The mempool for the whole session (`MempoolFeed`).
//!
//! The terminal's loading pattern: decoded transactions in bulk on the first load when the
//! node supports it, then fast refreshes of membership and metadata, with one request for
//! each new entry. The feed also keeps what the app saw in this session: each transaction's
//! arrival and departure (the sightings), what the page says about a transaction (the
//! details) and the transactions that the last blocks took.
//!
//! There is one feed for the app. `background` refreshes it every 2 seconds on each page,
//! so that the Mempool page has the last minutes when a person opens it.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;

use crate::bridge::{self, api};
use crate::format;
use crate::model::AppModel;

/// The terminal refreshes its mempool every 2 seconds.
pub const INTERVAL: Duration = Duration::from_secs(2);
/// How many blocks back the page looks for what left.
pub const LEFT_BLOCKS: usize = 30;
/// A sighting stays for this long after the transaction left.
pub const MEMORY: f64 = 20.0 * 60.0;
/// The most rows that the list of what left shows (`LeftList.shown`).
pub const LEFT_SHOWN: usize = 12;

/// A transaction that the app saw in the mempool (`MempoolSighting`).
#[derive(Clone, Debug, PartialEq)]
pub struct Sighting {
    pub txid: String,
    /// When the node first had it, in Unix seconds.
    pub arrival: f64,
    pub size: u64,
    pub fee_sats: Option<i64>,
    pub parents: Vec<String>,
    /// The refresh that first did not have it. None while it waits.
    pub left: Option<f64>,
    /// The block that took it.
    pub height: Option<u64>,
    pub block_hash: Option<String>,
    /// The node does not have it in a block: it expired, or another transaction replaced it.
    pub gone: bool,
    pub checks: u32,
}

/// The list's sort columns (`MempoolSort`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Arrival,
    Fee,
    Size,
    Height,
    Parents,
}

impl Sort {
    pub const ALL: [Sort; 5] = [Sort::Arrival, Sort::Fee, Sort::Size, Sort::Height, Sort::Parents];

    pub fn title(self) -> &'static str {
        match self {
            Sort::Arrival => "Arrival",
            Sort::Fee => "Fee",
            Sort::Size => "Size",
            Sort::Height => "Entry height",
            Sort::Parents => "Unconfirmed parents",
        }
    }
}

pub struct Feed {
    pub entries: Vec<api::MempoolEntry>,
    pub total_bytes: u64,
    pub loaded: bool,
    pub refreshing: bool,
    pub failure: Option<String>,
    pub last_refresh: Option<f64>,
    /// The source of the last answer. The mempool is always live: it is never stored.
    pub provenance: Option<api::Provenance>,
    /// The list's selection and order, kept for the session.
    pub selection: Option<String>,
    pub sort: Sort,
    pub descending: bool,
    /// The transactions that the app saw in the mempool in this session, by transaction ID.
    pub sightings: HashMap<String, Sighting>,
    /// What the Mempool page says about a transaction: tags, statement, expiry and block.
    pub details: HashMap<String, api::WaitingTransaction>,
    /// The transactions of the last blocks that were not block rewards or stakes.
    pub took: Vec<api::RecentTransaction>,
    /// The blocks whose transactions `took` holds, also when a block took none.
    pub took_blocks: HashSet<String>,

    bulk_done: bool,
    generation: Option<u64>,
    /// Tags already loaded, by transaction ID. An entry keeps them across refreshes.
    classified: HashMap<String, Vec<String>>,
    /// Entries that a request could not find. They are not requested again.
    unavailable: HashSet<String>,
    listeners: Vec<(glib::WeakRef<glib::Object>, Rc<dyn Fn()>)>,
}

impl Default for Feed {
    fn default() -> Self {
        Self {
            entries: vec![],
            total_bytes: 0,
            loaded: false,
            refreshing: false,
            failure: None,
            last_refresh: None,
            provenance: None,
            selection: None,
            sort: Sort::Arrival,
            descending: true,
            sightings: HashMap::new(),
            details: HashMap::new(),
            took: vec![],
            took_blocks: HashSet::new(),
            bulk_done: false,
            generation: None,
            classified: HashMap::new(),
            unavailable: HashSet::new(),
            listeners: vec![],
        }
    }
}

thread_local! {
    static FEED: RefCell<Feed> = RefCell::new(Feed::default());
}

/// Reads the feed.
pub fn with<T>(read: impl FnOnce(&Feed) -> T) -> T {
    FEED.with(|cell| read(&cell.borrow()))
}

/// Changes the feed. Call `changed` after it, so that the page redraws.
pub fn with_mut<T>(change: impl FnOnce(&mut Feed) -> T) -> T {
    FEED.with(|cell| change(&mut cell.borrow_mut()))
}

/// Calls `listener` after each change of the feed, while `owner` exists.
pub fn watch(owner: &impl gtk::prelude::IsA<glib::Object>, listener: impl Fn() + 'static) {
    use gtk::prelude::*;
    with_mut(|feed| feed.listeners.push((owner.upcast_ref::<glib::Object>().downgrade(), Rc::new(listener))));
}

pub fn changed() {
    let listeners: Vec<Rc<dyn Fn()>> = with_mut(|feed| {
        feed.listeners.retain(|(owner, _)| owner.upgrade().is_some());
        feed.listeners.iter().map(|(_, f)| f.clone()).collect()
    });
    for listener in listeners {
        listener();
    }
}

/// Refreshes the feed for the whole session: the work of the Swift sidebar's task.
pub fn keep(model: &Rc<AppModel>) {
    let model = Rc::downgrade(model);
    glib::spawn_future_local(async move {
        loop {
            let Some(strong) = model.upgrade() else { return };
            refresh(&strong).await;
            drop(strong);
            glib::timeout_future(INTERVAL).await;
        }
    });
}

/// One refresh (`MempoolFeed.refresh`).
pub async fn refresh(model: &Rc<AppModel>) {
    let Some(explorer) = model.explorer() else { return };
    let generation = model.generation();
    let busy = with_mut(|feed| {
        if feed.refreshing {
            return true;
        }
        if feed.generation != Some(generation) {
            feed.reset(generation);
        }
        feed.refreshing = true;
        false
    });
    if busy {
        return;
    }
    changed();
    run(model, explorer, generation).await;
    with_mut(|feed| feed.refreshing = false);
    changed();
}

async fn run(model: &Rc<AppModel>, explorer: bridge::Explorer, generation: u64) {
    let full = !with(|feed| feed.bulk_done);
    let ex = explorer.clone();
    let snapshot = bridge::run(async move { ex.mempool_snapshot(full).await }).await;
    if generation != model.generation() {
        return;
    }
    let snapshot = match snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => {
            with_mut(|feed| {
                feed.failure = Some(bridge::describe(&error));
                feed.provenance = None;
                feed.loaded = true;
                feed.last_refresh = Some(format::now());
            });
            return;
        }
    };
    let members: HashSet<String> = snapshot.entries.iter().map(|e| e.txid.clone()).collect();
    let missing = with_mut(|feed| {
        feed.bulk_done = true;
        for entry in snapshot.entries.iter().filter(|e| e.classified) {
            feed.classified.insert(entry.txid.clone(), entry.tags.clone());
        }
        feed.classified.retain(|txid, _| members.contains(txid));
        feed.unavailable.retain(|txid| members.contains(txid));
        feed.entries = snapshot.entries.iter().map(|e| feed.merged(e)).collect();
        feed.total_bytes = snapshot.total_bytes;
        feed.provenance = Some(snapshot.provenance.clone());
        feed.failure = None;
        feed.loaded = true;
        feed.last_refresh = Some(format::now());
        feed.entries
            .iter()
            .filter(|e| !e.classified && !feed.unavailable.contains(&e.txid))
            .map(|e| e.txid.clone())
            .collect::<Vec<String>>()
    });
    changed();

    if !missing.is_empty() {
        let ex = explorer.clone();
        let found = bridge::run(async move { ex.classify_mempool(missing).await }).await;
        if generation != model.generation() {
            return;
        }
        with_mut(|feed| {
            for item in found {
                if item.found {
                    feed.classified.insert(item.txid, item.tags);
                } else {
                    feed.unavailable.insert(item.txid);
                }
            }
            let entries = std::mem::take(&mut feed.entries);
            feed.entries = entries.iter().map(|e| feed.merged(e)).collect();
        });
        changed();
    }
    with_mut(|feed| {
        let entries = feed.entries.clone();
        note(&mut feed.sightings, &entries, &members, format::now());
    });
    changed();
    read_details(model, &explorer, generation).await;
    changed();
    read_took(model, &explorer, generation).await;
}

/// Reads the statement of each waiting transaction, the block of each one that left, and
/// the statement of the newest transactions that the last blocks took.
async fn read_details(model: &Rc<AppModel>, explorer: &bridge::Explorer, generation: u64) {
    let wanted = with(|feed| feed.wanted_details());
    if wanted.is_empty() {
        return;
    }
    let ex = explorer.clone();
    let found = bridge::run(async move { ex.waiting_transactions(wanted).await }).await;
    if generation != model.generation() {
        return;
    }
    with_mut(|feed| feed.apply_details(found));
}

/// The transactions of the last blocks, without block rewards and stakes.
async fn read_took(model: &Rc<AppModel>, explorer: &bridge::Explorer, generation: u64) {
    let window: Vec<String> = model.state().blocks.iter().take(LEFT_BLOCKS).map(|b| b.hash.clone()).collect();
    let current: HashSet<String> = window.iter().cloned().collect();
    let new: Vec<String> = with(|feed| window.iter().filter(|h| !feed.took_blocks.contains(*h)).cloned().collect());
    if new.is_empty() {
        return;
    }
    let ex = explorer.clone();
    let asked = new.clone();
    let fetched = bridge::run(async move { ex.recent_transactions(asked, 100).await }).await;
    let Ok(fetched) = fetched else { return };
    if generation != model.generation() {
        return;
    }
    with_mut(|feed| {
        let took = std::mem::take(&mut feed.took);
        feed.took = merge_took(fetched, took, &current);
        feed.took_blocks = feed.took_blocks.union(&new.iter().cloned().collect()).filter(|h| current.contains(*h)).cloned().collect();
    });
}

impl Feed {
    fn reset(&mut self, generation: u64) {
        self.generation = Some(generation);
        self.entries.clear();
        self.provenance = None;
        self.total_bytes = 0;
        self.loaded = false;
        self.bulk_done = false;
        self.classified.clear();
        self.unavailable.clear();
        self.failure = None;
        self.sightings.clear();
        self.details.clear();
        self.took.clear();
        self.took_blocks.clear();
    }

    fn merged(&self, entry: &api::MempoolEntry) -> api::MempoolEntry {
        let mut entry = entry.clone();
        if let Some(tags) = self.classified.get(&entry.txid) {
            entry.classified = true;
            entry.tags = tags.clone();
        }
        entry
    }

    /// The departed first, then what waits, then the newest that left with a block: 12 at
    /// most in one request.
    fn wanted_details(&self) -> Vec<String> {
        let waiting = self.entries.iter().map(|e| e.txid.clone()).filter(|t| !self.details.contains_key(t));
        let departed = self
            .sightings
            .values()
            .filter(|s| s.left.is_some() && s.height.is_none() && !s.gone)
            .map(|s| s.txid.clone());
        let recent = self
            .took
            .iter()
            .take(LEFT_SHOWN)
            .map(|t| t.txid.clone())
            .filter(|t| !self.details.contains_key(t));
        departed.chain(waiting).chain(recent).take(12).collect()
    }

    fn apply_details(&mut self, found: Vec<api::WaitingTransaction>) {
        for item in found {
            let txid = item.txid.clone();
            let (height, hash, was_found) = (item.height, item.block_hash.clone(), item.found);
            if item.found {
                self.details.insert(txid.clone(), item);
            }
            let Some(sighting) = self.sightings.get_mut(&txid) else { continue };
            if sighting.left.is_none() {
                continue;
            }
            if let Some(height) = height {
                sighting.height = Some(height);
                sighting.block_hash = hash;
            } else {
                // The node does not have it in a block. After some refreshes, say so.
                sighting.checks += 1;
                if !was_found || sighting.checks >= 5 {
                    sighting.gone = true;
                }
            }
        }
        let kept: HashSet<&String> =
            self.entries.iter().map(|e| &e.txid).chain(self.sightings.keys()).chain(self.took.iter().map(|t| &t.txid)).collect();
        let kept: HashSet<String> = kept.into_iter().cloned().collect();
        self.details.retain(|txid, _| kept.contains(txid));
    }
}

/// Records arrivals and departures. The node gives the arrival time. The departure time is
/// the refresh that first did not have the transaction.
pub fn note(sightings: &mut HashMap<String, Sighting>, entries: &[api::MempoolEntry], members: &HashSet<String>, now: f64) {
    for entry in entries {
        match sightings.get_mut(&entry.txid) {
            Some(known) => {
                known.left = None;
                known.gone = false;
                known.size = entry.size;
                known.fee_sats = entry.fee_sats;
                known.parents = entry.parents.clone();
            }
            None => {
                sightings.insert(
                    entry.txid.clone(),
                    Sighting {
                        txid: entry.txid.clone(),
                        arrival: entry.time.map(|t| t as f64).unwrap_or(now),
                        size: entry.size,
                        fee_sats: entry.fee_sats,
                        parents: entry.parents.clone(),
                        left: None,
                        height: None,
                        block_hash: None,
                        gone: false,
                        checks: 0,
                    },
                );
            }
        }
    }
    for (txid, sighting) in sightings.iter_mut() {
        if sighting.left.is_none() && !members.contains(txid) {
            sighting.left = Some(now);
        }
    }
    sightings.retain(|_, s| s.left.is_none_or(|left| now - left < MEMORY));
}

/// The new transactions of the last blocks with the ones the feed had, in the window,
/// newest block first and the last position first, once each. Block rewards and stakes
/// are not in it.
pub fn merge_took(
    fetched: Vec<api::RecentTransaction>,
    took: Vec<api::RecentTransaction>,
    current: &HashSet<String>,
) -> Vec<api::RecentTransaction> {
    let reward = |t: &api::RecentTransaction| t.tags.iter().any(|tag| tag == "Coinbase" || tag == "Stake");
    let mut all: Vec<api::RecentTransaction> =
        fetched.into_iter().filter(|t| !reward(t)).chain(took).filter(|t| current.contains(&t.block_hash)).collect();
    all.sort_by(|a, b| (b.block_height, b.position).cmp(&(a.block_height, a.position)));
    let mut seen = HashSet::new();
    all.retain(|t| seen.insert(t.txid.clone()));
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn entry(txid: &str, time: Option<u64>) -> api::MempoolEntry {
        api::MempoolEntry {
            txid: txid.into(),
            size: 300,
            fee_sats: Some(10_000),
            time,
            height: Some(100),
            parents: vec![],
            classified: false,
            tags: vec![],
        }
    }

    fn recent(txid: &str, block: &str, height: u64, position: u32, tags: &[&str]) -> api::RecentTransaction {
        api::RecentTransaction {
            txid: txid.into(),
            block_hash: block.into(),
            block_height: height,
            block_time: 0,
            position,
            loaded: true,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            input_count: 1,
            output_count: 2,
            output_sats: None,
        }
    }

    #[test]
    fn arrivals_and_departures() {
        let mut sightings = HashMap::new();
        let a = entry("a", Some(1000));
        let b = entry("b", None);
        let members: HashSet<String> = ["a".to_string(), "b".to_string()].into();
        note(&mut sightings, &[a.clone(), b], &members, 1010.0);
        assert_eq!(sightings["a"].arrival, 1000.0);
        assert_eq!(sightings["b"].arrival, 1010.0);
        // b left at the next refresh.
        let members: HashSet<String> = ["a".to_string()].into();
        note(&mut sightings, &[a.clone()], &members, 1012.0);
        assert_eq!(sightings["b"].left, Some(1012.0));
        assert_eq!(sightings["a"].left, None);
        // The departure time stays the first refresh without it.
        note(&mut sightings, &[a.clone()], &members, 1014.0);
        assert_eq!(sightings["b"].left, Some(1012.0));
        // It is forgotten after 20 minutes.
        note(&mut sightings, &[a], &members, 1012.0 + MEMORY + 1.0);
        assert!(!sightings.contains_key("b"));
    }

    #[test]
    fn what_left_with_blocks() {
        let current: HashSet<String> = ["h2".to_string(), "h1".to_string()].into();
        let fetched = vec![
            recent("x", "h2", 2, 1, &[]),
            recent("cb", "h2", 2, 0, &["Coinbase"]),
            recent("st", "h2", 2, 2, &["Stake"]),
            recent("y", "h2", 2, 3, &["Transfer"]),
        ];
        let before = vec![recent("old", "h0", 0, 1, &[]), recent("z", "h1", 1, 1, &[]), recent("x", "h2", 2, 1, &[])];
        let took = merge_took(fetched, before, &current);
        let ids: Vec<&str> = took.iter().map(|t| t.txid.as_str()).collect();
        assert_eq!(ids, ["y", "x", "z"]);
    }

    #[test]
    fn details_mark_what_is_gone() {
        let mut feed = Feed::default();
        let members: HashSet<String> = HashSet::new();
        feed.sightings.insert(
            "a".into(),
            Sighting {
                txid: "a".into(),
                arrival: 0.0,
                size: 1,
                fee_sats: None,
                parents: vec![],
                left: Some(5.0),
                height: None,
                block_hash: None,
                gone: false,
                checks: 0,
            },
        );
        let _ = members;
        let missing = api::WaitingTransaction {
            txid: "a".into(),
            found: true,
            tags: vec![],
            statement: None,
            coinbase: false,
            fee_pool_checkpoint: false,
            expiry_height: None,
            height: None,
            block_hash: None,
            coin: "VRSC".into(),
        };
        for _ in 0..4 {
            feed.apply_details(vec![missing.clone()]);
        }
        assert!(!feed.sightings["a"].gone);
        assert_eq!(feed.wanted_details(), vec!["a".to_string()]);
        feed.apply_details(vec![missing.clone()]);
        assert!(feed.sightings["a"].gone);
        assert!(feed.wanted_details().is_empty());
        // A block that took it gives its height.
        feed.sightings.get_mut("a").unwrap().gone = false;
        feed.apply_details(vec![api::WaitingTransaction { height: Some(7), block_hash: Some("h".into()), ..missing }]);
        assert_eq!(feed.sightings["a"].height, Some(7));
    }
}
