use super::{AppEvent, AppState, Config, IdentityVersionRecord, OutPoint, Screen, is_hash_like};
use crate::defi::{amount_sats, generated_region};
use crate::rpc::RpcClient;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

const TOKENIZED_CONTROL: u64 = 4;
const NFT_TOKEN: u64 = 0x800;
const LAUNCH_CLEAR_CONFIRMED: u64 = 8 | 16;
const LAUNCH_CLEAR_COMPLETE: u64 = LAUNCH_CLEAR_CONFIRMED | 32;
pub(super) const HOPS_PER_PASS: usize = 16;
const LAUNCH_LOOKUP_BLOCKS: u64 = 1000;
const RPC_TIMEOUT: Duration = Duration::from_secs(8);
const HOLDER_REFRESH: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub(super) struct Lookup {
    pub(super) request: Arc<()>,
    pub(super) status: Status,
    generation: u64,
    tip_height: Option<u64>,
    started: Instant,
}

#[derive(Clone, Debug)]
pub(super) enum Status {
    Loading,
    Paused(Frontier),
    Unresolved(Option<Frontier>, &'static str),
    Unspent(Frontier, Option<String>),
}

#[derive(Clone, Debug)]
pub(super) struct Frontier {
    source: String,
    current: OutPoint,
    visited: HashSet<OutPoint>,
    pub(super) hops: usize,
}

pub(super) fn enabled(version: &IdentityVersionRecord) -> bool {
    version.raw_json["identity"]["flags"]
        .as_u64()
        .is_some_and(|flags| flags & TOKENIZED_CONTROL != 0)
}

pub(super) fn request_current(
    state: &mut AppState,
    config: &Config,
    sender: &UnboundedSender<AppEvent>,
) -> bool {
    let Screen::Identity { target, .. } = &state.screen else {
        return false;
    };
    let Some(version) = state
        .store
        .identity_for_target(target)
        .filter(|v| enabled(v))
    else {
        return false;
    };
    let identity = version.identity_address.clone();
    request(state, config, sender, &identity, false)
}

pub(super) fn request(
    state: &mut AppState,
    config: &Config,
    sender: &UnboundedSender<AppEvent>,
    identity: &str,
    manual: bool,
) -> bool {
    if state.store.stale_identities.contains(identity) {
        return false;
    }
    let tip_height = state.tip.height;
    let generation = config.daemon.control.generation();
    let frontier = match state.store.token_control.get(identity) {
        Some(lookup) if lookup.generation == generation => match &lookup.status {
            Status::Paused(frontier) if manual => Some(frontier.clone()),
            Status::Unresolved(frontier, _) if manual => frontier.clone(),
            Status::Unspent(frontier, _)
                if !manual
                    && (tip_height != lookup.tip_height
                        || lookup.started.elapsed() >= HOLDER_REFRESH) =>
            {
                Some(frontier.clone())
            }
            _ => return false,
        },
        _ => None,
    };
    let mut daemon = config.daemon.clone();
    daemon.background = true;
    daemon.observations = None;
    let request = Arc::new(());
    state.store.token_control.insert(
        identity.to_owned(),
        Lookup {
            request: request.clone(),
            status: Status::Loading,
            generation,
            tip_height,
            started: Instant::now(),
        },
    );
    let identity = identity.to_owned();
    let sender = sender.clone();
    tokio::spawn(async move {
        let status = match RpcClient::new(daemon) {
            Ok(client) => match frontier {
                Some(frontier) => follow(&client, &identity, frontier).await,
                None => match origin(&client, &identity).await {
                    Ok(frontier) => follow(&client, &identity, frontier).await,
                    Err(message) => Status::Unresolved(None, message),
                },
            },
            Err(_) => Status::Unresolved(frontier, "Could not initialize the source connection"),
        };
        let _ = sender.send(AppEvent::TokenControl {
            identity,
            request,
            status,
        });
    });
    true
}

async fn rpc(client: &RpcClient, source: &str, method: &str, params: &[Value]) -> Option<Value> {
    tokio::time::timeout(RPC_TIMEOUT, client.call_on(source, method, params))
        .await
        .ok()?
        .ok()
}

async fn transaction(client: &RpcClient, source: &str, txid: &str) -> Option<Value> {
    let tx = rpc(
        client,
        source,
        "getrawtransaction",
        &[json!(txid), json!(1)],
    )
    .await?;
    (tx["txid"].as_str() == Some(txid)).then_some(tx)
}

fn indexed_output(tx: &Value, index: u64) -> Option<&Value> {
    let output = tx["vout"].get(usize::try_from(index).ok()?)?;
    (output["n"].as_u64() == Some(index)).then_some(output)
}

fn token_value<'a>(script: &'a Value, currency: &str) -> Option<&'a Value> {
    ["reserveoutput", "reservetransfer", "reservedeposit"]
        .into_iter()
        .find_map(|key| script[key]["currencyvalues"].get(currency))
}

