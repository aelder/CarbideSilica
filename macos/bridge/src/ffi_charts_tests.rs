use super::*;
use serde_json::json;

fn hash(height: u64) -> String {
    format!("{:064x}", height + 1)
}

fn header(height: u64, time: u64, production: BlockProduction, value: Option<f64>) -> Header {
    Header {
        height,
        hash: hash(height),
        parent: height.checked_sub(1).map(hash),
        time,
        production,
        mining: (production == BlockProduction::Mined).then_some(value).flatten(),
        staking: (production == BlockProduction::Staked).then_some(value).flatten(),
        target: None,
        problem: None,
    }
}

/// Blocks 0..count, one minute apart, mined at even heights with difficulty `height` and
/// staked at odd heights with staking difficulty 1,000 + `height`.
fn chain(count: u64) -> Vec<Header> {
    (0..count)
        .map(|h| {
            if h % 2 == 0 {
                header(h, 1_000 + h * 60, BlockProduction::Mined, Some(h as f64 + 1.0))
            } else {
                header(h, 1_000 + h * 60, BlockProduction::Staked, Some(1_000.0 + h as f64))
            }
        })
        .collect()
}

fn page_of(headers: &[Header], first: u64, last: u64) -> ChartPage {
    let refs: Vec<&Header> = headers.iter().collect();
    build_page(&refs, first, last)
}

#[test]
fn intervals_are_signed_and_need_a_linked_parent() {
    let mut headers = chain(4);
    headers[2].time = headers[1].time - 30;
    let page = page_of(&headers, 0, 3);
    let intervals: Vec<_> = page.samples.iter().map(|s| s.interval).collect();
    assert_eq!(intervals, [None, Some(60), Some(-30), Some(150)]);
    assert_eq!(page.negative_intervals, 1);
    assert!(!page.samples[2].time_increases);

    // A header whose parent hash does not match has no interval, and is not linked.
    headers[3].parent = Some(hash(99));
    let page = page_of(&headers, 0, 3);
    assert_eq!(page.samples[3].interval, None);
    assert!(!page.samples[3].linked);
    // A gap in heights also breaks the link.
    let gapped = [headers[0].clone(), headers[2].clone()];
    let page = page_of(&gapped, 0, 2);
    assert_eq!(page.samples[1].interval, None);
}

#[test]
fn averages_use_the_trailing_100_heights_and_only_the_matching_production() {
    let headers = chain(250);
    let page = page_of(&headers, 100, 249);
    // At height 99 the window 0..=99 would be complete, but 99 is context. 100 is visible.
    let at = |height: u64| page.samples.iter().find(|s| s.height == height).unwrap();
    let mined = at(199).mining_average.unwrap();
    // Even heights 100..=198, values height + 1: mean is 150.
    assert_eq!(mined.samples, 50);
    assert!((mined.value - 150.0).abs() < 1e-9);
    let staked = at(199).staking_average.unwrap();
    // Odd heights 101..=199, values 1,000 + height: mean is 1,150.
    assert_eq!(staked.samples, 50);
    assert!((staked.value - 1_150.0).abs() < 1e-9);
    assert_eq!(page.context, 100);

    // The first 99 heights of a chain cannot have a full window.
    let page = page_of(&headers, 0, 249);
    assert!(page.samples[98].mining_average.is_none());
    assert!(page.samples[99].mining_average.is_some());
}

#[test]
fn averages_leave_gaps_instead_of_inventing_zeros() {
    let mut headers = chain(300);
    // A mined block without a difficulty voids the mining average for 100 heights, but
    // not the staking average.
    headers[150].mining = None;
    // An unknown block voids both.
    headers[260].production = BlockProduction::Unknown;
    headers[260].staking = None;
    let page = page_of(&headers, 0, 299);
    let at = |height: u64| &page.samples[height as usize];
    assert!(at(149).mining_average.is_some());
    assert!(at(150).mining_average.is_none());
    assert!(at(249).mining_average.is_none());
    assert!(at(250).mining_average.is_some());
    assert!(at(200).staking_average.is_some());
    assert!(at(260).staking_average.is_none() && at(260).mining_average.is_none());
    assert!(at(299).staking_average.is_none());

    // A broken link restarts the window: 99 more heights without an average.
    let mut headers = chain(300);
    headers[200].parent = Some(hash(500));
    let page = page_of(&headers, 0, 299);
    assert!(page.samples[199].mining_average.is_some());
    assert!(page.samples[200].mining_average.is_none());
    assert!(page.samples[298].mining_average.is_none());
    assert!(page.samples[299].mining_average.is_some());
    assert_eq!(page.samples[299].mining_average.unwrap().samples, 50);

    // A window with no block of the matching type has no average.
    let mined: Vec<Header> = (0..100)
        .map(|h| header(h, h * 60, BlockProduction::Mined, Some(5.0)))
        .collect();
    let page = page_of(&mined, 0, 99);
    assert!(page.samples[99].staking_average.is_none());
    assert_eq!(page.samples[99].mining_average.unwrap().samples, 100);
}

