use super::*;
use chrono::TimeZone;
use serde_json::json;

fn listing(name: &str, options: Option<u64>, flags: Option<u64>) -> CurrencyListing {
    let state = flags.map_or(json!({}), |flags| json!({"flags": flags, "supply": 10.0}));
    let mut definition = json!({"name": name, "currencyid": format!("i{name}")});
    if let Some(options) = options {
        definition["options"] = json!(options);
    }
    currency_listings(&[json!({"currencydefinition": definition, "bestcurrencystate": state})])
        .remove(0)
}

// ------------------------------------------------------------------ currency browser

#[test]
fn listings_keep_carbide_option_tags_and_launch_state() {
    let rows = currency_listings(&[
        json!({"currencydefinition": {"name": "b", "fullyqualifiedname": "Bridge.vETH",
            "currencyid": "iB", "options": 33, "startblock": 7,
            "currencies": ["iV", "iE"], "currencynames": {"iV": "VRSC", "iE": "iE"}},
            "bestcurrencystate": {"flags": 49, "supply": 5.5}}),
        json!({"currencydefinition": {"name": "a", "currencyid": "iA", "options": 0x2000},
            "lastconfirmedcurrencystate": {"flags": 5}}),
        json!({"currencydefinition": {"name": "a", "currencyid": "iA", "options": 0x2000}}),
        json!({"currencydefinition": {"name": "no id"}}),
    ]);
    assert_eq!(rows.len(), 2, "one row per ID, rows without an ID dropped");
    assert_eq!(rows[0].name, "a");
    assert_eq!(rows[0].option_tags, ["Unknown options 0x2000"]);
    assert_eq!(rows[1].name, "Bridge.vETH");
    assert_eq!(rows[1].option_tags, ["Fractional", "Token"]);
    assert_eq!(rows[1].launch, LaunchState::Launched);
    assert_eq!(rows[1].start_block, Some(7));
    assert_eq!(rows[1].supply, Some(5.5));
    assert_eq!(rows[1].reserve_ids, ["iV", "iE"]);
    assert_eq!(rows[1].reserve_names, ["VRSC", ""], "an ID is not a name");
    assert_eq!(listing_tags(Some(0)), ["No option flags"]);
    assert_eq!(listing_tags(None), ["Options unavailable"]);
}

#[test]
fn default_filter_shows_pools_and_hides_known_failed_launches() {
    let rows = vec![
        listing("pool", Some(1), Some(49)),
        listing("token", Some(0x20), Some(49)),
        listing("refunding", Some(1), Some(5)),
        listing("prelaunch", Some(0x21), Some(3)),
        listing("unknown", Some(1), None),
        listing("nooptions", None, Some(49)),
    ];
    let names = |filter: &CurrencyFilter| {
        filter_currencies(rows.clone(), filter.clone())
            .into_iter()
            .map(|index| rows[index as usize].name.clone())
            .collect::<Vec<_>>()
    };
    let filter = default_currency_filter();
    assert_eq!(names(&filter), ["pool", "prelaunch", "unknown"]);

    let mut all = filter.clone();
    all.launch_mask = LaunchStatus::ALL_MASK;
    assert_eq!(names(&all), ["pool", "refunding", "prelaunch", "unknown"]);

    let mut failed = all.clone();
    failed.launch_mask = launch_state_bit(LaunchState::Failed);
    assert_eq!(names(&failed), ["refunding"]);

    let mut any = all.clone();
    any.required = 0;
    assert_eq!(names(&any).len(), 6, "no flag filter includes a definition without options");

    // Show and Hide together: pools that are not tokens.
    let hide_token = cycle_currency_flag(cycle_currency_flag(any.clone(), 0x20), 0x20);
    assert_eq!(currency_flag_state(hide_token.clone(), 0x20), FlagState::Hide);
    let mut pools = cycle_currency_flag(hide_token, 1);
    assert_eq!(names(&pools), ["pool", "refunding", "unknown"]);

    pools.text = "REFUND".into();
    assert_eq!(names(&pools), ["refunding"]);
    pools.text = "irefunding".into();
    assert_eq!(names(&pools), ["refunding"], "the text filter matches IDs");
    pools.text = "fractional".into();
    assert_eq!(names(&pools).len(), 3, "the text filter matches option tags");
}