fn has_control_token(script: &Value, currency: &str) -> bool {
    token_value(script, currency).and_then(amount_sats) == Some(1)
}

fn unique_token_output(outputs: &[Value], currency: &str) -> Option<u64> {
    let mut candidates = outputs.iter().filter(|out| {
        token_value(&out["scriptPubKey"], currency)
            .is_some_and(|value| amount_sats(value) != Some(0))
    });
    let candidate = candidates.next()?;
    if candidates.next().is_some() || !has_control_token(&candidate["scriptPubKey"], currency) {
        return None;
    }
    candidate["n"].as_u64()
}

fn holder(script: &Value, currency: &str) -> Option<String> {
    if amount_sats(&script["reserveoutput"]["currencyvalues"][currency]) != Some(1)
        || script["reqSigs"].as_u64() != Some(1)
    {
        return None;
    }
    let [address] = script["addresses"].as_array()?.as_slice() else {
        return None;
    };
    let address = address.as_str()?;
    let (prefix, _) = crate::defi::address_payload(address)?;
    matches!(prefix, 0 | 5 | 60 | 85 | 102).then(|| address.to_owned())
}

fn new_frontier(source: String, txid: &str, index: u64) -> Frontier {
    let current = OutPoint {
        txid: txid.into(),
        index,
    };
    Frontier {
        source,
        visited: HashSet::from([current.clone()]),
        current,
        hops: 0,
    }
}

fn launch_import(record: &Value, currency: &str) -> bool {
    let notarization = &record["importnotarization"];
    notarization["launchcleared"].as_bool() == Some(true)
        && notarization["launchconfirmed"].as_bool() == Some(true)
        && notarization["prelaunch"].as_bool() != Some(true)
        && notarization["refunding"].as_bool() != Some(true)
        && notarization["currencystate"]["currencyid"].as_str() == Some(currency)
        && matches!(
            notarization["currencystate"]["flags"].as_u64(),
            Some(LAUNCH_CLEAR_CONFIRMED | LAUNCH_CLEAR_COMPLETE)
        )
        && record["import"]["importcurrencyid"].as_str() == Some(currency)
}

