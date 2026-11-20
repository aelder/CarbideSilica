//! Resolve typed chain-data fields for presentation, preserving the JSON tree.
//! Confirmation counts describe an inclusion, not a height or a duration clock.
use super::*;

fn key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn number(object: &Value, names: &[&str]) -> Option<u64> {
    object.as_object()?.iter().find_map(|(name, value)| {
        names
            .contains(&key(name).as_str())
            .then(|| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
            .flatten()
    })
}

fn inclusion(object: &Value, store: &ObjectStore) -> (Option<u64>, Option<u64>) {
    let mut height = number(object, &["height", "blockheight"]);
    let mut timestamp = number(object, &["blocktime"]);
    if object.get("hash").is_some() && object.get("txid").is_none() {
        timestamp = timestamp.or_else(|| number(object, &["time"]));
    }
    if let Some(block) = object
        .get("blockhash")
        .or_else(|| object.get("hash"))
        .and_then(Value::as_str)
        .and_then(|hash| store.blocks_by_hash.get(hash))
    {
        height = height.or(Some(block.summary.height));
        timestamp = timestamp.or(Some(block.summary.time));
    }
    if let Some(tx) = object
        .get("txid")
        .and_then(Value::as_str)
        .and_then(|tx| store.transaction(tx))
        .filter(|tx| {
            object["blockhash"]
                .as_str()
                .is_none_or(|hash| tx.block_hash.as_deref() == Some(hash))
        })
    {
        height = height.or(tx.block_height);
        if tx.block_hash.is_some() {
            timestamp = timestamp.or(Some(tx.block_time));
        }
    }
    // Height/time pairs also occur in compact header records without a hash.
    if height.is_some() && object.get("txid").is_none() {
        timestamp = timestamp.or_else(|| number(object, &["time"]));
    }
    (height, timestamp.filter(|t| *t > 0))
}

fn date(theme: &Theme, time: u64) -> Option<String> {
    chrono::DateTime::from_timestamp(i64::try_from(time).ok()?, 0)?;
    Some(theme.format_time(time))
}

pub(super) fn display(
    theme: &Theme,
    root: &Value,
    pointer: &str,
    store: &ObjectStore,
) -> Option<String> {
    if !theme.heights.borrow().dates {
        return None;
    }
    if pointer
        .split('/')
        .any(|part| matches!(key(part).as_str(), "contentmap" | "contentmultimap"))
    {
        return None; // Identity user content is not a protocol field schema.
    }
    let value = root.pointer(pointer)?;
    let value = value.as_u64().or_else(|| value.as_str()?.parse().ok())?;
    let (parent, name) = pointer.rsplit_once('/')?;
    let name = key(&name.replace("~1", "/").replace("~0", "~"));
    let object = root.pointer(parent)?;
    let (own_height, own_time) = inclusion(object, store);
    let mut system = "";
    let height = match name.as_str() {
        "time"
        | "timestamp"
        | "ntime"
        | "blocktime"
        | "mediantime"
        | "tiptime"
        | "nextblocktime"
        | "lastblocktime"
        | "firstblocktime"
        | "creationtimestamp"
        | "registrationtimestamp"
        | "notarizedtime"
        | "timereceived"
        | "receivedtime"
        | "creationtime"
        | "registrationtime"
        | "firstseen"
        | "lastseen"
        | "lastsend"
        | "lastrecv"
        | "conntime" => return date(theme, value),
        "confirmations" => {
            if value == 0 {
                return None;
            }
            if let Some(t) = own_time {
                return date(theme, t);
            }
            // Never subtract a retained confirmation count from today's tip:
            // that would silently move a historical inclusion's date forward.
            own_height?
        }
        "locktime" if value >= 500_000_000 => return date(theme, value),
        "locktime"
        | "expiryheight"
        | "unlockheight"
        | "launchheight"
        | "definitionheight"
        | "startheight"
        | "endheight"
        | "fromheight"
        | "toheight"
        | "proofheight"
        | "lastconfirmedheight"
        | "anchorheight" => {
            if value == 0 {
                return None;
            }
            value
        }
        "timelock" => {
            if value == 0
                || object["version"].as_u64().is_none_or(|v| v < 2)
                || object["flags"]
                    .as_u64()
                    .is_none_or(|flags| flags & 2 != 0 || flags & 0x8000 != 0)
            {
                return None;
            }
            value
        }
        "startblock" | "endblock" => {
            if value == 0 {
                return None;
            }
            system = object[if name == "startblock" {
                "launchsystemid"
            } else {
                "systemid"
            }]
            .as_str()?;
            value
        }
        "sourceheight" | "sourceheightstart" | "sourceheightend" => {
            system = object["sourcesystemid"].as_str()?;
            value
        }
        "notarizationheight" => {
            let currency = object["currencyid"].as_str()?;
            system = store.currencies_by_id.get(currency)?.definition_json["systemid"].as_str()?;
            value
        }
        "height" | "blockheight" => value,
        "blocks" | "longestchain"
            if object.get("chainid").is_some() || object.get("bestblockhash").is_some() =>
        {
            value
        }
        _ => return None,
    };
    if system.is_empty()
        && own_height == Some(height)
        && let Some(time) = own_time
    {
        return date(theme, time);
    }
    if system.is_empty()
        && own_height == Some(height)
        && own_time.is_none()
        && (object["hash"].is_string() || object["blockhash"].is_string())
    {
        // An explicit inclusion hash can identify another branch. A timestamp
        // fetched solely by canonical height would not date that inclusion.
        return None;
    }
    Some(theme.format_height_on(system, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn block_and_transaction_dates_use_inclusion_evidence_without_rpc_or_mutation() {
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        theme.heights.borrow_mut().dates = true;
        let store = ObjectStore::default();
        let root = json!({"height":123,"time":1700000000,"confirmations":42,"amount":1700000000,"expiryheight":999});
        for path in ["/height", "/time", "/confirmations"] {
            assert_eq!(
                display(&theme, &root, path, &store).as_deref(),
                Some("2023-11-14T22:13:20Z")
            );
        }
        assert!(theme.heights.borrow().pending.is_empty());
        assert!(display(&theme, &root, "/amount", &store).is_none());
        assert!(
            display(&theme, &root, "/expiryheight", &store)
                .unwrap()
                .contains("loading date")
        );
        assert_eq!(root["height"], 123);
        assert_eq!(root["confirmations"], 42);
        assert_eq!(json_yank_text(&root["time"]), "1700000000");
        theme.heights.borrow_mut().dates = false;
        assert!(display(&theme, &root, "/height", &store).is_none());
    }
    #[test]
    fn counts_sentinels_and_ambiguous_confirmations_never_become_invented_dates() {
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        theme.heights.borrow_mut().dates = true;
        let store = ObjectStore::default();
        let root = json!({"confirmations":10,"expiryheight":0,"locktime":0,"blocktime":0,
            "identity":{"version":3,"flags":2,"timelock":100},"maxpreconvert":1800000000});
        for path in [
            "/confirmations",
            "/expiryheight",
            "/locktime",
            "/identity/timelock",
            "/maxpreconvert",
        ] {
            assert!(display(&theme, &root, path, &store).is_none(), "{path}");
        }
        assert!(theme.heights.borrow().pending.is_empty());
        let unknown_branch = json!({"height":123,"hash":"uncached-inclusion","confirmations":10});
        assert!(display(&theme, &unknown_branch, "/height", &store).is_none());
        assert!(display(&theme, &unknown_branch, "/confirmations", &store).is_none());
        let tx = json!({"confirmations":10,"blocktime":1700000000,"time":1800000000});
        assert_eq!(
            display(&theme, &tx, "/confirmations", &store).unwrap(),
            "2023-11-14T22:13:20Z"
        );
    }

    #[test]
    fn foreign_heights_and_identity_user_content_do_not_borrow_local_dates() {
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        {
            let mut d = theme.heights.borrow_mut();
            d.dates = true;
            d.scope = "local:revision".into();
            d.timestamps.insert(12, Some(1000));
            d.foreign
                .entry("remote".into())
                .or_default()
                .borrow_mut()
                .timestamps
                .insert(12, Some(2000));
        }
        let root = json!({"height":12,"time":1000,"sourcesystemid":"remote","sourceheight":12,
            "contentmap":{"height":12,"timestamp":1700000000}});
        let store = ObjectStore::default();
        assert_eq!(
            display(&theme, &root, "/sourceheight", &store).unwrap(),
            "1970-01-01T00:33:20Z"
        );
        assert!(display(&theme, &root, "/contentmap/height", &store).is_none());
        assert!(display(&theme, &root, "/contentmap/timestamp", &store).is_none());
        assert!(theme.heights.borrow().pending.is_empty());
    }
}