#[test]
fn flags_cycle_any_show_hide() {
    let filter = CurrencyFilter {
        required: 0,
        excluded: 0,
        launch_mask: 15,
        text: String::new(),
    };
    let show = cycle_currency_flag(filter.clone(), 4);
    assert_eq!(currency_flag_state(show.clone(), 4), FlagState::Show);
    let hide = cycle_currency_flag(show, 4);
    assert_eq!(currency_flag_state(hide.clone(), 4), FlagState::Hide);
    assert_eq!((hide.required, hide.excluded), (0, 4));
    let any = cycle_currency_flag(hide, 4);
    assert_eq!(any, filter);
    assert_eq!(currency_option_flags().len(), 13);
}

// ------------------------------------------------------------------------- seek

#[test]
fn seek_inference_matches_chart_seeking() {
    let now = chrono::Utc.with_ymd_and_hms(2026, 11, 24, 12, 0, 0).unwrap();
    let parse = |input: &str| parse_seek(input, now);
    assert_eq!(parse("3962166").unwrap(), SeekTarget::Number(3962166));
    assert_eq!(parse("height: 12").unwrap(), SeekTarget::Height(12));
    assert_eq!(parse("#12").unwrap(), SeekTarget::Height(12));
    assert_eq!(parse("unix:1700000000").unwrap(), SeekTarget::Time(1_700_000_000));
    let midnight = chrono::Utc.with_ymd_and_hms(2024, 3, 1, 0, 0, 0).unwrap().timestamp();
    assert_eq!(parse("2024-03-01").unwrap(), SeekTarget::Time(midnight), "a date means 00:00 UTC");
    assert!(parse("").is_err());
    assert!(parse("#-1").is_err());
    assert!(parse("100000000000").is_err(), "millisecond timestamps are refused");
    assert!(parse("11/07/2026").is_err(), "ambiguous numeric dates are refused");

    let tip = 4_000_000;
    assert_eq!(settle_seek(SeekTarget::Number(5), tip).unwrap(), SeekTarget::Height(5));
    assert_eq!(settle_seek(SeekTarget::Number(tip), tip).unwrap(), SeekTarget::Height(tip));
    assert_eq!(
        settle_seek(SeekTarget::Number(1_700_000_000), tip).unwrap(),
        SeekTarget::Time(1_700_000_000)
    );
    assert!(settle_seek(SeekTarget::Height(tip + 1), tip).is_err(), "future states are refused");
}

#[test]
fn state_reserves_carry_the_last_batch() {
    let state = json!({"reservecurrencies": [
        {"currencyid": "iV", "weight": 0.25, "reserves": 100.0, "priceinreserve": 2.0},
        {"weight": 0.25}],
        "currencies": {"iV": {"reservein": 1.5, "reserveout": 0.5, "conversionfees": 0.01}}});
    let reserves = state_reserves(&state);
    assert_eq!(reserves.len(), 1);
    assert_eq!(reserves[0].reserve_in, 1.5);
    assert_eq!(reserves[0].reserve_out, 0.5);
    assert_eq!(reserves[0].price_in_reserve, Some(2.0));
}

#[test]
fn activity_records_round_trip_and_windows_stop_at_the_anchor() {
    let record = ActivityRecord {
        height: 10,
        kind: ActivityKind::Import,
        output: OutputRef {
            system_id: "iDestination".into(),
            txid: "a".repeat(64),
            output: 3,
        },
        export: Some(OutputRef {
            system_id: "iSource".into(),
            txid: "b".repeat(64),
            output: 7,
        }),
        transfer_hash: None,
    };
    assert_eq!(activity_record(&core_record(&record).unwrap()), record);
    let window = activity_window_ending_at(3_962_166);
    assert_eq!((window.first, window.last), (3_961_167, 3_962_166));
    let range = Range { first: window.first, last: window.last };
    assert_eq!(range.newer(window.last), None);
    assert_eq!(range.older().unwrap().last, 3_961_166);
}

// ---------------------------------------------------------------------- marketplace

