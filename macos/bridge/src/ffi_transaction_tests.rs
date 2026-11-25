use super::*;
use serde_json::json;

const NATIVE: &str = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";

fn context_with(sources: &[Value]) -> Context {
    Context {
        native: Some(NATIVE.into()),
        sources: sources
            .iter()
            .map(|tx| (tx["txid"].as_str().unwrap().to_string(), tx.clone()))
            .collect(),
        complete: true,
        ..Context::default()
    }
}

fn accounting(raw: &Value, context: &Context) -> ValueAccounting {
    let flows = transaction_flows(raw, context, Some(NATIVE));
    value_accounting(raw, &flows, context, "VRSC")
}

fn output(raw: &Value, context: &Context, index: usize) -> Flow {
    transaction_flows(raw, context, Some(NATIVE))
        .into_iter()
        .filter(|flow| flow.kind == FlowKind::Output)
        .nth(index)
        .unwrap()
}

// ------------------------------------------------------------ fees and values

#[test]
fn a_transparent_transaction_has_a_known_fee() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0,"valueSat":150_000_000,"address":"RA"}],
        "vout":[{"n":0,"valueSat":100_000_000,"scriptPubKey":{"type":"pubkeyhash","addresses":["RB"]}},
                {"n":1,"valueSat":49_990_000,"scriptPubKey":{"type":"pubkeyhash","addresses":["RA"]}}]});
    let value = accounting(&raw, &Context::default());
    assert_eq!(value.fee_kind, FeeKind::Known);
    assert_eq!(value.fee_sats, Some(10_000));
    assert_eq!(value.fee_label, "0.00010000 VRSC");
    assert_eq!(value.inputs.label, "1.50000000 VRSC");
    assert_eq!(value.outputs.label, "1.49990000 VRSC");
    assert!(!value.show_public_delta);
    assert_eq!((value.input_count, value.output_count), (1, 2));
}

#[test]
fn a_missing_input_value_makes_the_fee_unknown() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0}],
        "vout":[{"n":0,"valueSat":100,"scriptPubKey":{"type":"pubkeyhash"}}]});
    let value = accounting(&raw, &Context::default());
    assert_eq!(value.fee_kind, FeeKind::Unknown);
    assert_eq!(value.fee_label, "unknown");
    assert_eq!(value.inputs.label, "0.00000000 VRSC");
    assert!(value.inputs.remaining_unknown);
    assert_eq!(value.public_native_delta, None);
}

#[test]
fn the_previous_output_supplies_a_missing_input_value() {
    let source = json!({"txid":"a","vout":[{"n":0,"valueSat":300,"scriptPubKey":{"type":"pubkeyhash","addresses":["RA"]}}]});
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0}],
        "vout":[{"n":0,"valueSat":100,"scriptPubKey":{"type":"pubkeyhash"}}]});
    let context = context_with(&[source]);
    let value = accounting(&raw, &context);
    assert_eq!(value.fee_sats, Some(200));
    let flows = transaction_flows(&raw, &context, Some(NATIVE));
    assert_eq!(flows[0].addresses, ["RA"]);
    assert_eq!(flows[0].script_type.as_deref(), Some("pubkeyhash"));
}

#[test]
fn a_coinbase_fee_does_not_apply() {
    let raw = json!({"txid":"t","vin":[{"coinbase":"03"}],
        "vout":[{"n":0,"valueSat":600_000_000,"scriptPubKey":{"type":"pubkey"}}]});
    let value = accounting(&raw, &Context::default());
    assert_eq!(value.fee_kind, FeeKind::NotApplicable);
    assert_eq!(value.fee_label, "n/a");
    assert_eq!(value.inputs.label, "Coinbase");
    assert!(value.inputs.coinbase);
    assert!(!value.show_public_delta);
}

#[test]
fn protocol_outputs_leave_the_fee_unknown_and_show_the_public_delta() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0,"valueSat":1_000_000}],
        "vout":[{"n":0,"valueSat":600_000,"scriptPubKey":{"type":"cryptocondition","reservetransfer":{
            "flags":1,"feecurrencyid":NATIVE,"fees":0.0002,
            "currencyvalues":{"iToken11111111111111111111111111111":5.5, NATIVE: 0.004}}}}]});
    let value = accounting(&raw, &Context::default());
    assert_eq!(value.fee_kind, FeeKind::ProtocolAccounting);
    assert_eq!(value.fee_label, "unknown (protocol accounting)");
    assert!(value.show_public_delta);
    assert_eq!(value.public_native_delta, Some(400_000));
    // Only the token counts as a reserve; native principal and fees are the chain's coin.
    assert_eq!(value.outputs.reserves.len(), 1);
    assert_eq!(
        value.outputs.reserves[0].currency,
        "iToken11111111111111111111111111111"
    );
    assert_eq!(value.outputs.reserves[0].amount, 5.5);
}

#[test]
fn minted_principal_has_no_custody_but_fee_budgets_do() {
    let output = json!({"scriptPubKey":{"reservetransfer":{
        "flags":0x21,"feecurrencyid":"iFee1111111111111111111111111111111","fees":0.5,
        "currencyvalues":{"iMint111111111111111111111111111111":100},
        "destination":{"type":130,"gateway":"iGate","fees":0.25}}}});
    let amounts = monetary_reserve_amounts(&output, Some(NATIVE));
    assert_eq!(amounts.len(), 1);
    assert_eq!(amounts["iFee1111111111111111111111111111111"], 75_000_000);
}

