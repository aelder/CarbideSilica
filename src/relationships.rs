//! Decode explicit object references. Never turn a proof/content hash into a txid.
use crate::world::{OutpointRef, SystemId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub label: String,
    pub field: String,
    pub carrier_vout: u32,
    pub target: OutpointRef,
}

pub fn transaction_links(system: &SystemId, transaction: &Value) -> Vec<Link> {
    let mut links = Vec::new();
    let carrier = transaction
        .get("txid")
        .and_then(Value::as_str)
        .unwrap_or_default();
    for (index, output) in transaction
        .get("vout")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let Some(script) = output.get("scriptPubKey") else {
            continue;
        };
        let Ok(index) = u32::try_from(index) else {
            continue;
        };
        if let Some(import) = script.get("crosschainimport")
            && let Some(source) = import
                .get("sourcesystemid")
                .and_then(Value::as_str)
                .and_then(|s| SystemId::try_from(s.to_owned()).ok())
        {
            add(
                &mut links,
                index,
                "Source export",
                "crosschainimport.exporttxid",
                source,
                import.get("exporttxid"),
                import.get("exporttxout"),
                carrier,
            );
        }
        let export_finalization = &script["finalizeexport"];
        if crate::domain::finalization_tag(export_finalization, 2).is_some() {
            let target = &export_finalization["output"];
            let vout = target.get("voutnum").or_else(|| target.get("vout"));
            let same_transaction = target["txid"].as_str().is_some_and(is_null_hash);
            if !same_transaction
                || vout
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .is_some_and(|n| {
                        transaction["vout"][n]["scriptPubKey"]["crosschainexport"].is_object()
                    })
            {
                add(
                    &mut links,
                    index,
                    "Export referenced by finalization",
                    "finalizeexport.output",
                    system.clone(),
                    target.get("txid"),
                    vout,
                    carrier,
                );
            }
        }
        for key in ["earnednotarization", "acceptednotarization"] {
            if let Some(notarization) = script.get(key) {
                // SetMirror swaps principal state/root but retains the earned
                // object's predecessor. It therefore remains on the origin system.
                let predecessor_system =
                    if notarization.get("ismirror").and_then(Value::as_bool) == Some(true) {
                        notarization
                            .get("currencyid")
                            .and_then(Value::as_str)
                            .and_then(|id| SystemId::try_from(id.to_owned()).ok())
                    } else {
                        Some(system.clone())
                    };
                let Some(predecessor_system) = predecessor_system else {
                    continue;
                };
                add(
                    &mut links,
                    index,
                    "Previous notarization",
                    &format!("{key}.prevnotarizationtxid"),
                    predecessor_system,
                    notarization.get("prevnotarizationtxid"),
                    notarization.get("prevnotarizationout"),
                    carrier,
                );
            }
        }
        if let Some(evidence) = script.get("notaryevidence")
            && let Some(source) = evidence
                .get("systemid")
                .and_then(Value::as_str)
                .and_then(|s| SystemId::try_from(s.to_owned()).ok())
            && let Some(output) = evidence.get("output")
        {
            add(
                &mut links,
                index,
                "Evidence target",
                "notaryevidence.output",
                if output
                    .get("txid")
                    .and_then(Value::as_str)
                    .is_some_and(is_null_hash)
                {
                    system.clone()
                } else {
                    source
                },
                output.get("txid"),
                output.get("voutnum").or_else(|| output.get("vout")),
                carrier,
            );
        }
        if let Some(finalization) = script.get("finalizenotarization") {
            if let Some(output) = finalization.get("output") {
                add(
                    &mut links,
                    index,
                    "Finalization target",
                    "finalizenotarization.output",
                    system.clone(),
                    output.get("txid"),
                    output.get("voutnum").or_else(|| output.get("vout")),
                    carrier,
                );
            }
            for position in finalization
                .get("evidenceoutputs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                add(
                    &mut links,
                    index,
                    "Evidence output",
                    "finalizenotarization.evidenceoutputs",
                    system.clone(),
                    transaction.get("txid"),
                    Some(position),
                    carrier,
                );
            }
            for input in finalization
                .get("evidenceinputs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
            {
                if let Some(input) = usize::try_from(input)
                    .ok()
                    .and_then(|i| transaction.get("vin")?.as_array()?.get(i))
                {
                    add(
                        &mut links,
                        index,
                        "Evidence input",
                        "finalizenotarization.evidenceinputs",
                        system.clone(),
                        input.get("txid"),
                        input.get("vout"),
                        carrier,
                    );
                }
            }
        }
    }
    links
}

/// Reverse links use only already loaded carriers; this never starts a scan or RPC.
pub fn reverse_links(
    target_system: &SystemId,
    target_txid: &str,
    carrier_system: &SystemId,
    carrier: &Value,
) -> Vec<Link> {
    let Some(txid) = carrier.get("txid").and_then(Value::as_str) else {
        return vec![];
    };
    transaction_links(carrier_system, carrier)
        .into_iter()
        .filter(|link| &link.target.system_id == target_system && link.target.txid == target_txid)
        .map(|link| Link {
            label: format!("Referenced by: {}", link.label),
            field: format!("reverse:{}", link.field),
            carrier_vout: link.target.vout,
            target: OutpointRef {
                system_id: carrier_system.clone(),
                txid: txid.into(),
                vout: link.carrier_vout,
            },
        })
        .collect()
}

