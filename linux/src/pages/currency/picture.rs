//! The arithmetic of the notarization browser (`NotarizationPicture.swift`): the state of a
//! link, the intervals of its record, the marks of the graph and their layout. The widgets
//! that draw them are in `graph.rs`.

use std::collections::{BTreeMap, HashMap};

use crate::bridge::api::{BrowserNotarization, NotarizationBrowser, RecordPoint};
use crate::format;
use crate::theme::{tone, Color};

/// The state of a link between two systems, from the record of its notarizations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// The wait is in the range of the record.
    Current,
    /// The wait is longer than each interval of the record.
    Late,
    /// The wait is more than 100 times the longest interval of the record.
    Stopped,
}

impl LinkState {
    /// A link with fewer intervals than this has no state.
    pub const MINIMUM_INTERVALS: usize = 5;
    pub const STOPPED_FACTOR: u64 = 100;

    pub fn new(wait: Option<u64>, intervals: &[u64]) -> Option<Self> {
        let wait = wait?;
        if intervals.len() < Self::MINIMUM_INTERVALS {
            return None;
        }
        let longest = *intervals.iter().max()?;
        Some(if wait > longest.saturating_mul(Self::STOPPED_FACTOR) {
            LinkState::Stopped
        } else if wait > longest {
            LinkState::Late
        } else {
            LinkState::Current
        })
    }

    pub fn word(self) -> &'static str {
        match self {
            LinkState::Current => "Current",
            LinkState::Late => "Late",
            LinkState::Stopped => "Stopped",
        }
    }

    pub fn rule(self) -> String {
        match self {
            LinkState::Current => "Current means that the wait is not longer than the longest interval of the record.".into(),
            LinkState::Late => "Late means that the wait is longer than each interval of the record.".into(),
            LinkState::Stopped => format!(
                "Stopped means that the wait is more than {} times the longest interval of the record.",
                Self::STOPPED_FACTOR
            ),
        }
    }

    pub fn color(self) -> Color {
        match self {
            LinkState::Current => tone::PRESENT,
            LinkState::Late => tone::MINED,
            LinkState::Stopped => tone::ALERT,
        }
    }
}

/// The blocks between the entries of a record in sequence. The record is newest first.
pub fn intervals(record: &[RecordPoint]) -> Vec<u64> {
    record
        .iter()
        .zip(record.iter().skip(1))
        .filter_map(|(newer, older)| (newer.height > older.height).then(|| newer.height - older.height))
        .collect()
}

/// A number of blocks as a length of time, at one block for each minute. Days stay days
/// for two years, because the wait of a link is compared in days.
pub fn span(blocks: u64) -> String {
    match blocks {
        1 => "1 minute".into(),
        b if b < 120 => format!("{b} minutes"),
        b if b < 2880 => format!("{} hours", (b as f64 / 60.0).round() as i64),
        b if b < 1_051_200 => format!("{} days", (b as f64 / 1440.0).round() as i64),
        b => format!("{} years", format::decimal(b as f64 / 525_600.0, 1)),
    }
}