#[test]
fn ratio_windows_count_the_selected_block_and_need_complete_known_history() {
    let headers = chain(1_200);
    let page = page_of(&headers, 0, 1_199);
    let at = |height: u64| &page.samples[height as usize];
    // Partial windows give counts but no share.
    let partial = at(50).window100;
    assert_eq!((partial.mined, partial.staked, partial.total()), (26, 25, 51));
    assert!(at(50).share100.is_none());
    // Height 99 completes the 100-block window: heights 0..=99, half staked.
    assert_eq!(at(99).share100, Some(0.5));
    assert!(at(99).share1000.is_none());
    assert_eq!(at(998).window1000.total(), 999);
    assert!(at(998).share1000.is_none());
    assert_eq!(at(999).share1000, Some(0.5));
    assert_eq!(at(999).window1000.window, 1000);

    // An unknown block in a window gives counts but no trend.
    let mut headers = chain(200);
    headers[150].production = BlockProduction::Unknown;
    let page = page_of(&headers, 0, 199);
    assert_eq!(page.samples[160].window100.unknown, 1);
    assert!(page.samples[160].share100.is_none());
    assert!(page.samples[199].share100.is_none());

    // All staked gives a share of one.
    let staked: Vec<Header> = (0..100)
        .map(|h| header(h, h * 60, BlockProduction::Staked, Some(1.0)))
        .collect();
    assert_eq!(page_of(&staked, 0, 99).samples[99].share100, Some(1.0));
}

#[test]
fn ratio_lines_break_at_non_increasing_timestamps() {
    let mut headers = chain(110);
    headers[105].time = headers[104].time;
    let page = page_of(&headers, 100, 109);
    let at = |height: u64| page.samples.iter().find(|s| s.height == height).unwrap();
    assert!(at(104).time_increases);
    assert!(!at(105).time_increases);
    assert!(at(105).linked, "an equal time is linked but breaks the line");
}

#[test]
fn staking_difficulty_matches_carbide_including_the_postarget_check() {
    // Compact 0x1c0f0f0f expands to 0x0f0f0f followed by 25 zero bytes, close to the
    // 28-byte reference, so the difficulty is close to one.
    let nonce = format!("{}1c0f0f0f", "0".repeat(56));
    let (value, target) = stake_difficulty(&json!({ "nonce": nonce })).unwrap();
    assert!((value - 1.0).abs() < 1e-6, "{value}");
    assert_eq!(target, format!("000000000f0f0f{}", "0".repeat(50)));

    // A live block: compact 0x1a1b4097 and the node's own postarget agree.
    let nonce = "f5ca3dd5da98f67af95a12d300dd961aa067075e2fcd84ab746fde141a1b4097";
    let postarget = "0000000000001b40970000000000000000000000000000000000000000000000";
    let (value, target) = stake_difficulty(&json!({ "nonce": nonce, "postarget": postarget })).unwrap();
    assert_eq!(target, postarget);
    assert!(value > 1.0);

    let wrong = format!("0000000000001b4098{}", "0".repeat(46));
    let error = stake_difficulty(&json!({ "nonce": nonce, "postarget": wrong })).unwrap_err();
    assert_eq!(error.to_string(), "Nonce and postarget disagree");
    assert!(stake_difficulty(&json!({ "nonce": nonce, "postarget": 5 })).is_err());
    assert!(stake_difficulty(&json!({})).is_err());
    assert!(stake_difficulty(&json!({ "nonce": "abc" })).is_err());

    // Integer expansion: small exponents shift right, negative and overflowing targets fail.
    assert_eq!(expand_compact(0x0212_3456).unwrap()[..2], [0x34, 0x12]);
    assert!(expand_compact(0x0180_0000).is_none());
    assert!(expand_compact(0x2201_0000).is_none());
    assert!(expand_compact(0x0300_0000).is_none());

    // A staked header without a usable target keeps Carbide's reason.
    let raw = json!({"height": 7, "hash": hash(7), "previousblockhash": hash(6), "time": 9,
        "validationtype": "stake", "nonce": nonce, "postarget": wrong, "difficulty": 3.0});
    let decoded = Header::decode(&raw).unwrap();
    assert_eq!(decoded.production, BlockProduction::Staked);
    assert_eq!((decoded.staking, decoded.mining), (None, None));
    assert_eq!(decoded.problem.as_deref(), Some("Nonce and postarget disagree"));
    // The ordinary difficulty field is used only for mined blocks.
    let raw = json!({"height": 7, "hash": hash(7), "previousblockhash": hash(6), "time": 9,
        "blocktype": "mined", "difficulty": 3.0});
    assert_eq!(Header::decode(&raw).unwrap().mining, Some(3.0));
    let raw = json!({"height": 7, "hash": hash(7), "time": 9, "blocktype": "mined"});
    assert!(Header::decode(&raw).is_err(), "a parent hash is required above genesis");
}

