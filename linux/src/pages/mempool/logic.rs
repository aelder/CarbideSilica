//! The reasoning of the waiting list, apart from its widgets (`WaitingRoom` and
//! `MempoolList` in `MempoolView.swift`): kinds, order, facts and the points of the
//! analysis. Everything here is plain data, so that it has tests.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::bridge::api;
use crate::format;

use super::feed::{Sighting, Sort};
use super::text::{count, span};

/// The serialized size that one block can hold: the consensus limit of Verus.
pub const BLOCK_LIMIT: u64 = 2_000_000;
/// Above this many rows, the page shows the number of each kind first.
pub const MANY_ROWS: usize = 12;
pub const UNTAGGED: &str = "No tag";

/// The kind of a waiting transaction: its first tag. None until the app has it.
pub fn kind_of(entry: &api::MempoolEntry) -> Option<String> {
    entry.classified.then(|| entry.tags.first().cloned().unwrap_or_else(|| UNTAGGED.to_string()))
}

#[derive(Clone, Debug, PartialEq)]
pub struct KindCount {
    pub name: String,
    pub count: usize,
    pub bytes: u64,
    pub fees: i64,
    pub oldest: Option<u64>,
}

/// One row for each kind that waits, the most first (`kindCounts`).
pub fn kind_counts(entries: &[api::MempoolEntry]) -> Vec<KindCount> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<&api::MempoolEntry>> = HashMap::new();
    for entry in entries {
        let name = kind_of(entry).unwrap_or_else(|| "Not read yet".to_string());
        if !groups.contains_key(&name) {
            order.push(name.clone());
        }
        groups.entry(name).or_default().push(entry);
    }
    let mut counts: Vec<KindCount> = order
        .into_iter()
        .map(|name| {
            let members = &groups[&name];
            KindCount {
                name: if name == UNTAGGED { "Payment".to_string() } else { name.clone() },
                count: members.len(),
                bytes: members.iter().map(|e| e.size).sum(),
                fees: members.iter().filter_map(|e| e.fee_sats).sum(),
                oldest: members.iter().filter_map(|e| e.time).min(),
            }
        })
        .collect();
    counts.sort_by(|a, b| b.count.cmp(&a.count));
    counts
}

/// The kinds for the filter plates: the most first, then by name (`kinds`).
pub fn kind_plates(entries: &[api::MempoolEntry]) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for kind in entries.iter().filter_map(kind_of) {
        *counts.entry(kind).or_default() += 1;
    }
    let mut list: Vec<(String, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    list
}

/// Order of arrival. A transaction that spends from a waiting transaction comes directly
/// below it, marked as a child (`ordered`). The result has the index of each entry.
pub fn ordered(entries: &[api::MempoolEntry]) -> Vec<(usize, bool)> {
    let waiting: HashSet<&str> = entries.iter().map(|e| e.txid.as_str()).collect();
    let mut by_arrival: Vec<usize> = (0..entries.len()).collect();
    by_arrival.sort_by(|a, b| {
        let (a, b) = (&entries[*a], &entries[*b]);
        (a.time.unwrap_or(0), &a.txid).cmp(&(b.time.unwrap_or(0), &b.txid))
    });
    let mut placed: HashSet<usize> = HashSet::new();
    let mut result = Vec::new();
    fn place(
        index: usize,
        child: bool,
        entries: &[api::MempoolEntry],
        by_arrival: &[usize],
        placed: &mut HashSet<usize>,
        result: &mut Vec<(usize, bool)>,
    ) {
        if !placed.insert(index) {
            return;
        }
        result.push((index, child));
        for next in by_arrival {
            if entries[*next].parents.contains(&entries[index].txid) {
                place(*next, true, entries, by_arrival, placed, result);
            }
        }
    }
    for index in &by_arrival {
        if entries[*index].parents.iter().all(|p| !waiting.contains(p.as_str())) {
            place(*index, false, entries, &by_arrival, &mut placed, &mut result);
        }
    }
    for index in &by_arrival {
        place(*index, false, entries, &by_arrival, &mut placed, &mut result);
    }
    result
}