#[test]
fn hidden_shielded_outputs_make_the_total_known_plus_hidden() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0,"valueSat":500}],
        "vout":[{"n":0,"valueSat":100,"scriptPubKey":{"type":"pubkeyhash"}}],
        "vShieldedOutput":[{},{}],"valueBalanceZat":-390});
    let value = accounting(&raw, &Context::default());
    assert_eq!(value.outputs.label, "0.00000100 VRSC known + hidden");
    assert!(value.outputs.hidden);
    // Transparent 500 in, 100 out, 390 into the shielded pool: a fee of 10.
    assert_eq!(value.fee_sats, Some(10));
    assert_eq!(value.output_count, 3);
}

#[test]
fn one_sapling_output_is_derived_from_the_value_balance() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0,"valueSat":500}],"vout":[],
        "vShieldedOutput":[{}],"valueBalanceZat":-490});
    let context = Context::default();
    let value = accounting(&raw, &context);
    assert_eq!(value.outputs.label, "0.00000490 VRSC (partly derived)");
    assert!(value.outputs.derived);
    let flow = output(&raw, &context, 0);
    assert_eq!(flow.private, Some("Sapling"));
    assert_eq!(flow.value_zat, Some(490));
    assert_eq!(flow.annotations, ["Derived from Sapling valueBalance"]);
    let record = output_record(&raw, &flow, Vec::new());
    assert!(record.derived_value);
    assert_eq!(record.shielded_pool.as_deref(), Some("Sapling"));
}

#[test]
fn one_sapling_spend_is_derived_and_sprout_stays_hidden() {
    let raw = json!({"txid":"t","vin":[],"vout":[{"n":0,"valueSat":90,"scriptPubKey":{"type":"pubkeyhash"}}],
        "vShieldedSpend":[{}],"valueBalanceZat":100,
        "vjoinsplit":[{"vpub_oldZat":0,"vpub_newZat":0}]});
    let flows = transaction_flows(&raw, &Context::default(), Some(NATIVE));
    let inputs: Vec<&Flow> = flows.iter().filter(|f| f.kind == FlowKind::Input).collect();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0].private, Some("Sapling"));
    assert_eq!(inputs[0].value_zat, Some(100));
    assert_eq!(inputs[1].private, Some("Sprout"));
    assert_eq!(inputs[1].value_zat, None);
    let value = accounting(&raw, &Context::default());
    assert_eq!(value.inputs.label, "0.00000100 VRSC known + hidden");
}

#[test]
fn shielded_lines_use_carbide_wording() {
    let raw = json!({"vShieldedOutput":[{}],"valueBalanceZat":-250_000_000,
        "vjoinsplit":[{"vpub_oldZat":5,"vpub_newZat":0}]});
    assert_eq!(
        shielded_lines(&ShieldedAccounting::from_raw(&raw), "VRSC"),
        [
            "Sapling 2.50000000 VRSC transparent -> shielded",
            "Sprout 0.00000005 VRSC transparent -> shielded"
        ]
    );
    let net = json!({"vShieldedSpend":[{}],"vShieldedOutput":[{}],"valueBalanceZat":0});
    assert_eq!(
        shielded_lines(&ShieldedAccounting::from_raw(&net), "VRSC"),
        ["Sapling public net 0 (individual values unknown)"]
    );
    let broken = json!({"vShieldedOutput":[{}]});
    assert_eq!(
        shielded_lines(&ShieldedAccounting::from_raw(&broken), "VRSC"),
        [
            "Sapling public balance unavailable",
            "accounting unavailable"
        ]
    );
    assert!(shielded_lines(&ShieldedAccounting::from_raw(&json!({})), "VRSC").is_empty());
}

#[test]
fn a_consumed_token_is_a_consumption_burn() {
    let source = json!({"txid":"a","vout":[{"n":0,"valueSat":1000,"scriptPubKey":{
        "type":"cryptocondition","reserveoutput":{"currencyvalues":{"iToken11111111111111111111111111111":3}}}}]});
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0}],
        "vout":[{"n":0,"valueSat":900,"scriptPubKey":{"type":"cryptocondition",
            "reserveoutput":{"currencyvalues":{"iToken11111111111111111111111111111":1}}}}]});
    let context = context_with(&[source]);
    let burns = consumed_token_burns(&raw, &context);
    assert_eq!(burns["iToken11111111111111111111111111111"], 200_000_000);
    assert!(display_tags(&raw, &context).contains(&"Consumption Burn".to_string()));
    // Without the previous output nothing is proven.
    assert!(consumed_token_burns(&raw, &Context::default()).is_empty());
}

// ---------------------------------------------------------------- status

#[test]
fn status_follows_carbide_order() {
    assert_eq!(
        transaction_status(true, 5, true, false).0,
        TransactionStatus::Unconfirmed
    );
    assert_eq!(
        transaction_status(false, 12, true, false),
        (TransactionStatus::Confirmed, "confirmed (12)".to_string())
    );
    assert_eq!(
        transaction_status(false, 0, true, true).1,
        "orphaned inclusion; current mempool membership unverified"
    );
    assert_eq!(
        transaction_status(false, 0, true, false),
        (
            TransactionStatus::ConfirmationsUnverified,
            "confirmations unverified".into()
        )
    );
    assert_eq!(transaction_status(false, 0, false, false).1, "unconfirmed");
}

#[test]
fn confirmations_come_from_the_chain_state() {
    use crate::chain_state::{ChainState, RpcObservation};
    let mut chain = ChainState::default();
    let observe = |chain: &mut ChainState, order, method: &str, result: Value| {
        chain.observe(RpcObservation {
            authorization: None,
            order,
            method: method.into(),
            params: Vec::new(),
            result,
        })
    };
    // No tip: the source's own count is used.
    assert_eq!(depth_for_status(&chain, Some("b"), Some(7)), 7);
    observe(
        &mut chain,
        1,
        "getinfo",
        json!({"blocks":110,"longestchain":110}),
    );
    // A tip but no header: the count is not established.
    assert_eq!(depth_for_status(&chain, Some("b"), Some(7)), 0);
    observe(
        &mut chain,
        2,
        "getblockheader",
        json!({"hash":"b","height":101,"time":5,"confirmations":10}),
    );
    assert_eq!(depth_for_status(&chain, Some("b"), Some(7)), 10);
}

