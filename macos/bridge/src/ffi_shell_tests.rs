use super::*;
use serde_json::json;

fn args(list: &[&str]) -> Vec<String> {
    std::iter::once("/Applications/Carbide Silica.app/Contents/MacOS/CarbideSilica")
        .chain(list.iter().copied())
        .map(String::from)
        .collect()
}

// ---------------------------------------------------------------- mempool

#[test]
fn builds_mempool_rows_newest_first_with_classification() {
    let values: HashMap<String, Value> = serde_json::from_value(json!({
        "aa": {"size": 300, "fee": 0.0001, "time": 1_795_000_100, "height": 4_337_000,
               "depends": []},
        "bb": {"size": 1200, "fee": 0.00025, "time": 1_795_000_200, "height": 4_337_001,
               "depends": ["aa"],
               "fulltx": {"txid": "bb", "vin": [{"txid": "aa", "vout": 0}],
                          "vout": [{"value": 1.0, "scriptPubKey": {"type": "pubkeyhash"}}]}},
        "cc": {"size": 500, "time": 1_795_000_200, "depends": ["aa", "bb"]}
    }))
    .unwrap();
    let snapshot = mempool_snapshot_from(values);
    assert_eq!(snapshot.total_bytes, 2000);
    assert_eq!(snapshot.decoded, 1);
    let order: Vec<&str> = snapshot.entries.iter().map(|e| e.txid.as_str()).collect();
    // Same arrival time sorts by ID.
    assert_eq!(order, ["bb", "cc", "aa"]);

    let bb = &snapshot.entries[0];
    assert!(bb.classified);
    assert_eq!(bb.fee_sats, Some(25_000));
    assert_eq!(bb.height, Some(4_337_001));
    assert_eq!(bb.parents, ["aa"]);

    let cc = &snapshot.entries[1];
    assert!(!cc.classified, "no body yet: classification pending");
    assert!(cc.tags.is_empty());
    assert_eq!(cc.fee_sats, None);
    assert_eq!(cc.height, None);
    assert_eq!(cc.parents.len(), 2);
}

#[test]
fn ignores_a_mempool_body_for_another_transaction() {
    let values: HashMap<String, Value> = serde_json::from_value(json!({
        "aa": {"size": 10, "time": 0, "fulltx": {"txid": "zz", "vin": [], "vout": []}}
    }))
    .unwrap();
    let snapshot = mempool_snapshot_from(values);
    assert!(!snapshot.entries[0].classified);
    assert_eq!(snapshot.entries[0].time, None, "a zero time means unknown");
}

// -------------------------------------------------------------- dashboard

#[test]
fn orders_dashboard_transactions_like_the_terminal() {
    let block = |hash: &str, height: u64, txids: &[&str]| -> BlockSummary {
        serde_json::from_value(json!({
            "hash": hash, "height": height, "time": 1000 + height, "tx": txids
        }))
        .unwrap()
    };
    let blocks = [
        block("h2", 2, &["c2", "t2a", "t2b"]),
        block("h1", 1, &["c1", "t1a", "t2a"]),
    ];
    let rows = dashboard_transactions(&blocks, 100);
    let order: Vec<(&str, u32)> = rows.iter().map(|r| (r.txid.as_str(), r.position)).collect();
    assert_eq!(
        order,
        [("t2b", 2), ("t2a", 1), ("c2", 0), ("t1a", 1), ("c1", 0)],
        "newest block first, last position first, repeats left out"
    );
    assert_eq!(rows[0].block_height, 2);
    assert_eq!(rows[3].block_hash, "h1");
    assert_eq!(dashboard_transactions(&blocks, 2).len(), 2);
}

// ------------------------------------------------------------------ peers