/// The facts of a waiting transaction: wait, fee, size, expiry and waiting parents.
pub fn facts(
    entry: &api::MempoolEntry,
    now: f64,
    coin: &str,
    detail: Option<&api::WaitingTransaction>,
    tip: Option<u64>,
    waiting: &HashSet<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(time) = entry.time {
        parts.push(format!("Waited {}", span(now - time as f64)));
    }
    if let Some(fee) = entry.fee_sats {
        parts.push(format!("fee {} {coin}", format::coins(fee)));
    }
    parts.push(format::bytes(entry.size));
    if let (Some(detail), Some(tip)) = (detail, tip) {
        match detail.expiry_height {
            Some(expiry) => parts.push(if expiry > tip {
                format!("expires in {}", count((expiry - tip) as usize, "block", "blocks"))
            } else {
                "at its expiry height".to_string()
            }),
            None => parts.push("does not expire".to_string()),
        }
    }
    let parents = entry.parents.iter().filter(|p| waiting.contains(p.as_str())).count();
    if parents > 0 {
        parts.push(format!(
            "spends from {} that also {}",
            count(parents, "transaction", "transactions"),
            if parents == 1 { "waits" } else { "wait" }
        ));
    }
    parts.join(" · ")
}

/// One point of the analysis: a name, a short value and, where it helps, what it means.
#[derive(Clone, Debug, PartialEq)]
pub struct Point {
    pub name: String,
    pub value: String,
    pub note: Option<String>,
}

impl Point {
    fn new(name: &str, value: impl Into<String>, note: Option<String>) -> Self {
        Self { name: name.to_string(), value: value.into(), note }
    }
}

/// The usual wait for a block, from the record of the last day: the middle wait and the
/// wait that 9 of 10 are below. The waits are in order.
pub fn usual_wait(waits: &[u32]) -> Option<(f64, f64)> {
    (waits.len() >= 20).then(|| (waits[waits.len() / 2] as f64, waits[waits.len() * 9 / 10] as f64))
}

fn usual_point(waits: Option<&[u32]>) -> Option<Point> {
    let (middle, most) = usual_wait(waits?)?;
    Some(Point::new(
        "Usual wait",
        span(middle),
        Some(format!(
            "In the last day, half of the blocks came within this time of the block before. 9 of 10 came within {}.",
            span(most)
        )),
    ))
}

/// What the analysis needs that is not in the feed.
pub struct Context<'a> {
    pub now: f64,
    pub coin: &'a str,
    pub tip: Option<u64>,
    /// The record of waits of the last day, in order.
    pub waits: Option<&'a [u32]>,
}

/// What the mempool is, what waits in it now, and what that means for a person who has a
/// transaction in it (`meaning`).
pub fn meaning(
    entries: &[api::MempoolEntry],
    total_bytes: u64,
    sightings: &HashMap<String, Sighting>,
    details: &HashMap<String, api::WaitingTransaction>,
    context: &Context,
) -> Vec<Point> {
    let mut points = Vec::new();
    if entries.is_empty() {
        points.push(Point::new(
            "Mempool",
            "Empty",
            Some(
                "The mempool holds the transactions that no block has yet. A transaction stays here until a miner or a staker puts it in a block."
                    .into(),
            ),
        ));
        points.extend(usual_point(context.waits));
        let passed: Vec<&Sighting> = sightings.values().filter(|s| s.left.is_some() && !s.gone).collect();
        if !passed.is_empty() {
            let longest = passed.iter().filter_map(|s| s.left.map(|left| left - s.arrival)).fold(None, |m: Option<f64>, v| {
                Some(m.map_or(v, |m| m.max(v)))
            });
            points.push(Point::new(
                "Went into a block",
                count(passed.len(), "transaction", "transactions"),
                Some(format!(
                    "This is the count while this page was open. The longest wait was {}.",
                    span(longest.unwrap_or(0.0))
                )),
            ));
        }
        return points;
    }

    // What waits, by kind.
    let names: Vec<String> = kind_counts(entries).iter().map(|k| format!("{} {}", k.count, k.name.to_lowercase())).collect();
    points.push(Point::new("Waiting", count(entries.len(), "transaction", "transactions"), Some(names.join(", "))));

    // How long, and how long more.
    if let Some(oldest) = entries.iter().filter_map(|e| e.time).min() {
        points.push(Point::new("Oldest", format!("{} ago", span(context.now - oldest as f64)), None));
    }
    points.extend(usual_point(context.waits));

    // Room, fees, and what can stop a transaction.
    points.push(if total_bytes <= BLOCK_LIMIT {
        Point::new("Next block", "All fit", None)
    } else {
        Point::new(
            "Next block",
            "Not all fit",
            Some("A block takes the transactions with the higher fee for each byte first.".into()),
        )
    });
    let fees: Vec<i64> = entries.iter().filter_map(|e| e.fee_sats).collect();
    if !fees.is_empty() {
        points.push(Point::new("Fees", format!("{} {}", format::coins(fees.iter().sum()), context.coin), None));
    }
    let waiting: HashSet<&str> = entries.iter().map(|e| e.txid.as_str()).collect();
    let chained = entries.iter().filter(|e| e.parents.iter().any(|p| waiting.contains(p.as_str()))).count();
    if chained > 0 {
        points.push(Point::new(
            "Spend from waiting",
            count(chained, "transaction", "transactions"),
            Some("Each spends from a transaction that also waits. A block must take that one first or with it.".into()),
        ));
    }
    if let Some(tip) = context.tip {
        let soon = entries
            .iter()
            .filter(|e| details.get(&e.txid).and_then(|d| d.expiry_height).is_some_and(|expiry| expiry <= tip + 10))
            .count();
        if soon > 0 {
            points.push(Point::new(
                "Expire soon",
                count(soon, "transaction", "transactions"),
                Some(
                    "These expire in 10 blocks or less. A transaction that expires leaves the mempool and does not go into a block."
                        .into(),
                ),
            ));
        }
    }
    points
}

