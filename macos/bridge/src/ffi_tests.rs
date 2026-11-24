use super::*;

fn explorer(api: Option<&str>) -> Explorer {
    Explorer::open(ConnectionOptions {
        chain: "VRSC".into(),
        testnet: false,
        // A config path that cannot exist keeps tests away from a real daemon's credentials.
        config_file: Some("/nonexistent/carbide-test/VRSC.conf".into()),
        api_endpoint: api.map(Into::into),
    })
    .unwrap()
}

#[test]
fn opens_without_a_daemon_config() {
    let explorer = explorer(None);
    let info = explorer.connection();
    assert_eq!(info.chain, "VRSC");
    assert_eq!(info.daemon_url, "http://127.0.0.1:27486");
    assert!(!info.config_found);
    assert!(!info.has_credentials);
    assert_eq!(info.api_endpoint, None);
}

#[test]
fn rejects_api_urls_with_credentials() {
    let result = Explorer::open(ConnectionOptions {
        chain: "VRSC".into(),
        testnet: false,
        config_file: None,
        api_endpoint: Some("https://user:secret@example.com".into()),
    });
    assert!(result.is_err());
}

#[tokio::test]
async fn resolves_queries_that_need_no_daemon() {
    let explorer = explorer(None);
    let route = |query: &str| explorer.resolve(query.to_string());
    assert_eq!(
        route("4226272").await.unwrap(),
        Route::Block {
            id: "4226272".into()
        }
    );
    assert_eq!(
        route("vETH@").await.unwrap(),
        Route::Identity {
            name: "vETH@".into()
        }
    );
    assert_eq!(
        route("vETH@@").await.unwrap(),
        Route::Currency {
            name: "vETH".into()
        }
    );
    assert_eq!(
        route("RXPzH3i4fZaaPFdkVHWKRDgXDqrFXErEGz").await.unwrap(),
        Route::Address {
            address: "RXPzH3i4fZaaPFdkVHWKRDgXDqrFXErEGz".into()
        }
    );
    assert!(route("???").await.is_err());
}

#[test]
fn reads_native_values_in_satoshis() {
    use serde_json::json;
    assert_eq!(
        sats(&json!({"valueSat": 300046564_i64, "value": 1.0})),
        Some(300046564)
    );
    assert_eq!(sats(&json!({"value": 3.00046564})), Some(300046564));
    assert_eq!(sats(&json!({})), None);
}

#[test]
fn shows_the_daemon_message_for_rpc_errors() {
    let error = anyhow::anyhow!(
        r#"RPC error from getidentity: {{"code":-5,"message":"Identity not found"}}"#
    );
    assert_eq!(readable(&error), "Identity not found");
}