#[test]
fn groups_peer_fields_like_the_terminal() {
    let groups = peer_groups(&json!({
        "id": 3, "addr": "136.243.31.96:27485", "inbound": false,
        "addrlocal": "45.21.145.82:50766", "conntime": 1_795_559_660, "version": 170010,
        "subver": "/MagicBean:2.0.73/", "tls_established": true,
        "bytessent": 77014, "bytesrecv": 194737, "lastsend": 0, "pingtime": 0.245298,
        "timeoffset": 0, "startingheight": 4_337_031, "synced_headers": 4_337_092,
        "inflight": [], "banscore": 0
    }));
    let titles: Vec<&str> = groups.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Connection",
            "Traffic and latency",
            "Chain progress",
            "Other fields"
        ]
    );
    let field = |group: usize, key: &str| {
        groups[group]
            .fields
            .iter()
            .find(|f| f.key == key)
            .cloned()
            .unwrap()
    };
    assert_eq!(field(0, "conntime").kind, PeerFieldKind::Time);
    assert_eq!(field(0, "subver").value, "/MagicBean:2.0.73/");
    assert_eq!(field(0, "tls_established").value, "true");
    assert_eq!(field(1, "bytesrecv").kind, PeerFieldKind::Bytes);
    assert_eq!(field(1, "pingtime").kind, PeerFieldKind::Seconds);
    assert_eq!(
        field(1, "lastsend").kind,
        PeerFieldKind::Text,
        "a zero time is not a date"
    );
    assert_eq!(field(2, "inflight").value, "[]");
    assert_eq!(field(3, "banscore").label, "banscore");
    assert!(
        groups
            .iter()
            .flat_map(|g| &g.fields)
            .all(|f| !["id", "addr", "inbound"].contains(&f.key.as_str()))
    );
    assert!(peer_field_groups("not json".into()).is_empty());
}

#[test]
fn reads_the_ban_list() {
    let banned = banned_peers_from(&json!([
        {"address": "203.0.113.9/32", "banned_until": 1_795_600_000,
         "ban_created": 1_795_513_600, "ban_reason": "manually added"},
        {"banned_until": 5}
    ]));
    assert_eq!(banned.len(), 1);
    assert_eq!(banned[0].address, "203.0.113.9/32");
    assert_eq!(banned[0].reason.as_deref(), Some("manually added"));
}

#[test]
fn places_peers_without_a_location_database() {
    let directory = std::env::temp_dir().join(format!("carbide-geoip-test-{}", std::process::id()));
    let result = locate_peers(
        &directory,
        &[
            "136.243.31.96:27485".into(),
            "192.168.1.4:27485".into(),
            "[fd00::1]:27485".into(),
            "not an address".into(),
        ],
    );
    assert_eq!(result.state, LocationDataState::NotInstalled);
    let places: Vec<PeerPlace> = result.locations.iter().map(|l| l.place).collect();
    assert_eq!(
        places,
        [
            PeerPlace::Unresolved,
            PeerPlace::Private,
            PeerPlace::Private,
            PeerPlace::Unresolved
        ]
    );
    assert!(result.locations.iter().all(|l| l.latitude.is_none()));
}

// ----------------------------------------------------------------- search

#[test]
fn plans_the_terminal_search_forms() {
    let now = 1_795_563_000;
    let plan = |query: &str| plan_search(query, now).unwrap();
    assert_eq!(plan("currencies"), SearchPlan::Route(Route::Currencies));
    assert_eq!(plan("Currencies"), SearchPlan::Route(Route::Currencies));
    assert_eq!(plan("currency:"), SearchPlan::Route(Route::Currencies));
    assert_eq!(
        plan("currency:vETH"),
        SearchPlan::Route(Route::Currency {
            name: "vETH".into()
        })
    );
    assert_eq!(
        plan("estimate:5000000"),
        SearchPlan::Route(Route::Estimate {
            query: "5000000".into()
        })
    );
    assert_eq!(
        plan("estimate:unix:1857254400"),
        SearchPlan::Route(Route::Estimate {
            query: "unix:1857254400".into()
        })
    );
    assert_eq!(
        plan("estimate:in 2 hours"),
        SearchPlan::Route(Route::Estimate {
            query: format!("unix:{}", now + 7200)
        }),
        "relative input is frozen at the search"
    );
    assert_eq!(
        plan("estimate:2028-11-08"),
        SearchPlan::Route(Route::Estimate {
            query: "unix:1857254400".into()
        })
    );
    assert_eq!(plan("4226272"), SearchPlan::Height(4_226_272));
    assert_eq!(plan("2026-11-07"), SearchPlan::Time(1_794_009_600));
    assert_eq!(
        plan("time:2030-01-01"),
        SearchPlan::Route(Route::Estimate {
            query: "unix:1893456000".into()
        }),
        "a future time opens the estimator"
    );
    assert!(matches!(
        plan("0000000000000000000000000000000000000000000000000000000000000abc"),
        SearchPlan::Hash(_)
    ));
    assert!(plan_search("???", now).is_err());
    assert!(plan_search("estimate:", now).is_err());
}