// -------------------------------------------------------------- maturity

fn coinbase(value: i64, script: &str, spendable: Option<bool>) -> Value {
    let mut script = json!({"type":script});
    if let Some(spendable) = spendable {
        script["spendableoutput"] = json!(spendable);
    }
    json!({"txid":"t","vin":[{"coinbase":"03"}],"vout":[{"n":0,"valueSat":value,"scriptPubKey":script}]})
}

#[test]
fn a_young_coinbase_is_immature_and_cannot_stake_yet() {
    let raw = coinbase(600_000_000, "pubkey", None);
    let flow = output(&raw, &Context::default(), 0);
    assert_eq!(
        output_badges(&raw, &flow, Some(40), Some(100), Some(5000)),
        ["Immature", "Stake age pending"]
    );
    assert_eq!(
        output_badges(&raw, &flow, Some(120), Some(100), Some(5000)),
        ["Stake age pending"]
    );
    assert!(output_badges(&raw, &flow, Some(150), Some(100), Some(5000)).is_empty());
    // Unknown depth shows nothing.
    assert!(output_badges(&raw, &flow, None, Some(100), Some(5000)).is_empty());
}

#[test]
fn pbaas_block_one_allocations_skip_maturity() {
    let raw = coinbase(600_000_000, "pubkeyhash", None);
    let flow = output(&raw, &Context::default(), 0);
    assert_eq!(
        output_badges(&raw, &flow, Some(3), Some(149), Some(1)),
        ["Stake age pending"]
    );
    assert_eq!(
        output_badges(&raw, &flow, Some(3), Some(100), Some(1)),
        ["Immature", "Stake age pending"]
    );
}

#[test]
fn protocol_and_valueless_outputs_get_no_maturity_badges() {
    let raw = coinbase(600_000_000, "cryptocondition", Some(false));
    let flow = output(&raw, &Context::default(), 0);
    assert_eq!(
        output_badges(&raw, &flow, Some(3), Some(100), Some(9)),
        ["Immature"]
    );
    let raw = coinbase(0, "pubkey", None);
    let flow = output(&raw, &Context::default(), 0);
    assert!(output_badges(&raw, &flow, Some(3), Some(100), Some(9)).is_empty());
}

#[test]
fn coinbase_maturity_comes_from_chain_state() {
    assert_eq!(crate::chain_state::coinbase_maturity(60, false), Some(100));
    assert_eq!(crate::chain_state::coinbase_maturity(10, false), Some(149));
}

// ------------------------------------------------------------ annotations

#[test]
fn an_arbitrage_transfer_spent_by_an_import_is_an_auto_arbitrage_input() {
    let sources = [
        json!({"txid":"prior-import","vout":[{"scriptPubKey":{"crosschainimport":{"version":1}}}]}),
        json!({"txid":"arb-take","vout":[{"scriptPubKey":{"reservetransfer":{"arbitrageonly":true,"convert":true}}}]}),
        json!({"txid":"prior-notarization","vout":[{"scriptPubKey":{"acceptednotarization":{"version":2}}}]}),
    ];
    let raw = json!({"txid":"import","vin":[
            {"txid":"prior-import","vout":0},{"txid":"arb-take","vout":0},{"txid":"prior-notarization","vout":0}],
        "vout":[{"scriptPubKey":{"crosschainimport":{"version":1,"importcurrencyid":"iPool"}}}]});
    let context = context_with(&sources);
    let flows = transaction_flows(&raw, &context, Some(NATIVE));
    assert!(flows[0].annotations.is_empty());
    assert_eq!(flows[1].annotations, ["Auto-Arbitrage Input"]);
    assert!(flows[2].annotations.is_empty());
    assert!(display_tags(&raw, &context).contains(&"Auto-Arbitrage Import".to_string()));
}

#[test]
fn closing_a_marketplace_offer_marks_cancellation_and_refund() {
    let funding = json!({"txid":"funding","vin":[],"vout":[
        {"n":0,"spentTxId":"close","scriptPubKey":{"commitmenthash":{}}},
        {"n":1,"spentTxId":"proof","scriptPubKey":{"type":"pubkeyhash"}}]});
    let proof = json!({"txid":"proof","vin":[{"txid":"funding","vout":1}],
        "vout":[{"scriptPubKey":{"hex":"6a050500000003"}}]});
    let close = json!({"txid":"close","vin":[{"txid":"funding","vout":0}],
        "vout":[{"n":0,"scriptPubKey":{"type":"pubkeyhash"}},{"n":1,"scriptPubKey":{"type":"nulldata","hex":"6a"}}],
        "vShieldedOutput":[{},{}],"valueBalanceZat":-5});
    let mut context = context_with(&[funding]);
    context.proofs.insert("proof".into(), proof);
    let flows = transaction_flows(&close, &context, Some(NATIVE));
    assert_eq!(flows[0].annotations, ["Marketplace Offer Cancellation"]);
    assert_eq!(flows[1].annotations, ["Marketplace Offer Refund"]);
    assert!(
        flows[2].annotations.is_empty(),
        "a nulldata output is not a refund"
    );
    assert_eq!(flows[3].annotations, ["Marketplace Offer Refund"]);
    assert!(display_tags(&close, &context).contains(&"Marketplace Offer Closed".to_string()));
    // Without the proof the funding transaction is not an offer.
    let unproven = context_with(&[json!({"txid":"funding","vout":[
        {"n":0,"spentTxId":"close","scriptPubKey":{"commitmenthash":{}}},{"n":1}]})]);
    assert!(
        transaction_flows(&close, &unproven, Some(NATIVE))[0]
            .annotations
            .is_empty()
    );
}