fn sample_offers() -> Value {
    json!({
        "ids_for_currency_iV": [
            {"identityid": "iGlam", "price": 375.0, "offer": {
                "offer": {"name": "Glam", "identityid": "iGlam"},
                "accept": {"iV": 375.0}, "blockexpiry": 500, "txid": "t-glam"}},
            {"identityid": "iAbc", "price": 21.0, "offer": {
                "offer": {"name": "abc", "identityid": "iAbc"},
                "accept": {"iV": 21}, "blockexpiry": 300, "txid": "t-abc"}}
        ],
        "currency_iV_offers_in_currency_iD": [
            {"currencyid": "iD", "price": 1000.0, "offer": {
                "offer": {"iV": 0.1}, "accept": {"iD": 100.0},
                "blockexpiry": 400, "txid": "t-swap"}}
        ],
        "nested": {"deeper": [
            {"offer": {"offer": {"iV": 0.0001, "iNft": 1e-8},
                "accept": {"name": "Wanted", "identityid": "iWanted"},
                "blockexpiry": 0, "txid": "t-wanted"}},
            {"offer": {"offer": {"iV": 0.0001, "iNft": 1e-8},
                "accept": {"name": "Wanted", "identityid": "iWanted"},
                "blockexpiry": 0, "txid": "t-wanted"}}
        ]}
    })
}

#[test]
fn offers_are_found_at_any_depth_with_exact_asks_and_associations() {
    let offers = parse_offers(&sample_offers());
    let ids: Vec<_> = offers.iter().map(|offer| offer.txid.as_str()).collect();
    assert_eq!(ids, ["t-wanted", "t-abc", "t-swap", "t-glam"], "sorted by expiry, repeats removed");
    let wanted = &offers[0];
    assert_eq!(wanted.category, "deeper");
    assert_eq!(wanted.kind, OfferKind::IdentityWanted);
    assert_eq!(wanted.ask_sats, None, "an identity is not an amount");
    assert_eq!(
        wanted.links,
        [
            OfferLink { kind: OfferLinkKind::Funding, value: "t-wanted".into() },
            OfferLink { kind: OfferLinkKind::Currency, value: "iNft".into() },
            OfferLink { kind: OfferLinkKind::Currency, value: "iV".into() },
            OfferLink { kind: OfferLinkKind::Identity, value: "iWanted".into() },
        ]
    );
    let glam = &offers[3];
    assert_eq!(glam.kind, OfferKind::IdentityForSale);
    assert_eq!(glam.subject_id.as_deref(), Some("iGlam"));
    assert_eq!((glam.ask_currency.as_deref(), glam.ask_sats), (Some("iV"), Some(37_500_000_000)));
    assert_eq!(glam.offered.identity_name.as_deref(), Some("Glam"));
    let swap = &offers[2];
    assert_eq!(swap.kind, OfferKind::CurrencySwap);
    assert_eq!(swap.subject_id.as_deref(), Some("iD"));
    assert_eq!(swap.offered.amounts.len(), 1);

    let links = offer_links_with(glam.clone(), vec!["RAddress".into(), "RAddress".into()]);
    assert_eq!(links.last().unwrap(), &OfferLink { kind: OfferLinkKind::Address, value: "RAddress".into() });
    assert_eq!(links.len(), glam.links.len() + 1);
}

#[test]
fn asks_use_exact_satoshis() {
    assert_eq!(decimal_sats(&json!(375.0)), Some(37_500_000_000));
    assert_eq!(decimal_sats(&json!(3.9e-7)), Some(39));
    assert_eq!(decimal_sats(&json!("0.00000001")), Some(1));
    assert_eq!(decimal_sats(&json!(1e-9)), None, "finer than a satoshi");
    assert_eq!(asking_price(&json!({"iV": 2, "iD": 1})), None, "a bundle is not one ask");
    assert_eq!(asking_price(&json!({"iV": 0})), None);
}