#[test]
fn reads_numbers_above_the_tip_like_the_terminal() {
    let genesis = 1_526_889_600;
    let now = 1_795_563_000;
    assert_eq!(above_tip(5_000_000, genesis, now), AboveTip::FutureHeight);
    assert_eq!(above_tip(1_700_000_000, genesis, now), AboveTip::PastTime);
    assert_eq!(above_tip(1_900_000_000, genesis, now), AboveTip::FutureTime);
}

#[tokio::test]
async fn resolves_search_forms_without_a_node() {
    let explorer = super::super::tests::explorer(None);
    let route = |query: &str| explorer.resolve(query.to_string());
    assert_eq!(route("/currencies").await.unwrap(), Route::Currencies);
    assert_eq!(
        route("/vETH@@").await.unwrap(),
        Route::Currency {
            name: "vETH".into()
        }
    );
    assert_eq!(
        route("estimate:4500000").await.unwrap(),
        Route::Estimate {
            query: "4500000".into()
        }
    );
    // Without a tip, a number opens as a block, as in the terminal.
    assert_eq!(
        route("99999999").await.unwrap(),
        Route::Block {
            id: "99999999".into()
        }
    );
}

// -------------------------------------------------------------- arguments

#[test]
fn reads_verus_style_start_arguments() {
    let parsed = start_arguments(args(&[
        "-chain=vrsctest",
        "-testnet",
        "-conf=/tmp/x/vrsctest.conf",
        "-datadir=/tmp/x",
        "-rpcconnect=10.0.0.2",
        "-rpcport=18843",
        "-rpcuser=user",
        "-rpcpassword=secret",
        "-pollinterval=5",
        "-open=vETH@",
    ]));
    assert_eq!(
        parsed,
        StartArguments {
            chain: Some("vrsctest".into()),
            testnet: true,
            config_file: Some("/tmp/x/vrsctest.conf".into()),
            data_dir: Some("/tmp/x".into()),
            rpc_host: Some("10.0.0.2".into()),
            rpc_port: Some(18843),
            rpc_user: Some("user".into()),
            rpc_password: Some("secret".into()),
            poll_interval_seconds: Some(5),
            open_object: Some("vETH@".into()),
            problem: None,
        }
    );
}

#[test]
fn reads_long_style_start_arguments() {
    let parsed = start_arguments(args(&[
        "--chain",
        "VRSC",
        "--conf",
        "/a/VRSC.conf",
        "--rpcport",
        "27486",
        "--poll-interval=3",
        "estimate:in 2 hours",
    ]));
    assert_eq!(parsed.chain.as_deref(), Some("VRSC"));
    assert_eq!(parsed.config_file.as_deref(), Some("/a/VRSC.conf"));
    assert_eq!(parsed.rpc_port, Some(27486));
    assert_eq!(parsed.poll_interval_seconds, Some(3));
    assert_eq!(parsed.open_object.as_deref(), Some("estimate:in 2 hours"));
    assert!(!parsed.testnet);

    // Separate values in Verus style, and the `--object` alias.
    let parsed = start_arguments(args(&["-chain", "vDEX", "--object", "123"]));
    assert_eq!(parsed.chain.as_deref(), Some("vDEX"));
    assert_eq!(parsed.open_object.as_deref(), Some("123"));
}

#[test]
fn ignores_system_arguments_and_reports_bad_ones() {
    let parsed = start_arguments(args(&[
        "-psn_0_12345",
        "-NSDocumentRevisionsDebugMode",
        "YES",
        "-ApplePersistenceIgnoreState",
        "YES",
        "-open",
        "vETH@@",
    ]));
    assert_eq!(parsed.problem, None);
    assert_eq!(
        parsed.chain, None,
        "no chain given: the saved setting applies"
    );
    assert_eq!(parsed.open_object.as_deref(), Some("vETH@@"));

    assert_eq!(start_arguments(args(&[])), StartArguments::default());
    assert!(
        start_arguments(args(&["-rpcport=notaport"]))
            .problem
            .is_some()
    );
    assert!(start_arguments(args(&["--unknown"])).problem.is_some());
    assert!(
        start_arguments(args(&["-open=a", "b"])).problem.is_some(),
        "an object and -open conflict, as in Carbide"
    );
}