#[test]
fn an_emission_timelock_shows_its_unlock_height_and_hides_its_metadata() {
    let raw = json!({"txid":"9a43ee09493a99a9733166c1803a77185575feee0b575872ff54a0b8cd10c2ef",
        "vin":[{"coinbase":"02c07b0101"}],
        "vout":[
            {"n":0,"valueSat":5,"scriptPubKey":{"type":"scripthash","hex":"a914c211e525380231b5dd364d4fed795af49281ec7c87"}},
            {"n":1,"scriptPubKey":{"type":"nulldata","hex":"6a200103cf610fb17576a914630bcfd6abe8125b6a9aeeb4fde7f8801c7a6de688ac"}}]});
    let context = Context::default();
    let locked = output_record(&raw, &output(&raw, &context, 0), Vec::new());
    assert_eq!(locked.annotations, ["Unlock height 1008079"]);
    assert!(locked.tags.contains(&"Emission Timelock".to_string()));
    let metadata = output_record(&raw, &output(&raw, &context, 1), Vec::new());
    assert!(
        metadata.data_hex.is_none(),
        "known data is not offered as raw bytes"
    );
}

#[test]
fn a_legacy_notarization_links_its_block_and_names_its_commitments() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/output-presentation.json"
    ))
    .unwrap();
    let legacy = &fixture["legacy"];
    let flow = output(legacy, &Context::default(), 1);
    let record = output_record(legacy, &flow, Vec::new());
    assert_eq!(
        record.annotations[0],
        format!(
            "Destination transaction {}",
            legacy["destination_txid"].as_str().unwrap()
        )
    );
    assert!(record.annotations[1].starts_with("Merkle-of-Merkle-roots depth "));
    assert!(record.annotations[2].starts_with("Merkle-of-Merkle-roots "));
    let block = record
        .references
        .iter()
        .find(|reference| reference.label == "Notarized VRSC block")
        .unwrap();
    match &block.target {
        ReferenceTarget::Block { hash, height } => {
            assert_eq!(hash, legacy["notarized_hash"].as_str().unwrap());
            assert_eq!(
                u64::from(*height),
                legacy["notarized_height"].as_u64().unwrap()
            );
        }
        other => panic!("unexpected target {other:?}"),
    }
    assert!(record.data_hex.is_none());
}

#[test]
fn an_export_finalization_tag_opens_the_referenced_export() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/output-presentation.json"
    ))
    .unwrap();
    let export = &fixture["export"];
    let flow = output(export, &Context::default(), 2);
    let tag = flow
        .tags
        .iter()
        .map(|tag| tag_info(&flow, tag))
        .find(|tag| tag.label.contains("Export Finalization"))
        .unwrap();
    let target = tag.target.unwrap();
    assert_eq!(
        (target.txid.as_str(), target.output),
        (export["txid"].as_str().unwrap(), 1)
    );
}

#[test]
fn a_reserve_transfer_tag_opens_the_trace_and_names_conversions() {
    let raw = json!({"txid":"t","vin":[],"vout":[{"n":0,"scriptPubKey":{"reservetransfer":{
        "flags":3,"convert":true,"destinationcurrencyid":"iPool111111111111111111111111111111",
        "currencyvalues":{NATIVE:1}}}}]});
    let flow = output(&raw, &Context::default(), 0);
    let tags: Vec<OutputTagInfo> = flow.tags.iter().map(|tag| tag_info(&flow, tag)).collect();
    let transfer = tags
        .iter()
        .find(|tag| tag.label == "Reserve Transfer")
        .unwrap();
    assert!(transfer.traceable);
    if let Some(conversion) = tags.iter().find(|tag| tag.label == "Conversion") {
        assert_eq!(
            conversion.currency_id.as_deref(),
            Some("iPool111111111111111111111111111111")
        );
    }
}

#[test]
fn registration_lines_name_the_reservation() {
    let raw = json!({"vout":[{"n":2,"scriptPubKey":{"addresses":["RPay","iSomething"],
        "identityreservation":{"name":"alice","nameid":"iAlice","parent":"iParent","referral":"iRef"}}}]});
    assert_eq!(
        registration_lines(&raw),
        [
            "Registers alice@ (iAlice) from reservation output 2",
            "Parent: iParent",
            "Referral: iRef",
            "Reservation output destination: RPay"
        ]
    );
    assert!(registration_lines(&json!({"vout":[]})).is_empty());
}

#[test]
fn a_notarization_is_confirmed_only_as_the_last_confirmed_entry() {
    let data = json!({"lastconfirmed":1,"notarizations":[
        {"index":0,"txid":"old","vout":1},{"index":1,"txid":"tip","vout":2},{"index":2,"txid":"new","vout":0}]});
    assert_eq!(notarization_status(Some(&data), "tip", 2), "confirmed");
    assert_eq!(notarization_status(Some(&data), "new", 0), "unconfirmed");
    assert_eq!(notarization_status(Some(&data), "old", 1), "unconfirmed");
    assert_eq!(
        notarization_status(Some(&data), "gone", 0),
        "status unavailable"
    );
    assert_eq!(notarization_status(None, "tip", 2), "status unavailable");
}

// --------------------------------------------------------------- byte view