/// Live smoke test. Run with a JSON-RPC endpoint:
/// `CARBIDE_TEST_API=https://… cargo test live -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn live_lookups_through_an_api() {
    let api = std::env::var("CARBIDE_TEST_API").expect("set CARBIDE_TEST_API");
    let explorer = explorer(Some(&api));
    let status = explorer.chain_status().await.unwrap();
    println!("status: {status:?}");
    assert_eq!(status.source, DataSource::PublicApi);

    let blocks = explorer.recent_blocks(None, 5).await.unwrap();
    println!("recent: {blocks:?}");
    assert_eq!(blocks.len(), 5);

    let block = explorer.block("4226272".into()).await.unwrap();
    println!(
        "block: {:?} {:?} {:?}",
        block.row, block.fields, block.transactions
    );
    let hash = block.row.hash.clone();
    assert_eq!(
        explorer.resolve(hash.clone()).await.unwrap(),
        Route::Block { id: hash }
    );

    let txid = block.transactions[0].txid.clone();
    assert_eq!(
        explorer.resolve(txid.clone()).await.unwrap(),
        Route::Transaction { txid: txid.clone() }
    );
    let tx = explorer.transaction(txid).await.unwrap();
    println!("tx: tags={:?} inputs={:?}", tx.tags, tx.inputs);
    for output in &tx.outputs {
        println!(
            "  out {} {:?} {:?} {:?} {:?}",
            output.index, output.value_sats, output.addresses, output.tags, output.currency_values
        );
    }

    let identity = explorer.identity("vETH@".into()).await.unwrap();
    println!(
        "identity: {:?}",
        IdentityDetail {
            json: String::new(),
            ..identity
        }
    );
    let currency = explorer.currency("vETH".into()).await.unwrap();
    println!(
        "currency: {:?}",
        CurrencyDetail {
            json: String::new(),
            ..currency
        }
    );
    let bridge = explorer.currency("Bridge.vETH".into()).await.unwrap();
    println!(
        "basket: supply={:?} reserves={:?}",
        bridge.supply, bridge.reserves
    );

    match explorer
        .address("RXPzH3i4fZaaPFdkVHWKRDgXDqrFXErEGz".into())
        .await
    {
        Ok(address) => println!(
            "address: {:?}",
            AddressDetail {
                json: String::new(),
                ..address
            }
        ),
        Err(error) => println!("address error: {error}"),
    }
    println!(
        "missing identity: {:?}",
        explorer
            .identity("no-such-identity-zzzz@".into())
            .await
            .err()
    );
    println!("date: {:?}", explorer.resolve("2026-09-06".into()).await);
    let names = explorer
        .names(vec![
            "i5v3h9FWVdRFbNHU7DfcpGykQjRaHtMqu7".into(),
            "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV".into(),
        ])
        .await;
    println!("names: {names:?}");
    let history = explorer.identity_history("vETH@".into()).await.unwrap();
    for version in &history {
        println!(
            "  v #{} {:?} {:?}",
            version.height, version.time, version.changes
        );
    }
    let tip = explorer.chain_status().await.unwrap().blocks;
    let points = explorer
        .currency_history("Bridge.vETH".into(), tip - 43200, tip, 6)
        .await
        .unwrap();
    for point in &points {
        println!(
            "  p #{} {:?} supply {:?} reserves {}",
            point.height,
            point.time,
            point.supply,
            point.reserves.len()
        );
    }
}

#[test]
fn reads_a_peer_and_drops_its_inflight_list() {
    use serde_json::json;
    let peer = peer_row(&json!({
        "id": 1, "addr": "203.0.113.18:27485", "inbound": false, "pingtime": 0.038,
        "synced_blocks": 388290, "synced_headers": 1604000, "startingheight": 4336719,
        "bytessent": 10, "bytesrecv": 20, "subver": "/MagicBean:2.0.73/", "conntime": 1795540710,
        "inflight": [388293, 388297]
    }));
    assert_eq!(peer.synced_headers, Some(1604000));
    assert_eq!(peer.row.address, "203.0.113.18:27485");
    assert!((peer.row.ping_ms.unwrap() - 38.0).abs() < 1e-9);
    assert_eq!(peer.row.synced_blocks, Some(388290));
    assert!(!peer.row.json.contains("inflight"));
}

#[test]
fn describes_identity_changes_in_plain_words() {
    use serde_json::json;
    let first = json!({"primaryaddresses": ["RA"], "minimumsignatures": 1, "flags": 0,
        "revocationauthority": "iSelf", "recoveryauthority": "iSelf", "timelock": 0});
    assert_eq!(identity_changes(None, &first), ["Registered"]);

    let second = json!({"primaryaddresses": ["RA", "RB", "RC"], "minimumsignatures": 2, "flags": 1,
        "revocationauthority": "iSelf", "recoveryauthority": "iVault", "timelock": 0});
    assert_eq!(
        identity_changes(Some(&first), &second),
        [
            "Signing keys changed (2 of 3 needed)",
            "Recovery authority changed",
            "Currency defined"
        ]
    );
    assert_eq!(
        identity_changes(Some(&second), &second),
        ["Updated, with no change to control"]
    );
    let revoked = json!({"primaryaddresses": ["RA", "RB", "RC"], "minimumsignatures": 2, "flags": 0x8001,
        "revocationauthority": "iSelf", "recoveryauthority": "iVault", "timelock": 0});
    assert_eq!(identity_changes(Some(&second), &revoked), ["Revoked"]);
}

#[test]
fn friendly_names_drop_the_chain_root() {
    let explorer = explorer(None);
    assert_eq!(
        explorer.short_name("Verus Coin Foundation.VRSC@"),
        "Verus Coin Foundation@"
    );
    assert_eq!(explorer.short_name("DAI.vETH@"), "DAI.vETH@");
    assert_eq!(
        explorer
            .cached_name("i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV")
            .as_deref(),
        Some("VRSC")
    );
}