// ------------------------------------------------------------------- live

/// `CARBIDE_TEST_CONF=… cargo test --lib live_shell -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn live_shell_mempool() {
    let explorer = super::super::tests::live_explorer();
    let full = explorer.mempool_snapshot(true).await.unwrap();
    println!(
        "mempool: {} entries, {} bytes, {} decoded",
        full.entries.len(),
        full.total_bytes,
        full.decoded
    );
    let quick = explorer.mempool_snapshot(false).await.unwrap();
    assert!(quick.entries.iter().all(|e| !e.classified));
    let txids: Vec<String> = quick.entries.iter().map(|e| e.txid.clone()).collect();
    let classified = explorer.classify_mempool(txids).await;
    println!("classified: {classified:?}");
}

#[tokio::test]
#[ignore]
async fn live_shell_recent_transactions() {
    let explorer = super::super::tests::live_explorer();
    let blocks = explorer.recent_blocks(None, 3).await.unwrap();
    let hashes = blocks.iter().map(|b| b.hash.clone()).collect();
    let rows = explorer.recent_transactions(hashes, 100).await.unwrap();
    for row in &rows {
        println!(
            "{} #{} {} {:?}",
            row.block_height, row.position, row.txid, row.tags
        );
    }
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|row| row.loaded));
    assert_eq!(rows[0].block_height, blocks[0].height);
}

#[tokio::test]
#[ignore]
async fn live_shell_peers() {
    let explorer = super::super::tests::live_explorer();
    let status = explorer.node_status().await.unwrap();
    let addresses: Vec<String> = status.peers.iter().map(|p| p.address.clone()).collect();
    let locations = explorer.peer_locations(addresses).await;
    println!("locations: {:?} {:?}", locations.state, locations.release);
    for location in &locations.locations {
        println!(
            "  {} {:?} {:?}",
            location.address, location.place, location.city
        );
    }
    if let Some(peer) = status.peers.first() {
        for group in peer_field_groups(peer.json.clone()) {
            println!("{}: {}", group.title, group.fields.len());
        }
    }
    println!("bans: {:?}", explorer.ban_list().await.unwrap());
    let totals = explorer.network_totals().await.unwrap();
    println!("totals: {totals:?}");
    assert!(totals.time_millis > 0);
}

#[tokio::test]
#[ignore]
async fn live_shell_search_and_dates() {
    let explorer = super::super::tests::live_explorer();
    let tip = explorer.chain_status().await.unwrap().blocks;
    let before = explorer
        .adjacent_block(tip.to_string(), false)
        .await
        .unwrap();
    let after = explorer
        .adjacent_block((tip - 2).to_string(), true)
        .await
        .unwrap();
    assert!(before.is_some() && after.is_some() && before == after);
    let route = explorer.resolve((tip + 1000).to_string()).await.unwrap();
    assert_eq!(
        route,
        Route::Estimate {
            query: (tip + 1000).to_string()
        }
    );
    let route = explorer.resolve("1700000000".into()).await.unwrap();
    println!("1700000000 -> {route:?}");
    assert!(matches!(route, Route::Block { .. }));
    let dates = explorer.height_dates(vec![1, tip, tip + 1440]).await;
    println!("dates: {dates:?}");
    assert_eq!(dates.len(), 3);
    assert!(dates[0].time.is_some() && !dates[0].estimated);
    assert!(dates[2].estimated);
}

// ---------------------------------------------- permission and stored data

/// An explorer with its own cache, so stored observations do not reach other tests.
/// The node configuration does not exist, so the node never answers. `api` is offered
/// and stays disabled.
fn isolated_explorer(name: &str, api: Option<&str>) -> Explorer {
    let dir = std::env::temp_dir().join(format!("carbide-shell-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    Explorer::open_with(
        ConnectionOptions {
            chain: "VRSC".into(),
            testnet: false,
            config_file: Some(format!("/nonexistent/carbide-shell-{name}/VRSC.conf")),
            api_endpoint: None,
            cache_dir: Some(dir.display().to_string()),
            ..Default::default()
        },
        &api.map(|url| vec![url.to_string()]).unwrap_or_default(),
    )
    .unwrap()
}

/// Records one answer in the explorer's cache, as Carbide's client does after a request.
async fn record(explorer: &Explorer, source: &str, method: &str, params: Value, result: Value) {
    let cache = explorer.cache.clone().expect("the test cache opens");
    let control = Arc::new(crate::sources::Control::default());
    let receipt = crate::cache::Receipt {
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
                crate::world::Network::Mainnet,
                receipt,
                source.into(),
                explorer.config.chain_cache_key(),
                method.into(),
                params.to_string(),
                response.to_string(),
            )
            .await
            .unwrap()
    );
}