/// The sentence of the waiting list (`WaitingRoom.sentence`). The Swift page has this
/// function and does not show it; the port keeps it with the same words.
/// `last` is the wait since the last block and the count of transactions it took.
#[cfg_attr(not(test), allow(dead_code))]
pub fn sentence(waiting: usize, last: Option<(f64, Option<usize>)>, share_shorter: Option<f64>, syncing: bool) -> String {
    let first = if waiting == 0 {
        "Nothing waits.".to_string()
    } else {
        format!("{}.", count(waiting, "transaction waits", "transactions wait"))
    };
    let Some((wait, took)) = last else { return first };
    let mut parts = vec![first];
    let mut block = format!("The last block came {} ago", span(wait));
    if let Some(share) = share_shorter.filter(|s| *s >= 0.5) {
        block += &format!(", longer than {} of 100 recent waits", (share * 100.0).floor() as i64);
    }
    parts.push(block + ".");
    if waiting == 0
        && let Some(took) = took
    {
        parts.push(if took == 0 {
            "It took no transaction from the mempool.".to_string()
        } else {
            format!("It took {}.", count(took, "transaction", "transactions"))
        });
    }
    if syncing {
        parts.push("A node does not collect unconfirmed transactions during synchronization.".into());
    }
    parts.join(" ")
}

/// Whether the waiting transactions fit in one block, in words (`roomText`). The Swift
/// page has this function and does not show it; the port keeps it with the same words.
#[cfg_attr(not(test), allow(dead_code))]
pub fn room_text(bytes: u64) -> String {
    if bytes <= BLOCK_LIMIT {
        format!("All that waits fits in one block: {} of {}.", format::bytes(bytes), format::bytes(BLOCK_LIMIT))
    } else {
        format!(
            "More waits than one block holds: {} of {}. A block takes the transactions with the higher fee for each byte first.",
            format::bytes(bytes),
            format::bytes(BLOCK_LIMIT)
        )
    }
}