#[test]
fn offers_sort_four_ways_in_both_directions() {
    let offers = parse_offers(&sample_offers());
    let order = |sort, descending, times: &[(&str, u64)]| {
        let names = HashMap::from([("iD".to_string(), "DAI.vETH".to_string())]);
        let times = times.iter().map(|(t, v)| (t.to_string(), *v)).collect();
        sort_offers(offers.clone(), sort, descending, names, times)
            .into_iter()
            .map(|offer| offer.txid)
            .collect::<Vec<_>>()
    };
    assert_eq!(order(OfferSort::Expiry, false, &[]), ["t-wanted", "t-abc", "t-swap", "t-glam"]);
    assert_eq!(order(OfferSort::Expiry, true, &[]), ["t-glam", "t-swap", "t-abc", "t-wanted"]);
    // Names: abc, dai.veth (resolved subject), deeper (category), glam.
    assert_eq!(order(OfferSort::Name, false, &[]), ["t-abc", "t-swap", "t-wanted", "t-glam"]);
    assert_eq!(order(OfferSort::Name, true, &[]), ["t-glam", "t-wanted", "t-swap", "t-abc"]);
    // Asks group by currency (iD before iV), and offers without an ask go last.
    assert_eq!(order(OfferSort::AskAmount, false, &[]), ["t-swap", "t-abc", "t-glam", "t-wanted"]);
    assert_eq!(order(OfferSort::AskAmount, true, &[]), ["t-swap", "t-glam", "t-abc", "t-wanted"]);
    // Newest listing first; unknown ages last in both directions.
    let times = [("t-abc", 100), ("t-glam", 300), ("t-swap", 200)];
    assert_eq!(order(OfferSort::ListingAge, false, &times), ["t-glam", "t-swap", "t-abc", "t-wanted"]);
    assert_eq!(order(OfferSort::ListingAge, true, &times), ["t-abc", "t-swap", "t-glam", "t-wanted"]);
}

// ------------------------------------------------------------------- notarizations

const OBSERVER: &str = "iObserver";
const OTHER: &str = "iOther";

fn hash(c: char) -> String {
    c.to_string().repeat(64)
}

fn graph_sample() -> Value {
    json!({
        "lastconfirmed": 0, "bestchain": 1, "forks": [[0, 1], [0, 2]],
        "notarizations": [
            {"index": 0, "txid": hash('a'), "vout": 1, "blockhash": hash('b'),
             "notarization": {"currencyid": OTHER, "proofroots": [
                {"systemid": OBSERVER, "height": 10, "blockhash": hash('c')},
                {"systemid": OTHER, "height": 20, "blockhash": hash('d')}]}},
            {"index": 1, "txid": hash('e'), "vout": 0, "blockhash": hash('f'),
             "notarization": {"currencyid": OTHER, "ismirror": true,
                "prevnotarizationtxid": hash('9'), "prevnotarizationout": 2,
                "proofroots": [{"systemid": OTHER, "height": 30, "blockhash": "0".repeat(64)}]}},
            {"index": 2, "txid": hash('e'), "vout": 0, "blockhash": hash('f'),
             "notarization": {"proofroots": []}},
            {"index": 3, "txid": "0".repeat(64), "vout": 0,
             "notarization": {"proofroots": []}}
        ]
    })
}

#[test]
fn notarization_graph_decodes_confirmed_pending_synthetic_and_mirror_rows() {
    let graph = decode_graph(&graph_sample(), OBSERVER).unwrap();
    assert_eq!(graph.confirmed, Some(0));
    assert_eq!(graph.rows.len(), 4);
    assert_eq!(graph.fork_count, 2);
    assert_eq!(graph.best.as_deref(), Some(&[0, 2][..]));
    assert_eq!(graph.pending(), 1, "rows 1 and 2 are the same output; row 3 is synthetic");
    assert!(graph.rows[3].synthetic);
    assert_eq!(graph.rows[1].roots[0].hash, None, "an all-zero hash is missing");
    let previous = graph.rows[1].foreign_previous.as_ref().unwrap();
    assert_eq!((previous.system_id.as_ref(), previous.vout), (OTHER, 2));
    assert!(graph.rows[0].foreign_previous.is_none());

    let mut bad = graph_sample();
    bad["lastconfirmed"] = json!(9);
    assert!(decode_graph(&bad, OBSERVER).is_err());
    let empty = json!({"notarizations": [], "lastconfirmed": -1});
    assert_eq!(decode_graph(&empty, OBSERVER).unwrap().confirmed, None);
}