#[test]
fn reads_fee_pool_checkpoints_exactly() {
    let coinbase = |values: Value| {
        json!({"vin": [{"coinbase": "00"}], "vout": [
            {"scriptPubKey": {"addresses": ["R1"]}},
            {"scriptPubKey": {"feepool": {"version": 1, "currencyvalues": values}}}]})
    };
    let tx = coinbase(json!({"iNative": 12.34567891, "iVRSC": 0.00000001}));
    assert_eq!(
        fee_pool_amounts(&tx, "iNative", Some("iVRSC")).unwrap(),
        Some([1_234_567_891, 1])
    );
    // A valid checkpoint may omit a currency to mean zero.
    let tx = coinbase(json!({"iNative": 5}));
    assert_eq!(fee_pool_amounts(&tx, "iNative", Some("iVRSC")).unwrap(), Some([500_000_000, 0]));
    // No checkpoint is unknown, not zero.
    let none = json!({"vin": [{"coinbase": "00"}], "vout": [{"scriptPubKey": {}}]});
    assert_eq!(fee_pool_amounts(&none, "iNative", None).unwrap(), None);
    // Malformed or unexpected data is an error.
    let bad = [
        json!({"vin": [{"txid": "a", "vout": 0}], "vout": []}),
        coinbase(json!({"iOther": 1})),
        coinbase(json!({"iNative": -1})),
        coinbase(json!({"iNative": 0.000000001})),
        coinbase(Value::Null),
    ];
    for tx in bad {
        assert!(fee_pool_amounts(&tx, "iNative", Some("iVRSC")).is_err(), "{tx}");
    }
    let mut two = coinbase(json!({"iNative": 1}));
    two["vout"][0]["scriptPubKey"]["feepool"] = json!({"version": 1, "currencyvalues": {}});
    assert!(fee_pool_amounts(&two, "iNative", None).is_err());
    let mut old = coinbase(json!({"iNative": 1}));
    old["vout"][1]["scriptPubKey"]["feepool"]["version"] = json!(2);
    assert!(fee_pool_amounts(&old, "iNative", None).is_err());
    // An unexpected currency with a zero amount is allowed.
    assert!(fee_pool_amounts(&coinbase(json!({"iOther": 0})), "iNative", None).unwrap().is_some());

    assert_eq!(decimal_sats(&json!(1e-8)), Some(1));
    assert_eq!(decimal_sats(&json!(21000000)), Some(2_100_000_000_000_000));
    assert_eq!(sats_text(1_234_567_891), "12.34567891");
    assert_eq!(sats_text(5), "0.00000005");
}

#[test]
fn fee_pool_samples_keep_a_stable_grid_and_load_endpoints_first() {
    assert_eq!(fee_pool_indices(5, None, 100), [0, 1, 2, 3, 4]);
    let grid = fee_pool_indices(1_000, None, 100);
    assert_eq!((grid.len(), grid[0], grid[99]), (100, 0, 999));
    // The selected sample replaces its nearest interior grid point.
    let with = fee_pool_indices(1_000, Some(15), 100);
    assert!(with.contains(&15) && with.contains(&20) && !with.contains(&10) && with.len() == 100);
    assert!(with.windows(2).all(|pair| pair[0] < pair[1]));

    let mut rng = rand::rng();
    let order = fetch_order(&with, Some(15), &mut rng);
    assert_eq!(&order[..3], &[0, 999, 15]);
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, with, "each grid point is read once");

    let order = fee_pool_order(3_000, None, true);
    assert_eq!(order.len(), 2_000);
    assert_eq!(&order[..2], &[0, 2_999]);
    // Detail mode includes every block of a window of up to 2,000 blocks.
    assert_eq!(fee_pool_order(2_000, None, true).len(), 2_000);
    assert_eq!(fee_pool_order(2_000, None, false).len(), 100);
}

