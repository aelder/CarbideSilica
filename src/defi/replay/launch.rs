//! Non-final same-system prelaunch receipts.
//! See the launch replay fixtures for supported cases.
use super::*;

pub(super) fn phase(
    import: &Value,
    notarization: &Value,
    export: &Value,
    definition: &Value,
    system: &str,
) -> Option<bool> {
    let currency = import["importcurrencyid"].as_str()?;
    let state = &notarization["currencystate"];
    let options = definition["options"].as_u64()?;
    if currency == system
        || import["version"].as_u64() != Some(1)
        || !matches!(import["flags"].as_u64(), Some(8 | 24))
        || import["sourcesystemid"].as_str() != Some(system)
        || export["version"].as_u64() != Some(1)
        || !matches!(export["flags"].as_u64(), Some(1 | 5))
        || export["sourcesystemid"].as_str() != Some(system)
        || export["destinationsystemid"].as_str() != Some(system)
        || export["destinationcurrencyid"].as_str() != Some(currency)
        || notarization["version"].as_u64() != Some(2)
        || notarization["currencyid"].as_str() != Some(currency)
        || notarization["prelaunch"].as_bool() != Some(true)
        || [
            "launchcleared",
            "launchconfirmed",
            "launchcomplete",
            "refunding",
            "isdefinition",
            "isblockonenotarization",
        ]
        .iter()
        .any(|field| notarization[*field].as_bool() == Some(true))
        || state["version"].as_u64() != Some(1)
        || !matches!(state["flags"].as_u64(), Some(2 | 3))
        || state["currencyid"].as_str() != Some(currency)
        || definition["version"].as_u64() != Some(1)
        || definition["currencyid"].as_str() != Some(currency)
        || definition["systemid"].as_str() != Some(system)
        || definition["launchsystemid"].as_str() != Some(system)
        || options & 0x20 == 0
        || options & !0x21 != 0
        || (options & 1 != 0) != (state["flags"].as_u64()? & 1 != 0)
        || [
            "prelaunchcarveout",
            "prelaunchdiscount",
            "gatewayconverterissuance",
        ]
        .iter()
        .any(|field| {
            definition
                .get(*field)
                .is_some_and(|value| amount_sats(value) != Some(0))
        })
        || definition
            .get("maxpreconversion")
            .is_some_and(|value| value.as_array().is_none_or(|values| !values.is_empty()))
    {
        return None;
    }
    Some(options & 1 != 0)
}

fn output_matches(
    output: &Value,
    currency: &str,
    amount: i128,
    destination: &Value,
    system: &str,
) -> bool {
    if currency == system {
        output["valueSat"].as_i64().map(i128::from) == Some(amount)
            && match destination["type"].as_u64() {
                Some(4) => native_identity_refund_matches(destination, output),
                Some(2) => p2pkh_script(destination["address"].as_str().unwrap_or("")).is_some_and(
                    |expected| {
                        output["scriptPubKey"]["type"].as_str() == Some("pubkeyhash")
                            && output["scriptPubKey"]["hex"]
                                .as_str()
                                .is_some_and(|hex| hex.eq_ignore_ascii_case(&expected))
                    },
                ),
                _ => false,
            }
    } else {
        let transfer = serde_json::json!({"destination":destination});
        if !reserve_output_matches(
            &transfer,
            output,
            &std::collections::BTreeMap::from([(currency.to_owned(), amount)]),
        ) {
            return false;
        }
        if destination["type"].as_u64() == Some(4) {
            let Some(hash) = destination["address"].as_str().and_then(identity_hash) else {
                return false;
            };
            let script = &output["scriptPubKey"];
            script["type"].as_str() == Some("cryptocondition")
                && script["reqSigs"].as_u64() == Some(1)
                && script["spendableoutput"].as_bool() == Some(true)
                && script["reserveoutput"]["version"].as_u64() == Some(1)
                && script["hex"].as_str().is_some_and(|hex| {
                    hex.to_ascii_lowercase()
                        .starts_with(&format!("1b04030001011504{hash}cc"))
                })
        } else {
            destination["type"].as_u64() == Some(2)
        }
    }
}