#[test]
fn proof_root_checks_distinguish_every_outcome() {
    let root = Root { system: OTHER.into(), height: Some(20), hash: Some(hash('d')) };
    let found = |height, confirmations| Check::Found {
        source: "local-daemon:x".into(),
        value: Header { height, time: Some(1), confirmations },
    };
    let classify = |check: Option<Check<Header>>, root: &Root| classify_root(check.as_ref(), root).unwrap();
    assert_eq!(classify(Some(found(20, Some(3))), &root), (RootCheck::Active, "Active".into()));
    assert_eq!(classify(Some(found(20, Some(-1))), &root).0, RootCheck::KnownOffChain);
    assert_eq!(classify(Some(found(21, Some(3))), &root).0, RootCheck::HeightMismatch { found: 21 });
    assert_eq!(classify(Some(found(20, None)), &root).0, RootCheck::PresentMembershipUnknown);
    let missing = |tip| Check::NotFound { source: "s".into(), tip };
    assert_eq!(
        classify(Some(missing(Some(15))), &root),
        (RootCheck::SourceBehind { tip: 15 }, "Not found • source tip #15 is behind".into())
    );
    assert_eq!(classify(Some(missing(Some(25))), &root).0, RootCheck::NotFound);
    assert_eq!(
        classify(Some(Check::Unavailable(CheckFailure::NoSourceConfigured)), &root).0,
        RootCheck::NoSourceConfigured
    );
    assert_eq!(
        classify(Some(Check::Unavailable(CheckFailure::NoSourceEnabled)), &root).0,
        RootCheck::NoSourceEnabled
    );
    assert_eq!(
        classify(Some(Check::Unavailable(CheckFailure::BlockLookupFailed)), &root).0,
        RootCheck::BlockLookupFailed
    );
    assert_eq!(classify(None, &root).0, RootCheck::NotChecked);
    let no_hash = Root { hash: None, ..root.clone() };
    assert_eq!(classify(None, &no_hash).0, RootCheck::MissingHash);
    let ethereum = Root { system: VETH_SYSTEM_ID.into(), ..root };
    assert!(classify_root(None, &ethereum).is_none());
    assert!(
        classify_root(Some(&Check::Unavailable(CheckFailure::NoSourceConfigured)), &ethereum).is_none(),
        "a Verus source cannot check an Ethereum root"
    );
}

#[test]
fn a_known_pbaas_pair_has_two_perspectives() {
    let parent = "iVRSC";
    let pairs = |observer, subject, options, system| {
        perspective_pairs(observer, subject, parent, options, system)
    };
    assert_eq!(
        pairs(parent, "iChain", Some(0x100), Some("iChain")),
        [(parent.to_string(), "iChain".to_string()), ("iChain".to_string(), parent.to_string())]
    );
    assert_eq!(pairs(parent, "iGateway", Some(0x80), Some("iGateway")).len(), 1);
    assert_eq!(pairs(parent, "iChild", Some(0x100), Some("iOther")).len(), 1);
    assert_eq!(pairs("iChain", parent, None, None).len(), 2, "a PBaaS chain's view of VRSC");
    assert_eq!(pairs(parent, parent, None, None).len(), 1);
}

#[test]
fn block_headers_must_match_the_requested_hash() {
    let value = json!({"hash": hash('a'), "height": 5, "time": 9, "confirmations": -1});
    let header = header_of(&value, &hash('a')).unwrap();
    assert_eq!((header.height, header.time, header.confirmations), (5, Some(9), Some(-1)));
    assert!(header_of(&value, &hash('b')).is_none());
}

// ------------------------------------------------------------------------ bookmarks

