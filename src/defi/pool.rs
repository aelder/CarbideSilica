//! Explicit inspection of the importing state and its linked prior notarization.
//! Their difference is a pool-wide observation, never a per-transfer balance.
use super::*;
use anyhow::{Context, Result, ensure};

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub output: OutpointRef,
    pub height: Option<u64>,
    pub state: Value,
}

#[derive(Debug, Clone)]
pub struct Comparison {
    pub previous: Option<Snapshot>,
    pub imported: Snapshot,
    pub note: Option<String>,
}

pub fn previous_state(pool: &str, expected: &OutpointRef, transaction: &Value) -> Result<Snapshot> {
    ensure!(
        transaction["txid"].as_str() == Some(&expected.txid),
        "previous transaction ID differs from its reference"
    );
    ensure!(
        transaction["confirmations"]
            .as_i64()
            .is_none_or(|count| count > 0),
        "previous notarization is unconfirmed or outside the active chain"
    );
    let script = transaction["vout"]
        .as_array()
        .and_then(|outs| outs.get(expected.vout as usize))
        .and_then(|out| out.get("scriptPubKey"))
        .context("previous notarization output absent")?;
    let notarization = script
        .get("acceptednotarization")
        .or_else(|| script.get("earnednotarization"))
        .context("previous output is not a notarization")?;
    let state = &notarization["currencystate"];
    ensure!(
        notarization["currencyid"].as_str() == Some(pool)
            && state["currencyid"].as_str() == Some(pool)
            && state["version"].as_u64() == Some(1),
        "previous state belongs to another currency or unsupported version"
    );
    Ok(Snapshot {
        output: expected.clone(),
        height: transaction["height"].as_u64(),
        state: state.clone(),
    })
}

pub async fn inspect(configs: Vec<Config>, leg: LegSummary) -> Result<Comparison> {
    let execution = leg.execution.context("no importing state for this leg")?;
    let pool = leg.pool.context("no fractional pool for this leg")?;
    let mut result = Comparison {
        previous: None,
        imported: Snapshot {
            output: execution.import.clone(),
            height: execution.height,
            state: execution.state,
        },
        note: None,
    };
    let lookup = async {
        let previous = execution
            .previous
            .context("no linked previous notarization")?;
        ensure!(
            previous.txid != execution.import.txid,
            "previous notarization points back to this import"
        );
        let source = execution
            .source
            .context("import state source is not qualified")?;
        let config = configs
            .iter()
            .find(|config| config.chain.system_id.get() == Some(&previous.system_id))
            .context("no configured pool source")?;
        let client = RpcClient::for_config(&config.noninteractive())?;
        let response = client.get_raw_transaction(&previous.txid).await?;
        ensure!(
            response.source_id == source,
            "previous notarization came from a different source"
        );
        let snapshot = previous_state(&pool, &previous, &response.result)?;
        ensure!(
            !snapshot
                .height
                .zip(execution.height)
                .is_some_and(|(before, after)| before > after),
            "previous notarization is newer than this import"
        );
        Ok::<Snapshot, anyhow::Error>(snapshot)
    }
    .await;
    match lookup {
        Ok(snapshot) => result.previous = Some(snapshot),
        Err(error) => result.note = Some(format!("Previous state unavailable: {error}")),
    }
    Ok(result)
}

pub fn state_amount(value: &Value) -> Option<i128> {
    replay::amount_sats(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previous_notarization_requires_the_exact_outpoint_pool_and_supported_state() {
        let target = OutpointRef {
            system_id: crate::world::SystemId::try_from("iSystem".to_string()).unwrap(),
            txid: "a".repeat(64),
            vout: 1,
        };
        let tx = json!({"txid":target.txid,"height":10,"confirmations":1,"vout":[{},
            {"scriptPubKey":{"acceptednotarization":{"currencyid":"iPool","currencystate":{
                "version":1,"currencyid":"iPool","supply":100,"reservecurrencies":[]}}}}]});
        let snapshot = previous_state("iPool", &target, &tx).unwrap();
        assert_eq!(snapshot.height, Some(10));
        assert_eq!(snapshot.state["supply"], 100);
        for (path, value) in [
            ("/txid", json!("wrong")),
            ("/confirmations", json!(0)),
            (
                "/vout/1/scriptPubKey/acceptednotarization/currencyid",
                json!("iOther"),
            ),
            (
                "/vout/1/scriptPubKey/acceptednotarization/currencystate/currencyid",
                json!("iOther"),
            ),
            (
                "/vout/1/scriptPubKey/acceptednotarization/currencystate/version",
                json!(2),
            ),
        ] {
            let mut changed = tx.clone();
            *changed.pointer_mut(path).unwrap() = value;
            assert!(
                previous_state("iPool", &target, &changed).is_err(),
                "accepted {path}"
            );
        }
        assert!(previous_state("iPool", &OutpointRef { vout: 2, ..target }, &tx).is_err());
    }
}