/// True when no permission request arrives within a short wait.
async fn nothing_asked(explorer: &Explorer) -> bool {
    tokio::time::timeout(
        Duration::from_millis(200),
        explorer.next_permission_request(),
    )
    .await
    .is_err()
}

#[tokio::test]
async fn automatic_requests_never_ask_for_permission() {
    // Port 7 on localhost refuses connections. The API stays disabled throughout.
    let explorer = isolated_explorer("automatic", Some("http://127.0.0.1:7"));
    let limit = Duration::from_secs(10);
    macro_rules! quick {
        ($call:expr) => {
            tokio::time::timeout(limit, $call)
                .await
                .expect("an automatic request returns without waiting for the person")
        };
    }
    assert!(quick!(explorer.chain_status()).is_err());
    assert!(quick!(explorer.node_status()).is_err());
    assert!(quick!(explorer.recent_blocks(None, 3)).is_err());
    assert!(quick!(explorer.mempool_snapshot(true)).is_err());
    assert!(quick!(explorer.mempool_snapshot(false)).is_err());
    let hash = "1b".repeat(32);
    assert!(!quick!(explorer.classify_mempool(vec![hash.clone()]))[0].found);
    assert!(quick!(explorer.recent_transactions(vec![hash.clone()], 10)).is_err());
    let dates = quick!(explorer.height_dates(vec![1, 2]));
    assert!(dates.iter().all(|date| date.time.is_none()));
    assert!(quick!(explorer.ban_list()).is_err());
    assert!(quick!(explorer.network_totals()).is_err());
    assert!(quick!(explorer.adjacent_block(hash, true)).is_err());
    quick!(explorer.peer_locations(vec!["136.243.31.96:27485".into()]));
    // A search that names no object to request does not ask either.
    assert_eq!(
        quick!(explorer.resolve("4226272".into())).unwrap(),
        Route::Block {
            id: "4226272".into()
        }
    );
    assert!(nothing_asked(&explorer).await);
}

#[tokio::test]
async fn a_hash_search_asks_once_and_opens_nothing_when_cancelled() {
    let explorer = Arc::new(isolated_explorer("asks", Some("http://127.0.0.1:7")));
    let searching = explorer.clone();
    let search = tokio::spawn(async move { searching.resolve("2c".repeat(32)).await });
    let pending = explorer
        .next_permission_request()
        .await
        .expect("an explicit search asks");
    assert_eq!(pending.method, "getrawtransaction");
    explorer
        .answer_permission(pending.id, super::super::sources::PermissionAnswer::Cancel)
        .unwrap();
    assert!(search.await.unwrap().is_err());
    // After the person cancelled, the block is looked up in stored data only.
    assert!(nothing_asked(&explorer).await);
}

#[tokio::test]
async fn a_hash_search_opens_stored_objects_when_no_source_answers() {
    let explorer = isolated_explorer("stored-search", None);
    let txid = "3d".repeat(32);
    let block = "4e".repeat(32);
    let api = crate::rpc::source_id("public-api", "https://api.example.invalid");
    record(
        &explorer,
        &api,
        "getrawtransaction",
        json!([txid, 1]),
        json!({"txid": txid, "height": 9, "vin": [], "vout": []}),
    )
    .await;
    record(
        &explorer,
        &api,
        "getblock",
        json!([block]),
        json!({"hash": block, "height": 9, "time": 1700000000, "tx": []}),
    )
    .await;
    assert_eq!(
        explorer.resolve(txid.clone()).await.unwrap(),
        Route::Transaction { txid }
    );
    assert_eq!(
        explorer.resolve(block.clone()).await.unwrap(),
        Route::Block { id: block }
    );
    assert!(explorer.resolve("5f".repeat(32)).await.is_err());
}

