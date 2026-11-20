//! Fee pools are adjacent coinbase state checkpoints, never spendable UTXOs.
use super::*;
use anyhow::{Context, ensure};
use std::collections::BTreeMap;

fn checkpoints(tx: &Value) -> Option<Vec<(u32, BTreeMap<String, i128>)>> {
    let coinbase = tx["vin"]
        .as_array()?
        .iter()
        .any(|input| input.get("coinbase").is_some());
    if !coinbase {
        return None;
    }
    tx["vout"]
        .as_array()?
        .iter()
        .enumerate()
        .filter_map(|(index, out)| out["scriptPubKey"].get("feepool").map(|pool| (index, pool)))
        .map(|(index, pool)| {
            if pool["version"].as_u64() != Some(1) {
                return None;
            }
            let amounts = pool["currencyvalues"]
                .as_object()?
                .iter()
                .map(|(id, amount)| {
                    let amount = replay::amount_sats(amount)?;
                    (amount <= i128::from(i64::MAX)).then(|| (id.clone(), amount))
                })
                .collect::<Option<BTreeMap<_, _>>>()?;
            Some((u32::try_from(index).ok()?, amounts))
        })
        .collect()
}

pub(super) async fn inspect(configs: &[Config], root: &OutpointRef) -> Trail {
    let mut trail = Trail::default();
    trail.entries.push(entry(
        root,
        "Fee pool checkpoint",
        "Unspendable coinbase state",
    ));
    trail.notes.push("Fee pools carry protocol state across blocks. No UTXO spender exists for this checkpoint; successor coinbases do not spend it.".into());
    if let Err(error) = inspect_inner(configs, root, &mut trail).await {
        trail.stop = Some(Stop {
            message: error.to_string(),
        });
    }
    trail
}

async fn inspect_inner(
    configs: &[Config],
    root: &OutpointRef,
    trail: &mut Trail,
) -> anyhow::Result<()> {
    let config = configs
        .iter()
        .find(|c| c.chain.system_id.get() == Some(&root.system_id))
        .context("Fee pool system has no configured source")?;
    let client = RpcClient::for_config(&config.noninteractive())?;
    let response = client.get_raw_transaction(&root.txid).await?;
    let tx = &response.result;
    ensure!(
        tx["txid"].as_str() == Some(&root.txid)
            && tx["confirmations"].as_i64().is_some_and(|n| n > 0),
        "Confirmed coinbase unavailable"
    );
    let pools = checkpoints(tx).context("Invalid coinbase fee-pool data")?;
    ensure!(
        pools.len() == 1 && pools[0].0 == root.vout,
        "Expected one selected coinbase checkpoint"
    );
    let current = &pools[0].1;
    let hash = tx["blockhash"]
        .as_str()
        .context("Coinbase block hash absent")?;
    let header = client
        .call_with_raw::<Value>("getblockheader", &[json!(hash), json!(true)])
        .await?;
    ensure!(
        header.source_id == response.source_id && header.result["hash"].as_str() == Some(hash),
        "Checkpoint block/source mismatch"
    );
    let height = header.result["height"]
        .as_u64()
        .context("Checkpoint height absent")?;
    let canonical = client
        .call_with_raw::<String>("getblockhash", &[json!(height)])
        .await?;
    ensure!(
        canonical.source_id == response.source_id && canonical.result == hash,
        "Checkpoint is outside the active chain"
    );
    for (currency, amount) in current {
        trail.notes.push(format!("Checkpoint #{height}: {amount} satoshis of {currency} (protocol balance, not output value)"));
    }
    let previous_hash = header.result["previousblockhash"]
        .as_str()
        .context("No preceding block")?;
    let previous_block = client
        .call_with_raw::<Value>("getblock", &[json!(previous_hash), json!(1)])
        .await?;
    ensure!(
        previous_block.source_id == response.source_id
            && previous_block.result["hash"].as_str() == Some(previous_hash)
            && previous_block.result["height"]
                .as_u64()
                .and_then(|h| h.checked_add(1))
                == Some(height),
        "Preceding block/source mismatch"
    );
    let previous_id = previous_block.result["tx"]
        .as_array()
        .and_then(|txs| txs.first())
        .and_then(Value::as_str)
        .context("Preceding coinbase reference absent")?;
    let previous = client.get_raw_transaction(previous_id).await?;
    ensure!(
        previous.source_id == response.source_id
            && previous.result["txid"].as_str() == Some(previous_id)
            && previous.result["blockhash"].as_str() == Some(previous_hash),
        "Preceding coinbase/source mismatch"
    );
    let previous_pools =
        checkpoints(&previous.result).context("Preceding checkpoint data incomplete")?;
    ensure!(
        previous_pools.len() == 1,
        "Preceding fee pool unavailable; no zero balance is assumed"
    );
    let previous_ref = OutpointRef {
        system_id: root.system_id.clone(),
        txid: previous_id.into(),
        vout: previous_pools[0].0,
    };
    trail.entries.push(entry(
        &previous_ref,
        "Previous fee pool checkpoint",
        "Adjacent block's coinbase state; not a spent input",
    ));
    // Recheck membership after loading all dependencies, before publishing deltas.
    let checked = client
        .call_with_raw::<String>("getblockhash", &[json!(height)])
        .await?;
    ensure!(
        checked.source_id == response.source_id && checked.result == hash,
        "Chain changed during checkpoint comparison"
    );
    let previous_values = &previous_pools[0].1;
    let currencies: std::collections::BTreeSet<_> =
        current.keys().chain(previous_values.keys()).collect();
    for currency in currencies {
        // A valid checkpoint's omitted currency is a canonical map zero. Missing
        // or invalid checkpoints are rejected above instead of becoming zero.
        let before = previous_values.get(currency).copied().unwrap_or(0);
        let after = current.get(currency).copied().unwrap_or(0);
        trail.notes.push(format!(
            "Adjacent checkpoint delta: {:+} satoshis of {currency}; {before} -> {after}",
            after - before
        ));
    }
    trail.edges.push(LifecycleEdge {
        source: previous_ref,
        target: root.clone(),
        relation: "fee-pool-checkpoint-successor".into(),
        resolution: "structural".into(),
        detail: "Adjacent parent-linked coinbase checkpoints; no spend relationship".into(),
    });
    trail.notes.push("Delta = assessed fee inflow minus producer fee debit. Decoded transactions do not expose all descriptor-assessed fees, so neither gross inflow nor payout is inferred from this delta. An individual earlier fee cannot be assigned uniquely to a later producer.".into());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zero_is_known_but_missing_invalid_and_spendable_contexts_are_not_checkpoints() {
        let mut tx = json!({"vin":[{"coinbase":"00"}],"vout":[{"scriptPubKey":{
            "feepool":{"version":1,"currencyvalues":{"VRSC":0}}}}]});
        assert_eq!(checkpoints(&tx).unwrap()[0].1["VRSC"], 0);
        tx["vout"][0]["scriptPubKey"]["feepool"]["currencyvalues"] = json!(null);
        assert!(checkpoints(&tx).is_none());
        tx["vin"] = json!([{"txid":"not-coinbase","vout":0}]);
        assert!(checkpoints(&tx).is_none());
    }
}
