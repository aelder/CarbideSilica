//! The geometry of the statement as a flow (`StatementFlow.Node`, `StatementFlow.Scene` and
//! the levels of `StatementFlow` in `TransactionStatement.swift`): the nodes of each side,
//! the levels into the joined addresses, and which band is under a point.

use super::layout::{RowStyle, StatementLayout, StatementRow};

pub const BAR: f64 = 4.0;
pub const CORE: f64 = 9.0;
const BAND_SPACE: f64 = 210.0;
const MIN_THICK: f64 = 1.5;
const MIN_SLOT: f64 = 22.0;
pub const GAP: f64 = 5.0;
pub const STEP: usize = StatementLayout::KEPT_RECEIVERS;

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub row: StatementRow,
    pub sats: i64,
    /// The thickness of the band.
    pub thick: f64,
    /// The top of the place of the node, and the height of the place.
    pub top: f64,
    pub slot: f64,
}

impl Node {
    pub fn middle(&self) -> f64 {
        self.top + self.slot / 2.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub left: Vec<Node>,
    pub right: Vec<Node>,
    pub height: f64,
}

impl Default for Scene {
    fn default() -> Self {
        Self { left: vec![], right: vec![], height: 60.0 }
    }
}

/// The curve of a band from its party (0) to the middle (1): the x of a cubic with its
/// control points at the half distance, as a part of the distance.
fn curve_x(t: f64) -> f64 {
    1.5 * t * (1.0 - t) * (1.0 - t) + 1.5 * t * t * (1.0 - t) + t * t * t
}

impl Scene {
    /// The node of the group that opens to the next level.
    pub fn group(&self) -> Option<&Node> {
        self.right.iter().rev().find(|n| n.row.style == RowStyle::Rest)
    }

    pub fn core(&self) -> (f64, f64) {
        let left: f64 = self.left.iter().map(|n| n.thick).sum();
        let right: f64 = self.right.iter().map(|n| n.thick).sum();
        let height = left.max(right);
        ((self.height - height).max(0.0) / 2.0, height)
    }

    /// The top of each band where it meets the bar in the middle.
    pub fn inner(&self, nodes: &[Node]) -> Vec<f64> {
        let (core_top, core_height) = self.core();
        let total: f64 = nodes.iter().map(|n| n.thick).sum();
        let mut y = core_top + (core_height - total) / 2.0;
        nodes
            .iter()
            .map(|node| {
                let at = y;
                y += node.thick;
                at
            })
            .collect()
    }

    /// The band at a point of the picture: the band itself, not the row of its name.
    pub fn band(&self, x: f64, y: f64, width: f64) -> Option<&Node> {
        let mid = width / 2.0;
        let gave = x < mid;
        let nodes = if gave { &self.left } else { &self.right };
        let outer = if gave { BAR } else { width - BAR };
        let inner_x = if gave { mid - CORE / 2.0 } else { mid + CORE / 2.0 };
        // How far along the band the point is, from its party (0) to the middle (1).
        let along = ((x - outer) / (inner_x - outer)).clamp(0.0, 1.0);
        // The band is a curve with its control points at the half distance: find the
        // parameter of the curve for this distance.
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..18 {
            let t = (low + high) / 2.0;
            if curve_x(t) < along {
                low = t;
            } else {
                high = t;
            }
        }
        let t = (low + high) / 2.0;
        let ease = 3.0 * t * t - 2.0 * t * t * t;
        let mut best: Option<(&Node, f64)> = None;
        for (node, end) in nodes.iter().zip(self.inner(nodes)) {
            let start = node.middle() - node.thick / 2.0;
            let top = start + (end - start) * ease;
            // A thin band takes the pointer from a little above and below it.
            let reach = ((6.0 - node.thick) / 2.0).max(0.0);
            if y < top - reach || y > top + node.thick + reach {
                continue;
            }
            let distance = (y - (top + node.thick / 2.0)).abs();
            if best.is_none_or(|(_, d)| distance < d) {
                best = Some((node, distance));
            }
        }
        best.map(|(node, _)| node)
    }

