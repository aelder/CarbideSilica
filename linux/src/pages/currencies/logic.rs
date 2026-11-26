//! The pure parts of the currency browser: the figures of the console, the order of the
//! table and the clicks on a column name (`CurrenciesView.apply`, `heading`, `count`).

use std::cmp::Ordering;

use crate::bridge::api::{CurrencyListing, LaunchState};

use super::state::CurrencySort;

/// "27 of 57", or a dash before the list comes.
pub fn shown_text(visible: usize, rows: usize) -> String {
    if rows == 0 { "–".into() } else { format!("{visible} of {rows}") }
}

/// The number of currencies in a launch state, or a dash before the list comes.
pub fn launch_count(rows: &[CurrencyListing], state: LaunchState) -> String {
    if rows.is_empty() { "–".into() } else { rows.iter().filter(|r| r.launch == state).count().to_string() }
}

/// The number of fractional currencies: option bit 1.
pub fn fractional_count(rows: &[CurrencyListing]) -> String {
    if rows.is_empty() {
        "–".into()
    } else {
        rows.iter().filter(|r| r.options.unwrap_or(0) & 1 != 0).count().to_string()
    }
}

/// A click on a column name. The first click puts the list in the order of the column, a
/// second click turns the order, and a third click gives the order of the source again.
/// Supply, start and reserves begin with the largest.
pub fn heading_click(sort: Option<CurrencySort>, descending: bool, column: CurrencySort) -> (Option<CurrencySort>, bool) {
    if sort != Some(column) {
        let first = matches!(column, CurrencySort::Supply | CurrencySort::Start | CurrencySort::Reserves);
        (Some(column), first)
    } else if descending == (column == CurrencySort::Name) {
        (None, descending)
    } else {
        (sort, !descending)
    }
}

/// Puts the positions of the matching rows in the order of a column. Rows that are equal
/// in the column keep the order of the source.
pub fn order(rows: &[CurrencyListing], mut found: Vec<usize>, sort: Option<CurrencySort>, descending: bool) -> Vec<usize> {
    let Some(sort) = sort else { return found };
    found.sort_by(|&left, &right| {
        let (a, b) = (&rows[left], &rows[right]);
        let ordering = match sort {
            CurrencySort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            CurrencySort::Reserves => a.reserve_ids.len().cmp(&b.reserve_ids.len()),
            CurrencySort::Supply => {
                a.supply.unwrap_or(-1.0).partial_cmp(&b.supply.unwrap_or(-1.0)).unwrap_or(Ordering::Equal)
            }
            CurrencySort::Start => a.start_block.unwrap_or(0).cmp(&b.start_block.unwrap_or(0)),
        };
        match ordering {
            Ordering::Equal => left.cmp(&right),
            other if descending => other.reverse(),
            other => other,
        }
    });
    found
}

/// The reserves in words: each reserve's name, else the definition's name, else the ID.
pub fn reserves_text(row: &CurrencyListing, name: impl Fn(&str, Option<&str>) -> String) -> String {
    if row.reserve_ids.is_empty() {
        return "No reserve".into();
    }
    row.reserve_ids
        .iter()
        .enumerate()
        .map(|(index, id)| name(id, row.reserve_names.get(index).map(String::as_str)))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, supply: Option<f64>, start: Option<u64>, reserves: usize, launch: LaunchState, options: Option<u64>) -> CurrencyListing {
        CurrencyListing {
            name: name.into(),
            currency_id: format!("i{name}"),
            options,
            option_tags: vec![],
            launch,
            launch_label: String::new(),
            start_block: start,
            supply,
            reserve_ids: (0..reserves).map(|i| format!("r{i}")).collect(),
            reserve_names: vec![],
        }
    }

    fn sample() -> Vec<CurrencyListing> {
        vec![
            row("beta", Some(10.0), Some(5), 2, LaunchState::Launched, Some(33)),
            row("Alpha", None, None, 0, LaunchState::Prelaunch, None),
            row("gamma", Some(30.0), Some(1), 3, LaunchState::Failed, Some(32)),
            row("delta", Some(10.0), Some(9), 2, LaunchState::Launched, Some(1)),
        ]
    }

    #[test]
    fn figures_of_the_console() {
        let rows = sample();
        assert_eq!(shown_text(2, rows.len()), "2 of 4");
        assert_eq!(shown_text(0, 0), "–");
        assert_eq!(launch_count(&rows, LaunchState::Launched), "2");
        assert_eq!(launch_count(&rows, LaunchState::Unknown), "0");
        assert_eq!(launch_count(&[], LaunchState::Failed), "–");
        assert_eq!(fractional_count(&rows), "2");
    }

    #[test]
    fn a_column_name_cycles_through_three_orders() {
        // Name: ascending, descending, source.
        let (sort, down) = heading_click(None, false, CurrencySort::Name);
        assert_eq!((sort, down), (Some(CurrencySort::Name), false));
        let (sort, down) = heading_click(sort, down, CurrencySort::Name);
        assert_eq!((sort, down), (Some(CurrencySort::Name), true));
        let (sort, _) = heading_click(sort, down, CurrencySort::Name);
        assert_eq!(sort, None);
        // Supply: descending, ascending, source.
        let (sort, down) = heading_click(None, false, CurrencySort::Supply);
        assert_eq!((sort, down), (Some(CurrencySort::Supply), true));
        let (sort, down) = heading_click(sort, down, CurrencySort::Supply);
        assert_eq!((sort, down), (Some(CurrencySort::Supply), false));
        let (sort, _) = heading_click(sort, down, CurrencySort::Supply);
        assert_eq!(sort, None);
        // Another column starts again.
        let (sort, down) = heading_click(Some(CurrencySort::Name), true, CurrencySort::Start);
        assert_eq!((sort, down), (Some(CurrencySort::Start), true));
    }

    #[test]
    fn order_of_the_table() {
        let rows = sample();
        let all = vec![0, 1, 2, 3];
        assert_eq!(order(&rows, all.clone(), None, false), all);
        assert_eq!(order(&rows, all.clone(), Some(CurrencySort::Name), false), vec![1, 0, 3, 2]);
        assert_eq!(order(&rows, all.clone(), Some(CurrencySort::Name), true), vec![2, 3, 0, 1]);
        // Equal supplies keep the order of the source, in either direction.
        assert_eq!(order(&rows, all.clone(), Some(CurrencySort::Supply), true), vec![2, 0, 3, 1]);
        assert_eq!(order(&rows, all.clone(), Some(CurrencySort::Supply), false), vec![1, 0, 3, 2]);
        assert_eq!(order(&rows, all.clone(), Some(CurrencySort::Start), true), vec![3, 0, 2, 1]);
        assert_eq!(order(&rows, all, Some(CurrencySort::Reserves), true), vec![2, 0, 3, 1]);
        assert_eq!(order(&rows, vec![3, 1], Some(CurrencySort::Name), false), vec![1, 3]);
    }

    #[test]
    fn reserves_in_words() {
        let mut currency = row("x", None, None, 2, LaunchState::Launched, None);
        currency.reserve_names = vec!["VRSC".into()];
        let text = reserves_text(&currency, |id, fallback| fallback.map(str::to_string).unwrap_or_else(|| id.to_uppercase()));
        assert_eq!(text, "VRSC, R1");
        assert_eq!(reserves_text(&row("y", None, None, 0, LaunchState::Launched, None), |id, _| id.into()), "No reserve");
    }
}