#[test]
fn seek_input_keeps_numeric_inference_and_explicit_overrides() {
    let now = DateTime::from_timestamp(1_793_491_200, 0).unwrap();
    assert_eq!(parse_seek("4000000", now).unwrap(), SeekTarget::Number { value: 4_000_000 });
    assert_eq!(parse_seek("#0", now).unwrap(), SeekTarget::Height { value: 0 });
    assert_eq!(
        parse_seek("height:4000000", now).unwrap(),
        SeekTarget::Height { value: 4_000_000 }
    );
    assert_eq!(parse_seek("unix:0", now).unwrap(), SeekTarget::Time { unix: 0 });
    assert_eq!(parse_seek("2026-11-01", now).unwrap(), SeekTarget::Time { unix: 1_793_491_200 });
    assert_eq!(
        parse_seek("November 1 2026 14:30 UTC", now).unwrap(),
        SeekTarget::Time { unix: 1_793_543_400 }
    );
    for input in ["", "height:-1", "unix:bad", "1793491200000", "bad date"] {
        assert!(parse_seek(input, now).is_err(), "{input}");
    }
    let now = 1_793_491_200;
    assert_eq!(chart_seek_preview("4000000".into(), now, Some(4_300_000)), "Block 4000000");
    assert!(chart_seek_preview("1793491200".into(), now, Some(4_300_000)).starts_with("Unix time 2026-11-01"));
    assert!(chart_seek_preview("2026-11-01".into(), now, None).starts_with("Date and time 2026-11-01T00:00:00"));
    assert!(chart_seek_preview("".into(), now, None).starts_with("Enter"));
    assert!(chart_seek_note_has_signed_difference());
}

fn chart_seek_note_has_signed_difference() -> bool {
    seek_note(100, 90, "x").contains("difference of -10 s")
}

fn request(span: u64, end: Option<u64>, newer: bool, context: u32) -> ChartRequest {
    ChartRequest {
        span_seconds: span,
        end_height: end,
        anchor_hash: end.map(hash),
        newer,
        context,
        focus_height: None,
        focus_hash: None,
        chunk: 64,
    }
}

/// Runs a loader against a synthetic chain, as `ChartLoad::step` does against a node.
fn run(loader: &mut Loader, chain: &[Header], tip: u64) {
    loader.start(tip).unwrap();
    let mut steps = 0;
    while let Some(heights) = loader.next_chunk() {
        let headers = heights.iter().map(|h| chain[*h as usize].clone()).collect();
        loader.accept(headers).unwrap();
        steps += 1;
        assert!(steps < 1_000);
    }
}

#[test]
fn older_ranges_cover_the_span_by_header_time_and_load_context() {
    let headers = chain(2_000);
    // One hour of one-minute blocks ending at 1,500, with 99 preceding headers.
    let mut loader = Loader::new(request(3_600, Some(1_500), false, 100));
    run(&mut loader, &headers, 1_999);
    let page = loader.page().unwrap();
    assert_eq!((page.first, page.last, page.tip), (1_440, 1_500, 1_999));
    assert_eq!(page.context, 99);
    assert!(page.complete && !page.limited);
    assert!(page.samples[0].mining_average.is_some() || page.samples[0].staking_average.is_some());
    assert_eq!(page.samples.first().unwrap().interval, Some(60));

    // Timestamps decide coverage, not an assumed interval: two-minute blocks halve the count.
    let slow: Vec<Header> = (0..2_000)
        .map(|h| header(h, h * 120, BlockProduction::Mined, Some(1.0)))
        .collect();
    let mut loader = Loader::new(request(3_600, Some(1_500), false, 2));
    run(&mut loader, &slow, 1_999);
    let page = loader.page().unwrap();
    assert_eq!((page.first, page.last), (1_470, 1_500));
    assert_eq!(page.context, 1, "intervals need only the parent");

    // Genesis shortens a range; it is not complete coverage of the span.
    let mut loader = Loader::new(request(86_400, Some(100), false, 100));
    run(&mut loader, &headers, 1_999);
    let page = loader.page().unwrap();
    assert_eq!((page.first, page.last, page.context), (0, 100, 0));
}

#[test]
fn newer_ranges_do_not_overlap_and_stop_at_the_tip() {
    let headers = chain(2_000);
    let mut loader = Loader::new(request(3_600, Some(1_500), true, 100));
    run(&mut loader, &headers, 1_999);
    let page = loader.page().unwrap();
    assert_eq!((page.first, page.last), (1_501, 1_561));
    assert_eq!(page.context, 99);
    assert!(page.samples[0].mining_average.is_some() || page.samples[0].staking_average.is_some());

    let mut loader = Loader::new(request(86_400, Some(1_900), true, 100));
    run(&mut loader, &headers, 1_999);
    let page = loader.page().unwrap();
    assert_eq!((page.first, page.last), (1_901, 1_999));
    assert!(page.complete);

    let mut loader = Loader::new(request(3_600, Some(1_999), true, 100));
    loader.start(1_999).unwrap();
    assert!(loader.done && loader.stopped.is_some() && loader.page().is_none());
}