#[tokio::test]
async fn the_dashboard_shows_stored_blocks_when_no_source_answers() {
    let explorer = isolated_explorer("stored-dashboard", None);
    let source = "local-daemon:test";
    record(
        &explorer,
        source,
        "getinfo",
        json!([]),
        json!({"blocks": 101, "longestchain": 101, "connections": 8, "tiptime": 1700000101}),
    )
    .await;
    for height in [100_u64, 101] {
        record(
            &explorer,
            source,
            "getblock",
            json!([height.to_string()]),
            json!({"hash": format!("{height:064x}"), "height": height,
                   "time": 1700000000 + height, "tx": ["aa"]}),
        )
        .await;
    }
    let status = explorer.chain_status().await.unwrap();
    assert_eq!(status.blocks, 101);
    assert!(status.provenance.retained);
    assert!(status.provenance.failure.is_some());
    // Height 99 was never stored. The stored blocks show without it.
    let blocks = explorer.recent_blocks(None, 3).await.unwrap();
    let heights: Vec<u64> = blocks.iter().map(|b| b.height).collect();
    assert_eq!(heights, [101, 100]);
    // The mempool is never served from stored data.
    assert!(explorer.mempool_snapshot(false).await.is_err());
}

#[tokio::test]
#[ignore]
async fn live_shell_records_what_the_dashboard_can_show_later() {
    let explorer = super::super::tests::live_explorer();
    let status = explorer.chain_status().await.unwrap();
    assert!(!status.provenance.retained);
    println!("status from {}", status.provenance.source);
    let blocks = explorer.recent_blocks(None, 2).await.unwrap();
    assert!(
        explorer
            .retained::<Value>("getinfo", &[], None)
            .await
            .is_some(),
        "getinfo is stored"
    );
    let params = [block_selector(&BlockId::Height(blocks[0].height))];
    assert!(
        explorer
            .retained::<BlockSummary>("getblock", &params, None)
            .await
            .is_some(),
        "a block by height is stored"
    );
    let snapshot = explorer.mempool_snapshot(false).await.unwrap();
    println!("mempool from {}", snapshot.provenance.source);
    assert!(!snapshot.provenance.source.is_empty());
}

#[test]
fn waits_run_between_bursts_and_ignore_blocks_inside_one() {
    // Three bursts: (100, 101, 102), (400, 405), (1000). A timestamp before the block
    // before (399 after 405) stays in its burst.
    let times = [100, 101, 102, 400, 405, 399, 1000];
    assert_eq!(waits_between_bursts(&times, BURST_SECONDS), vec![298, 601]);
    // A gap equal to the limit is still one burst.
    assert_eq!(waits_between_bursts(&[0, 10, 21], 10), vec![11]);
    assert!(waits_between_bursts(&[5], 10).is_empty());
}

#[test]
fn window_counts_use_only_complete_windows() {
    // One block each 60 seconds for 20 minutes, then nothing for 10 minutes, then one.
    let mut times: Vec<u64> = (0..20).map(|minute| minute * 60).collect();
    times.push(1_800);
    let counts = window_counts(&times, 600);
    // Windows start each 120 seconds: 0, 120, ... 1200. The last complete one ends at 1800.
    assert_eq!(counts.len(), 11);
    assert_eq!(counts.first(), Some(&0));
    assert_eq!(counts.last(), Some(&10));
    assert!(window_counts(&[], 600).is_empty());
    assert!(window_counts(&[1, 2, 3], 600).is_empty());
}

#[tokio::test]
#[ignore]
async fn live_shell_pulse_history_reads_without_recording() {
    let explorer = super::super::tests::live_explorer();
    let tip = explorer.chain_status().await.unwrap().blocks;
    let before = explorer.cache_info().bytes;
    let history = explorer.pulse_history(tip, 300, 1_500).await.unwrap();
    println!(
        "{} blocks over {} s, {} waits, median {:?} s, longest {:?} s, windows {:?}..{:?}",
        history.blocks,
        history.covered_seconds,
        history.waits.len(),
        history.waits.get(history.waits.len() / 2),
        history.waits.last(),
        history.window_counts.first(),
        history.window_counts.last()
    );
    assert_eq!(history.blocks, 300);
    assert_eq!(history.last_height, tip);
    assert!(!history.waits.is_empty());
    assert!(history.waits.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(history.waits.iter().all(|wait| u64::from(*wait) > BURST_SECONDS));
    assert!(!history.window_counts.is_empty());
    // A bulk read adds nothing to the cache.
    assert_eq!(explorer.cache_info().bytes, before);
}