async fn origin(client: &RpcClient, identity: &str) -> Result<Frontier, &'static str> {
    let response = tokio::time::timeout(
        RPC_TIMEOUT,
        client.call_with_raw::<Value>("getcurrency", &[json!(identity)]),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .ok_or("Currency definition unavailable")?;
    // Keep the definition, spend links, and UTXO check on the same source.
    let source = response.source_id;
    let definition = response.result;
    if definition["currencyid"].as_str() != Some(identity)
        || !definition["options"]
            .as_u64()
            .is_some_and(|f| f & NFT_TOKEN != 0)
    {
        return Err("Control token definition could not be verified");
    }
    let txid = definition["definitiontxid"]
        .as_str()
        .filter(|txid| is_hash_like(txid))
        .ok_or("Currency definition outpoint unavailable")?;
    let index = definition["definitiontxout"]
        .as_u64()
        .ok_or("Currency definition outpoint unavailable")?;
    let tx = transaction(client, &source, txid)
        .await
        .ok_or("Currency definition transaction unavailable")?;
    let defined = &indexed_output(&tx, index).ok_or("Currency definition output unavailable")?["scriptPubKey"]
        ["currencydefinition"];
    if defined["currencyid"].as_str() != Some(identity)
        || !defined["options"]
            .as_u64()
            .is_some_and(|f| f & NFT_TOKEN != 0)
    {
        return Err("Control token definition outpoint could not be verified");
    }
    if let Some(index) = tx["vout"]
        .as_array()
        .and_then(|outputs| unique_token_output(outputs, identity))
        && indexed_output(&tx, index).is_some()
    {
        return Ok(new_frontier(source, txid, index));
    }
    let definition_height = match tx["height"].as_u64() {
        Some(height) if height > 0 => height,
        _ => {
            let hash = tx["blockhash"]
                .as_str()
                .filter(|hash| is_hash_like(hash))
                .ok_or("Waiting for the currency definition to confirm")?;
            rpc(client, &source, "getblockheader", &[json!(hash)])
                .await
                .and_then(|header| header["height"].as_u64())
                .ok_or("Currency definition height unavailable")?
        }
    };
    let first = definition_height.max(
        definition["startblock"]
            .as_u64()
            .unwrap_or_default()
            .saturating_sub(1),
    );
    let best = rpc(client, &source, "getblockcount", &[])
        .await
        .and_then(|value| value.as_u64())
        .ok_or("Source chain height unavailable")?;
    if best < first {
        return Err("Waiting for the currency launch");
    }
    let last = first.saturating_add(LAUNCH_LOOKUP_BLOCKS - 1).min(best);
    let imports = rpc(
        client,
        &source,
        "getimports",
        &[json!(identity), json!(first), json!(last)],
    )
    .await
    .ok_or("Currency launch imports unavailable")?;
    let records = if imports.is_null() {
        &[][..]
    } else {
        imports
            .as_array()
            .ok_or("Currency launch imports unavailable")?
            .as_slice()
    };
    let mut candidates = records
        .iter()
        .filter(|record| launch_import(record, identity));
    let record = candidates
        .next()
        .ok_or("No confirmed control token launch found in the launch window")?;
    if candidates.next().is_some() {
        return Err("Control token launch is ambiguous");
    }
    let txid = record["importtxid"]
        .as_str()
        .filter(|txid| is_hash_like(txid))
        .ok_or("Currency launch outpoint unavailable")?;
    let index = record["importvout"]
        .as_u64()
        .and_then(|index| u32::try_from(index).ok())
        .ok_or("Currency launch outpoint unavailable")?;
    let tx = transaction(client, &source, txid)
        .await
        .ok_or("Currency launch transaction unavailable")?;
    let import = &indexed_output(&tx, index.into())
        .ok_or("Currency launch outpoint unavailable")?["scriptPubKey"]["crosschainimport"];
    if import != &record["import"] {
        return Err("Currency launch outpoint could not be verified");
    }
    let (_, outputs) =
        generated_region(&tx, import, index).ok_or("Currency launch outputs unavailable")?;
    let index = unique_token_output(outputs, identity)
        .ok_or("No unique control token output in the currency launch")?;
    let output = indexed_output(&tx, index).ok_or("Control token launch output unavailable")?;
    if !has_control_token(&output["scriptPubKey"], identity) {
        return Err("Control token launch output could not be verified");
    }
    Ok(new_frontier(source, txid, index))
}

fn verify_successor(
    tx: &Value,
    current: &OutPoint,
    input_index: Option<u64>,
    currency: &str,
) -> Option<OutPoint> {
    let mut matching = tx["vin"]
        .as_array()?
        .iter()
        .enumerate()
        .filter(|(_, input)| {
            input["txid"].as_str() == Some(current.txid.as_str())
                && input["vout"].as_u64() == Some(current.index)
        });
    let (index, _) = matching.next()?;
    if matching.next().is_some() || input_index.is_some_and(|expected| index as u64 != expected) {
        return None;
    }
    let next = unique_token_output(tx["vout"].as_array()?, currency)?;
    indexed_output(tx, next)?;
    Some(OutPoint {
        txid: tx["txid"].as_str()?.to_owned(),
        index: next,
    })
}

