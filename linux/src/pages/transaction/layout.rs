//! What the statement shows (`StatementRow`, `StatementSelection` and `StatementLayout` in
//! `TransactionStatement.swift`).
//!
//! The bridge groups the inputs and outputs by party and calculates the net change. This
//! selects what the page shows: five receivers at most, five currencies at most. The chain
//! does not record which input paid which output, thus the statement has no line from one
//! to the other.

use std::collections::{BTreeSet, HashMap};

use crate::bridge::api::{ClosingKind, PartyKind, StatementAmount, StatementParty, TransactionStatement};

/// The inputs and outputs of one party, for the marks in the lists below the statement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub id: String,
    pub inputs: BTreeSet<u32>,
    pub outputs: BTreeSet<u32>,
}

impl Selection {
    pub fn of(row: &StatementRow) -> Self {
        Self { id: row.id.clone(), inputs: row.inputs.iter().copied().collect(), outputs: row.outputs.iter().copied().collect() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowStyle {
    Party,
    Hidden,
    Rest,
    Closing,
    Unchanged,
}

/// One line of the statement.
#[derive(Clone, Debug, PartialEq)]
pub struct StatementRow {
    pub id: String,
    pub title: String,
    /// The address or identity that the title opens.
    pub address: Option<String>,
    pub style: RowStyle,
    /// The net change in each shown column, in satoshis. Negative is given.
    pub cells: Vec<i64>,
    pub inputs: Vec<u32>,
    pub outputs: Vec<u32>,
}

impl StatementRow {
    pub fn selectable(&self) -> bool {
        !(self.inputs.is_empty() && self.outputs.is_empty())
    }
}

pub struct StatementLayout {
    pub columns: Vec<Option<String>>,
    pub rows: Vec<StatementRow>,
    /// The largest amount in each shown column. The bars of a column share this scale.
    pub scale: Vec<i64>,
    /// The net amounts of all receivers in the first column, largest first, when the
    /// statement joins some of them into one row.
    pub spread: Vec<i64>,
    /// The receivers that the statement joins into one row, largest first, one row each.
    pub joined: Vec<StatementRow>,
}

impl StatementLayout {
    pub const MAX_RECEIVERS: usize = crate::shared::statement_words::MAX_RECEIVERS;
    pub const KEPT_RECEIVERS: usize = 5;
    pub const MAX_COLUMNS: usize = 5;

    /// `only` is the one column to show; None shows the first five.
    pub fn new(statement: &TransactionStatement, only: Option<Option<String>>, title: impl Fn(&StatementParty) -> String) -> Self {
        let columns: Vec<Option<String>> = match only {
            Some(only) => vec![only],
            None => statement.columns.iter().take(Self::MAX_COLUMNS).cloned().collect(),
        };
        let cells = |amounts: &[StatementAmount]| -> Vec<i64> {
            columns
                .iter()
                .map(|column| amounts.iter().find(|a| &a.currency_id == column).map(|a| a.sats).unwrap_or(0))
                .collect()
        };
        let row = |party: &StatementParty, position: usize| StatementRow {
            id: format!("party-{position}"),
            title: title(party),
            address: if party.kind == PartyKind::Address { party.addresses.first().cloned() } else { None },
            style: if party.net.is_empty() {
                RowStyle::Unchanged
            } else if party.kind == PartyKind::ShieldedPool {
                RowStyle::Hidden
            } else {
                RowStyle::Party
            },
            cells: cells(&party.net),
            inputs: party.inputs.clone(),
            outputs: party.outputs.clone(),
        };

        let parties: Vec<(usize, &StatementParty)> = statement.parties.iter().enumerate().collect();
        let gives = |p: &StatementParty| p.net.iter().any(|a| a.sats < 0);
        let givers: Vec<(usize, &StatementParty)> = parties.iter().copied().filter(|(_, p)| gives(p)).collect();
        let unchanged: Vec<(usize, &StatementParty)> = parties.iter().copied().filter(|(_, p)| p.net.is_empty()).collect();
        let all_receivers: Vec<(usize, &StatementParty)> =
            parties.iter().copied().filter(|(i, p)| !p.net.is_empty() && !givers.iter().any(|(g, _)| g == i)).collect();

        // Only addresses are joined into one row. The reserve deposits and the shielded
        // pool keep their own rows.
        let others: Vec<_> = all_receivers.iter().copied().filter(|(_, p)| p.kind != PartyKind::Address).collect();
        let receivers: Vec<_> = all_receivers.iter().copied().filter(|(_, p)| p.kind == PartyKind::Address).collect();
        let mut rows: Vec<StatementRow> = givers.iter().map(|(i, p)| row(p, *i)).collect();
        let mut spread = Vec::new();
        let mut joined = Vec::new();
        if receivers.len() > Self::MAX_RECEIVERS {
            let first: Option<String> = columns.first().cloned().flatten();
            let amount = |p: &StatementParty| p.net.iter().find(|a| a.currency_id == first).map(|a| a.sats).unwrap_or(0);
            let mut sorted = receivers.clone();
            sorted.sort_by(|a, b| amount(b.1).cmp(&amount(a.1)));
            rows.extend(sorted.iter().take(Self::KEPT_RECEIVERS).map(|(i, p)| row(p, *i)));
            let rest = &sorted[Self::KEPT_RECEIVERS..];
            let mut sums: HashMap<Option<String>, i64> = HashMap::new();
            for (_, party) in rest {
                for a in &party.net {
                    *sums.entry(a.currency_id.clone()).or_default() += a.sats;
                }
            }
            rows.push(StatementRow {
                id: "rest".into(),
                title: format!("{} more addresses", rest.len()),
                address: None,
                style: RowStyle::Rest,
                cells: columns.iter().map(|c| sums.get(c).copied().unwrap_or(0)).collect(),
                inputs: rest.iter().flat_map(|(_, p)| p.inputs.iter().copied()).collect(),
                outputs: rest.iter().flat_map(|(_, p)| p.outputs.iter().copied()).collect(),
            });
            spread = sorted.iter().map(|(_, p)| amount(p)).filter(|a| *a > 0).collect();
            joined = rest.iter().map(|(i, p)| row(p, *i)).collect();
        } else {
            rows.extend(receivers.iter().map(|(i, p)| row(p, *i)));
        }
        rows.extend(others.iter().map(|(i, p)| row(p, *i)));
        if statement.closing_kind != ClosingKind::None {
            rows.push(StatementRow {
                id: "closing".into(),
                title: if statement.closing_kind == ClosingKind::Fee { "Fee".into() } else { "Inputs less outputs".into() },
                address: None,
                style: RowStyle::Closing,
                cells: cells(&statement.closing),
                inputs: vec![],
                outputs: vec![],
            });
        }
        rows.extend(unchanged.iter().map(|(i, p)| row(p, *i)));
        let scale = (0..columns.len())
            .map(|column| rows.iter().map(|r| r.cells[column].abs()).max().unwrap_or(1).max(1))
            .collect();
        // A row with no amount in the shown columns says nothing.
        let rows = rows.into_iter().filter(|r| r.style == RowStyle::Unchanged || r.cells.iter().any(|c| *c != 0)).collect();
        Self { columns, rows, scale, spread, joined }
    }
}

/// The figures under the spread bar (`spread` of `TransactionStatementView`).
#[derive(Debug, PartialEq)]
pub struct Spread {
    pub total: i64,
    pub median: i64,
    /// The share of the five largest, in whole percent.
    pub share: i64,
    /// The addresses that have a segment of their own, largest first.
    pub single: Vec<i64>,
    /// How many small addresses are joined in the hatched segment, and their total.
    pub small: usize,
    pub small_total: i64,
}

impl Spread {
    pub fn new(values: &[i64]) -> Self {
        let total: i64 = values.iter().sum();
        let mut sorted = values.to_vec();
        sorted.sort();
        let median = sorted.get(values.len() / 2).copied().unwrap_or(0);
        let top: i64 = values.iter().take(StatementLayout::KEPT_RECEIVERS).sum();
        // An address with less than 1 of each 400 gets less than two points of the bar.
        // Those addresses are one segment at the end, so that the bar stays clear.
        let single: Vec<i64> = values.iter().copied().filter(|v| v * 400 >= total).collect();
        let small = values.len() - single.len();
        let small_total = total - single.iter().sum::<i64>();
        let share = (top as f64 / total.max(1) as f64 * 100.0).round() as i64;
        Self { total, median, share, single, small, small_total }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::api::PartyKind;

    fn amount(sats: i64) -> StatementAmount {
        StatementAmount { currency_id: None, sats }
    }

    fn party(kind: PartyKind, address: &str, sats: i64, output: u32) -> StatementParty {
        StatementParty {
            kind,
            addresses: if address.is_empty() { vec![] } else { vec![address.to_string()] },
            currency_id: None,
            net: if sats == 0 { vec![] } else { vec![amount(sats)] },
            carried: vec![],
            inputs: if sats < 0 { vec![output] } else { vec![] },
            outputs: if sats > 0 { vec![output] } else { vec![] },
        }
    }

    fn payout(receivers: usize) -> TransactionStatement {
        let mut parties = vec![party(PartyKind::Address, "giver", -1_000_000_000, 0)];
        for i in 0..receivers {
            parties.push(party(PartyKind::Address, &format!("R{i}"), (i as i64 + 1) * 1_000_000, i as u32));
        }
        parties.push(party(PartyKind::ShieldedPool, "", 5_000, 99));
        parties.push(party(PartyKind::Address, "idle", 0, 100));
        TransactionStatement {
            columns: vec![None],
            parties,
            closing: vec![amount(4_000)],
            closing_kind: ClosingKind::Fee,
            marks: vec![],
            routes: vec![],
            complete: true,
        }
    }

    #[test]
    fn few_receivers_keep_their_rows() {
        let layout = StatementLayout::new(&payout(3), None, |p| p.addresses.first().cloned().unwrap_or("pool".into()));
        let ids: Vec<&str> = layout.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["party-0", "party-1", "party-2", "party-3", "party-4", "closing", "party-5"]);
        assert_eq!(layout.rows[4].style, RowStyle::Hidden);
        assert_eq!(layout.rows[6].style, RowStyle::Unchanged);
        assert_eq!(layout.scale, vec![1_000_000_000]);
        assert!(layout.spread.is_empty() && layout.joined.is_empty());
    }

    #[test]
    fn many_receivers_are_joined_largest_first() {
        let layout = StatementLayout::new(&payout(12), None, |_| String::new());
        // The giver, the five largest, the rest, the pool, the fee, the unchanged.
        assert_eq!(layout.rows.len(), 10);
        assert_eq!(layout.rows[1].cells, vec![12_000_000]);
        assert_eq!(layout.rows[5].cells, vec![8_000_000]);
        let rest = &layout.rows[6];
        assert_eq!(rest.id, "rest");
        assert_eq!(rest.title, "7 more addresses");
        assert_eq!(rest.style, RowStyle::Rest);
        assert_eq!(rest.cells, vec![(1..=7).map(|i| i * 1_000_000).sum::<i64>()]);
        assert_eq!(rest.outputs.len(), 7);
        assert_eq!(layout.spread.len(), 12);
        assert_eq!(layout.spread[0], 12_000_000);
        assert_eq!(layout.joined.len(), 7);
        assert_eq!(layout.joined[0].cells, vec![7_000_000]);
    }

    #[test]
    fn one_column_can_be_chosen() {
        let mut statement = payout(2);
        statement.columns.push(Some("iToken".into()));
        statement.parties[1].net.push(StatementAmount { currency_id: Some("iToken".into()), sats: 7 });
        let layout = StatementLayout::new(&statement, Some(Some("iToken".into())), |_| String::new());
        assert_eq!(layout.columns, vec![Some("iToken".to_string())]);
        // Only the row with an amount in that column, and the unchanged row, remain.
        assert_eq!(layout.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["party-1", "party-4"]);
        assert_eq!(layout.scale, vec![7]);
    }

    #[test]
    fn spread_groups_small_addresses() {
        let mut values = vec![500, 300, 100];
        values.extend(std::iter::repeat_n(1, 50));
        let spread = Spread::new(&values);
        assert_eq!(spread.total, 950);
        // 950 / 400 = 2.4: an address of 1 is below it and joins the hatched segment.
        assert_eq!(spread.single, vec![500, 300, 100]);
        assert_eq!(spread.small, 50);
        assert_eq!(spread.small_total, 50);
        assert_eq!(spread.median, 1);
        assert_eq!(spread.share, ((902.0 / 950.0) * 100.0f64).round() as i64);
    }
}