/// The list's rows in the chosen order (`MempoolList.sorted`).
pub fn sorted(entries: &[api::MempoolEntry], sort: Sort, descending: bool) -> Vec<api::MempoolEntry> {
    let mut list = entries.to_vec();
    let ascending = |a: &api::MempoolEntry, b: &api::MempoolEntry| -> Ordering {
        match sort {
            Sort::Arrival => (a.time.unwrap_or(0), &a.txid).cmp(&(b.time.unwrap_or(0), &b.txid)),
            Sort::Fee => (a.fee_sats.unwrap_or(-1), &a.txid).cmp(&(b.fee_sats.unwrap_or(-1), &b.txid)),
            Sort::Size => (a.size, &a.txid).cmp(&(b.size, &b.txid)),
            Sort::Height => (a.height.unwrap_or(0), &a.txid).cmp(&(b.height.unwrap_or(0), &b.txid)),
            Sort::Parents => (a.parents.len(), &a.txid).cmp(&(b.parents.len(), &b.txid)),
        }
    };
    list.sort_by(ascending);
    if descending {
        list.reverse();
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(txid: &str, time: u64, parents: &[&str], tags: Option<&[&str]>) -> api::MempoolEntry {
        api::MempoolEntry {
            txid: txid.into(),
            size: 1000,
            fee_sats: Some(20_000),
            time: Some(time),
            height: Some(50),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            classified: tags.is_some(),
            tags: tags.unwrap_or(&[]).iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn children_follow_their_parent() {
        let entries = vec![
            entry("c", 30, &["a"], None),
            entry("b", 20, &[], None),
            entry("a", 10, &[], None),
            entry("d", 40, &["c"], None),
            entry("e", 5, &["outside"], None),
        ];
        let order: Vec<(&str, bool)> = ordered(&entries).into_iter().map(|(i, c)| (entries[i].txid.as_str(), c)).collect();
        assert_eq!(order, [("e", false), ("a", false), ("c", true), ("d", true), ("b", false)]);
    }

    #[test]
    fn kinds() {
        let entries = vec![
            entry("a", 10, &[], Some(&["Transfer"])),
            entry("b", 20, &[], Some(&[])),
            entry("c", 30, &[], None),
            entry("d", 5, &[], Some(&["Transfer", "Conversion"])),
        ];
        let counts = kind_counts(&entries);
        assert_eq!(counts[0], KindCount { name: "Transfer".into(), count: 2, bytes: 2000, fees: 40_000, oldest: Some(5) });
        assert_eq!(counts[1].name, "Payment");
        assert_eq!(counts[2].name, "Not read yet");
        assert_eq!(kind_plates(&entries), vec![("Transfer".to_string(), 2), (UNTAGGED.to_string(), 1)]);
    }

    #[test]
    fn facts_of_a_row() {
        let mut e = entry("b", 100, &["a", "outside"], None);
        e.fee_sats = Some(12_345);
        let waiting: HashSet<&str> = ["a", "b"].into();
        let detail = api::WaitingTransaction {
            txid: "b".into(),
            found: true,
            tags: vec![],
            statement: None,
            coinbase: false,
            fee_pool_checkpoint: false,
            expiry_height: Some(120),
            height: None,
            block_hash: None,
            coin: "VRSC".into(),
        };
        assert_eq!(
            facts(&e, 190.0, "VRSC", Some(&detail), Some(100), &waiting),
            "Waited 90 s · fee 0.00012345 VRSC · 1,000 bytes · expires in 20 blocks · spends from 1 transaction that also waits"
        );
        assert_eq!(facts(&e, 190.0, "VRSC", Some(&detail), Some(120), &HashSet::new()), "Waited 90 s · fee 0.00012345 VRSC · 1,000 bytes · at its expiry height");
    }

    #[test]
    fn analysis_points() {
        let waits: Vec<u32> = (1..=40).collect();
        let context = Context { now: 1000.0, coin: "VRSC", tip: Some(100), waits: Some(&waits) };
        let points = meaning(&[], 0, &HashMap::new(), &HashMap::new(), &context);
        assert_eq!(points[0].value, "Empty");
        assert_eq!(points[1].name, "Usual wait");
        assert_eq!(points[1].value, "21 s");
        let entries = vec![entry("a", 900, &[], Some(&["Transfer"])), entry("b", 950, &["a"], None)];
        let points = meaning(&entries, 3_000_000, &HashMap::new(), &HashMap::new(), &context);
        let names: Vec<&str> = points.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Waiting", "Oldest", "Usual wait", "Next block", "Fees", "Spend from waiting"]);
        assert_eq!(points[0].note.as_deref(), Some("1 transfer, 1 not read yet"));
        assert_eq!(points[1].value, "100 s ago");
        assert_eq!(points[3].value, "Not all fit");
        assert_eq!(points[4].value, "0.0004 VRSC");
    }

    #[test]
    fn sentences() {
        assert_eq!(sentence(0, None, None, false), "Nothing waits.");
        assert_eq!(
            sentence(0, Some((95.0, Some(0))), Some(0.62), true),
            "Nothing waits. The last block came 95 s ago, longer than 62 of 100 recent waits. It took no transaction from the mempool. A node does not collect unconfirmed transactions during synchronization."
        );
        assert_eq!(sentence(2, Some((30.0, Some(3))), Some(0.2), false), "2 transactions wait. The last block came 30 s ago.");
        assert!(room_text(1024).starts_with("All that waits fits in one block: 1 KB of 1.9 MB."));
    }

    #[test]
    fn sorting() {
        let mut a = entry("a", 10, &[], None);
        a.fee_sats = None;
        let b = entry("b", 20, &["x"], None);
        let list = sorted(&[a.clone(), b.clone()], Sort::Fee, true);
        assert_eq!(list[0].txid, "b");
        let list = sorted(&[b, a], Sort::Arrival, false);
        assert_eq!(list[0].txid, "a");
    }
}