async fn unspent_or_successor(
    client: &RpcClient,
    currency: &str,
    frontier: &Frontier,
) -> Result<Step, &'static str> {
    let source = &frontier.source;
    let point = &frontier.current;
    let response = rpc(
        client,
        source,
        "gettxout",
        &[json!(point.txid), json!(point.index), json!(true)],
    )
    .await
    .ok_or("Source could not check the unspent output")?;
    if !response.is_null() {
        let script = &response["scriptPubKey"];
        if has_control_token(script, currency) {
            return Ok(Step::Unspent(holder(script, currency)));
        }
        // Some daemon versions omit decoded reserve data from gettxout.
        let tx = transaction(client, source, &point.txid)
            .await
            .ok_or("Unspent token transaction unavailable")?;
        let decoded = &indexed_output(&tx, point.index)
            .ok_or("Unspent token output unavailable")?["scriptPubKey"];
        if !has_control_token(decoded, currency)
            || script["hex"]
                .as_str()
                .is_none_or(|hex| hex.is_empty() || decoded["hex"].as_str() != Some(hex))
        {
            return Err("Unspent token output could not be verified");
        }
        return Ok(Step::Unspent(holder(decoded, currency)));
    }
    let tx = transaction(client, source, &point.txid).await;
    let metadata = tx.as_ref().and_then(|tx| indexed_output(tx, point.index));
    let spend =
        match metadata.and_then(|out| out["spentTxId"].as_str().filter(|id| is_hash_like(id))) {
            Some(txid) => (
                txid.to_owned(),
                metadata.and_then(|out| out["spentIndex"].as_u64()),
            ),
            None => {
                let indexed = rpc(
                    client,
                    source,
                    "getspentinfo",
                    &[json!({"txid": point.txid, "index": point.index})],
                )
                .await
                .ok_or("Source could not identify the spending transaction")?;
                let txid = indexed["txid"]
                    .as_str()
                    .filter(|txid| is_hash_like(txid))
                    .ok_or("Source could not identify the spending transaction")?;
                (txid.to_owned(), indexed["index"].as_u64())
            }
        };
    let tx = transaction(client, source, &spend.0)
        .await
        .ok_or("Spending transaction unavailable")?;
    let next = verify_successor(&tx, point, spend.1, currency)
        .ok_or("No unique on-chain continuation for this control token")?;
    Ok(Step::Spent(next))
}

enum Step {
    Unspent(Option<String>),
    Spent(OutPoint),
}

async fn follow(client: &RpcClient, currency: &str, mut frontier: Frontier) -> Status {
    for _ in 0..HOPS_PER_PASS {
        match unspent_or_successor(client, currency, &frontier).await {
            Ok(Step::Unspent(address)) => return Status::Unspent(frontier, address),
            Ok(Step::Spent(outpoint)) => {
                if !frontier.visited.insert(outpoint.clone()) {
                    return Status::Unresolved(
                        Some(frontier),
                        "Control token path contains a loop",
                    );
                }
                frontier.current = outpoint;
                frontier.hops += 1;
            }
            Err(message) => return Status::Unresolved(Some(frontier), message),
        }
    }
    Status::Paused(frontier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn successor_requires_the_exact_input_and_one_control_token_output() {
        let current = OutPoint {
            txid: "prior".into(),
            index: 2,
        };
        let mut tx = json!({
            "txid": "next",
            "vin": [{"txid":"prior","vout":1}, {"txid":"prior","vout":2}],
            "vout": [{"n":0, "scriptPubKey":{"reserveoutput":{"currencyvalues":{"token":0.00000001}}}}]
        });
        assert_eq!(
            verify_successor(&tx, &current, Some(1), "token"),
            Some(OutPoint {
                txid: "next".into(),
                index: 0
            })
        );
        assert!(verify_successor(&tx, &current, Some(0), "token").is_none());
        tx["vout"].as_array_mut().unwrap().push(json!({
            "n":1, "scriptPubKey":{"reservedeposit":{"currencyvalues":{"token":0.00000001}}}
        }));
        assert!(verify_successor(&tx, &current, Some(1), "token").is_none());
    }
}
