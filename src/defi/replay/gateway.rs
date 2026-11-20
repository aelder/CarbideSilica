//! A narrow local dropout: plain settled fractional transfer, already-native
//! PBaaS next-leg fees below its immutable import threshold. No gas estimate.
use super::*;
use std::collections::BTreeMap;

pub(super) fn fallback(
    transfer: &Value,
    output: &Value,
    import: &Value,
    notarization: &Value,
    system: &str,
    definitions: &BTreeMap<String, Value>,
    definition: &Value,
) -> Option<Option<i128>> {
    let pool = import["importcurrencyid"].as_str()?;
    let state = &notarization["currencystate"];
    let destination = &transfer["destination"];
    let gateway = destination["gateway"].as_str()?;
    let next = definitions.get(gateway)?;
    if definition["version"].as_u64() != Some(1)
        || definition["currencyid"].as_str() != Some(pool)
        || definition["systemid"].as_str() != Some(system)
        || definition["options"].as_u64() != Some(33)
        || !definition["currencies"]
            .as_array()?
            .iter()
            .any(|value| value.as_str() == Some(gateway))
        || !state["currencies"].as_object()?.contains_key(gateway)
        || transfer["version"].as_u64() != Some(1)
        || transfer["flags"].as_u64() != Some(1)
        || transfer["convert"].as_bool() == Some(true)
        || transfer["refund"].as_bool() == Some(true)
        || transfer.get("exportto").is_some()
        || import["sourcesystemid"].as_str() != Some(system)
        || transfer["destinationcurrencyid"].as_str() != Some(pool)
        || state["currencyid"].as_str() != Some(pool)
        || state["flags"].as_u64() != Some(49)
        || destination["type"]
            .as_u64()
            .is_none_or(|kind| kind & 128 == 0)
        || gateway == system
        || transfer["feecurrencyid"].as_str() != Some(gateway)
        || next["version"].as_u64() != Some(1)
        || next["currencyid"].as_str() != Some(gateway)
        || next["systemid"].as_str() != Some(gateway)
        || next["options"].as_u64() != Some(256)
        || next["proofprotocol"].as_u64() != Some(1)
    {
        return None;
    }
    let budget = amount_sats(&destination["fees"])?;
    let threshold = amount_sats(&next["transactionimportfee"])?;
    if !(1..=i128::from(i64::MAX)).contains(&budget)
        || !(1..=i128::from(i64::MAX)).contains(&threshold)
        || budget >= threshold
    {
        return None;
    }
    // If export eligibility fails, GetTxOut drops out before constructing the
    // next transfer. If eligible, this insufficient fee forces the same local
    // payout. Both branches recover the identical unchanged gateway budget.
    let mut values = amount_vector(&transfer["currencyvalues"])?;
    if values.len() != 1 || values.values().any(|amount| *amount > i128::from(i64::MAX)) {
        return None;
    }
    *values.entry(gateway.to_owned()).or_default() = values
        .get(gateway)
        .copied()
        .unwrap_or(0)
        .checked_add(budget)?;
    if values.values().any(|amount| *amount > i128::from(i64::MAX)) {
        return None;
    }
    let native = values.remove(system).unwrap_or(0);
    if output["valueSat"].as_i64().map(i128::from) != Some(native) {
        return None;
    }
    let destination = refund_destination(destination)?;
    if values.is_empty() {
        return None;
    } // current subset always retains a nonnative fee budget
    let mut decoded = output.clone();
    decoded["valueSat"] = Value::from(0);
    if !reserve_output_matches(
        &serde_json::json!({"destination":destination}),
        &decoded,
        &values,
    ) {
        return None;
    }
    let script = &output["scriptPubKey"];
    if script["type"].as_str() != Some("cryptocondition")
        || script["reqSigs"].as_u64() != Some(1)
        || script["spendableoutput"].as_bool() != Some(true)
        || script["reserveoutput"]["version"].as_u64() != Some(1)
    {
        return None;
    }
    let address = destination["address"].as_str()?;
    let prefix = match destination["type"].as_u64()? {
        2 => format!(
            "1a040300010114{}cc",
            payload_hex(&address_payload(address)?.1)
        ),
        4 => format!("1b04030001011504{}cc", identity_hash(address)?),
        _ => return None,
    };
    if !script["hex"]
        .as_str()?
        .to_ascii_lowercase()
        .starts_with(&prefix)
    {
        return None;
    }
    Some((native > 0).then_some(native))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn insufficient_pbaas_budget_returns_native_principal_and_reserve_fee_custody() {
        let system = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        let pool = "i3f7tSctFkiPpiedY8QR5Tep9p4qDVebDx";
        let gateway = "iHog9UCTrn95qpUBFCZ7kKz7qWdMA8MQ6N";
        let address = "RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp";
        let mut transfer = json!({"version":1,"flags":1,"currencyvalues":{system:1},
            "destinationcurrencyid":pool,"feecurrencyid":gateway,
            "destination":{"type":130,"address":address,"gateway":gateway,"fees":0.0001}});
        let import = json!({"sourcesystemid":system,"importcurrencyid":pool});
        let state = json!({"launchcomplete":true,"launchconfirmed":true,
            "currencystate":{"version":1,"flags":49,"currencyid":pool,"currencies":{gateway:{"lastconversionprice":1}}}});
        let definition = json!({"version":1,"currencyid":pool,"systemid":system,
            "options":33,"currencies":[gateway]});
        let definitions = BTreeMap::from([(
            gateway.into(),
            json!({"version":1,"currencyid":gateway,
            "systemid":gateway,"options":256,"proofprotocol":1,"transactionimportfee":0.0002}),
        )]);
        let mut output = json!({"valueSat":100000000,"scriptPubKey":{"type":"cryptocondition",
            "reqSigs":1,"spendableoutput":true,"addresses":[address],
            "hex":format!("1a040300010114{}cc",payload_hex(&address_payload(address).unwrap().1)),
            "reserveoutput":{"version":1,"currencyvalues":{gateway:0.0001}}}});
        let run = |transfer: &Value, output: &Value, defs: &BTreeMap<String, Value>| {
            replay_with_evidence(
                std::slice::from_ref(transfer),
                0,
                &import,
                &state,
                system,
                std::slice::from_ref(output),
                Evidence::Gateways(defs, &definition),
            )
        };
        let proof = run(&transfer, &output, &definitions).unwrap();
        assert_eq!(proof.kind, ReplayKind::GatewayFallback);
        assert!(!proof.kind.is_forwarded());
        output["valueSat"] = json!(100000001);
        assert!(run(&transfer, &output, &definitions).is_none());
        output["valueSat"] = json!(100000000);
        transfer["destination"]["fees"] = json!(0.0002);
        output["scriptPubKey"]["reserveoutput"]["currencyvalues"][gateway] = json!(0.0002);
        assert!(
            run(&transfer, &output, &definitions).is_none(),
            "equality permits a continuation; no fallback proof"
        );
        transfer["destination"]["fees"] = json!(0.0001);
        let mut eth = definitions.clone();
        eth.get_mut(gateway).unwrap()["proofprotocol"] = json!(3);
        assert!(
            run(&transfer, &output, &eth).is_none(),
            "historical Ethereum gas is required"
        );
    }
}
