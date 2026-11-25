use super::*;
use crate::cache::{Cache, Receipt};

/// Records one JSON-RPC answer in a cache the way Carbide's client does after a request.
async fn record(
    cache: &Cache,
    source: &str,
    system: &str,
    method: &str,
    params: Value,
    result: Value,
) {
    let control = Arc::new(Control::default());
    let receipt = Receipt {
        branch_epoch: 0,
        generation: control.generation(),
        control,
        order: crate::rpc::next_request_order(),
        epoch: cache.epoch(),
        once: false,
    };
    let response = json!({"result": result, "error": null, "id": "verus-explorer-tui"});
    assert!(
        cache
            .observe(
                Network::Mainnet,
                receipt,
                source.into(),
                system.into(),
                method.into(),
                params.to_string(),
                response.to_string(),
            )
            .await
            .unwrap()
    );
}

/// The same folder as `test_cache_dir` in `ffi_tests.rs`: one cache for each test process.
fn test_cache_dir() -> String {
    let dir = std::env::temp_dir().join(format!("carbide-bridge-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.display().to_string()
}

fn test_explorer() -> Explorer {
    super::super::tests::explorer(None)
}

// ------------------------------------------------------------ activity

#[test]
fn block_events_match_both_sources_without_crossing_chains() {
    // The same case as Carbide's own test in src/app/source_ui/activity.rs.
    let local = crate::rpc::source_id("local-daemon", "http://127.0.0.1:21000");
    let api = crate::rpc::source_id("public-api", "https://example.invalid/rpc");
    let key = format!("iTestSystem:42:{local}:{api}:new:old");
    let message = "iTestSystem: sources disagree at height 42: endpoints and hashes";
    assert!(event_involves(&key, "iTestSystem", &local));
    assert!(event_involves(&key, "iTestSystem", &api));
    assert!(!event_involves(&key, "iOtherSystem", &local));
    assert!(!event_involves(&key, "iTestSystem", &api[..api.len() - 1]));
    assert_eq!(event_status(&key, message), "Not rechecked");
    assert_eq!(
        event_summary(&key, message),
        "Sources differed at height 42"
    );
    assert_eq!(event_system(&key), Some("iTestSystem"));
}

#[test]
fn reorg_events_belong_to_one_source_and_read_as_reorgs() {
    let local = crate::rpc::source_id("local-daemon", "http://127.0.0.1:27486");
    let key = format!("reorg:iSystem:{local}:7");
    let message = "iSystem: replaced 2 blocks after 100 (local)\nmore detail";
    assert!(event_involves(&key, "iSystem", &local));
    assert!(!event_involves(&key, "iSystem", "public-api:abc"));
    assert_eq!(event_status(&key, message), "Reconciled");
    assert_eq!(
        event_summary(&key, message),
        "Reorg · replaced 2 blocks after 100"
    );
    let rows = activity_rows(
        vec![
            crate::cache::SourceEvent {
                key: key.clone(),
                message: message.into(),
                observed_at: 5,
            },
            crate::cache::SourceEvent {
                key: "tip:iSystem:x:y".into(),
                message: "iSystem: tip change".into(),
                observed_at: 4,
            },
        ],
        Some(("iSystem", &local)),
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].system_id.as_deref(), Some("iSystem"));
    let everything = activity_rows(Vec::new(), None);
    assert!(everything.is_empty());
}

#[test]
fn ages_and_labels_read_as_in_the_terminal() {
    assert_eq!(short_age(0), "<1s");
    assert_eq!(short_age(59), "59s");
    assert_eq!(short_age(61), "1m");
    assert_eq!(short_age(7200), "2h");
    assert_eq!(short_age(200_000), "2d");
    assert_eq!(chain_label("VRSC"), "VRSC");
    assert_eq!(
        chain_label("52c7a71ed15802d33778235e7988d61339b84c45"),
        "PBaaS chain 52c7a71e"
    );
    assert_eq!(
        source_label("local-daemon:0123456789abcdef0123"),
        "local-daemon:0123456789ab"
    );
    assert_eq!(
        origin_of("https://api.example.com/rpc/"),
        "https://api.example.com"
    );
}

#[test]
fn traffic_follows_state_and_requests() {
    let control = Arc::new(Control::default());
    let stale = Duration::from_secs(30);
    assert_eq!(
        traffic(control.state(), &control.communication(), stale),
        SourceTraffic::NotUsed
    );
    let mut request = control.begin_request(control.generation(), 1);
    assert_eq!(
        traffic(control.state(), &control.communication(), stale),
        SourceTraffic::Working
    );
    request.responding(true);
    drop(request);
    assert_eq!(
        traffic(control.state(), &control.communication(), stale),
        SourceTraffic::Responding
    );
    drop(control.begin_request(control.generation(), 2));
    assert_eq!(
        traffic(control.state(), &control.communication(), stale),
        SourceTraffic::Unavailable
    );
    control.set(ConnectionState::Disabled);
    assert_eq!(
        traffic(control.state(), &control.communication(), stale),
        SourceTraffic::Off
    );
}

#[test]
fn chain_check_text_reports_failures_and_successes() {
    let control = Control::default();
    control.set(ConnectionState::Monitored);
    assert_eq!(
        chain_check_text(&control, Duration::from_secs(30)),
        ("Awaiting first check".to_string(), false)
    );
    control.record_chain_check(
        control.generation(),
        ChainCheck::Finished {
            at: Instant::now(),
            error: Some("tip changed\nsecond line".into()),
        },
    );
    let (text, problem) = chain_check_text(&control, Duration::from_secs(30));
    assert!(problem);
    assert!(text.starts_with("Needs attention · "));
    assert!(text.ends_with("tip changed"));
    control.record_chain_check(
        control.generation(),
        ChainCheck::Finished {
            at: Instant::now(),
            error: None,
        },
    );
    let (text, problem) = chain_check_text(&control, Duration::from_secs(30));
    assert!(!problem);
    assert!(text.starts_with("Reconciled · "));
    control.set(ConnectionState::Disabled);
    assert_eq!(
        chain_check_text(&control, Duration::from_secs(30)).0,
        "Not polling"
    );
}

// ---------------------------------------------------------- proof roots

#[test]
fn proof_root_status_distinguishes_branches_misses_and_lagging_sources() {
    let hash = "ab".repeat(32);
    let header = root_header(
        &json!({"hash": hash, "height": 800, "time": 1700000000, "confirmations": 12}),
        &hash,
    )
    .unwrap();
    assert_eq!(found_status(Some(800), &header).0, ProofRootStatus::Active);
    assert_eq!(
        found_status(Some(801), &header),
        (
            ProofRootStatus::HeightMismatch,
            "Height mismatch (found #800)".to_string()
        )
    );
    let off = RootHeader {
        confirmations: Some(-1),
        ..header.clone()
    };
    assert_eq!(found_status(Some(800), &off).0, ProofRootStatus::OffChain);
    let unknown = RootHeader {
        confirmations: None,
        ..header
    };
    assert_eq!(
        found_status(None, &unknown).0,
        ProofRootStatus::PresentMembershipUnknown
    );
    assert!(root_header(&json!({"hash": "other", "height": 1}), &hash).is_none());
    assert_eq!(
        missing_status(Some(900), Some(850)),
        (
            ProofRootStatus::SourceBehind,
            "Not found • source tip #850 is behind".to_string()
        )
    );
    assert_eq!(
        missing_status(Some(800), Some(850)).0,
        ProofRootStatus::NotFound
    );
    assert_eq!(missing_status(None, None).0, ProofRootStatus::NotFound);
    assert_eq!(
        root_failure(ProofRootStatus::MissingHash).label,
        "Proof root has no block hash"
    );
}

#[tokio::test]
async fn proof_root_checks_explain_missing_hashes_and_unconfigured_chains() {
    let explorer = test_explorer();
    let system = explorer.config.chain_cache_key();
    assert_eq!(
        explorer
            .check_proof_root(system.clone(), None, Some(1))
            .await
            .status,
        ProofRootStatus::MissingHash
    );
    let unknown = explorer
        .check_proof_root("iUnknownSystemForTests".into(), Some("ab".repeat(32)), None)
        .await;
    assert_eq!(unknown.status, ProofRootStatus::NoSourceConfigured);
    assert_eq!(unknown.label, "No source configured for this chain");
}

// ------------------------------------------------------- observations

#[test]
fn heights_come_from_blocks_transactions_and_identities() {
    assert_eq!(observed_height(r#"{"result":{"height":12}}"#), Some(12));
    assert_eq!(observed_height(r#"{"result":{"blockheight":7}}"#), Some(7));
    assert_eq!(observed_height(r#"{"result":{"height":-1}}"#), None);
    assert_eq!(observed_height(r#"{"result":[1,2]}"#), None);
    assert_eq!(observed_height("not json"), None);
}

#[test]
fn only_a_source_without_an_answer_allows_a_retained_answer() {
    assert!(!source_answered(&anyhow::anyhow!("error sending request")));
    assert!(!source_answered(&anyhow::anyhow!(
        crate::sources::ApiCancelled
    )));
}

#[tokio::test]
async fn retained_rows_come_from_the_latest_observation_of_the_same_request() {
    let explorer = test_explorer();
    let cache = explorer.cache.clone().expect("the test cache opens");
    let system = explorer.config.chain_cache_key();
    let path = explorer.config.cache_path.clone();
    let txid = "1f".repeat(32);
    let api = crate::rpc::source_id("public-api", "https://api.example.invalid");
    record(
        &cache,
        &api,
        &system,
        "getrawtransaction",
        json!([txid, 1]),
        json!({"txid": txid, "height": 41, "confirmations": 3, "vin": [], "vout": []}),
    )
    .await;
    let row = retained_row(
        &path,
        &system,
        "getrawtransaction",
        &[json!(txid), json!(1)],
    )
    .unwrap()
    .expect("the stored transaction");
    assert_eq!(row.source, api);
    assert!(row.payload.contains(&txid));
    assert!(row.observed_at > 0);
    assert_eq!(observed_height(&row.payload), Some(41));

    // A request by height is a stored response, not an object observation.
    let block = "2e".repeat(32);
    record(
        &cache,
        &api,
        &system,
        "getblock",
        json!(["41"]),
        json!({"hash": block, "height": 41, "time": 1, "tx": [txid], "confirmations": 1}),
    )
    .await;
    let by_height = retained_row(&path, &system, "getblock", &[json!("41")])
        .unwrap()
        .expect("the stored block");
    assert!(by_height.payload.contains(&block));
    let by_hash = retained_row(&path, &system, "getblock", &[json!(block)])
        .unwrap()
        .expect("the stored block by hash");
    assert!(by_hash.payload.contains(&block));

    assert!(
        retained_row(
            &path,
            &system,
            "getrawtransaction",
            &[json!("00".repeat(32)), json!(1)]
        )
        .unwrap()
        .is_none()
    );
    assert!(
        retained_row(
            &path,
            "iOtherSystem",
            "getrawtransaction",
            &[json!(txid), json!(1)]
        )
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
async fn an_unreachable_node_serves_the_retained_transaction_with_its_provenance() {
    let explorer = test_explorer();
    let cache = explorer.cache.clone().expect("the test cache opens");
    let system = explorer.config.chain_cache_key();
    let txid = "3c".repeat(32);
    let api = crate::rpc::source_id("public-api", "https://api.example.invalid");
    record(
        &cache,
        &api,
        &system,
        "getrawtransaction",
        json!([txid, 1]),
        json!({"txid": txid, "height": 99, "confirmations": 5, "vin": [], "vout": []}),
    )
    .await;
    // The test explorer's node has no credentials, so the node does not answer.
    let detail = explorer.transaction(txid.clone()).await.unwrap();
    assert_eq!(detail.txid, txid);
    assert!(detail.provenance.retained);
    assert_eq!(detail.provenance.source_id, api);
    assert_eq!(detail.provenance.kind, SourceKind::PublicApi);
    assert_eq!(detail.provenance.observed_height, Some(99));
    assert!(detail.provenance.failure.is_some());

    // Nothing stored: the error stays.
    assert!(explorer.transaction("4d".repeat(32)).await.is_err());
}

#[tokio::test]
async fn retained_blocks_by_height_carry_a_caution() {
    let explorer = test_explorer();
    let cache = explorer.cache.clone().expect("the test cache opens");
    let system = explorer.config.chain_cache_key();
    let block = "5e".repeat(32);
    record(
        &cache,
        "local-daemon:test",
        &system,
        "getblock",
        json!(["77"]),
        json!({"hash": block, "height": 77, "time": 1700000000, "tx": [], "confirmations": 1}),
    )
    .await;
    let detail = explorer.block("77".into()).await.unwrap();
    assert_eq!(detail.row.hash, block);
    assert!(detail.provenance.retained);
    assert_eq!(detail.provenance.kind, SourceKind::LocalNode);
    assert!(detail.provenance.caution.is_some());
}

#[tokio::test]
async fn block_history_reads_stored_inclusions() {
    let explorer = test_explorer();
    let cache = explorer.cache.clone().expect("the test cache opens");
    let system = explorer.config.chain_cache_key();
    let txid = "6f".repeat(32);
    let active = "7a".repeat(32);
    let source = crate::rpc::source_id("public-api", "https://api.example.invalid");
    record(
        &cache,
        &source,
        &system,
        "getrawtransaction",
        json!([txid, 1]),
        json!({"txid": txid, "blockhash": active, "height": 10, "confirmations": 2, "vin": [], "vout": []}),
    )
    .await;
    record(
        &cache,
        &source,
        &system,
        "getblock",
        json!([active]),
        json!({"hash": active, "height": 10, "time": 1700000000, "tx": [txid], "confirmations": 2,
               "previousblockhash": "8b".repeat(32)}),
    )
    .await;
    let page = explorer.inclusion_history(txid.clone(), 0).await.unwrap();
    assert_eq!(page.rows.len(), 1);
    let row = &page.rows[0];
    assert_eq!(row.block_hash, active);
    assert_eq!(row.height, Some(10));
    assert_eq!(row.membership, Membership::Active);
    assert!(row.cached_body);
    // The API is not configured in this session, so its ID is shown in short form.
    assert!(row.source.starts_with("public-api:"));
    assert!(!page.more);

    let children = explorer.block_children("8b".repeat(32), 0).await.unwrap();
    assert_eq!(children.rows.len(), 1);
    assert_eq!(children.rows[0].block_hash, active);
}

// ------------------------------------------------------------- sources

#[test]
fn offered_and_allowed_apis_join_the_current_chain() {
    let explorer = Explorer::open_with(
        ConnectionOptions {
            chain: "VRSC".into(),
            testnet: false,
            config_file: Some("/nonexistent/carbide-sources-test/VRSC.conf".into()),
            api_endpoint: Some("https://allowed.example.invalid".into()),
            cache_dir: Some(test_cache_dir()),
            ..Default::default()
        },
        &[
            "https://offered.example.invalid/".into(),
            "ftp://bad".into(),
        ],
    )
    .unwrap();
    let rows = explorer.sources_overview().rows;
    let api = |url: &str| {
        rows.iter()
            .find(|row| row.kind == SourceKind::PublicApi && row.endpoint.starts_with(url))
            .cloned()
    };
    let offered = api("https://offered.example.invalid").expect("offered API listed");
    assert_eq!(offered.state, SourceState::Disabled);
    assert!(offered.current);
    let allowed = api("https://allowed.example.invalid").expect("allowed API listed");
    assert_eq!(allowed.state, SourceState::Enabled);
    assert!(api("ftp://bad").is_none());
    assert!(
        explorer
            .set_source_state(offered.key.clone(), SourceState::Monitored)
            .is_err()
    );
    assert!(
        explorer
            .set_source_state("node:99".into(), SourceState::Enabled)
            .is_err()
    );

    // A session change applies at once and survives a new explorer for the same chain.
    explorer
        .set_source_state(offered.key.clone(), SourceState::Enabled)
        .unwrap();
    assert!(explorer.sources_overview().unsaved);
    let again = Explorer::open_with(
        ConnectionOptions {
            chain: "VRSC".into(),
            testnet: false,
            config_file: Some("/nonexistent/carbide-sources-test/VRSC.conf".into()),
            api_endpoint: None,
            cache_dir: Some(test_cache_dir()),
            ..Default::default()
        },
        &["https://offered.example.invalid".into()],
    )
    .unwrap();
    let state = again
        .sources_overview()
        .rows
        .into_iter()
        .find(|row| row.endpoint.starts_with("https://offered.example.invalid"))
        .unwrap()
        .state;
    assert_eq!(state, SourceState::Enabled);
    // Leave the session as it was for other tests in this process.
    again
        .set_source_state(offered.key, SourceState::Disabled)
        .unwrap();
}

#[test]
fn the_current_chain_comes_first_and_labels_its_node() {
    let explorer = test_explorer();
    let overview = explorer.sources_overview();
    assert!(overview.settings_file.is_some());
    assert!(overview.cache_problem.is_none());
    let current = explorer.sources.current();
    assert_eq!(current.chain.name, "VRSC");
    let (label, kind) = explorer.sources.label(&current.daemon_cache_key());
    assert_eq!(kind, SourceKind::LocalNode);
    assert!(label.starts_with("VRSC node at 127.0.0.1:"));
    assert_eq!(
        explorer.sources.label("public-api:0123456789abcdef9999").1,
        SourceKind::PublicApi
    );
    assert_eq!(explorer.sources.label("bookmark").1, SourceKind::Unknown);
}

#[tokio::test]
async fn permission_requests_wait_for_an_answer_and_enable_one_api() {
    let explorer = Arc::new(
        Explorer::open_with(
            ConnectionOptions {
                chain: "VRSC".into(),
                testnet: false,
                config_file: Some("/nonexistent/carbide-permission-test/VRSC.conf".into()),
                api_endpoint: None,
                cache_dir: Some(test_cache_dir()),
                ..Default::default()
            },
            // Port 9 on localhost refuses connections, so enabling it sends nothing anywhere.
            &["http://127.0.0.1:9".into()],
        )
        .unwrap(),
    );
    let asking = explorer.clone();
    let request = tokio::spawn(async move { asking.transaction("9a".repeat(32)).await });
    let pending = explorer
        .next_permission_request()
        .await
        .expect("a primary request asks");
    assert_eq!(pending.chain, "VRSC");
    assert_eq!(pending.method, "getrawtransaction");
    assert_eq!(pending.endpoints, vec!["http://127.0.0.1:9".to_string()]);
    assert!(!pending.failure.is_empty());
    explorer
        .answer_permission(pending.id, PermissionAnswer::Enable { index: 0 })
        .unwrap();
    // The endpoint refuses the connection, so the request fails, after using the API.
    assert!(request.await.unwrap().is_err());
    let api = explorer
        .sources_overview()
        .rows
        .into_iter()
        .find(|row| row.endpoint == "http://127.0.0.1:9")
        .unwrap();
    assert_eq!(api.state, SourceState::Enabled);
    assert!(
        explorer
            .answer_permission(pending.id, PermissionAnswer::Cancel)
            .is_err()
    );
    explorer
        .set_source_state(api.key, SourceState::Disabled)
        .unwrap();

    // Automatic requests never ask.
    assert!(explorer.chain_status().await.is_err());
    assert!(
        tokio::time::timeout(
            Duration::from_millis(200),
            explorer.next_permission_request()
        )
        .await
        .is_err()
    );
}

// ---------------------------------------------------------- live tests

/// A recent block and one of its transactions from the live source.
async fn recent_transaction(
    explorer: &Explorer,
) -> (super::super::transaction::BlockDetail, String) {
    let tip = explorer.chain_status().await.unwrap().blocks;
    let block = explorer.block((tip - 3).to_string()).await.unwrap();
    let txid = block.transactions[0].txid.clone();
    (block, txid)
}

#[tokio::test]
#[ignore]
async fn live_sources_list_the_current_node() {
    let explorer = super::super::tests::live_explorer();
    let overview = explorer.sources_overview();
    for row in &overview.rows {
        println!(
            "{} {:?} {} {:?} current={} {:?}",
            row.chain, row.kind, row.endpoint, row.state, row.current, row.chain_check
        );
    }
    let node = overview
        .rows
        .iter()
        .find(|row| row.current && row.kind == SourceKind::LocalNode)
        .expect("the current node");
    assert_ne!(node.state, SourceState::Disabled);
    assert!(node.has_credentials);
}

#[tokio::test]
#[ignore]
async fn live_answers_are_recorded_and_carry_provenance() {
    let explorer = super::super::tests::live_explorer();
    let (block, txid) = recent_transaction(&explorer).await;
    assert!(!block.provenance.retained);
    let detail = explorer.transaction(txid.clone()).await.unwrap();
    println!("{:?}", detail.provenance);
    assert!(!detail.provenance.retained);
    assert_eq!(detail.provenance.observed_height, Some(block.row.height));
    let retained: Answer<Value> = explorer
        .retained("getrawtransaction", &[json!(txid), json!(1)], None)
        .await
        .expect("the answer was recorded");
    assert!(retained.provenance.retained);
    assert_eq!(retained.source_id, detail.provenance.source_id);
}

/// Measures how much one 7-day chart load adds to a fresh cache, and what Carbide's
/// retention keeps. Uses its own cache folder.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_cache_growth_of_a_seven_day_chart() {
    let conf = std::env::var("CARBIDE_TEST_CONF").expect("set CARBIDE_TEST_CONF");
    let dir = std::env::temp_dir().join(format!("carbide-growth-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let explorer = Arc::new(
        Explorer::open(ConnectionOptions {
            chain: "VRSC".into(),
            config_file: Some(conf),
            cache_dir: Some(dir.display().to_string()),
            ..Default::default()
        })
        .unwrap(),
    );
    let path = explorer.config.cache_path.clone();
    let before = crate::cache::cache_disk_usage(&path).unwrap();
    let measure = |label: &str| {
        let bytes = crate::cache::cache_disk_usage(&path).unwrap();
        let connection = rusqlite::Connection::open_with_flags(
            &path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let count = |table: &str| -> i64 {
            connection
                .query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))
                .unwrap()
        };
        let payload: i64 = connection
            .query_row("select coalesce(sum(length(payload)),0) from raw_blobs", [], |r| r.get(0))
            .unwrap();
        println!(
            "{label}: {:.1} MiB on disk, raw_rpc_cache {} rows, objects {}, object_observations {}, \
             block_membership {}, raw_blobs {} ({:.1} MiB of payload)",
            bytes as f64 / 1048576.0,
            count("raw_rpc_cache"),
            count("objects"),
            count("object_observations"),
            count("block_membership"),
            count("raw_blobs"),
            payload as f64 / 1048576.0,
        );
        bytes
    };
    let started = Instant::now();
    for round in 0..2 {
        let load = explorer.clone().chart_load(crate::ffi::charts::ChartRequest {
            span_seconds: 604_800,
            end_height: None,
            anchor_hash: None,
            newer: false,
            context: 100,
            focus_height: None,
            focus_hash: None,
            chunk: 500,
        });
        let mut progress = load.step().await.unwrap();
        while !progress.done {
            progress = load.step().await.unwrap();
        }
        println!("round {round}: {} headers, stopped {:?}, {:?}", progress.headers, progress.stopped, started.elapsed());
        measure(if round == 0 { "after one load" } else { "after the same load again" });
    }
    println!("empty cache: {before} bytes");
    drop(explorer);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The unrecorded client answers from the node and stores nothing.
#[tokio::test]
#[ignore]
async fn live_unrecorded_client_stores_nothing() {
    let explorer = super::super::tests::live_explorer();
    let tip = explorer.chain_status().await.unwrap().blocks;
    let height = tip - 5_000;
    let client = explorer.unrecorded_client();
    let block = client
        .call_with_raw::<Value>("getblock", &[Value::from(height.to_string())])
        .await
        .unwrap();
    assert_eq!(block.result["height"].as_u64(), Some(height));
    let stored: Option<Answer<Value>> = explorer
        .retained("getblock", &[Value::from(height.to_string())], None)
        .await;
    assert!(stored.is_none());
}

/// Recording every answer must not make bulk requests, such as a chart's blocks, slow.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_recording_keeps_bulk_requests_usable() {
    let explorer = super::super::tests::live_explorer();
    let tip = explorer.chain_status().await.unwrap().blocks;
    let fetch = |client: RpcClient, first: u64| async move {
        let started = Instant::now();
        let mut tasks = tokio::task::JoinSet::new();
        for height in first..first + 500 {
            let client = client.clone();
            tasks.spawn(async move {
                client
                    .call_with_raw::<Value>("getblock", &[Value::from(height.to_string())])
                    .await
            });
        }
        while let Some(done) = tasks.join_next().await {
            done.unwrap().unwrap();
        }
        started.elapsed()
    };
    let recorded = fetch(explorer.client.clone(), tip - 510).await;
    let unrecorded = fetch(explorer.unrecorded_client(), tip - 1010).await;
    println!("500 blocks: {recorded:?} recorded, {unrecorded:?} not recorded");
}

#[tokio::test]
#[ignore]
async fn live_inclusion_history_reads_the_recorded_transaction() {
    let explorer = super::super::tests::live_explorer();
    let (block, txid) = recent_transaction(&explorer).await;
    explorer.transaction(txid.clone()).await.unwrap();
    let page = explorer.inclusion_history(txid, 0).await.unwrap();
    for row in &page.rows {
        println!("{row:?}");
    }
    assert!(page.rows.iter().any(|row| row.block_hash == block.row.hash));
    let parent = block.previous_hash.clone().unwrap();
    let children = explorer.block_children(parent, 0).await.unwrap();
    assert!(
        children
            .rows
            .iter()
            .any(|row| row.block_hash == block.row.hash)
    );
}

#[tokio::test]
#[ignore]
async fn live_proof_root_check_finds_an_active_block() {
    let explorer = super::super::tests::live_explorer();
    let (block, _) = recent_transaction(&explorer).await;
    let system = explorer.config.chain_cache_key();
    let check = explorer
        .check_proof_root(
            system.clone(),
            Some(block.row.hash.clone()),
            Some(block.row.height),
        )
        .await;
    println!("{check:?}");
    assert_eq!(check.status, ProofRootStatus::Active);
    let missing = explorer
        .check_proof_root(system, Some("00".repeat(32)), Some(block.row.height))
        .await;
    assert_eq!(missing.status, ProofRootStatus::NotFound);
    let ahead = explorer
        .check_proof_root(
            explorer.config.chain_cache_key(),
            Some("00".repeat(32)),
            Some(block.row.height + 1_000_000),
        )
        .await;
    assert_eq!(ahead.status, ProofRootStatus::SourceBehind);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn live_background_work_reconciles_the_current_node() {
    let explorer = super::super::tests::live_explorer();
    explorer.start_background().await;
    let mut text = String::new();
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let overview = explorer.sources_overview();
        let node = overview
            .rows
            .iter()
            .find(|row| row.current && row.kind == SourceKind::LocalNode)
            .unwrap();
        text = node.chain_check.clone().unwrap_or_default();
        if text.starts_with("Reconciled") || text.starts_with("Needs attention") {
            break;
        }
    }
    println!("{text}");
    assert!(text.starts_with("Reconciled"), "{text}");
}

/// Needs `CARBIDE_TEST_API`. Sends one request to that API, after the test's own consent.
#[tokio::test]
#[ignore]
async fn live_permission_enables_a_public_api_for_one_primary_request() {
    let api = std::env::var("CARBIDE_TEST_API").expect("set CARBIDE_TEST_API");
    let explorer = Arc::new(
        Explorer::open_with(
            ConnectionOptions {
                chain: "VRSC".into(),
                testnet: false,
                config_file: Some("/nonexistent/carbide-live-permission/VRSC.conf".into()),
                api_endpoint: None,
                cache_dir: Some(test_cache_dir()),
                ..Default::default()
            },
            std::slice::from_ref(&api),
        )
        .unwrap(),
    );
    let asking = explorer.clone();
    let request =
        tokio::spawn(async move { asking.identity_timeline("Verus Coin Foundation@".into()).await });
    let pending = explorer.next_permission_request().await.expect("asks");
    println!("{pending:?}");
    explorer
        .answer_permission(pending.id, PermissionAnswer::Enable { index: 0 })
        .unwrap();
    let identity = request.await.unwrap().unwrap();
    println!("{:?}", identity.provenance);
    assert_eq!(identity.provenance.kind, SourceKind::PublicApi);
    assert!(!identity.provenance.retained);
    let key = explorer
        .sources_overview()
        .rows
        .into_iter()
        .find(|row| row.kind == SourceKind::PublicApi)
        .unwrap()
        .key;
    explorer
        .set_source_state(key, SourceState::Disabled)
        .unwrap();
}

// ------------------------------------------------------ start-up options

#[test]
fn start_up_options_become_carbide_arguments() {
    let options = ConnectionOptions {
        chain: "VRSC".into(),
        data_dir: Some("/Volumes/Node/VRSC".into()),
        rpc_host: Some("10.0.0.2".into()),
        rpc_port: Some(27486),
        rpc_user: Some("user".into()),
        rpc_password: Some("-starts with a dash".into()),
        ..Default::default()
    };
    let args = cli_arguments(&options, "VRSC", Network::Mainnet).unwrap();
    assert_eq!(
        args,
        vec![
            "carbide",
            "--chain=VRSC",
            "--datadir=/Volumes/Node/VRSC",
            "--rpcconnect=10.0.0.2",
            "--rpcport=27486",
            "--rpcuser=user",
            "--rpcpassword=-starts with a dash",
        ]
    );
    let cli = Cli::try_parse_from(&args).unwrap();
    assert_eq!(cli.rpc_password.as_deref(), Some("-starts with a dash"));
    assert_eq!(cli.data_dir.as_deref(), Some(Path::new("/Volumes/Node/VRSC")));
    assert!(cli.config_file.is_none());

    // With a data directory, a relative file is relative to it, as for the node.
    let relative = ConnectionOptions {
        config_file: Some("other.conf".into()),
        ..options.clone()
    };
    assert!(
        cli_arguments(&relative, "VRSC", Network::Mainnet)
            .unwrap()
            .contains(&"--conf=other.conf".to_string())
    );
    // Without one, the file must be absolute.
    let bare = ConnectionOptions {
        config_file: Some("other.conf".into()),
        ..Default::default()
    };
    assert!(cli_arguments(&bare, "VRSC", Network::Mainnet).is_err());
    assert!(
        cli_arguments(&ConnectionOptions::default(), "vrsctest", Network::Testnet)
            .unwrap()
            .contains(&"--testnet".to_string())
    );
    // The password never appears in debug output.
    assert!(!format!("{options:?}").contains("dash"));
}

#[test]
fn a_data_directory_that_does_not_exist_is_reported() {
    let result = Explorer::open(ConnectionOptions {
        chain: "VRSC".into(),
        data_dir: Some("/nonexistent/carbide-datadir-test".into()),
        cache_dir: Some(test_cache_dir()),
        ..Default::default()
    });
    assert!(result.is_err());
}

#[tokio::test]
async fn the_revision_waits_and_reports_changes() {
    let explorer = test_explorer();
    let start = explorer.observation_revision();
    explorer.sources.revision.send_modify(|value| *value += 1);
    assert_eq!(explorer.wait_for_revision(start).await, start + 1);
    let waiting = tokio::time::timeout(
        Duration::from_millis(100),
        explorer.wait_for_revision(start + 1),
    )
    .await;
    assert!(waiting.is_err());
}

#[test]
fn the_current_chain_has_a_name_and_hex_folders_do_not() {
    let explorer = test_explorer();
    let rows = explorer.sources_overview().rows;
    let current = rows.iter().find(|row| row.current).unwrap();
    assert_eq!(current.name.as_deref(), Some("VRSC"));
    for row in rows.iter().filter(|row| row.chain.len() == 40) {
        assert_ne!(row.name.as_deref(), Some(row.chain.as_str()));
    }
}