pub fn link_status(
    link: &Link,
    carrier_system: &SystemId,
    carrier: &Value,
    target_system: &SystemId,
    target: Option<&Value>,
) -> &'static str {
    let status = target_status(link, target_system, target);
    if status != "Both sides resolved" || link.field != "crosschainimport.exporttxid" {
        return status;
    }
    let import = &carrier["vout"][link.carrier_vout as usize]["scriptPubKey"]["crosschainimport"];
    let export =
        &target.unwrap()["vout"][link.target.vout as usize]["scriptPubKey"]["crosschainexport"];
    if export.get("destinationsystemid").and_then(Value::as_str) != Some(carrier_system.as_ref()) {
        return "Export destination mismatch";
    }
    if export.get("sourcesystemid").and_then(Value::as_str) != Some(target_system.as_ref()) {
        return "Export source mismatch";
    }
    // Compare the supplied identifiers; do not recompute or verify a proof.
    if import.get("hashtransfers").is_some()
        && import.get("hashtransfers") != export.get("hashtransfers")
    {
        return "Transfer reference mismatch";
    }
    status
}

fn is_null_hash(txid: &str) -> bool {
    txid.len() == 64 && txid.bytes().all(|byte| byte == b'0')
}

/// Check decoded navigation targets only. Membership remains a source observation.
pub fn target_status(link: &Link, system: &SystemId, transaction: Option<&Value>) -> &'static str {
    if system != &link.target.system_id {
        return "Source discrepancy";
    }
    let Some(transaction) = transaction else {
        return "Counterpart not loaded";
    };
    if transaction.get("txid").and_then(Value::as_str) != Some(&link.target.txid) {
        return "Target transaction mismatch";
    }
    let Some(script) = transaction
        .get("vout")
        .and_then(Value::as_array)
        .and_then(|outputs| outputs.get(link.target.vout as usize))
        .and_then(|output| output.get("scriptPubKey"))
    else {
        return "Referenced output absent";
    };
    if link.field.starts_with("reverse:") {
        return "Both sides resolved";
    }
    let expected = match link.field.as_str() {
        "crosschainimport.exporttxid" | "finalizeexport.output" => {
            script["crosschainexport"].is_object()
        }
        "finalizenotarization.evidenceoutputs" => script.get("notaryevidence").is_some(),
        "finalizenotarization.evidenceinputs" => {
            script.get("notaryevidence").is_some() || script.get("finalizenotarization").is_some()
        }
        _ => {
            script.get("earnednotarization").is_some()
                || script.get("acceptednotarization").is_some()
        }
    };
    if expected {
        "Both sides resolved"
    } else {
        "Unexpected target output type"
    }
}