#[test]
fn op_return_pushes_are_concatenated_payload_bytes() {
    let raw = json!({"txid":"t","vin":[],"vout":[{"n":0,"scriptPubKey":{"type":"nulldata","hex":"6a0241ff4c010a"}}]});
    let record = output_record(&raw, &output(&raw, &Context::default(), 0), Vec::new());
    assert_eq!(record.data_hex.as_deref(), Some("41ff0a"));
    match &record.references.last().unwrap().target {
        ReferenceTarget::Bytes { hex, script_body } => {
            assert_eq!(hex, "41ff0a");
            assert!(!script_body);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn undecodable_pushes_show_the_script_after_op_return() {
    let raw = json!({"txid":"t","vin":[],"vout":[{"n":0,"scriptPubKey":{"type":"nulldata","hex":"6a0241"}}]});
    let record = output_record(&raw, &output(&raw, &Context::default(), 0), Vec::new());
    assert_eq!(record.data_hex.as_deref(), Some("0241"));
    assert!(matches!(
        record.references.last().unwrap().target,
        ReferenceTarget::Bytes {
            script_body: true,
            ..
        }
    ));
}

#[test]
fn a_stake_transactions_second_output_is_known_data() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0}],"vout":[
        {"n":0,"valueSat":5,"scriptPubKey":{"type":"pubkeyhash"}},
        {"n":1,"scriptPubKey":{"type":"nulldata","hex":"6a0241ff"}}]});
    // Outside a stake transaction the same output is offered as bytes.
    let record = output_record(&raw, &output(&raw, &Context::default(), 1), Vec::new());
    assert_eq!(record.data_hex.as_deref(), Some("41ff"));
}

#[test]
fn the_offer_proof_prefix_is_recognised() {
    assert!(op_return_starts_with_transaction_proof("6a050500000003"));
    assert!(!op_return_starts_with_transaction_proof("6a0405000000"));
    assert_eq!(extract_op_return_payload("6a4c020102"), Some(vec![1, 2]));
    assert_eq!(decode_hex("0a0B"), Some(vec![10, 11]));
    assert_eq!(decode_hex("0"), None);
}

// ------------------------------------------------------------------ links

fn link(field: &str, vout: u32, target: &str) -> crate::relationships::Link {
    crate::relationships::Link {
        label: field.into(),
        field: field.into(),
        carrier_vout: vout,
        target: OutpointRef {
            system_id: SystemId::try_from(NATIVE.to_string()).unwrap(),
            txid: target.into(),
            vout: 0,
        },
    }
}

#[test]
fn forward_links_sort_before_reverse_links_without_duplicates() {
    let links = sorted_links(vec![
        link("reverse:finalizenotarization.output", 0, "b"),
        link("finalizenotarization.output", 2, "a"),
        link("crosschainimport.exporttxid", 1, "c"),
        link("finalizenotarization.output", 2, "a"),
    ]);
    let fields: Vec<_> = links
        .iter()
        .map(|link| (link.field.as_str(), link.carrier_vout))
        .collect();
    assert_eq!(
        fields,
        [
            ("crosschainimport.exporttxid", 1),
            ("finalizenotarization.output", 2),
            ("reverse:finalizenotarization.output", 0)
        ]
    );
}

#[test]
fn link_statuses_separate_resolution_from_disagreement() {
    let resolved = protocol_link(
        &link("finalizeexport.output", 2, "a"),
        false,
        Some("Both sides resolved"),
    );
    assert!(resolved.resolved && !resolved.problem);
    for status in [
        "Counterpart not loaded",
        "No connection configured",
        "Unresolved",
    ] {
        let record = protocol_link(&link("x", 0, "a"), false, Some(status));
        assert!(!record.resolved && !record.problem, "{status}");
    }
    for status in [
        "Source discrepancy",
        "Target transaction mismatch",
        "Referenced output absent",
        "Unexpected target output type",
        "Export source mismatch",
        "Export destination mismatch",
        "Transfer reference mismatch",
    ] {
        assert!(
            protocol_link(&link("x", 0, "a"), false, Some(status)).problem,
            "{status}"
        );
    }
    let unchecked = protocol_link(&link("x", 0, "a"), true, None);
    assert!(unchecked.reverse && unchecked.status.is_none());
}

#[test]
fn an_import_with_another_destination_is_an_export_destination_mismatch() {
    let system = SystemId::try_from(NATIVE.to_string()).unwrap();
    let export_txid = "e".repeat(64);
    let carrier = json!({"txid":"import","vout":[{"scriptPubKey":{"crosschainimport":{
        "exporttxid":export_txid,"exporttxout":0,"sourcesystemid":NATIVE,"hashtransfers":"aa"}}}]});
    let link = crate::relationships::transaction_links(&system, &carrier)
        .into_iter()
        .find(|link| link.field == "crosschainimport.exporttxid")
        .unwrap();
    let mut export = json!({"txid":export_txid,"vout":[{"scriptPubKey":{"crosschainexport":{
        "destinationsystemid":"iOther","sourcesystemid":NATIVE,"hashtransfers":"aa"}}}]});
    let status = |export: &Value| {
        crate::relationships::link_status(&link, &system, &carrier, &system, Some(export))
    };
    assert_eq!(status(&export), "Export destination mismatch");
    export["vout"][0]["scriptPubKey"]["crosschainexport"]["destinationsystemid"] = json!(NATIVE);
    assert_eq!(status(&export), "Both sides resolved");
    export["vout"][0]["scriptPubKey"]["crosschainexport"]["hashtransfers"] = json!("bb");
    assert_eq!(status(&export), "Transfer reference mismatch");
    assert_eq!(
        crate::relationships::link_status(&link, &system, &carrier, &system, None),
        "Counterpart not loaded"
    );
}

// ------------------------------------------------------------------ block