#[test]
fn a_sought_block_stays_inside_the_range() {
    let mut headers = chain(2_000);
    // A block with a time far in the past would end the span early.
    headers[1_480].time = 10;
    let mut plain = Loader::new(request(3_600, Some(1_500), false, 2));
    run(&mut plain, &headers, 1_999);
    assert_eq!(plain.page().unwrap().first, 1_480);
    let mut focused = Loader::new(ChartRequest {
        focus_height: Some(1_470),
        focus_hash: Some(hash(1_470)),
        ..request(3_600, Some(1_500), false, 2)
    });
    run(&mut focused, &headers, 1_999);
    assert!(focused.page().unwrap().first <= 1_470);
}

#[test]
fn loading_rejects_a_changed_chain_and_reports_the_header_limit() {
    let headers = chain(2_000);
    let mut loader = Loader::new(ChartRequest {
        anchor_hash: Some(hash(77)),
        ..request(3_600, Some(1_500), false, 2)
    });
    loader.start(1_999).unwrap();
    let first = loader.next_chunk().unwrap();
    let error = loader
        .accept(first.iter().map(|h| headers[*h as usize].clone()).collect())
        .unwrap_err();
    assert!(error.to_string().contains("not the requested block"));

    let mut changed = headers.clone();
    changed[1_450].hash = hash(9_999);
    let mut loader = Loader::new(request(3_600, Some(1_500), false, 2));
    loader.start(1_999).unwrap();
    let mut failed = false;
    while let Some(heights) = loader.next_chunk() {
        if loader
            .accept(heights.iter().map(|h| changed[*h as usize].clone()).collect())
            .is_err()
        {
            failed = true;
            break;
        }
    }
    assert!(failed, "a broken parent link stops loading");

    // Fast blocks: a week of one-second blocks needs more than 20,000 headers.
    let fast: Vec<Header> = (0..25_000)
        .map(|h| header(h, h, BlockProduction::Mined, Some(1.0)))
        .collect();
    let mut loader = Loader::new(ChartRequest {
        chunk: 2_000,
        ..request(604_800, Some(24_999), false, 100)
    });
    run(&mut loader, &fast, 24_999);
    let page = loader.page().unwrap();
    assert!(loader.limited && !page.complete && page.limited);
    assert_eq!(page.headers as usize, MAX_HEADERS);
    assert!(loader.stopped.as_deref().unwrap().contains("20000-header limit"));
}

// ------------------------------------------------------------- live tests

fn live() -> Arc<Explorer> {
    Arc::new(super::super::tests::live_explorer())
}

async fn load_all(explorer: &Arc<Explorer>, request: ChartRequest) -> (ChartPage, std::time::Duration) {
    let started = std::time::Instant::now();
    let load = explorer.clone().chart_load(request);
    loop {
        let progress = load.step().await.unwrap();
        if progress.done {
            println!("stopped: {:?}", progress.stopped);
            break;
        }
    }
    (load.page().await.unwrap(), started.elapsed())
}

#[tokio::test]
#[ignore]
async fn live_chart_load_spans() {
    let explorer = live();
    for (span, context) in [(7_200, 100), (86_400, 100), (604_800, 1_000)] {
        let (page, elapsed) = load_all(
            &explorer,
            ChartRequest {
                span_seconds: span,
                end_height: None,
                anchor_hash: None,
                newer: false,
                context,
                focus_height: None,
                focus_hash: None,
                chunk: 500,
            },
        )
        .await;
        let first = page.samples.first().unwrap();
        let last = page.samples.last().unwrap();
        println!(
            "span {span}: heights {}..{} ({} samples, {} headers, context {}) covering {} s in {:.1?}; status {}; negative {}; unavailable stake {}",
            page.first,
            page.last,
            page.samples.len(),
            page.headers,
            page.context,
            last.time - first.time,
            elapsed,
            page.status,
            page.negative_intervals,
            page.unavailable_stake
        );
        println!(
            "  last: {:?} mining MA {:?} staking MA {:?} share100 {:?} share1000 {:?}",
            last.production, last.mining_average, last.staking_average, last.share100, last.share1000
        );
        assert!(page.complete);
        assert!(last.time - first.time >= span.saturating_sub(600));
        assert!(page.samples.iter().any(|s| s.staking.is_some()));
    }
}