pub fn median(values: &[u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Some(sorted[sorted.len() / 2])
}

/// One mark of the graph: one notarization, or entries in sequence that commit to the same
/// block of the other system.
#[derive(Clone, Debug)]
pub struct NotarizationMark {
    pub rows: Vec<BrowserNotarization>,
    /// 0 is the chain that the node gives as the best one.
    pub lane: usize,
    pub column: usize,
    pub parent: Option<u32>,
}

impl NotarizationMark {
    pub fn id(&self) -> u32 {
        self.rows.first().map(|r| r.index).unwrap_or(0)
    }

    pub fn confirmed(&self) -> bool {
        self.rows.iter().any(|r| r.confirmed)
    }

    pub fn newest(&self) -> Option<&BrowserNotarization> {
        self.rows.last()
    }
}

pub fn observer_height(row: &BrowserNotarization) -> Option<u64> {
    row.roots.iter().find(|r| !r.foreign).and_then(|r| r.height).or(row.included_height)
}

pub fn other_height(row: &BrowserNotarization) -> Option<u64> {
    row.roots.iter().find(|r| r.foreign).and_then(|r| r.height)
}

/// The marks of a view. The best chain is lane 0. Each other fork is a new lane from the
/// mark where it leaves the entries that are placed.
pub fn marks(view: &NotarizationBrowser) -> Vec<NotarizationMark> {
    let mut by_index: HashMap<u32, &BrowserNotarization> = HashMap::new();
    for row in &view.rows {
        by_index.entry(row.index).or_insert(row);
    }
    let mut forks = view.forks.clone();
    if let Some(best) = view.best_fork.map(|b| b as usize).filter(|b| *b < forks.len()) {
        let fork = forks.remove(best);
        forks.insert(0, fork);
    }
    if forks.is_empty() {
        forks = vec![view.rows.iter().map(|r| r.index).collect()];
    }
    let mut marks: Vec<NotarizationMark> = Vec::new();
    let mut place: HashMap<u32, usize> = HashMap::new();
    let mut lane = 0;
    for fork in &forks {
        let mut previous: Option<usize> = None;
        let mut added = false;
        for index in fork {
            if let Some(&known) = place.get(index) {
                previous = Some(known);
                continue;
            }
            let Some(row) = by_index.get(index) else { continue };
            if let Some(at) = previous
                && marks[at].lane == lane
                && !marks[at].confirmed()
                && let Some(height) = other_height(row)
                && marks[at].newest().and_then(other_height) == Some(height)
            {
                marks[at].rows.push((*row).clone());
                place.insert(*index, at);
                continue;
            }
            marks.push(NotarizationMark {
                rows: vec![(*row).clone()],
                lane,
                column: previous.map(|p| marks[p].column + 1).unwrap_or(0),
                parent: previous.map(|p| marks[p].id()),
            });
            place.insert(*index, marks.len() - 1);
            previous = Some(marks.len() - 1);
            added = true;
        }
        if added || lane == 0 {
            lane += 1;
        }
    }
    // Rows that no fork has.
    for row in &view.rows {
        if !place.contains_key(&row.index) {
            marks.push(NotarizationMark { rows: vec![row.clone()], lane, column: 0, parent: None });
        }
    }
    marks
}

/// The rows in the order of the graph: each lane from the newest entry back.
pub fn ordered(marks: &[NotarizationMark]) -> Vec<(usize, Vec<BrowserNotarization>)> {
    let mut lanes: BTreeMap<usize, Vec<&NotarizationMark>> = BTreeMap::new();
    for mark in marks {
        lanes.entry(mark.lane).or_default().push(mark);
    }
    lanes
        .into_iter()
        .map(|(lane, mut group)| {
            group.sort_by(|a, b| b.column.cmp(&a.column));
            (lane, group.iter().flat_map(|m| m.rows.iter().rev().cloned()).collect())
        })
        .collect()
}

/// "5 later entries wait on the best chain. 4 entries are on a side branch."
pub fn waiting(marks: &[NotarizationMark]) -> String {
    let best = marks
        .iter()
        .filter(|m| m.lane == 0)
        .flat_map(|m| m.rows.iter())
        .filter(|r| !r.confirmed && !r.synthetic)
        .count();
    let side: usize = marks.iter().filter(|m| m.lane > 0).map(|m| m.rows.len()).sum();
    let mut parts = Vec::new();
    if best > 0 {
        parts.push(format!("{best} later {} on the best chain.", if best == 1 { "entry waits" } else { "entries wait" }));
    }
    if side > 0 {
        parts.push(format!("{side} {} on a side branch.", if side == 1 { "entry is" } else { "entries are" }));
    }
    if parts.is_empty() { "No later entry waits.".into() } else { parts.join(" ") }
}

/// The name of a lane, as its plate.
pub fn lane_title(lane: usize, lanes: usize) -> String {
    if lane == 0 {
        "Best chain".into()
    } else if lanes > 2 {
        format!("Side branch {lane}")
    } else {
        "Side branch".into()
    }
}

// MARK: Layout

/// The left edge of the scale of the rhythm and of the lanes of the graph, so that the two
/// parts are one instrument.
pub const GUTTER: f32 = 118.0;
pub const LANE_HEIGHT: f32 = 84.0;
pub const TOP: f32 = 72.0;

/// The middle of a lane. The labels of the best chain are above its marks; those of a side
/// branch are below.
pub fn lane_y(lane: usize) -> f32 {
    TOP + lane as f32 * LANE_HEIGHT
}

/// The height of the graph.
pub fn graph_height(lanes: usize) -> f32 {
    lane_y(lanes.max(1) - 1) + if lanes > 1 { 68.0 } else { 22.0 }
}

/// The room between two columns for a width of the graph.
pub fn step(width: f32, columns: usize) -> f32 {
    ((width - GUTTER - 90.0) / (columns.max(2) - 1) as f32).clamp(150.0, 260.0)
}

/// The least width of the graph: each column at the least step.
pub fn least_width(columns: usize) -> f32 {
    GUTTER + columns.saturating_sub(1) as f32 * 150.0 + 90.0
}

/// The centre of a mark. A mark of a side branch is a part of a step back, so that its line
/// is a curve.
pub fn point(mark: &NotarizationMark, step: f32) -> (f32, f32) {
    let back = if mark.lane > 0 { 0.3 } else { 0.0 };
    (GUTTER + (mark.column as f32 - back) * step, lane_y(mark.lane))
}

/// The scale of the rhythm: the number of powers of ten, and the place of a number of
/// blocks between `left` and `right`.
pub struct RhythmScale {
    pub left: f32,
    pub right: f32,
    pub decades: u32,
}

impl RhythmScale {
    pub fn new(width: f32, intervals: &[u64], wait: Option<u64>) -> Self {
        let top = intervals.iter().copied().max().unwrap_or(1).max(wait.unwrap_or(1)).max(100) as f64;
        let decades = (top.log10().ceil() as u32).max(2);
        Self { left: GUTTER, right: width - 84.0, decades }
    }

    pub fn x(&self, blocks: u64) -> f32 {
        self.left + ((blocks.max(1) as f64).log10() / self.decades as f64) as f32 * (self.right - self.left)
    }

    /// The dots of the intervals, in columns of 6, from the base up. A column holds `room`
    /// dots at most.
    pub fn dots(&self, intervals: &[u64], base: f32) -> Vec<(f32, f32)> {
        let column = 6.0_f32;
        let room = ((base - 26.0) / 5.6) as usize;
        let mut heights: HashMap<i64, usize> = HashMap::new();
        let mut dots = Vec::new();
        for interval in intervals {
            let slot = (self.x(*interval) / column).round() as i64;
            let level = *heights.get(&slot).unwrap_or(&0);
            heights.insert(slot, level + 1);
            if level >= room {
                continue;
            }
            dots.push((slot as f32 * column, base - 5.0 - level as f32 * 5.6));
        }
        dots
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::api::{BrowserLoad, NotarizationPerspective, ProofRootRow};

    fn record(heights: &[u64]) -> Vec<RecordPoint> {
        heights.iter().map(|h| RecordPoint { txid: format!("t{h}"), height: *h, time: None }).collect()
    }

    #[test]
    fn intervals_are_between_entries_in_sequence() {
        assert_eq!(intervals(&record(&[100, 90, 70, 70, 40])), vec![10, 20, 30]);
        assert!(intervals(&record(&[5])).is_empty());
        assert_eq!(median(&[30, 10, 20]), Some(20));
        assert_eq!(median(&[4, 1, 3, 2]), Some(3));
        assert_eq!(median(&[]), None);
    }

    #[test]
    fn link_state_rules() {
        let record = [10, 20, 30, 40, 50];
        assert_eq!(LinkState::new(Some(50), &record), Some(LinkState::Current));
        assert_eq!(LinkState::new(Some(51), &record), Some(LinkState::Late));
        assert_eq!(LinkState::new(Some(5000), &record), Some(LinkState::Late));
        assert_eq!(LinkState::new(Some(5001), &record), Some(LinkState::Stopped));
        assert_eq!(LinkState::new(Some(51), &record[..4]), None);
        assert_eq!(LinkState::new(None, &record), None);
        assert_eq!(LinkState::Stopped.word(), "Stopped");
        assert!(LinkState::Stopped.rule().contains("more than 100 times"));
    }

    #[test]
    fn spans_in_words() {
        assert_eq!(span(1), "1 minute");
        assert_eq!(span(0), "0 minutes");
        assert_eq!(span(119), "119 minutes");
        assert_eq!(span(120), "2 hours");
        assert_eq!(span(2879), "48 hours");
        assert_eq!(span(2880), "2 days");
        assert_eq!(span(1_051_199), "730 days");
        assert_eq!(span(1_051_200), "2 years");
        assert_eq!(span(1_314_000), "2.5 years");
    }

    fn root(system: &str, height: u64, foreign: bool) -> ProofRootRow {
        ProofRootRow {
            system_id: system.into(),
            system_name: None,
            height: Some(height),
            block_hash: None,
            foreign,
            check: None,
            status: None,
            checked_on: None,
            possible_fork: false,
            resolved: false,
        }
    }

    fn row(index: u32, observer: u64, other: u64, confirmed: bool) -> BrowserNotarization {
        BrowserNotarization {
            index,
            txid: Some(format!("tx{index}")),
            output: Some(0),
            block_hash: None,
            included_height: Some(observer),
            included_time: None,
            inclusion: String::new(),
            confirmed,
            synthetic: false,
            on_best_chain: true,
            roots: vec![root("iObserver", observer, false), root("iOther", other, true)],
            previous: None,
        }
    }

    fn view(rows: Vec<BrowserNotarization>, forks: Vec<Vec<u32>>, best: Option<u32>) -> NotarizationBrowser {
        NotarizationBrowser {
            perspective: NotarizationPerspective {
                observer_id: "iObserver".into(),
                observer_name: "VRSC".into(),
                subject_id: "iOther".into(),
                subject_name: "vETH".into(),
                local: true,
                configured: true,
            },
            load: BrowserLoad::Ready,
            message: None,
            rows,
            last_confirmed: Some(0),
            unconfirmed: 0,
            fork_count: 0,
            forks,
            best_fork: best,
            source: None,
            checks_limited: false,
            json: "{}".into(),
            provenance: None,
        }
    }

    #[test]
    fn the_best_chain_is_lane_zero_and_a_side_branch_leaves_it() {
        // Fork 1 is the best: 0 → 1 → 2 → 3. Fork 0 leaves at 1: 0 → 1 → 4.
        let rows = vec![row(0, 100, 10, true), row(1, 110, 11, false), row(2, 120, 12, false), row(3, 130, 12, false), row(4, 125, 13, false)];
        let marks = marks(&view(rows, vec![vec![0, 1, 4], vec![0, 1, 2, 3]], Some(1)));
        // Rows 2 and 3 commit to the same block of the other system: one mark.
        assert_eq!(marks.len(), 4);
        let lanes: Vec<(usize, usize, Option<u32>)> = marks.iter().map(|m| (m.lane, m.column, m.parent)).collect();
        assert_eq!(lanes, vec![(0, 0, None), (0, 1, Some(0)), (0, 2, Some(1)), (1, 2, Some(1))]);
        assert_eq!(marks[2].rows.len(), 2);
        assert!(marks[0].confirmed());
        assert_eq!(waiting(&marks), "3 later entries wait on the best chain. 1 entry is on a side branch.");
        let groups = ordered(&marks);
        assert_eq!(groups.len(), 2);
        let order: Vec<u32> = groups[0].1.iter().map(|r| r.index).collect();
        assert_eq!(order, vec![3, 2, 1, 0]);
    }

    #[test]
    fn a_view_without_forks_is_one_line_and_rows_outside_forks_get_a_lane() {
        let rows = vec![row(0, 100, 10, true), row(1, 110, 11, false)];
        let one = marks(&view(rows.clone(), vec![], None));
        assert_eq!(one.iter().map(|m| (m.lane, m.column)).collect::<Vec<_>>(), vec![(0, 0), (0, 1)]);
        let loose = marks(&view(rows, vec![vec![0]], None));
        assert_eq!(loose.iter().map(|m| (m.lane, m.column)).collect::<Vec<_>>(), vec![(0, 0), (1, 0)]);
        assert_eq!(waiting(&[]), "No later entry waits.");
    }

    #[test]
    fn graph_layout() {
        assert_eq!(lane_y(0), 72.0);
        assert_eq!(graph_height(1), 94.0);
        assert_eq!(graph_height(2), 224.0);
        assert_eq!(step(1000.0, 4), 260.0);
        assert_eq!(step(700.0, 4), 164.0);
        assert_eq!(step(300.0, 4), 150.0);
        assert_eq!(least_width(4), 658.0);
        let side = NotarizationMark { rows: vec![], lane: 1, column: 2, parent: None };
        assert_eq!(point(&side, 200.0), (118.0 + 1.7 * 200.0, 156.0));
        assert_eq!(lane_title(0, 3), "Best chain");
        assert_eq!(lane_title(2, 3), "Side branch 2");
        assert_eq!(lane_title(1, 2), "Side branch");
    }

    #[test]
    fn rhythm_scale_is_in_powers_of_ten() {
        let scale = RhythmScale::new(1000.0, &[50, 60], Some(5000));
        assert_eq!(scale.decades, 4);
        assert_eq!(scale.x(1), 118.0);
        assert!((scale.x(10_000) - 916.0).abs() < 0.01);
        let small = RhythmScale::new(1000.0, &[3], None);
        assert_eq!(small.decades, 2);
        let dots = small.dots(&[3, 3, 3], 90.0);
        assert_eq!(dots.len(), 3);
        assert_eq!(dots[0].0, dots[1].0);
        assert!(dots[1].1 < dots[0].1);
    }
}