#[test]
fn block_tags_combine_without_coinbase_and_stake() {
    let row = |tags: &[&str]| TransactionRow {
        txid: "t".into(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
        input_count: 0,
        output_count: 0,
        output_sats: None,
    };
    assert_eq!(
        combined_block_tags(&[
            row(&["Coinbase", "Emission Timelock"]),
            row(&["Stake"]),
            row(&["Conversion", "Reserve Transfer"]),
            row(&["Reserve Transfer"]),
        ]),
        ["Emission Timelock", "Conversion", "Reserve Transfer"]
    );
}

#[test]
fn block_chain_status_reads_the_confirmation_count() {
    assert_eq!(block_chain_status(Some(-1), 10, Some(20)), Some("Orphaned"));
    assert_eq!(block_chain_status(Some(1), 20, Some(20)), Some("Chain tip"));
    assert_eq!(block_chain_status(Some(1), 20, Some(25)), Some("Active"));
    assert_eq!(block_chain_status(Some(3), 18, Some(20)), Some("Active"));
    assert_eq!(block_chain_status(None, 18, Some(20)), None);
}

// ------------------------------------------------------------------- trace

#[test]
fn an_empty_trail_is_reported_as_unknown() {
    let trace = transfer_trace(crate::defi::Trail::default());
    assert_eq!(trace.confidence, Confidence::Unknown);
    assert!(trace.legs.is_empty());
    assert!(!trace.searching);
}

#[test]
fn a_refused_request_is_said_plainly() {
    assert!(plain_stop("RPC error from getimports: {\"code\":-32601}").contains("does not permit"));
    assert_eq!(
        plain_stop("Continuation cycle detected"),
        "Continuation cycle detected"
    );
}

// -------------------------------------------------------------------- JSON

fn mark<'a>(marks: &'a [JsonMark], pointer: &str) -> Option<&'a JsonMark> {
    marks.iter().find(|mark| mark.pointer == pointer)
}

#[test]
fn json_links_open_transactions_blocks_addresses_identities_and_currencies() {
    let root = json!({
        "txid": "a".repeat(64),
        "blockhash": "b".repeat(64),
        "hashtransfers": "c".repeat(64),
        "vin": [{"txid": "d".repeat(64), "address": "RXPzH3i4fZaaPFdkVHWKRDgXDqrFXErEGz"}],
        "vout": [{"spentTxId": "e".repeat(64), "scriptPubKey": {
            "addresses": ["iJhCezBExJHvtyH3fGhNnt2NhU4Ztkf2yq"],
            "reservetransfer": {"currencyvalues": {}, "feecurrencyid": "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV"},
            "identityprimary": {"identityaddress": "iJhCezBExJHvtyH3fGhNnt2NhU4Ztkf2yq", "parent": "iJhCezBExJHvtyH3fGhNnt2NhU4Ztkf2yq", "name": "alice@"}
        }}]
    });
    let marks = json_links(&root);
    let kind = |pointer: &str| mark(&marks, pointer).and_then(|mark| mark.link);
    assert_eq!(kind("/txid"), Some(JsonLinkKind::Transaction));
    assert_eq!(kind("/blockhash"), Some(JsonLinkKind::Block));
    assert_eq!(
        kind("/hashtransfers"),
        None,
        "a proof hash is not a transaction"
    );
    assert_eq!(kind("/vin/0/txid"), Some(JsonLinkKind::Transaction));
    assert_eq!(kind("/vin/0/address"), Some(JsonLinkKind::Address));
    assert_eq!(kind("/vout/0/spentTxId"), Some(JsonLinkKind::Transaction));
    assert_eq!(
        kind("/vout/0/scriptPubKey/addresses/0"),
        Some(JsonLinkKind::Identity)
    );
    assert_eq!(
        kind("/vout/0/scriptPubKey/reservetransfer/feecurrencyid"),
        Some(JsonLinkKind::Currency)
    );
    assert_eq!(
        kind("/vout/0/scriptPubKey/identityprimary/parent"),
        Some(JsonLinkKind::Identity)
    );
    assert_eq!(
        kind("/vout/0/scriptPubKey/identityprimary/name"),
        Some(JsonLinkKind::Identity)
    );
    let raw = mark(
        &marks,
        "/vout/0/scriptPubKey/identityprimary/identityaddress",
    )
    .unwrap();
    assert_eq!(raw.link, Some(JsonLinkKind::Identity));
    assert!(!raw.nameable, "identityaddress stays raw");
    assert!(
        mark(&marks, "/vout/0/scriptPubKey/addresses/0")
            .unwrap()
            .nameable
    );
    assert!(!mark(&marks, "/vin/0/address").unwrap().nameable);
}

#[test]
fn json_pointers_escape_keys() {
    let marks = json_links(&json!({"a/b": {"c~d": "RXPzH3i4fZaaPFdkVHWKRDgXDqrFXErEGz"}}));
    assert_eq!(marks[0].pointer, "/a~1b/c~0d");
}