    /// The node whose row of names holds `y`, on one side.
    pub fn row_at(&self, left: bool, y: f64) -> Option<&Node> {
        let side = if left { &self.left } else { &self.right };
        side.iter().find(|n| y >= n.top && y <= n.top + n.slot)
    }
}

/// The levels of one flow: level 0 is the transaction; each level after it takes the next
/// addresses from the joined ones.
pub struct Levels<'a> {
    pub layout: &'a StatementLayout,
    pub column: usize,
}

impl Levels<'_> {
    pub fn max_depth(&self) -> usize {
        if self.layout.joined.is_empty() { 0 } else { (self.layout.joined.len() - 1) / STEP + 1 }
    }

    /// The nodes of one side, from the top.
    fn nodes(&self, rows: Vec<StatementRow>, scale: f64) -> Vec<Node> {
        let mut top = 0.0;
        rows.into_iter()
            .map(|row| {
                let sats = row.cells[self.column].abs();
                let thick = (sats as f64 * scale).max(MIN_THICK);
                let slot = thick.max(MIN_SLOT);
                let node = Node { row, sats, thick, top, slot };
                top += slot + GAP;
                node
            })
            .collect()
    }

    /// One row for a group of joined addresses.
    fn group(&self, rows: &[StatementRow], id: &str) -> StatementRow {
        StatementRow {
            id: id.to_string(),
            title: format!("{} more addresses", rows.len()),
            address: None,
            style: RowStyle::Rest,
            cells: (0..self.layout.columns.len()).map(|index| rows.iter().map(|r| r.cells[index]).sum()).collect(),
            inputs: rows.iter().flat_map(|r| r.inputs.iter().copied()).collect(),
            outputs: rows.iter().flat_map(|r| r.outputs.iter().copied()).collect(),
        }
    }

    /// The picture of a level.
    pub fn scene(&self, level: usize) -> Scene {
        let column = self.column;
        let (gave, received): (Vec<StatementRow>, Vec<StatementRow>) = if level == 0 {
            let rows: Vec<&StatementRow> =
                self.layout.rows.iter().filter(|r| r.style != RowStyle::Unchanged && r.cells[column] != 0).collect();
            (
                rows.iter().filter(|r| r.cells[column] < 0).map(|r| (*r).clone()).collect(),
                rows.iter().filter(|r| r.cells[column] > 0).map(|r| (*r).clone()).collect(),
            )
        } else {
            let joined = &self.layout.joined;
            let from = ((level - 1) * STEP).min(joined.len());
            let gave = vec![self.group(&joined[from..], &format!("group-{level}"))];
            let end = (from + STEP).min(joined.len());
            let mut received: Vec<StatementRow> = joined[from..end].to_vec();
            if end < joined.len() {
                received.push(self.group(&joined[end..], "rest"));
            }
            (gave, received.into_iter().filter(|r| r.cells[column] != 0).collect())
        };
        let total = gave
            .iter()
            .map(|r| r.cells[column].abs())
            .sum::<i64>()
            .max(received.iter().map(|r| r.cells[column].abs()).sum::<i64>())
            .max(1);
        let scale = BAND_SPACE / total as f64;
        let left = self.nodes(gave, scale);
        let right = self.nodes(received, scale);
        let height = left
            .last()
            .map(|n| n.top + n.slot)
            .unwrap_or(0.0)
            .max(right.last().map(|n| n.top + n.slot).unwrap_or(0.0))
            .max(60.0);
        Scene { left, right, height }
    }

    /// "Level 1 of 3: addresses 6 to 10 by amount."
    pub fn place(&self, level: usize) -> String {
        let first = StatementLayout::KEPT_RECEIVERS + (level - 1) * STEP + 1;
        let last = (first + STEP - 1).min(StatementLayout::KEPT_RECEIVERS + self.layout.joined.len());
        format!("Level {level} of {}: addresses {first} to {last} by amount.", self.max_depth())
    }

    pub fn caption(&self, level: usize, scene: &Scene) -> String {
        let mut text = if level == 0 { "Click a band to mark its inputs and outputs in the lists.".to_string() } else { self.place(level) };
        if scene.group().is_some() {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str("Click the \"more addresses\" band to open that group.");
        }
        text
    }
}