pub(super) fn receipt(
    transfer: &Value,
    context: (&Value, &Value, &Value),
    fractional: bool,
    system: &str,
    outputs: &[Value],
    offset: usize,
) -> Option<(ReplayProof, usize)> {
    let (import, notarization, definition) = context;
    let currency = import["importcurrencyid"].as_str()?;
    let destination = &transfer["destination"];
    if transfer["version"].as_u64() != Some(1)
        || transfer["flags"].as_u64() != Some(7)
        || transfer["destinationcurrencyid"].as_str() != Some(currency)
        || transfer["refund"].as_bool() == Some(true)
        || transfer.get("exportto").is_some()
        || !matches!(destination["type"].as_u64(), Some(2 | 4))
        || destination.get("gateway").is_some()
        || destination.get("auxdests").is_some()
    {
        return None;
    }
    let values = amount_vector(&transfer["currencyvalues"])?;
    if values.len() != 1 {
        return None;
    }
    let (source, amount) = values.iter().next()?;
    if source == currency
        || *amount > i128::from(i64::MAX)
        || !definition["currencies"]
            .as_array()?
            .iter()
            .any(|value| value.as_str() == Some(source))
        || (!fractional && source != system)
    {
        return None;
    }
    let net = amount.checked_sub(conversion_fee(*amount, 1)?)?;
    let price =
        amount_sats(&notarization["currencystate"]["currencies"][source]["lastconversionprice"])?;
    if !(1..=i128::from(i64::MAX)).contains(&price) {
        return None;
    }
    let principal = net.checked_mul(SATOSHI)?.checked_div(price)?;
    if !(1..=i128::from(i64::MAX)).contains(&principal) {
        return None;
    }
    let proceeds = if fractional {
        None
    } else {
        let destination = serde_json::json!({"type":4,"address":currency});
        if !output_matches(outputs.get(offset)?, source, net, &destination, system) {
            return None;
        }
        Some(offset)
    };
    let offset = offset.checked_add(usize::from(proceeds.is_some()))?;
    if !output_matches(
        outputs.get(offset)?,
        currency,
        principal,
        destination,
        system,
    ) {
        return None;
    }
    Some((
        ReplayProof {
            offset,
            proceeds,
            satoshis: (currency == system).then_some(principal),
            kind: ReplayKind::Preconversion,
        },
        offset + 1,
    ))
}