#[test]
fn dates_follow_carbide_field_rules() {
    let root = json!({
        "txid": "t", "height": 100, "blocktime": 1_700_000_000, "confirmations": 3,
        "locktime": 0, "expiryheight": 120, "time": 1_700_000_001,
        "vout": [{"scriptPubKey": {"crosschainexport": {
            "sourcesystemid": "iSource", "sourceheightstart": 50, "sourceheightend": 0}}}],
        "identity": {"contentmap": {"height": 5}, "timelock": 7, "version": 3, "flags": 0},
        "lock": {"timelock": 9, "version": 3, "flags": 2},
        "header": {"hash": "h", "height": 90}
    });
    let targets: HashMap<String, DateTarget> = date_targets(&root).into_iter().collect();
    assert_eq!(targets["/blocktime"], DateTarget::Time(1_700_000_000));
    assert_eq!(targets["/time"], DateTarget::Time(1_700_000_001));
    // The inclusion is dated by its own block time.
    assert_eq!(targets["/confirmations"], DateTarget::Time(1_700_000_000));
    assert_eq!(targets["/height"], DateTarget::Time(1_700_000_000));
    assert_eq!(
        targets["/expiryheight"],
        DateTarget::Height {
            system: String::new(),
            height: 120
        }
    );
    assert!(!targets.contains_key("/locktime"), "zero is not a height");
    assert_eq!(
        targets["/vout/0/scriptPubKey/crosschainexport/sourceheightstart"],
        DateTarget::Height {
            system: "iSource".into(),
            height: 50
        }
    );
    assert!(
        !targets.contains_key("/identity/contentmap/height"),
        "user content is not dated"
    );
    assert_eq!(
        targets["/identity/timelock"],
        DateTarget::Height {
            system: String::new(),
            height: 7
        }
    );
    assert!(
        !targets.contains_key("/lock/timelock"),
        "a relative delay stays a count"
    );
    assert!(
        !targets.contains_key("/header/height"),
        "a hash without a time names its own branch"
    );
}

#[test]
fn a_large_locktime_is_a_timestamp() {
    let root = json!({"locktime": 1_600_000_000u64});
    assert_eq!(
        date_target(&root, "/locktime"),
        Some(DateTarget::Time(1_600_000_000))
    );
}

// ---------------------------------------------------------------- live

const CONVERSION: &str = "42f1195338518ec1bb4a08d6d156bbcbb67454fd68cbd3b92da7c5b96ea6824c";
const NOTARIZATION: &str = "45e56e40dc2d03ba79ec8a8d02cf11de6be431e04f27896178d9ae1cc5a61d09";