/// The short span the app offers first through a public API, read in small steps.
#[tokio::test]
#[ignore]
async fn live_chart_load_two_hours_in_small_steps() {
    let explorer = live();
    let (page, elapsed) = load_all(
        &explorer,
        ChartRequest {
            span_seconds: 7_200,
            end_height: None,
            anchor_hash: None,
            newer: false,
            context: 100,
            focus_height: None,
            focus_hash: None,
            chunk: 25,
        },
    )
    .await;
    println!(
        "2 hours: {}..{}, {} headers in {elapsed:.1?}, status {}",
        page.first, page.last, page.headers, page.status
    );
    assert!(page.complete);
}

#[tokio::test]
#[ignore]
async fn live_chart_pages_older_and_newer() {
    let explorer = live();
    let base = |end: Option<u64>, newer: bool| ChartRequest {
        span_seconds: 7_200,
        end_height: end,
        anchor_hash: None,
        newer,
        context: 100,
        focus_height: None,
        focus_hash: None,
        chunk: 500,
    };
    let (latest, _) = load_all(&explorer, base(None, false)).await;
    let (older, _) = load_all(&explorer, base(Some(latest.first - 1), false)).await;
    assert_eq!(older.last + 1, latest.first);
    let (newer, _) = load_all(&explorer, base(Some(older.last), true)).await;
    assert_eq!(newer.first, older.last + 1);
    println!(
        "latest {}..{}, older {}..{}, newer {}..{}",
        latest.first, latest.last, older.first, older.last, newer.first, newer.last
    );
}

#[tokio::test]
#[ignore]
async fn live_chart_seek() {
    let explorer = live();
    let now = Utc::now();
    // A number above the tip height but before genesis time is neither a block nor a time.
    let between = explorer.chart_seek(parse_seek("99999999", now).unwrap(), 86_400).await;
    assert!(between.is_err(), "{between:?}");
    for input in ["4000000", "2026-01-01", "unix:1767225600", "height:3000000", "2099-01-01"] {
        let target = parse_seek(input, now).unwrap();
        let outcome = explorer.chart_seek(target, 86_400).await;
        println!("{input}: {outcome:?}");
        let outcome = outcome.unwrap();
        if !outcome.future {
            assert!(outcome.end.height >= outcome.block.height);
        }
    }
    assert!(explorer.chart_seek(parse_seek("2099-01-01", now).unwrap(), 7_200).await.unwrap().future);
}

#[tokio::test]
#[ignore]
async fn live_fee_pool() {
    let explorer = live();
    let current = explorer.fee_pool_current().await.unwrap();
    println!("current: {current:?}");
    assert!(current.checkpoint.known);
    let tip = current.checkpoint.height;
    let blocks: Vec<ChartBlock> = blocks_at(&explorer, &[tip - 10, tip - 5, 1]).await;
    let checkpoints = explorer.fee_pool_checkpoints(blocks).await.unwrap();
    println!("checkpoints: {checkpoints:?}");
    assert!(checkpoints[0].known && checkpoints[1].known);
    assert!(!checkpoints[2].known, "block 1 predates the fee pool");
}

async fn blocks_at(explorer: &Explorer, heights: &[u64]) -> Vec<ChartBlock> {
    let headers = fetch_headers(&explorer.unrecorded_client(), heights).await.unwrap();
    headers
        .into_iter()
        .map(|(h, _)| ChartBlock {
            height: h.height,
            hash: h.hash,
            time: h.time,
        })
        .collect()
}

#[tokio::test]
#[ignore]
async fn live_forecast() {
    let explorer = live();
    for query in ["5000000", "estimate:in 2 hours", "2028-11-08", "unix:1857254400", "100"] {
        println!("{query}: {:?}", explorer.forecast(query.into()).await);
    }
    let forecast = explorer.forecast("5000000".into()).await.unwrap();
    assert!(forecast.target_is_height && forecast.time > forecast.observed_at);
    assert!(explorer.forecast("100".into()).await.is_err(), "an existing height has no estimate");
}

#[test]
fn time_headers_are_checked_and_source_labels_hide_credentials() {
    let raw = json!({"height": 5, "hash": hash(5), "previousblockhash": hash(4), "time": 77});
    let block = time_header(&raw, 5).unwrap();
    assert_eq!((block.height, block.timestamp, block.parent), (5, 77, Some(hash(4))));
    assert!(time_header(&raw, 6).is_err());
    assert!(time_header(&json!({"height": 5, "hash": hash(5), "time": 77}), 5).is_err());
    assert!(time_header(&json!({"height": 0, "hash": hash(0), "time": 77}), 0).is_ok());
    assert!(time_header(&json!({"height": 0, "hash": hash(0), "time": -1}), 0).is_err());

    assert_eq!(
        endpoint_label("Local daemon", "http://user:secret@[::1]:27486/private?token=secret"),
        "Local daemon — [::1]:27486"
    );
    assert_eq!(
        endpoint_label("API", "https://user:secret@example.org:8443/private-token?key=secret#token"),
        "API — https://example.org:8443"
    );
}