/// Empty clear-launch batches emit configured preallocations before fees. This
/// branch proves that whole ordered vector, never a matching individual address.
pub(super) fn allocations(
    import: &Value,
    notarization: &Value,
    export: &Value,
    definition: &Value,
    transfers: &[Value],
    system: &str,
    outputs: &[Value],
) -> Option<Vec<(usize, String, i128)>> {
    let currency = import["importcurrencyid"].as_str()?;
    if currency == system
        || !transfers.is_empty()
        || export["numinputs"].as_u64() != Some(0)
        || import["version"].as_u64() != Some(1)
        || !matches!(import["flags"].as_u64(), Some(12 | 14 | 28 | 30))
        || import["sourcesystemid"].as_str() != Some(system)
        || export["version"].as_u64() != Some(1)
        || !matches!(export["flags"].as_u64(), Some(3 | 7))
        || export["sourcesystemid"].as_str() != Some(system)
        || export["destinationsystemid"].as_str() != Some(system)
        || export["destinationcurrencyid"].as_str() != Some(currency)
        || notarization["version"].as_u64() != Some(2)
        || notarization["currencyid"].as_str() != Some(currency)
        || notarization["launchcleared"].as_bool() != Some(true)
        || notarization["launchconfirmed"].as_bool() != Some(true)
        || [
            "prelaunch",
            "refunding",
            "isdefinition",
            "isblockonenotarization",
        ]
        .iter()
        .any(|field| notarization[*field].as_bool() == Some(true))
        || notarization["currencystate"]["version"].as_u64() != Some(1)
        || notarization["currencystate"]["currencyid"].as_str() != Some(currency)
        || !matches!(
            notarization["currencystate"]["flags"].as_u64(),
            Some(24 | 56)
        )
        || definition["version"].as_u64() != Some(1)
        || definition["currencyid"].as_str() != Some(currency)
        || definition["systemid"].as_str() != Some(system)
        || definition["launchsystemid"].as_str() != Some(system)
        || definition["options"].as_u64() != Some(32)
        || [
            "prelaunchcarveout",
            "prelaunchdiscount",
            "gatewayconverterissuance",
        ]
        .iter()
        .any(|field| {
            definition
                .get(*field)
                .is_some_and(|value| amount_sats(value) != Some(0))
        })
        || definition
            .get("maxpreconversion")
            .is_some_and(|value| value.as_array().is_none_or(|values| !values.is_empty()))
    {
        return None;
    }
    let allocations = definition["preallocations"].as_array()?;
    if allocations.is_empty() || allocations.len() > 128 {
        return None;
    }
    allocations
        .iter()
        .enumerate()
        .map(|(offset, allocation)| {
            let allocation = allocation.as_object()?;
            if allocation.len() != 1 {
                return None;
            }
            let (recipient, value) = allocation.iter().next()?;
            identity_hash(recipient)?; // excludes the exporter-dependent blockoneminer sentinel
            let amount = amount_sats(value)?;
            if !(1..=i128::from(i64::MAX)).contains(&amount) {
                return None;
            }
            let destination = serde_json::json!({"type":4,"address":recipient});
            output_matches(outputs.get(offset)?, currency, amount, &destination, system)
                .then(|| (offset, recipient.clone(), amount))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const SYSTEM: &str = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
    const CURRENCY: &str = "i3f7tSctFkiPpiedY8QR5Tep9p4qDVebDx";
    const ADDRESS: &str = "RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp";

    // Source-derived cases using currency IDs, not captured successful launches.
    fn context(fractional: bool) -> (Value, Value, Value, Value, Vec<Value>, Vec<Value>) {
        let definition = json!({"version":1,"currencyid":CURRENCY,"systemid":SYSTEM,
            "launchsystemid":SYSTEM,"options":if fractional {33} else {32},
            "currencies":[SYSTEM]});
        let import =
            json!({"version":1,"flags":8,"sourcesystemid":SYSTEM,"importcurrencyid":CURRENCY});
        let export = json!({"version":1,"flags":1,"sourcesystemid":SYSTEM,
            "destinationsystemid":SYSTEM,"destinationcurrencyid":CURRENCY});
        let state = json!({"version":2,"currencyid":CURRENCY,"prelaunch":true,"currencystate":{
            "version":1,"currencyid":CURRENCY,"flags":if fractional {3} else {2},
            "currencies":{SYSTEM:{"lastconversionprice":2}}}});
        let transfer = json!({"version":1,"flags":7,"currencyvalues":{SYSTEM:100},
            "destinationcurrencyid":CURRENCY,"destination":{"type":2,"address":ADDRESS}});
        let principal = json!({"valueSat":0,"scriptPubKey":{"addresses":[ADDRESS],
            "reserveoutput":{"version":1,"currencyvalues":{CURRENCY:49.9875}}}});
        let hash = identity_hash(CURRENCY).unwrap();
        let proceeds = json!({"valueSat":9997500000_i64,"scriptPubKey":{
            "type":"cryptocondition","evalnone":"valid","reqSigs":1,
            "spendableoutput":true,"addresses":[CURRENCY],
            "hex":format!("050403000000cc1b04030001011504{hash}75")}});
        let outputs = if fractional {
            vec![principal.clone(), principal]
        } else {
            vec![proceeds.clone(), principal.clone(), proceeds, principal]
        };
        (
            definition,
            import,
            export,
            state,
            vec![transfer.clone(), transfer],
            outputs,
        )
    }

    #[test]
    fn nonfractional_preconversion_consumes_proceeds_then_principal_for_each_member() {
        let (def, import, export, state, batch, mut outputs) = context(false);
        let run = |outputs: &[Value]| {
            replay_with_launch(
                &batch,
                1,
                &import,
                &state,
                SYSTEM,
                outputs,
                Some((&def, &export)),
            )
        };
        let proof = run(&outputs).unwrap();
        assert_eq!((proof.proceeds, proof.offset), (Some(2), 3));
        assert_eq!(proof.kind, ReplayKind::Preconversion);
        outputs[0]["valueSat"] = json!(9997500001_i64);
        assert!(
            run(&outputs).is_none(),
            "matching later outputs cannot rescue altered proceeds"
        );
        outputs[0]["valueSat"] = json!(9997500000_i64);
        outputs.swap(0, 1);
        assert!(
            run(&outputs).is_none(),
            "output order is consensus-significant"
        );
    }

    #[test]
    fn fractional_preconversion_has_no_per_customer_proceeds_output() {
        let (def, import, export, mut state, batch, outputs) = context(true);
        let proof = replay_with_launch(
            &batch,
            1,
            &import,
            &state,
            SYSTEM,
            &outputs,
            Some((&def, &export)),
        )
        .unwrap();
        assert_eq!((proof.proceeds, proof.offset), (None, 1));
        state["currencystate"]["currencies"][SYSTEM]["lastconversionprice"] = json!(2.00000001);
        assert!(
            replay_with_launch(
                &batch,
                1,
                &import,
                &state,
                SYSTEM,
                &outputs,
                Some((&def, &export))
            )
            .is_none()
        );
    }

    #[test]
    fn exceptional_launch_phases_definitions_and_missing_evidence_remain_unresolved() {
        let (def, import, export, state, batch, outputs) = context(false);
        assert!(replay_supported_prefix(&batch, 1, &import, &state, SYSTEM, &outputs).is_none());
        for field in [
            "launchcleared",
            "launchcomplete",
            "launchconfirmed",
            "refunding",
            "isdefinition",
            "isblockonenotarization",
        ] {
            let mut changed = state.clone();
            changed[field] = json!(true);
            assert!(
                replay_with_launch(
                    &batch,
                    1,
                    &import,
                    &changed,
                    SYSTEM,
                    &outputs,
                    Some((&def, &export))
                )
                .is_none(),
                "{field}"
            );
        }
        for (field, value) in [
            ("maxpreconversion", json!([1000])),
            ("prelaunchcarveout", json!(0.1)),
            ("prelaunchdiscount", json!(0.1)),
            ("options", json!(288)),
            ("systemid", json!(CURRENCY)),
        ] {
            let mut changed = def.clone();
            changed[field] = value;
            assert!(
                replay_with_launch(
                    &batch,
                    1,
                    &import,
                    &state,
                    SYSTEM,
                    &outputs,
                    Some((&changed, &export))
                )
                .is_none(),
                "{field}"
            );
        }
        for flags in [2, 4, 10, 12, 40] {
            let mut changed = import.clone();
            changed["flags"] = json!(flags);
            assert!(
                replay_with_launch(
                    &batch,
                    1,
                    &changed,
                    &state,
                    SYSTEM,
                    &outputs,
                    Some((&def, &export))
                )
                .is_none()
            );
        }
        let mut changed = export.clone();
        changed["flags"] = json!(3);
        assert!(
            replay_with_launch(
                &batch,
                1,
                &import,
                &state,
                SYSTEM,
                &outputs,
                Some((&def, &changed))
            )
            .is_none()
        );
    }
}

#[cfg(test)]
mod allocation_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn empty_clear_launch_allocations_require_the_entire_ordered_definition_vector() {
        let system = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        let currency = "i3f7tSctFkiPpiedY8QR5Tep9p4qDVebDx";
        let import =
            json!({"version":1,"flags":14,"sourcesystemid":system,"importcurrencyid":currency});
        let export = json!({"version":1,"flags":3,"numinputs":0,"sourcesystemid":system,
            "destinationsystemid":system,"destinationcurrencyid":currency});
        let state = json!({"version":2,"currencyid":currency,"launchcleared":true,"launchconfirmed":true,
            "currencystate":{"version":1,"currencyid":currency,"flags":56}});
        let mut definition = json!({"version":1,"currencyid":currency,"options":32,"systemid":system,
            "launchsystemid":system,"preallocations":[{system:12},{currency:3}]});
        let make = |recipient: &str, amount: i64| {
            json!({"valueSat":0,"scriptPubKey":{
            "type":"cryptocondition","reqSigs":1,"spendableoutput":true,"addresses":[recipient],
            "hex":format!("1b04030001011504{}cc",identity_hash(recipient).unwrap()),
            "reserveoutput":{"version":1,"currencyvalues":{currency:amount}}}})
        };
        let mut outputs = vec![
            make(system, 12),
            make(currency, 3),
            json!({"valueSat":10000}),
        ];
        let proven =
            allocations(&import, &state, &export, &definition, &[], system, &outputs).unwrap();
        assert_eq!(proven.len(), 2);
        assert_eq!(proven[0], (0, system.into(), 1_200_000_000));
        outputs[1]["scriptPubKey"]["reserveoutput"]["currencyvalues"][currency] = json!(3.00000001);
        assert!(
            allocations(&import, &state, &export, &definition, &[], system, &outputs).is_none()
        );
        outputs[1] = make(currency, 3);
        definition["preallocations"][0] = json!({"blockoneminer":12});
        assert!(
            allocations(&import, &state, &export, &definition, &[], system, &outputs).is_none()
        );
        definition["preallocations"][0] = json!({system:12});
        assert!(
            allocations(
                &import,
                &state,
                &export,
                &definition,
                &[json!({"flags":7})],
                system,
                &outputs
            )
            .is_none()
        );
        let mut failed = state.clone();
        failed["currencystate"]["flags"] = json!(12);
        assert!(
            allocations(
                &import,
                &failed,
                &export,
                &definition,
                &[],
                system,
                &outputs
            )
            .is_none()
        );
    }
}