/// What a click on a band or its name does.
#[derive(Debug, PartialEq, Eq)]
pub enum Choice {
    /// Go to a level.
    Level(usize),
    /// Mark the row, or remove the mark.
    Mark,
    Nothing,
}

/// The band of the joined addresses opens the next level. The band of the group on the
/// left goes back.
pub fn choice(row: &StatementRow, level: usize, joined: bool) -> Choice {
    if row.id == "rest" && joined {
        return Choice::Level(level + 1);
    }
    if row.id.starts_with("group-") {
        return Choice::Level(level.saturating_sub(1));
    }
    if !row.selectable() {
        return Choice::Nothing;
    }
    Choice::Mark
}

/// True for a row whose band does something on a click.
pub fn acts(row: &StatementRow) -> bool {
    row.id == "rest" || row.id.starts_with("group-") || row.selectable()
}

/// The ease of the move between levels, as SwiftUI's `easeInOut`: a cubic Bézier with the
/// control points (0.42, 0) and (0.58, 1).
pub fn ease_in_out(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    let bezier = |t: f64, a: f64, b: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * a + 3.0 * (1.0 - t) * t * t * b + t * t * t;
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..24 {
        let t = (low + high) / 2.0;
        if bezier(t, 0.42, 0.58) < x {
            low = t;
        } else {
            high = t;
        }
    }
    bezier((low + high) / 2.0, 0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, sats: i64, style: RowStyle) -> StatementRow {
        StatementRow {
            id: id.into(),
            title: id.into(),
            address: None,
            style,
            cells: vec![sats],
            inputs: if sats < 0 { vec![0] } else { vec![] },
            outputs: if sats > 0 { vec![1] } else { vec![] },
        }
    }

    fn layout(joined: usize) -> StatementLayout {
        let mut rows = vec![row("giver", -1_000, RowStyle::Party)];
        rows.push(row("a", 400, RowStyle::Party));
        rows.push(row("rest", 590, RowStyle::Rest));
        rows.push(row("closing", 10, RowStyle::Closing));
        rows.push(row("idle", 0, RowStyle::Unchanged));
        StatementLayout {
            columns: vec![None],
            rows,
            scale: vec![1_000],
            spread: vec![],
            joined: (0..joined).map(|i| row(&format!("party-{i}"), 100 - i as i64, RowStyle::Party)).collect(),
        }
    }

    #[test]
    fn the_first_level_has_givers_and_receivers() {
        let l = layout(12);
        let levels = Levels { layout: &l, column: 0 };
        let scene = levels.scene(0);
        assert_eq!(scene.left.len(), 1);
        assert_eq!(scene.right.iter().map(|n| n.row.id.as_str()).collect::<Vec<_>>(), ["a", "rest", "closing"]);
        // The scale: the larger side is 210 high in all.
        assert!((scene.left[0].thick - 210.0).abs() < 1e-9);
        assert!((scene.right.iter().map(|n| n.thick).sum::<f64>() - 210.0).abs() < 1e-9);
        // A thin band has the least thickness and the least slot.
        assert!((scene.right[2].thick - 2.1).abs() < 1e-9);
        assert_eq!(scene.right[2].slot, 22.0);
        assert_eq!(scene.right[1].top, scene.right[0].slot + GAP);
        assert_eq!(scene.group().map(|n| n.row.id.as_str()), Some("rest"));
        assert_eq!(levels.max_depth(), 3);
    }

    #[test]
    fn deeper_levels_take_the_next_addresses() {
        let l = layout(12);
        let levels = Levels { layout: &l, column: 0 };
        let one = levels.scene(1);
        assert_eq!(one.left[0].row.id, "group-1");
        assert_eq!(one.left[0].row.title, "12 more addresses");
        assert_eq!(one.right.len(), 6);
        assert_eq!(one.right[5].row.title, "7 more addresses");
        let three = levels.scene(3);
        assert_eq!(three.left[0].row.title, "2 more addresses");
        assert_eq!(three.right.len(), 2);
        assert!(three.group().is_none());
        assert_eq!(levels.place(1), "Level 1 of 3: addresses 6 to 10 by amount.");
        assert_eq!(levels.place(3), "Level 3 of 3: addresses 16 to 17 by amount.");
        assert_eq!(
            levels.caption(1, &one),
            "Level 1 of 3: addresses 6 to 10 by amount. Click the \"more addresses\" band to open that group."
        );
        assert_eq!(levels.caption(3, &three), "Level 3 of 3: addresses 16 to 17 by amount.");
    }

    #[test]
    fn hit_testing_follows_the_curve() {
        let l = layout(0);
        let levels = Levels { layout: &l, column: 0 };
        let scene = levels.scene(0);
        let width = 400.0;
        let right = &scene.right;
        let inner = scene.inner(right);
        // At the party end, a band is where its node is.
        let first = &right[0];
        let at_party = scene.band(width - BAR - 0.5, first.middle(), width).unwrap();
        assert_eq!(at_party.row.id, "a");
        // At the middle, the band is where it meets the bar.
        let x = width / 2.0 + CORE / 2.0 + 0.1;
        let y = inner[1] + right[1].thick / 2.0;
        assert_eq!(scene.band(x, y, width).unwrap().row.id, "rest");
        // The thin band of the fee takes the pointer from a little above it.
        let fee = &right[2];
        let near = scene.band(width - BAR - 0.5, fee.middle() - fee.thick / 2.0 - 1.5, width);
        assert_eq!(near.map(|n| n.row.id.as_str()), Some("closing"));
        // Between the bands at the party end there is nothing.
        let between = right[0].top + right[0].slot + GAP / 2.0;
        assert!(between > right[0].middle() + right[0].thick / 2.0 + 2.0);
        assert!(scene.band(width - BAR - 0.5, between, width).is_none_or(|n| n.row.id != "a"));
        // On the left, the giver's band.
        assert_eq!(scene.band(BAR + 1.0, scene.left[0].middle(), width).unwrap().row.id, "giver");
        // The rows of the names.
        assert_eq!(scene.row_at(false, right[2].top + 1.0).unwrap().row.id, "closing");
        assert!(scene.row_at(false, right[0].top + right[0].slot + 1.0).is_none());
    }

    #[test]
    fn clicks_choose_levels_or_marks() {
        let rest = row("rest", 5, RowStyle::Rest);
        assert_eq!(choice(&rest, 0, true), Choice::Level(1));
        assert_eq!(choice(&row("group-2", 5, RowStyle::Rest), 2, true), Choice::Level(1));
        assert_eq!(choice(&row("party-1", 5, RowStyle::Party), 0, true), Choice::Mark);
        let mut closing = row("closing", 5, RowStyle::Closing);
        closing.outputs.clear();
        assert_eq!(choice(&closing, 0, true), Choice::Nothing);
        assert!(!acts(&closing));
    }

    #[test]
    fn the_ease_starts_and_ends_slowly() {
        assert!(ease_in_out(0.0).abs() < 1e-6);
        assert!((ease_in_out(1.0) - 1.0).abs() < 1e-6);
        assert!((ease_in_out(0.5) - 0.5).abs() < 1e-4);
        assert!(ease_in_out(0.1) < 0.1);
        assert!(ease_in_out(0.9) > 0.9);
    }
}
