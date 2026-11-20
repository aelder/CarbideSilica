use super::*;
use serde_json::json;

fn memory() -> StoreConnection {
    let store = StoreConnection {
        connection: Connection::open_in_memory().unwrap(),
        _ownership: None,
        writes: Default::default(),
    };
    store
        .connection
        .execute_batch("pragma foreign_keys=ON")
        .unwrap();
    store.migrate().unwrap();
    store
}

#[test]
fn currency_lists_cache_definitions_without_turning_list_states_into_current_snapshots() {
    let cache = memory();
    let id = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
    let response = json!({"result":[{"currencydefinition":{
        "currencyid":id,"name":"VRSC","fullyqualifiedname":"VRSC","options":256},
        "bestheight":100,"bestcurrencystate":{"supply":123}}]});
    cache
        .put_raw_rpc("fixture", id, "listcurrencies", "[]", &response.to_string())
        .unwrap();
    let definitions = cache.currency_definitions(id).unwrap();
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].1, "VRSC");
    assert_eq!(definitions[0].2["options"], 256);
    assert!(cache.currency_snapshots(id, id).unwrap().is_empty());
    assert!(
        cache
            .raw_rpc_response("fixture", id, "listcurrencies", "[]")
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn reconciliation_commits_atomically_retains_inclusions_and_survives_restart() {
    use crate::reconciliation::{Header, State, plan};
    let root =
        std::env::temp_dir().join(format!("reorg-store-{}", crate::rpc::next_request_order()));
    let path = root.join("mainnet.sqlite3");
    let store = StoreConnection::open(&path).unwrap();
    let a = Header {
        hash: "a".into(),
        height: 0,
        parent: None,
        time: 0,
    };
    let b = Header {
        hash: "b".into(),
        height: 1,
        parent: Some("a".into()),
        time: 60,
    };
    let c = Header {
        hash: "c".into(),
        height: 1,
        parent: Some("a".into()),
        time: 61,
    };
    let headers = [a.clone(), b.clone(), c.clone()];
    let fetch = |hash: String| {
        let result = headers
            .iter()
            .find(|h| h.hash == hash)
            .cloned()
            .context("missing");
        async move { result }
    };
    store.reconciliation_state("iSystem", "local").unwrap();
    for h in [&b, &c] {
        store.put_raw_rpc("local","iSystem","getblock",&json!([h.hash,1]).to_string(),&json!({"result":{"hash":h.hash,"height":h.height,"time":h.time,"previousblockhash":"a","confirmations":1,"tx":["same"]}}).to_string()).unwrap();
    }
    assert_eq!(count(&store, "select count(*) from block_membership"), 0);
    let initial = plan(&State::default(), b.clone(), fetch).await.unwrap();
    store
        .commit_reconciliation("iSystem", "local", 100, 0, &initial)
        .unwrap();
    let changed = plan(&initial.state, c.clone(), fetch).await.unwrap();
    store.put_raw_rpc("local","iSystem","getidentity","[\"name\",1]",&json!({"result":{"fullyqualifiedname":"name@","blockheight":1,"identity":{"identityaddress":"iIdentity","name":"name"}}}).to_string()).unwrap();
    assert!(
        store
            .commit_reconciliation("iSystem", "local", 101, 0, &changed)
            .is_err()
    );
    assert_eq!(
        store
            .reconciliation_state("iSystem", "local")
            .unwrap()
            .tip(),
        Some(&b)
    );
    store
        .commit_reconciliation("iSystem", "local", 102, 1, &changed)
        .unwrap();
    assert_eq!(
        count(
            &store,
            "select count(*) from raw_rpc_cache where method='getidentity'"
        ),
        0
    );
    assert_eq!(
        count(&store, "select count(*) from identity_observations"),
        1
    );
    assert_eq!(
        count(
            &store,
            "select count(*) from transaction_inclusions where txid='same'"
        ),
        2
    );
    assert_eq!(
        count(
            &store,
            "select count(*) from transaction_inclusions where txid='same' and block_hash='c' and confirmed=1"
        ),
        1
    );
    assert_eq!(
        count(
            &store,
            "select count(*) from transaction_inclusions where block_hash='b' and confirmed=1"
        ),
        0
    );
    assert_eq!(
        count(
            &store,
            "select count(*) from membership_history where object_id='b' and state='orphan'"
        ),
        1
    );
    assert_eq!(
        count(&store, "select count(*) from pragma_foreign_key_check"),
        0
    );
    drop(store);
    let store = StoreConnection::open(&path).unwrap();
    let restored = store.reconciliation_state("iSystem", "local").unwrap();
    assert_eq!(restored.tip(), Some(&c));
    assert!(restored.detached.contains_key("b"));
    assert!(store.events().unwrap().iter().any(|event| {
        event.key.starts_with("reorg:iSystem:local:")
            && event.message.contains("Reorg")
            && event.observed_at > 0
    }));
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

fn observe(
    store: &StoreConnection,
    method: &str,
    parameters: Value,
    result: Value,
    order: u64,
) -> bool {
    store
        .observe(
            "local",
            "iSystem",
            method,
            &parameters.to_string(),
            &json!({"result":result}).to_string(),
            order,
            1,
        )
        .unwrap()
}

fn count(store: &StoreConnection, sql: &str) -> i64 {
    store.connection.query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn height_aliases_are_snapshots_and_headers_preserve_full_blocks() {
    let store = memory();
    let hash = "a".repeat(64);
    observe(
        &store,
        "getblock",
        json!(["42", 1]),
        json!({"hash":hash,"height":42,"confirmations":1,"tx":["tx"]}),
        1,
    );
    observe(
        &store,
        "getblockheader",
        json!([hash]),
        json!({"hash":hash,"height":42,"confirmations":2}),
        2,
    );
    assert!(
        store
            .raw_rpc_response("local", "iSystem", "getblock", "[\"42\",1]")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .bookmark_response("iSystem", "getblock", "[\"42\",1]", "42")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        count(
            &store,
            "select json_array_length(b.payload,'$.tx') from objects o join raw_blobs b on b.hash=o.payload_hash"
        ),
        1
    );
}

#[test]
fn immutable_transaction_body_deduplicates_observations_and_rejects_old_aliases() {
    let store = memory();
    let first = json!({"txid":"tx","confirmations":1,"blockhash":"one","height":1,"vout":[{"n":0,"spentTxId":"old","spentHeight":1}],"vin":[{"txid":"parent","vout":0}]});
    let mut next = first.clone();
    next["confirmations"] = json!(3);
    next["blockhash"] = json!("two");
    next["vout"][0]["spentTxId"] = json!("new");
    observe(
        &store,
        "getrawtransaction",
        json!(["tx", 1]),
        first.clone(),
        1,
    );
    observe(&store, "getrawtransaction", json!(["tx", 1]), next, 3);
    assert!(!observe(
        &store,
        "getrawtransaction",
        json!(["tx", true]),
        first,
        2
    ));
    assert_eq!(count(&store, "select count(*) from object_observations"), 1);
    assert_eq!(
        count(
            &store,
            "select count(*) from transaction_inclusions where confirmed=1 and block_hash='two'"
        ),
        1
    );
    assert_eq!(
        count(
            &store,
            "select count(*) from membership_history where kind='transaction'"
        ),
        2
    );
}

#[test]
fn retention_removes_diagnostics_and_unreferenced_blobs_but_keeps_evidence() {
    let store = memory();
    observe(
        &store,
        "getrawtransaction",
        json!(["tx", 1]),
        json!({"txid":"tx","vin":[{"txid":"parent","vout":0}]}),
        1,
    );
    observe(&store, "getrawmempool", json!([]), json!(["tx"]), 2);
    observe(&store, "getrawmempool", json!([]), json!([]), 3);
    assert_eq!(
        count(
            &store,
            "select count(*) from membership_history where kind='mempool' and state='absent'"
        ),
        1
    );
    assert_eq!(
        count(
            &store,
            "select count(*) from transaction_inclusions where confirmed=1"
        ),
        0
    );
    store.connection.execute_batch("update raw_rpc_cache set fetched_at=0; update membership_history set observed_at=0 where kind='mempool'; insert into raw_blobs values('unused','{}');").unwrap();
    let tx = store.connection.unchecked_transaction().unwrap();
    StoreConnection::retain_snapshots(&tx).unwrap();
    StoreConnection::collect_blobs(&tx, 1024).unwrap();
    tx.commit().unwrap();
    assert_eq!(count(&store, "select count(*) from raw_rpc_cache"), 1);
    assert_eq!(
        count(
            &store,
            "select count(*) from membership_history where kind='mempool'"
        ),
        0
    );
    assert_eq!(
        count(&store, "select count(*) from raw_blobs where hash='unused'"),
        0
    );
    assert_eq!(count(&store, "select count(*) from relationships"), 1);
    assert_eq!(
        count(&store, "select count(*) from pragma_foreign_key_check"),
        0
    );
}

#[test]
fn identical_currency_payload_preserves_each_observing_block() {
    let store = memory();
    let raw = json!({"currencyid":"iCurrency","name":"Currency","bestheight":9000,"bestcurrencystate":{"supply":1}});
    for height in [10, 11] {
        observe(
            &store,
            "getcurrency",
            json!(["iCurrency"]),
            raw.clone(),
            height,
        );
        store
            .qualify_currency(
                "local",
                "iSystem",
                &raw,
                &BlockContext {
                    height,
                    hash: height.to_string(),
                },
                height,
            )
            .unwrap();
    }
    assert_eq!(
        count(
            &store,
            "select count(*) from currency_states where observed_block_hash is not null"
        ),
        2
    );
    assert_eq!(
        count(&store, "select count(*) from currency_definitions"),
        1
    );
    assert_eq!(
        count(
            &store,
            "select count(*) from currency_states where reported_height=9000 and observed_height in(10,11)"
        ),
        2
    );
}

#[tokio::test]
async fn writer_rechecks_generation_reset_epoch_and_network() {
    use crate::{
        sources::{ConnectionState, Control},
        world::Network,
    };
    let root = std::env::temp_dir().join(format!(
        "storage-receipts-{}-{}",
        std::process::id(),
        crate::rpc::next_request_order()
    ));
    let path = root.join("mainnet.sqlite3");
    let cache = Cache::open(&path).unwrap();
    let control = Arc::new(Control::default());
    let receipt = Receipt {
        branch_epoch: 0,
        control: control.clone(),
        generation: control.generation(),
        order: 1,
        epoch: cache.epoch(),
        once: false,
    };
    let write = |receipt, network| {
        cache.observe(
            network,
            receipt,
            "local".into(),
            "iSystem".into(),
            "getinfo".into(),
            "[]".into(),
            "{\"result\":{\"blocks\":1}}".into(),
        )
    };
    assert!(write(receipt.clone(), Network::Mainnet).await.unwrap());
    control.set(ConnectionState::Disabled);
    control.set(ConnectionState::Enabled);
    assert!(
        !write(
            Receipt {
                order: 2,
                ..receipt.clone()
            },
            Network::Mainnet
        )
        .await
        .unwrap()
    );
    let current = Receipt {
        generation: control.generation(),
        order: 3,
        ..receipt
    };
    assert!(write(current.clone(), Network::Testnet).await.is_err());
    control.branch_epoch.store(1, Ordering::Release);
    assert!(!write(current.clone(), Network::Mainnet).await.unwrap());
    let current = Receipt {
        branch_epoch: 1,
        ..current
    };
    Cache::clear_at(&path).unwrap();
    assert!(!write(current, Network::Mainnet).await.unwrap());
    drop(cache);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn schema_three_upgrade_preserves_existing_objects() {
    let root = std::env::temp_dir().join(format!(
        "reorg-upgrade-{}",
        crate::rpc::next_request_order()
    ));
    let path = root.join("mainnet.sqlite3");
    let store = StoreConnection::open(&path).unwrap();
    observe(
        &store,
        "getrawtransaction",
        json!(["tx", 1]),
        json!({"txid":"tx"}),
        1,
    );
    store
        .connection
        .execute_batch(
            "drop table reconciled_chain; drop table reconciliation_headers; pragma user_version=3",
        )
        .unwrap();
    drop(store);
    let store = StoreConnection::open(&path).unwrap();
    assert_eq!(count(&store, "pragma user_version"), SCHEMA_VERSION);
    assert_eq!(count(&store, "select count(*) from objects"), 1);
    assert_eq!(
        count(&store, "select count(*) from pragma_foreign_key_check"),
        0
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn schema_four_upgrades_identity_evidence_without_losing_objects() {
    let root = std::env::temp_dir().join(format!(
        "storage-identity-upgrade-{}-{}",
        std::process::id(),
        crate::rpc::next_request_order()
    ));
    let path = root.join("mainnet.sqlite3");
    let store = StoreConnection::open(&path).unwrap();
    observe(
        &store,
        "getrawtransaction",
        json!(["tx", 1]),
        json!({"txid":"tx"}),
        1,
    );
    store
        .connection
        .execute_batch(
            "drop table identity_evidence_edges;
         drop table identity_evidence_reports;
         drop table defi_lifecycle_edges;
         pragma user_version=4;",
        )
        .unwrap();
    drop(store);
    let store = StoreConnection::open(&path).unwrap();
    assert_eq!(count(&store, "pragma user_version"), SCHEMA_VERSION);
    assert_eq!(count(&store, "select count(*) from objects"), 1);
    assert_eq!(
        count(&store, "select count(*) from identity_evidence_reports"),
        0
    );
    assert_eq!(
        count(&store, "select count(*) from defi_lifecycle_edges"),
        0
    );
    assert_eq!(
        count(&store, "select count(*) from pragma_foreign_key_check"),
        0
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reset_rejects_active_writer_and_wrong_network_then_rebuilds_old_schema() {
    use crate::world::Network;
    let root = std::env::temp_dir().join(format!(
        "storage-reset-{}-{}",
        std::process::id(),
        crate::rpc::next_request_order()
    ));
    let path = root.join("mainnet.sqlite3");
    let cache = Cache::open(&path).unwrap();
    cache.bind_network(Network::Mainnet).unwrap();
    assert!(Cache::reset_at(&path, Network::Mainnet).is_err());
    drop(cache);
    let db = Connection::open(&path).unwrap();
    db.execute_batch("pragma user_version=2").unwrap();
    drop(db);
    assert!(Cache::reset_at(&path, Network::Testnet).is_err());
    Cache::reset_at(&path, Network::Mainnet).unwrap();
    let store = StoreConnection::open(&path).unwrap();
    assert_eq!(count(&store, "pragma user_version"), SCHEMA_VERSION);
    assert_eq!(
        count(&store, "select count(*) from pragma_foreign_key_check"),
        0
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn reverse_relationships_survive_restart_and_reproject_only_selected_carriers() {
    let root = std::env::temp_dir().join(format!(
        "storage-graph-{}-{}",
        std::process::id(),
        crate::rpc::next_request_order()
    ));
    let path = root.join("mainnet.sqlite3");
    let store = StoreConnection::open(&path).unwrap();
    let import: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/cross-chain/vdex-import.json"
    ))
    .unwrap();
    let export: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/cross-chain/vrsc-export.json"
    ))
    .unwrap();
    store
        .put_raw_rpc(
            "local",
            "iHog9UCTrn95qpUBFCZ7kKz7qWdMA8MQ6N",
            "getrawtransaction",
            "[]",
            &json!({"result":import}).to_string(),
        )
        .unwrap();
    observe(
        &store,
        "getrawtransaction",
        json!(["unrelated", 1]),
        json!({"txid":"unrelated"}),
        1,
    );
    store.connection.execute_batch("update objects set decoder_version=0; update relationships set label='obsolete decoder';").unwrap();
    let observations = count(&store, "select count(*) from object_observations");
    drop(store);
    let store = StoreConnection::open(&path).unwrap();
    let links = store
        .reverse_links(
            "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV",
            export["txid"].as_str().unwrap(),
        )
        .unwrap();
    assert_eq!(links.len(), 2);
    assert!(
        links
            .iter()
            .all(|link| link.target.txid == import["txid"]
                && !link.label.contains("obsolete decoder"))
    );
    assert_eq!(
        count(
            &store,
            "select decoder_version from objects where object_id='unrelated'"
        ),
        0
    );
    assert_eq!(
        count(&store, "select count(*) from object_observations"),
        observations
    );
    assert_eq!(
        count(&store, "select count(*) from pragma_foreign_key_check"),
        0
    );
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn investigation_preserves_source_system_and_unknown_membership() {
    use super::investigation::Investigation;
    let store = memory();
    for (system, source, hash, membership) in [
        ("iOne", "local", "a", "orphan"),
        ("iOne", "local", "b", "member"),
        ("iOne", "api", "a", "member"),
        ("iOther", "local", "a", "member"),
    ] {
        store
            .connection
            .execute(
                "insert into block_membership values(?1,?2,?3,10,?4,123)",
                params![system, source, hash, membership],
            )
            .unwrap();
        store
            .connection
            .execute(
                "insert into transaction_inclusions values(?1,?2,'same',?3,10,1,124)",
                params![system, source, hash],
            )
            .unwrap();
    }
    // Mempool observations have no inclusion, and a zero confirmation flag is not proof of orphaning.
    for hash in ["", "unknown"] {
        store
            .connection
            .execute(
                "insert into transaction_inclusions values('iOne','local','same',?1,null,0,125)",
                [hash],
            )
            .unwrap();
    }
    let subject = Investigation::Inclusions("same".into());
    let page = store.investigate("iOne", &subject, 0).unwrap();
    assert_eq!(page.rows.len(), 4);
    assert!(!page.more);
    assert_eq!(
        page.rows
            .iter()
            .filter(|r| r.membership == "member")
            .count(),
        2
    );
    assert!(
        page.rows
            .iter()
            .any(|r| r.hash == "a" && r.source == "local" && r.membership == "orphan")
    );
    assert!(
        page.rows
            .iter()
            .any(|r| r.hash == "unknown" && r.membership == "unverified" && r.height.is_none())
    );
    store.connection.execute("update block_membership set membership='orphan' where system_id='iOne' and source='api'",[]).unwrap();
    let refreshed = store.investigate("iOne", &subject, 0).unwrap();
    assert_eq!(
        refreshed
            .rows
            .iter()
            .filter(|r| r.membership == "member")
            .count(),
        1
    );
    assert!(
        store
            .investigate("absent", &subject, 0)
            .unwrap()
            .rows
            .is_empty()
    );
}

#[test]
fn child_investigation_merges_header_and_body_evidence_and_pages() {
    use super::investigation::{Investigation, PAGE_SIZE};
    let store = memory();
    for i in 0..=PAGE_SIZE {
        let hash = format!("{i:064x}");
        let raw = json!({"result":{"hash":hash,"height":2,"previousblockhash":"parent","time":100,"tx":[],"confirmations":1}});
        store
            .put_raw_rpc(
                "local",
                "iOne",
                "getblock",
                &json!([hash, 1]).to_string(),
                &raw.to_string(),
            )
            .unwrap();
        store
            .connection
            .execute(
                "insert into reconciliation_headers values('iOne',?1,2,'parent',100)",
                [&hash],
            )
            .unwrap();
    }
    // A header with no membership record but a source observation should appear once.
    store
        .connection
        .execute(
            "delete from block_membership where block_hash=?1",
            [format!("{:064x}", 0)],
        )
        .unwrap();
    let subject = Investigation::Children("parent".into());
    let first = store.investigate("iOne", &subject, 0).unwrap();
    assert_eq!(first.rows.len(), PAGE_SIZE);
    assert!(first.more && first.rows.iter().all(|r| r.cached && r.source == "local"));
    assert_eq!(first.rows[0].membership, "unverified");
    assert!(first.rows.iter().all(|r| r.timestamp == Some(100)));
    let second = store.investigate("iOne", &subject, 1).unwrap();
    assert_eq!(second.rows.len(), 1);
    assert!(!second.more);
    assert!(!first.rows.iter().any(|r| r.hash == second.rows[0].hash));
    assert!(
        store
            .investigate("iOther", &subject, 0)
            .unwrap()
            .rows
            .is_empty()
    );
    assert!(
        store
            .investigate("iOne", &Investigation::Children("other-parent".into()), 0)
            .unwrap()
            .rows
            .is_empty()
    );
}