#[allow(clippy::too_many_arguments)]
fn add(
    links: &mut Vec<Link>,
    carrier_vout: u32,
    label: &str,
    field: &str,
    system_id: SystemId,
    txid: Option<&Value>,
    vout: Option<&Value>,
    carrier: &str,
) {
    let Some(txid) = txid.and_then(Value::as_str) else {
        return;
    };
    let Some(vout) = vout
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
    else {
        return;
    };
    if txid.len() != 64 || !txid.bytes().all(|c| c.is_ascii_hexdigit()) {
        return;
    }
    // A null hash is a same-transaction locator only in fields that allow it.
    let txid = if txid.bytes().all(|c| c == b'0') {
        if matches!(
            field,
            "finalizenotarization.output" | "notaryevidence.output" | "finalizeexport.output"
        ) && carrier.len() == 64
            && carrier.bytes().all(|c| c.is_ascii_hexdigit())
            && !is_null_hash(carrier)
        {
            carrier
        } else {
            return;
        }
    } else {
        txid
    };
    links.push(Link {
        label: label.into(),
        field: field.into(),
        carrier_vout,
        target: OutpointRef {
            system_id,
            txid: txid.into(),
            vout,
        },
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const VRSC: &str = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
    const VDEX: &str = "iHog9UCTrn95qpUBFCZ7kKz7qWdMA8MQ6N";
    fn fixture(name: &str) -> Value {
        serde_json::from_str(match name {
            "accepted" => include_str!("../tests/fixtures/cross-chain/vrsc-accepted.json"),
            _ => include_str!("../tests/fixtures/cross-chain/vdex-earned.json"),
        })
        .unwrap()
    }

    #[test]
    fn real_import_and_export_link_both_system_and_currency_outputs() {
        let import: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/cross-chain/vdex-import.json"
        ))
        .unwrap();
        let export: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/cross-chain/vrsc-export.json"
        ))
        .unwrap();
        let vrsc = SystemId::try_from(VRSC.to_owned()).unwrap();
        let vdex = SystemId::try_from(VDEX.to_owned()).unwrap();
        let links = transaction_links(&vdex, &import);
        let exports: Vec<_> = links
            .iter()
            .filter(|l| l.field == "crosschainimport.exporttxid")
            .collect();
        assert_eq!(exports.len(), 2);
        assert_eq!(
            exports.iter().map(|l| l.target.vout).collect::<Vec<_>>(),
            vec![1, 2]
        );
        for link in &exports {
            assert_eq!(
                link_status(link, &vdex, &import, &vrsc, Some(&export)),
                "Both sides resolved"
            );
        }
        let reverse = reverse_links(&vrsc, export["txid"].as_str().unwrap(), &vdex, &import);
        assert_eq!(reverse.len(), 2);
        assert_eq!(reverse[0].target.txid, import["txid"]);
        assert_eq!(reverse[0].target.system_id, vdex);
        let mut mismatched = export.clone();
        mismatched["vout"][1]["scriptPubKey"]["crosschainexport"]["destinationsystemid"] =
            json!(VRSC);
        assert_eq!(
            link_status(exports[0], &vdex, &import, &vrsc, Some(&mismatched)),
            "Export destination mismatch"
        );
    }

    #[test]
    fn real_mirrored_notarization_links_to_earned_and_predecessor_on_vdex() {
        let accepted = fixture("accepted");
        let earned = fixture("earned");
        let vrsc = SystemId::try_from(VRSC.to_owned()).unwrap();
        let vdex = SystemId::try_from(VDEX.to_owned()).unwrap();
        let links = transaction_links(&vrsc, &accepted);
        let evidence: Vec<_> = links
            .iter()
            .filter(|l| l.field == "notaryevidence.output" && l.carrier_vout < 3)
            .collect();
        assert_eq!(evidence.len(), 2);
        for link in evidence {
            assert_eq!(link.target.txid, earned["txid"]);
            assert_eq!(link.target.vout, 1);
            assert_eq!(
                target_status(link, &vdex, Some(&earned)),
                "Both sides resolved"
            );
            assert_eq!(
                target_status(link, &vrsc, Some(&earned)),
                "Source discrepancy"
            );
        }
        let predecessor = links
            .iter()
            .find(|l| l.field.ends_with("prevnotarizationtxid"))
            .unwrap();
        assert_eq!(predecessor.target.system_id, vdex);
        let previous: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/cross-chain/vdex-predecessor.json"
        ))
        .unwrap();
        assert_eq!(
            target_status(predecessor, &vdex, Some(&previous)),
            "Both sides resolved"
        );
        let finalization = links
            .iter()
            .find(|l| l.carrier_vout == 3 && l.field == "finalizenotarization.output")
            .unwrap();
        assert_eq!(finalization.target.system_id, vrsc);
        assert_eq!(finalization.target.vout, 0);
        assert_eq!(
            target_status(finalization, &vrsc, Some(&accepted)),
            "Both sides resolved"
        );
        let input = links
            .iter()
            .find(|l| l.field == "finalizenotarization.evidenceinputs")
            .unwrap();
        assert_eq!(input.target.vout, 3); // vin[2] spends predecessor vout[3]
    }

    #[test]
    fn real_null_evidence_and_finalization_targets_stay_on_carrier_chain() {
        let earned = fixture("earned");
        let vdex = SystemId::try_from(VDEX.to_owned()).unwrap();
        let links = transaction_links(&vdex, &earned);
        for index in [2, 3, 4] {
            let link = links
                .iter()
                .find(|link| link.carrier_vout == index && link.field.ends_with(".output"))
                .unwrap();
            assert_eq!(link.target.system_id, vdex);
            assert_eq!(link.target.txid, earned["txid"]);
            assert_eq!(link.target.vout, 1);
            assert_eq!(
                target_status(link, &vdex, Some(&earned)),
                "Both sides resolved"
            );
        }
    }

    #[test]
    fn import_routes_to_source_and_notarization_predecessor_stays_local() {
        let carrier = SystemId::try_from("iCarrier".to_string()).unwrap();
        let export = "a".repeat(64);
        let previous = "b".repeat(64);
        let tx = json!({"txid":"c".repeat(64),"vout":[{"scriptPubKey":{
            "crosschainimport":{"sourcesystemid":"iSource","exporttxid":export,"exporttxout":2},
            "acceptednotarization":{"currencyid":"iSource","prevnotarizationtxid":previous,"prevnotarizationout":1,"hashprevcrossnotarization":"d".repeat(64)}
        }}]});
        let links = transaction_links(&carrier, &tx);
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].target.system_id.as_ref(), "iSource");
        assert_eq!(links[0].target.txid, export);
        assert_eq!(links[1].target.system_id, carrier);
        assert_eq!(links[1].target.txid, previous);
    }
    #[test]
    fn evidence_inputs_are_input_indices_not_output_indices() {
        let tx = json!({"txid":"c".repeat(64),"vin":[{"txid":"a".repeat(64),"vout":7}],"vout":[{"scriptPubKey":{"finalizenotarization":{"evidenceinputs":[0,99]}}}]});
        let links = transaction_links(&SystemId::try_from("iChain".to_string()).unwrap(), &tx);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target.vout, 7);
    }
}
