//! Helpers on the bridge's `Route`: the titles of the path bar and of the Recent list.

use std::collections::HashMap;

use crate::bridge::api::Route;
use crate::format;

pub fn title(route: &Route) -> String {
    match route {
        Route::Block { id } => {
            if id.chars().count() > 12 {
                format!("Block {}", format::short(id, 6, 6))
            } else {
                format!("Block {id}")
            }
        }
        Route::Transaction { txid } => format!("Tx {}", format::short(txid, 6, 6)),
        Route::Identity { name } => format::short(name, 12, 6),
        Route::Currency { name } => format::short(name, 12, 6),
        Route::Address { address } => format::short(address, 8, 6),
        Route::Currencies => "Currencies".into(),
        Route::Estimate { query } => {
            if query.starts_with("unix:") {
                "Estimate for a date".into()
            } else {
                format!("Estimate for block {query}")
            }
        }
    }
}

/// Only objects that Carbide's bookmark file can hold.
pub fn is_bookmarkable(route: &Route) -> bool {
    matches!(
        route,
        Route::Block { .. }
            | Route::Transaction { .. }
            | Route::Identity { .. }
            | Route::Currency { .. }
            | Route::Address { .. }
    )
}

/// A short label for the path line. `names` is empty when names are turned off.
pub fn crumb_title(route: &Route, names: &HashMap<String, String>) -> String {
    match route {
        Route::Identity { name } => names.get(name).cloned().unwrap_or_else(|| title(route)),
        Route::Currency { name } => names
            .get(name)
            .map(|found| found.strip_suffix('@').unwrap_or(found).to_string())
            .unwrap_or_else(|| title(route)),
        _ => title(route),
    }
}

/// The route for an address-like string: identities are i-addresses or names.
pub fn holder(value: &str) -> Route {
    if value.ends_with('@') || (value.starts_with('i') && value.chars().count() == 34) {
        Route::Identity { name: value.to_string() }
    } else {
        Route::Address { address: value.to_string() }
    }
}

/// The kind and target that Carbide's bookmark file stores for a route.
pub fn bookmark_target(route: &Route) -> Option<(&'static str, String)> {
    match route {
        Route::Block { id } => Some(("block", id.clone())),
        Route::Transaction { txid } => Some(("transaction", txid.clone())),
        Route::Identity { name } => Some(("identity", name.clone())),
        Route::Currency { name } => Some(("currency", name.clone())),
        Route::Address { address } => Some(("address", address.clone())),
        Route::Currencies | Route::Estimate { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_follow_the_swift_app() {
        assert_eq!(title(&Route::Block { id: "4339576".into() }), "Block 4339576");
        assert_eq!(title(&Route::Block { id: "0000000012345678abcdef".into() }), "Block 000000…abcdef");
        assert_eq!(title(&Route::Estimate { query: "unix:1857254400".into() }), "Estimate for a date");
        let names: HashMap<String, String> = [("iABC".to_string(), "vETH@".to_string())].into();
        assert_eq!(crumb_title(&Route::Currency { name: "iABC".into() }, &names), "vETH");
        assert!(matches!(holder("alice@"), Route::Identity { .. }));
        assert!(matches!(holder("RAddress"), Route::Address { .. }));
    }
}
