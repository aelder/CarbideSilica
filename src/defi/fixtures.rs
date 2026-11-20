//! Captured chain transactions, with independently recorded expected receipts.
use super::*;

const EXPECTED: &str = include_str!("../../tests/fixtures/defi-refunds/verification.json");
const REFUNDS: [&str; 5] = [
    include_str!("../../tests/fixtures/defi-refunds/refund-bots-launch.json"),
    include_str!("../../tests/fixtures/defi-refunds/refund-bots.json"),
    include_str!("../../tests/fixtures/defi-refunds/refund-superbasket.json"),
    include_str!("../../tests/fixtures/defi-refunds/refund-tbtc.json"),
    include_str!("../../tests/fixtures/defi-refunds/refund-eurc.json"),
];

fn captured_refund(raw: &str) {
    let capture: Value = serde_json::from_str(raw).unwrap();
    let expected: Value = serde_json::from_str(EXPECTED).unwrap();
    let record = &capture["record"];
    let transaction = &capture["transaction"];
    let expected = expected["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["txid"] == transaction["txid"])
        .unwrap();
    let batch = record["transfers"].as_array().unwrap();
    let import = &record["import"];
    let (start, region) = generated_region(
        transaction,
        import,
        record["importvout"].as_u64().unwrap() as u32,
    )
    .unwrap();
    assert_eq!(start, 2);
    let export = &capture["export_transaction"];
    assert_eq!(export["txid"], import["exporttxid"]);
    let export_info = &export["vout"][import["exporttxout"].as_u64().unwrap() as usize]["scriptPubKey"]
        ["crosschainexport"];
    assert_eq!(export_info["hashtransfers"], import["hashtransfers"]);
    assert_eq!(
        export_info["numinputs"].as_u64().unwrap() as usize,
        batch.len()
    );
    for receipt in expected["receipts"].as_array().unwrap() {
        let ordinal = receipt["ordinal"].as_u64().unwrap() as usize;
        let source = capture["source_transactions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|source| source["txid"] == receipt["source_txid"])
            .unwrap();
        let vout = receipt["source_vout"].as_u64().unwrap() as usize;
        assert!(transfer_matches(
            &source["vout"][vout]["scriptPubKey"]["reservetransfer"],
            &batch[ordinal]
        ));
        let input = export_info["firstinput"].as_u64().unwrap() as usize + ordinal;
        assert_eq!(export["vin"][input]["txid"], source["txid"]);
        assert_eq!(export["vin"][input]["vout"].as_u64(), Some(vout as u64));
        let proof = replay::replay_supported_prefix(
            batch,
            ordinal,
            import,
            &record["importnotarization"],
            import["sourcesystemid"].as_str().unwrap(),
            region,
        )
        .unwrap_or_else(|| panic!("{} ordinal {ordinal} was not replayed", transaction["txid"]));
        assert!(proof.kind.is_refund());
        assert_eq!(
            (start + proof.offset) as u64,
            receipt["result_vout"].as_u64().unwrap()
        );
        let amount: Value = serde_json::from_str(receipt["amount"].as_str().unwrap()).unwrap();
        let expected_sats = replay::amount_sats(&amount).unwrap();
        if receipt["currency"] == import["sourcesystemid"] {
            assert_eq!(proof.satoshis, Some(expected_sats));
        } else {
            let actual = &region[proof.offset]["scriptPubKey"]["reserveoutput"]["currencyvalues"]
                [receipt["currency"].as_str().unwrap()];
            assert_eq!(replay::amount_sats(actual), Some(expected_sats));
        }
        assert_eq!(
            region[proof.offset]["scriptPubKey"]["addresses"][0],
            receipt["destination"]
        );
        let root = OutpointRef {
            system_id: crate::world::SystemId::try_from(
                import["sourcesystemid"].as_str().unwrap().to_string(),
            )
            .unwrap(),
            txid: source["txid"].as_str().unwrap().into(),
            vout: vout as u32,
        };
        let mut summary = LegSummary::new(root, &batch[ordinal]);
        summary.imported(
            OutpointRef {
                system_id: summary.source.system_id.clone(),
                txid: transaction["txid"].as_str().unwrap().into(),
                vout: 0,
            },
            record,
        );
        summary.replayed(
            proof,
            OutpointRef {
                system_id: summary.source.system_id.clone(),
                txid: transaction["txid"].as_str().unwrap().into(),
                vout: (start + proof.offset) as u32,
            },
            &region[proof.offset],
        );
        assert_eq!(summary.stage, Stage::Refunded);
        assert!(summary.conversion_fee.is_none());
        assert!(summary.execution_rates().is_empty());
        assert_eq!(
            summary.receipt.as_ref().unwrap().amounts[0].satoshis,
            expected_sats
        );

        // A later matching recipient must never rescue a changed prefix output.
        let mut changed = region.to_vec();
        changed[0]["valueSat"] = json!(1);
        assert!(
            replay::replay_supported_prefix(
                batch,
                ordinal,
                import,
                &record["importnotarization"],
                import["sourcesystemid"].as_str().unwrap(),
                &changed
            )
            .is_none()
        );
    }
}

#[test]
fn captured_late_failed_launch_refunds_replay_native_and_reserve_receipts() {
    for raw in &REFUNDS[1..] {
        captured_refund(raw);
    }
}

#[test]
fn captured_bots_preconversions_refund_all_five_assets_to_the_identity() {
    captured_refund(REFUNDS[0]);
}

#[test]
fn captured_identity_refund_rejects_changed_state_routing_script_and_reserve_prefix() {
    let fixture: Value = serde_json::from_str(REFUNDS[0]).unwrap();
    let record = &fixture["record"];
    let (_, region) = generated_region(&fixture["transaction"], &record["import"], 0).unwrap();
    let replay = |record: &Value, outputs: &[Value]| {
        replay::replay_supported_prefix(
            record["transfers"].as_array().unwrap(),
            4,
            &record["import"],
            &record["importnotarization"],
            "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV",
            outputs,
        )
    };
    for (path, value) in [
        ("/importnotarization/currencystate/flags", json!(49)),
        ("/importnotarization/currencystate/flags", json!(7)),
        (
            "/importnotarization/currencystate/currencyid",
            json!("i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV"),
        ),
        (
            "/import/sourcesystemid",
            json!("i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X"),
        ),
        ("/transfers/0/flags", json!(129)),
    ] {
        let mut changed = record.clone();
        *changed.pointer_mut(path).unwrap() = value;
        assert!(replay(&changed, region).is_none(), "accepted {path}");
    }
    for (index, path, value) in [
        (0, "/scriptPubKey/hex", json!("050403000000cc00")),
        (
            0,
            "/scriptPubKey/addresses/0",
            json!("i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV"),
        ),
        (1, "/scriptPubKey/reqSigs", json!(2)),
        (1, "/scriptPubKey/hex", json!("00")),
        (
            1,
            "/scriptPubKey/reserveoutput/currencyvalues/i9oCSqKALwJtcv49xUKS2U2i79h1kX6NEY",
            json!(124.17411861),
        ),
    ] {
        let mut changed = region.to_vec();
        *changed[index].pointer_mut(path).unwrap() = value;
        assert!(
            replay(record, &changed).is_none(),
            "accepted output {index} {path}"
        );
    }
}

#[test]
fn captured_dai_burn_preserves_supply_and_separates_reserve_custody_from_recipient_outputs() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/defi-refunds/burn-ethereum-dai.json"
    ))
    .unwrap();
    let record = &fixture["record"];
    let transfer = &record["transfers"][0];
    let pool = record["import"]["importcurrencyid"].as_str().unwrap();
    let dai = "iGBs4DWztRNvNEJBt4mqHszLxfKTNHTkhM";
    let notarization = &record["importnotarization"];
    let reference = OutpointRef {
        system_id: crate::world::SystemId::try_from(
            "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV".to_string(),
        )
        .unwrap(),
        txid: notarization["prevnotarizationtxid"]
            .as_str()
            .unwrap()
            .into(),
        vout: notarization["prevnotarizationout"].as_u64().unwrap() as u32,
    };
    let previous =
        pool::previous_state(pool, &reference, &fixture["previous_transaction"]).unwrap();
    let state = &notarization["currencystate"];
    let reserve = |state: &Value| {
        let value = state["reservecurrencies"]
            .as_array()
            .unwrap()
            .iter()
            .find(|reserve| reserve["currencyid"] == dai)
            .unwrap();
        replay::amount_sats(&value["reserves"]).unwrap()
    };
    let contribution = replay::amount_sats(&transfer["currencyvalues"][dai]).unwrap()
        + replay::amount_sats(&transfer["fees"]).unwrap();
    assert_eq!(contribution, 344_911_864_296);
    assert_eq!(reserve(state) - reserve(&previous.state), contribution);
    assert_eq!(
        replay::amount_sats(&state["supply"]),
        replay::amount_sats(&previous.state["supply"])
    );
    assert_eq!(
        fixture["transaction"]["vout"][5]["scriptPubKey"]["crosschainexport"]["transfers"][0],
        *transfer
    );
    let deposit = &fixture["transaction"]["vout"][9]["scriptPubKey"]["reservedeposit"];
    assert_eq!(deposit["controllingcurrencyid"], pool);
    assert_eq!(
        replay::amount_sats(&deposit["currencyvalues"][dai]),
        Some(contribution)
    );
    let (start, region) = generated_region(&fixture["transaction"], &record["import"], 0).unwrap();
    assert_eq!((start, region.len()), (6, 2));
    assert!(
        replay::replay_supported_prefix(
            record["transfers"].as_array().unwrap(),
            0,
            &record["import"],
            notarization,
            "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV",
            region
        )
        .is_none(),
        "a reserve burn is not a customer receipt"
    );
    assert!(
        crate::domain::transaction_output_tags(&fixture["transaction"])
            .iter()
            .any(|tag| tag.kind == crate::domain::OutputTagKind::Burn)
    );
    let mut no_supplement = fixture["transaction"].clone();
    no_supplement["vout"][5]["scriptPubKey"]["crosschainexport"]["issupplemental"] = json!(false);
    assert!(
        !crate::domain::transaction_output_tags(&no_supplement)
            .iter()
            .any(|tag| tag.kind == crate::domain::OutputTagKind::Burn)
    );
    let mut no_burn = fixture["transaction"].clone();
    let transfer = &mut no_burn["vout"][5]["scriptPubKey"]["crosschainexport"]["transfers"][0];
    transfer["flags"] = json!(1);
    transfer["burnchangeprice"] = json!(false);
    assert!(
        !crate::domain::transaction_output_tags(&no_burn)
            .iter()
            .any(|tag| tag.kind == crate::domain::OutputTagKind::Burn)
    );
    no_burn["vout"][5]["scriptPubKey"]["crosschainexport"]["transfers"][0]["burnchangeprice"] =
        json!(true);
    assert!(
        !crate::domain::transaction_output_tags(&no_burn)
            .iter()
            .any(|tag| tag.kind == crate::domain::OutputTagKind::Burn),
        "supplemental evidence needs a valid numeric burn flag, not a contradictory display hint"
    );
}