// ------------------------------------------------------------- walk tests

/// A source made of samples: an active chain, blocks only reachable by hash (a side
/// branch), and headers recorded in the cache.
#[derive(Default)]
struct Samples {
    active: Vec<Header>,
    by_hash: std::collections::HashMap<String, Header>,
    recorded: std::collections::HashMap<String, Stored>,
    offline: bool,
    /// After this many height requests, answers come from another source.
    switch_after: Option<usize>,
    height_requests: usize,
    heights_read: usize,
}

impl Samples {
    fn chain(count: u64) -> Self {
        let active = chain(count);
        let by_hash = active.iter().map(|h| (h.hash.clone(), h.clone())).collect();
        Self {
            active,
            by_hash,
            ..Default::default()
        }
    }

    fn source(&self) -> String {
        if self.switch_after.is_some_and(|n| self.height_requests > n) {
            "public-api:other".into()
        } else {
            "local-daemon:test".into()
        }
    }

    fn record(&mut self, heights: std::ops::RangeInclusive<u64>) {
        for height in heights {
            let header = self.active[height as usize].clone();
            self.recorded.insert(
                header.hash.clone(),
                Stored {
                    header,
                    source: "local-daemon:test".into(),
                    observed_at: 1_000 + height,
                },
            );
        }
    }
}

impl HeaderSource for Samples {
    async fn tip(&mut self) -> Result<(u64, String, String)> {
        anyhow::ensure!(!self.offline, "error sending request");
        let tip = self.active.last().unwrap();
        Ok((tip.height, tip.hash.clone(), self.source()))
    }
    async fn active_hash(&mut self, height: u64) -> Result<(String, String)> {
        Ok((self.active[height as usize].hash.clone(), self.source()))
    }
    async fn by_hash(&mut self, hash: &str) -> Result<Option<(Header, String)>> {
        Ok(self.by_hash.get(hash).cloned().map(|h| (h, self.source())))
    }
    async fn by_heights(&mut self, heights: &[u64]) -> Result<Vec<(Header, String)>> {
        self.height_requests += 1;
        self.heights_read += heights.len();
        let source = self.source();
        Ok(heights.iter().map(|h| (self.active[*h as usize].clone(), source.clone())).collect())
    }
    async fn recorded(&mut self, hash: &str) -> Option<Stored> {
        self.recorded.get(hash).cloned()
    }
    async fn recorded_at_height(&mut self, height: u64) -> Option<Stored> {
        self.recorded.values().find(|s| s.header.height == height).cloned()
    }
    async fn recorded_tip(&mut self) -> Option<Stored> {
        self.recorded.values().max_by_key(|s| s.header.height).cloned()
    }
    fn label(&self, source: &str) -> String {
        source.into()
    }
}

async fn walk_all(walk: &mut Walk, source: &mut Samples) -> Result<()> {
    for _ in 0..1_000 {
        if walk.step(source).await?.done {
            return Ok(());
        }
    }
    panic!("the walk did not finish");
}

fn branch_hash(height: u64) -> String {
    format!("{:064x}", 900_000 + height)
}

/// Active chain 0..2000 and a side branch at heights 1451..=1500 that leaves the active
/// chain after block 1450.
fn with_branch() -> Samples {
    let mut samples = Samples::chain(2_000);
    for height in 1_451..=1_500 {
        let mut header = samples.active[height as usize].clone();
        header.hash = branch_hash(height);
        header.parent = Some(if height == 1_451 { hash(1_450) } else { branch_hash(height - 1) });
        samples.by_hash.insert(header.hash.clone(), header);
    }
    samples
}

#[tokio::test]
async fn a_side_branch_anchor_follows_parent_hashes_to_the_active_chain() {
    let mut samples = with_branch();
    let mut walk = Walk::new(ChartRequest {
        anchor_hash: Some(branch_hash(1_500)),
        chunk: 500,
        ..request(3_600, Some(1_500), false, 2)
    });
    walk_all(&mut walk, &mut samples).await.unwrap();
    let page = walk.loader.page().unwrap();
    assert_eq!((page.first, page.last), (1_440, 1_500));
    let at = |height: u64| &walk.loader.headers[&height];
    assert_eq!(at(1_500).hash, branch_hash(1_500));
    assert_eq!(at(1_451).hash, branch_hash(1_451));
    assert_eq!(at(1_450).hash, hash(1_450), "the branch joins the active chain");
    assert!(page.samples.iter().all(|s| s.linked));
    assert!(walk.branch_seen && !walk.branch);
    assert!(walk.loader.status.starts_with("Side branch"));
    // Only the active part was read by height.
    assert!(samples.heights_read > 0);

    // A branch block that the node no longer has stops the walk with a plain message.
    let mut samples = with_branch();
    samples.by_hash.remove(&branch_hash(1_470));
    let mut walk = Walk::new(ChartRequest {
        anchor_hash: Some(branch_hash(1_500)),
        ..request(3_600, Some(1_500), false, 2)
    });
    let error = walk_all(&mut walk, &mut samples).await.unwrap_err();
    assert!(error.to_string().contains("no longer has block"), "{error}");

    // Newer blocks of a side branch are not known.
    let mut samples = with_branch();
    let mut walk = Walk::new(ChartRequest {
        anchor_hash: Some(branch_hash(1_500)),
        ..request(3_600, Some(1_500), true, 2)
    });
    assert!(walk_all(&mut walk, &mut samples).await.is_err());
}