#[test]
fn bookmark_kinds_and_manual_notarization_targets() {
    use crate::bookmarks::{Book, Bookmark, Kind};
    assert_eq!(bookmark_kind("Notarizations"), Some(Kind::Notarizations));
    assert_eq!(bookmark_kind(""), Some(Kind::Auto));
    assert_eq!(bookmark_kind("peer"), None);

    let dir = std::env::temp_dir().join(format!("carbide-bridge-bookmarks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bookmarks-mainnet.jsonl");
    let _ = std::fs::remove_file(&path);
    let system = SystemId::try_from("iJPsMEQ9SfFpUwyaJiumzFfnWFUeSL8bYw".to_string()).unwrap();
    let mut book = Book::load(path.clone()).unwrap();
    // The terminal's manual form: the observing chain's system ID and `notarizations:vETH`.
    let manual = Bookmark::new(system.clone(), Kind::Auto, "notarizations:vETH".into(), "lanes".into()).unwrap();
    assert_eq!((manual.kind, manual.target.as_str()), (Kind::Notarizations, "vETH"));
    book.add(manual).unwrap();
    assert!(
        Bookmark::new(system.clone(), Kind::Notarizations, "vETH@".into(), String::new()).is_err(),
        "a notarization target is a bare currency name or ID"
    );
    let reverse = Bookmark::new(
        SystemId::try_from("iOtherChain".to_string()).unwrap(),
        Kind::Notarizations,
        system.to_string(),
        String::new(),
    )
    .unwrap();
    book.add(reverse).unwrap();
    let reloaded = Book::load(path).unwrap();
    assert_eq!(reloaded.entries.len(), 2);
    assert_eq!(reloaded.entries[1].1.system_id.as_ref(), "iOtherChain", "the perspective is saved");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn startup_population_is_off_unless_carbide_config_enables_it() {
    let off = startup_options(None).unwrap();
    assert!(!off.fetch_missing_on_startup);
    assert_eq!(off.startup_limit, 20);
    let other_tables = startup_options(Some("[[connections]]\nchain = \"VRSC\"\n")).unwrap();
    assert!(!other_tables.fetch_missing_on_startup);
    let on = startup_options(Some("[bookmarks]\nfetch_missing_on_startup = true\nstartup_limit = 500\n")).unwrap();
    assert!(on.fetch_missing_on_startup);
    assert_eq!(on.startup_limit.min(STARTUP_LIMIT_CAP), 100, "capped at 100");
    assert!(startup_options(Some("[bookmarks]\nunknown = 1\n")).is_err());
    assert!(startup_options(Some("not toml = [")).is_err());
}

#[test]
fn bookmark_tags_come_from_the_cached_object() {
    use crate::bookmarks::{Kind, Snapshot};
    let snapshot = |kind, raw| Snapshot { kind, raw, source: "local-daemon:x".into() };
    assert_eq!(snapshot_tags(&snapshot(Kind::Identity, json!({}))), ["Identity"]);
    assert_eq!(snapshot_tags(&snapshot(Kind::Notarizations, json!({}))), ["Notarizations"]);
    let coinbase = json!({"txid": hash('a'), "vin": [{"coinbase": "00"}], "vout": []});
    let tags = snapshot_tags(&snapshot(Kind::Transaction, coinbase.clone()));
    assert!(!tags.is_empty(), "a coinbase transaction has a tag");
    let block = snapshot_tags(&snapshot(Kind::Block, json!({"tx": [coinbase, "txid-only"]})));
    assert_eq!(block, tags, "a block's tags are the union of its transactions' tags");
}

// ----------------------------------------------------------------------- live tests

fn live() -> Explorer {
    super::super::tests::live_explorer()
}

#[tokio::test]
#[ignore]
async fn live_currency_list() {
    let list = live().currency_list().await.unwrap();
    println!("{} currencies from {}", list.rows.len(), list.source);
    assert!(list.rows.iter().any(|row| row.name == "Bridge.vETH"));
    let shown = filter_currencies(list.rows.clone(), default_currency_filter());
    println!("{} shown by the default filter", shown.len());
    assert!(!shown.is_empty());
}

#[tokio::test]
#[ignore]
async fn live_currency_state_and_activity() {
    let explorer = live();
    let state = explorer
        .currency_state_at("Bridge.vETH".into(), "3962166".into())
        .await
        .unwrap();
    println!("{} at {} from {}: supply {:?}", state.interpreted, state.height, state.source, state.supply);
    assert_eq!(state.height, 3_962_166);
    assert_eq!(state.reserves.len(), 4);

    let dated = explorer
        .currency_state_at("Bridge.vETH".into(), "2024-06-01 12:00".into())
        .await
        .unwrap();
    let coverage = dated.coverage.unwrap();
    println!("{}", coverage.description);
    assert!(coverage.difference_seconds.abs() < 3600);

    assert!(explorer
        .currency_state_at("Bridge.vETH".into(), "#999999999".into())
        .await
        .is_err());

    let window = activity_window_ending_at(state.height);
    let page = explorer
        .currency_activity(state.currency_id.clone(), window.first, window.last, state.height, state.block_hash.clone())
        .await
        .unwrap();
    println!("{} records, truncated {}", page.records.len(), page.truncated);
    assert!(!page.records.is_empty());
    assert!(page.newer.is_none());
    let record = page
        .records
        .iter()
        .find(|record| record.export.as_ref().is_some_and(|e| e.system_id == "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV"))
        .unwrap();
    let inputs = explorer.export_inputs(record.clone()).await.unwrap();
    println!("export {}:{} spent {} inputs", inputs.export.txid, inputs.export.output, inputs.inputs.len());
    assert!(!inputs.inputs.is_empty());
}

#[tokio::test]
#[ignore]
async fn live_marketplace() {
    let explorer = live();
    let market = explorer.marketplace().await.unwrap();
    println!("{} offers for {} from {}", market.offers.len(), market.subject, market.source);
    assert!(!market.offers.is_empty());
    let txids: Vec<String> = market.offers.iter().take(5).map(|offer| offer.txid.clone()).collect();
    let funding = explorer.offer_funding(txids).await;
    for item in &funding {
        println!("{} listed {:?} addresses {:?} {:?}", item.txid, item.block_time, item.addresses, item.error);
    }
    assert_eq!(funding.len(), 5);
}

#[tokio::test]
#[ignore]
async fn live_notarization_browser() {
    let explorer = live();
    let _ = explorer.chain_status().await;
    let veth = "i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X";
    let perspectives = explorer.notarization_perspectives(veth.into(), Some("vETH".into()), Some(0x80), Some(veth.into()));
    assert_eq!(perspectives.len(), 1);
    let view = explorer
        .notarization_browser(perspectives[0].observer_id.clone(), veth.into(), None, true)
        .await
        .unwrap();
    println!("{:?}: {} rows, {} unconfirmed, source {:?}", view.load, view.rows.len(), view.unconfirmed, view.source);
    for row in &view.rows {
        println!(
            "  {} {:?}:{:?} confirmed {} included {} {:?}",
            row.index,
            row.txid.as_deref().map(|t| &t[..8]),
            row.output,
            row.confirmed,
            row.inclusion,
            row.roots.iter().map(|r| (r.system_name.clone(), r.height, r.status.clone())).collect::<Vec<_>>()
        );
    }
    assert_eq!(view.load, BrowserLoad::Ready);
    assert!(view.rows.iter().any(|row| row.included_height.is_some()));

    // A PBaaS chain configured on this Mac, whose node may not be running.
    for chain in &explorer.other_chains {
        let id = chain.chain_cache_key();
        let pair = explorer.notarization_perspectives(id.clone(), None, Some(0x100), Some(id.clone()));
        println!("{} perspectives for {}: {:?}", pair.len(), chain.chain.name, pair);
        if let Some(reverse) = pair.get(1) {
            let view = explorer
                .notarization_browser(reverse.observer_id.clone(), reverse.subject_id.clone(), None, true)
                .await
                .unwrap();
            println!("  reverse: {:?} {:?}", view.load, view.message);
        }
    }
}

#[tokio::test]
#[ignore]
async fn live_bookmark_file() {
    let explorer = live();
    let file = explorer.bookmark_file().await.unwrap();
    println!("{} bookmarks in {}; start-up population {}", file.entries.len(), file.path, file.startup_enabled);
    for entry in &file.entries {
        println!("  {} {} {:?}", entry.kind, entry.target, entry.cache);
    }
    let report = explorer.populate_bookmarks().await.unwrap();
    println!("{}", report.message);

    // A fetched object is cached, so its tags and source can be shown later.
    let _ = explorer.chain_status().await;
    let system = explorer.config.chain.system_id.get().cloned().unwrap();
    let bookmark = crate::bookmarks::Bookmark::new(
        system,
        crate::bookmarks::Kind::Block,
        "3962166".into(),
        String::new(),
    )
    .unwrap();
    crate::bookmarks::fetch(explorer.config.noninteractive(), &bookmark, false)
        .await
        .unwrap();
    let cache = crate::cache::Cache::open(&explorer.config.cache_path).unwrap();
    let snapshot = crate::bookmarks::cached(&cache, &bookmark).unwrap();
    let snapshot = snapshot.expect("the fetched block is cached");
    println!("cached from {} with tags {:?}", snapshot.source, snapshot_tags(&snapshot));
}

// ------------------------------------------------------------- stored currency states

#[test]
fn stored_states_reopen_by_height_and_keep_their_source() {
    let dir = std::env::temp_dir().join(format!("carbide-bridge-states-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mainnet.sqlite3");
    let _ = std::fs::remove_file(&path);
    // Carbide's own schema.
    drop(crate::cache::Cache::open(&path).unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let state = json!({"currencyid": "iPool", "supply": 5.0, "flags": 49,
        "reservecurrencies": [{"currencyid": "iV", "reserves": 1.0, "priceinreserve": 2.0}]});
    let response = json!([{"height": 10, "blocktime": 99, "currencystate": state.clone()}]);
    for (key, value) in [("s", &state), ("r", &response)] {
        connection
            .execute(
                "insert into raw_blobs(hash,payload) values(?1,?2)",
                rusqlite::params![key, value.to_string()],
            )
            .unwrap();
    }
    connection
        .execute(
            "insert into currency_states(system_id,source,currency_id,state_kind,observation_key,
             reported_height,observed_height,observed_block_hash,payload_hash,state_hash,request_order,observed_at)
             values('iSys','local-daemon:x','iPool','historical','k',10,10,?1,'r','s',1,1234)",
            rusqlite::params![hash('b')],
        )
        .unwrap();
    drop(connection);

    let states = stored_states(&path, "iSys", "iPool").unwrap();
    assert_eq!(states.len(), 1);
    assert_eq!((states[0].height, states[0].observed_at), (10, 1234));
    assert_eq!(states[0].source, "local-daemon:x");
    assert_eq!(state_reserves(&states[0].state).len(), 1);
    assert!(
        stored_states(&path, "iOther", "iPool").unwrap().is_empty(),
        "another system's states are not served"
    );
    assert!(stored_states(&dir.join("missing.sqlite3"), "iSys", "iPool").unwrap().is_empty());
    std::fs::remove_dir_all(dir).unwrap();

    assert_eq!(stored_height(SeekTarget::Height(5)), Some(5));
    assert_eq!(stored_height(SeekTarget::Number(3_962_166)), Some(3_962_166));
    assert_eq!(stored_height(SeekTarget::Number(1_700_000_000)), None, "a Unix time needs a source");
    assert_eq!(stored_height(SeekTarget::Time(1)), None);
}

#[tokio::test]
#[ignore]
async fn live_exact_state_is_recorded_for_offline_use() {
    let explorer = live();
    let state = explorer
        .currency_state_at("Bridge.vETH".into(), "#3962166".into())
        .await
        .unwrap();
    assert!(!state.provenance.retained);
    let stored = stored_states(
        &explorer.config.cache_path,
        &explorer.config.chain_cache_key(),
        &state.currency_id,
    )
    .unwrap();
    println!("{} stored states; newest at {:?}", stored.len(), stored.first().map(|s| s.height));
    assert!(stored.iter().any(|s| s.height == 3_962_166 && s.block_hash == state.block_hash));
    let list = explorer.currency_list().await.unwrap();
    println!("list from {} retained {}", list.provenance.source, list.provenance.retained);
    let market = explorer.marketplace().await.unwrap();
    println!("market from {} retained {}", market.provenance.source, market.provenance.retained);

    // The same cache with no usable source: each answer is the stored one, marked so.
    let offline = super::super::tests::explorer(None);
    let stored = offline
        .currency_state_at(state.currency_id.clone(), "3962166".into())
        .await
        .unwrap();
    println!("offline state: {} {:?}", stored.interpreted, stored.provenance.failure);
    assert!(stored.provenance.retained);
    assert_eq!(stored.block_hash, state.block_hash);
    assert!(offline.currency_state_at(state.currency_id.clone(), "#5".into()).await.is_err());
    assert!(offline.currency_list().await.unwrap().provenance.retained);
    assert!(offline.marketplace().await.unwrap().provenance.retained);
}