#[tokio::test]
#[ignore]
async fn live_transaction_overview() {
    let explorer = super::super::tests::live_explorer();
    let tx = explorer.transaction(CONVERSION.into()).await.unwrap();
    println!(
        "status {:?} {} mempool {} api {} fee {} ({:?}) delta {:?}",
        tx.status,
        tx.status_text,
        tx.in_mempool,
        tx.from_public_api,
        tx.accounting.fee_label,
        tx.accounting.fee_kind,
        tx.accounting.public_native_delta
    );
    println!(
        "inputs {} | outputs {}",
        tx.accounting.inputs.label, tx.accounting.outputs.label
    );
    for input in &tx.inputs {
        println!(
            "  in {} {:?} {:?} when spent {:?} {:?}",
            input.index,
            input.value_sats,
            input.annotations,
            input.confirmations_when_spent,
            input.sequence
        );
    }
    for output in &tx.outputs {
        println!(
            "  out {} {:?} {:?} {:?} {:?} refs {}",
            output.index,
            output.value_sats,
            output.tags,
            output.annotations,
            output.badges,
            output.references.len()
        );
    }
    assert_eq!(tx.status, TransactionStatus::Confirmed);
    assert_eq!(tx.accounting.fee_kind, FeeKind::ProtocolAccounting);
    assert!(tx.accounting.show_public_delta);
    assert_eq!(tx.trace_roots.len(), 2);

    let notarization = explorer.transaction(NOTARIZATION.into()).await.unwrap();
    println!(
        "notarization context {:?} tags {:?}",
        notarization.notarization_context, notarization.tags
    );
    assert!(!notarization.witness_roots.is_empty());

    let block = explorer.block("4226272".into()).await.unwrap();
    let coinbase = explorer
        .transaction(block.transactions[0].txid.clone())
        .await
        .unwrap();
    println!(
        "coinbase {} fee {} badges {:?}",
        coinbase.status_text,
        coinbase.accounting.fee_label,
        coinbase
            .outputs
            .iter()
            .map(|o| o.badges.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(coinbase.accounting.fee_kind, FeeKind::NotApplicable);
}

#[tokio::test]
#[ignore]
async fn live_protocol_links() {
    let explorer = super::super::tests::live_explorer();
    let report = explorer.protocol_links(NOTARIZATION.into()).await.unwrap();
    for link in &report.links {
        println!(
            "  {} out {} -> {}:{} [{:?}] reverse {}",
            link.label,
            link.output,
            &link.target_txid[..12],
            link.target_output,
            link.status,
            link.reverse
        );
    }
    println!(
        "reverse {} unavailable {:?}",
        report.reverse_count, report.reverse_unavailable
    );
    assert!(report.links.iter().filter(|link| !link.reverse).count() >= 2);
    assert!(report.links.iter().all(|link| link.status.is_some()));
}

#[tokio::test]
#[ignore]
async fn live_trace_with_progress_and_pool() {
    let explorer = std::sync::Arc::new(super::super::tests::live_explorer());
    let token = 77;
    let watcher = {
        let explorer = explorer.clone();
        tokio::spawn(async move {
            let mut seen = 0;
            for _ in 0..600 {
                if explorer.trace_progress(token).is_some() {
                    seen += 1;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            seen
        })
    };
    let plan = TracePlan {
        request: TraceRequest::Initial,
        target: None,
        known_imports: Vec::new(),
    };
    let trace = explorer
        .trace_transfer(CONVERSION.into(), 0, plan, token)
        .await
        .unwrap();
    println!(
        "trace {} {:?} searching {}",
        trace.verdict, trace.confidence, trace.searching
    );
    for leg in &trace.legs {
        println!(
            "  leg {} fee budget {:?} rates {:?} pool {:?}",
            leg.stage, leg.transfer_fee_budget, leg.rates, leg.pool_inspection
        );
    }
    assert_eq!(trace.confidence, Confidence::Verified);
    assert!(
        explorer.trace_progress(token).is_none(),
        "the slot is cleared"
    );
    if let Some(index) = trace
        .legs
        .iter()
        .position(|leg| leg.pool_inspection.is_some())
    {
        let comparison = explorer
            .pool_comparison(CONVERSION.into(), 0, index as u32)
            .await
            .unwrap();
        println!("pool {:?}", comparison);
        assert!(!comparison.changes.is_empty());
    }
    watcher.abort();
}

#[tokio::test]
#[ignore]
async fn live_witnesses_and_json_marks() {
    let explorer = super::super::tests::live_explorer();
    let tx = explorer.transaction(NOTARIZATION.into()).await.unwrap();
    let root = tx.witness_roots.first().unwrap().output;
    let report = explorer
        .witnesses(NOTARIZATION.into(), root, false)
        .await
        .unwrap();
    println!(
        "signatures {} sets {} issues {:?}",
        report.signatures.len(),
        report.multipart_sets,
        report.issues
    );
    for signature in &report.signatures {
        println!(
            "  {} at {:?} keys {} origins {}",
            signature.identity,
            signature.height,
            signature.key_count,
            signature.origins.len()
        );
    }
    assert!(report.json.contains("\"report\""));
    let refreshed = explorer
        .witnesses(NOTARIZATION.into(), root, true)
        .await
        .unwrap();
    assert_eq!(refreshed.signatures.len(), report.signatures.len());

    let marks = explorer.json_marks(tx.json.clone(), true).await;
    let dated = marks.iter().filter(|mark| mark.date.is_some()).count();
    let linked = marks.iter().filter(|mark| mark.link.is_some()).count();
    println!("json marks {} dated {dated} linked {linked}", marks.len());
    assert!(dated > 0 && linked > 0);
}

#[tokio::test]
#[ignore]
async fn live_block_detail() {
    let explorer = super::super::tests::live_explorer();
    let block = explorer.block("4226272".into()).await.unwrap();
    println!(
        "{} {:?} {:?} loaded {} tags {:?}",
        block.production,
        block.chain_status,
        block.on_active_chain,
        block.bodies_loaded,
        block.combined_tags
    );
    assert!(block.on_active_chain);
    assert_eq!(block.chain_status.as_deref(), Some("Active"));
    assert_eq!(block.bodies_loaded as usize, block.transactions.len());
}

// ------------------------------------------------------------ session store

#[test]
fn a_loaded_transaction_that_references_this_one_is_a_reverse_link() {
    let system = SystemId::try_from(NATIVE.to_string()).unwrap();
    let export_txid = "1".repeat(64);
    let import = json!({"txid":"2".repeat(64),"vout":[{"scriptPubKey":{"crosschainimport":{
        "exporttxid":export_txid,"exporttxout":0,"sourcesystemid":NATIVE}}}]});
    assert!(session_reverse_links(&system, &export_txid).is_empty());
    remember_transaction(&system, &import);
    let links = session_reverse_links(&system, &export_txid);
    assert_eq!(links.len(), 1);
    assert!(links[0].field.starts_with("reverse:"));
    assert_eq!(links[0].target.txid, "2".repeat(64));
    // The carrier itself is never its own reverse link.
    assert!(session_reverse_links(&system, &"2".repeat(64)).is_empty());
}

#[test]
fn block_tags_use_only_loaded_sources() {
    let source = json!({"txid":"src-a","vout":[{"n":0,"valueSat":1000,"scriptPubKey":{
        "type":"cryptocondition","reserveoutput":{"currencyvalues":{"iToken11111111111111111111111111111":3}}}}]});
    let raw = json!({"txid":"t","vin":[{"txid":"src-a","vout":0}],
        "vout":[{"n":0,"valueSat":900,"scriptPubKey":{"type":"cryptocondition",
            "reserveoutput":{"currencyvalues":{"iToken11111111111111111111111111111":1}}}}]});
    let loaded = HashMap::from([("src-a".to_string(), source)]);
    let context = loaded_context(&raw, &loaded, Some(NATIVE.into()));
    assert!(context.complete);
    assert!(display_tags(&raw, &context).contains(&"Consumption Burn".to_string()));
    let empty = loaded_context(&raw, &HashMap::new(), Some(NATIVE.into()));
    assert!(!empty.complete);
    assert!(!display_tags(&raw, &empty).contains(&"Consumption Burn".to_string()));
}

#[tokio::test]
#[ignore]
async fn live_reverse_link_appears_after_opening_both_ends() {
    let explorer = super::super::tests::live_explorer();
    forget_session();
    let carrier = explorer.transaction(NOTARIZATION.into()).await.unwrap();
    let forward = carrier
        .links
        .iter()
        .find(|link| link.target_system == NATIVE)
        .expect("a forward link on this chain")
        .clone();
    explorer
        .transaction(forward.target_txid.clone())
        .await
        .unwrap();
    let report = explorer
        .protocol_links(forward.target_txid.clone())
        .await
        .unwrap();
    for link in &report.links {
        println!(
            "  {} -> {}:{} {:?}",
            link.label,
            &link.target_txid[..12],
            link.target_output,
            link.status
        );
    }
    assert!(
        report
            .links
            .iter()
            .any(|link| link.reverse && link.target_txid == NOTARIZATION),
        "the reverse link from the session store"
    );
    // Without the session store, the answers recorded in the cache give the same link.
    forget_session();
    let report = explorer
        .protocol_links(forward.target_txid.clone())
        .await
        .unwrap();
    println!(
        "from the cache: {} reverse, unavailable {:?}",
        report.reverse_count, report.reverse_unavailable
    );
    assert!(
        report
            .links
            .iter()
            .any(|link| link.reverse && link.target_txid == NOTARIZATION),
        "the reverse link from the cache"
    );
}