#[tokio::test]
async fn recorded_headers_are_read_before_the_source() {
    let mut samples = Samples::chain(2_000);
    samples.record(1_300..=1_500);
    let mut walk = Walk::new(ChartRequest {
        chunk: 500,
        ..request(3_600, Some(1_500), false, 100)
    });
    walk_all(&mut walk, &mut samples).await.unwrap();
    let page = walk.loader.page().unwrap();
    assert_eq!((page.first, page.last), (1_440, 1_500));
    assert_eq!(samples.heights_read, 0, "every header was recorded");
    assert_eq!(walk.stored, 160);
    assert_eq!(walk.loader.status, "Observed active chain");
}

#[tokio::test]
async fn without_a_source_the_recorded_headers_are_drawn_as_retained() {
    let mut samples = Samples::chain(2_000);
    samples.offline = true;
    samples.record(1_420..=1_500);
    let mut walk = Walk::new(request(3_600, Some(1_500), false, 2));
    walk_all(&mut walk, &mut samples).await.unwrap();
    let page = walk.loader.page().unwrap();
    assert_eq!((page.first, page.last), (1_440, 1_500));
    assert!(walk.offline.is_some());
    assert_eq!(walk.oldest_stored(), Some(("local-daemon:test".into(), 2_439)));
    assert!(walk.loader.status.starts_with("Stored observations"));

    // Fewer recorded headers than the span: a partial page and a plain message.
    let mut samples = Samples::chain(2_000);
    samples.offline = true;
    samples.record(1_480..=1_500);
    let mut walk = Walk::new(request(3_600, None, false, 2));
    walk_all(&mut walk, &mut samples).await.unwrap();
    assert_eq!(walk.loader.page().unwrap().first, 1_480);
    assert!(walk.loader.stopped.as_deref().unwrap().contains("No source answered"));

    // Nothing recorded: an error that says why.
    let mut samples = Samples::chain(2_000);
    samples.offline = true;
    let mut walk = Walk::new(request(3_600, Some(1_500), false, 2));
    assert!(walk_all(&mut walk, &mut samples).await.is_err());
}

#[tokio::test]
async fn a_change_of_source_during_loading_stops_the_walk() {
    let mut samples = Samples::chain(2_000);
    samples.switch_after = Some(1);
    let mut walk = Walk::new(ChartRequest {
        chunk: 20,
        ..request(3_600, Some(1_500), false, 100)
    });
    let error = walk_all(&mut walk, &mut samples).await.unwrap_err();
    assert!(error.to_string().contains("The source changed during loading"), "{error}");
}

/// Charts read without recording: a 7-day load leaves Carbide's cache the same size, and
/// a second load takes about as long as the first.
#[tokio::test]
#[ignore]
async fn live_chart_load_leaves_the_cache_unchanged() {
    let explorer = live();
    let usage = || crate::cache::cache_disk_usage(&explorer.config.cache_path).unwrap_or(0);
    let week = || ChartRequest {
        span_seconds: 604_800,
        end_height: None,
        anchor_hash: None,
        newer: false,
        context: 1_000,
        focus_height: None,
        focus_hash: None,
        chunk: 500,
    };
    // Open the cache with one ordinary request first, so its size is settled.
    let _ = explorer.chain_status().await;
    let before = usage();
    let (first, first_time) = load_all(&explorer, week()).await;
    let middle = usage();
    let (second, second_time) = load_all(&explorer, week()).await;
    let after = usage();
    println!(
        "cache bytes: before {before}, after first load {middle}, after second load {after}; \
         first load {} headers in {first_time:.1?}, second {} headers in {second_time:.1?}, {} recorded headers reused",
        first.headers, second.headers, second.stored
    );
    assert!(first.complete && second.complete);
    assert!(after <= before + 256 * 1024, "a chart load must not grow the cache");
}
