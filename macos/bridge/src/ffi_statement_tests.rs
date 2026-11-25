use super::super::{Context, transaction_flows, value_accounting};
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

fn state(raw: &Value, context: &Context) -> TransactionStatement {
    let flows = transaction_flows(raw, context, Some(NATIVE));
    let accounting = value_accounting(raw, &flows, context, "VRSC");
    statement(raw, &flows, context, Some(NATIVE), &accounting)
}

fn amount(list: &[StatementAmount], currency: Option<&str>) -> i64 {
    list.iter()
        .find(|amount| amount.currency_id.as_deref() == currency)
        .map_or(0, |amount| amount.sats)
}

/// Each column sums to zero: what the parties gave is what the parties received and the
/// closing row.
fn assert_balanced(statement: &TransactionStatement) {
    for column in &statement.columns {
        let parties: i64 = statement
            .parties
            .iter()
            .map(|party| amount(&party.net, column.as_deref()))
            .sum();
        assert_eq!(
            parties + amount(&statement.closing, column.as_deref()),
            0,
            "column {column:?}"
        );
    }
}

fn plain(n: u64, sats: i64, address: &str) -> Value {
    json!({"n":n,"valueSat":sats,"scriptPubKey":{"type":"pubkeyhash","addresses":[address]}})
}

#[test]
fn a_payment_is_a_giver_two_receivers_and_a_fee() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0,"valueSat":150_000_000,"address":"RA"}],
        "vout":[plain(0, 100_000_000, "RB"), plain(1, 49_990_000, "RC")]});
    let statement = state(&raw, &Context::default());
    assert!(statement.complete);
    assert_eq!(statement.columns, [None]);
    assert_eq!(statement.parties.len(), 3);
    assert_eq!(statement.parties[0].addresses, ["RA"]);
    assert_eq!(amount(&statement.parties[0].net, None), -150_000_000);
    assert_eq!(amount(&statement.parties[1].net, None), 100_000_000);
    assert_eq!(statement.closing_kind, ClosingKind::Fee);
    assert_eq!(amount(&statement.closing, None), 10_000);
    assert!(statement.marks.is_empty());
    assert_balanced(&statement);
}

#[test]
fn coins_that_go_back_to_a_paying_address_are_carried_and_not_a_change() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0,"valueSat":150_000_000,"address":"RA"}],
        "vout":[plain(0, 100_000_000, "RB"), plain(1, 49_990_000, "RA")]});
    let statement = state(&raw, &Context::default());
    let payer = &statement.parties[0];
    assert_eq!(amount(&payer.net, None), -100_010_000);
    assert_eq!(amount(&payer.carried, None), 49_990_000);
    assert_eq!((payer.inputs.as_slice(), payer.outputs.as_slice()), (&[0][..], &[1][..]));
    assert_balanced(&statement);
}

#[test]
fn many_inputs_of_one_address_are_one_party() {
    let raw = json!({"txid":"t","vin":[
        {"txid":"a","vout":0,"valueSat":100,"address":"RA"},
        {"txid":"b","vout":0,"valueSat":200,"address":"RA"},
        {"txid":"c","vout":0,"valueSat":300,"address":"RA"}],
        "vout":[plain(0, 590, "RB")]});
    let statement = state(&raw, &Context::default());
    assert_eq!(statement.parties.len(), 2);
    assert_eq!(statement.parties[0].inputs, [0, 1, 2]);
    assert_eq!(amount(&statement.parties[0].net, None), -600);
    assert_eq!(amount(&statement.closing, None), 10);
    assert_balanced(&statement);
}

#[test]
fn a_stake_has_one_party_with_no_change() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0,"valueSat":168_000_000_000_i64,"address":"RA"}],
        "vout":[plain(0, 168_000_000_000, "RA"),
                {"n":1,"valueSat":0,"scriptPubKey":{"type":"nulldata"}}]});
    let statement = state(&raw, &Context::default());
    assert_eq!(statement.parties.len(), 1);
    assert!(statement.parties[0].net.is_empty());
    assert_eq!(amount(&statement.parties[0].carried, None), 168_000_000_000);
    assert!(statement.columns.is_empty());
    assert_eq!(statement.closing_kind, ClosingKind::None);
    assert_eq!(statement.marks.len(), 1);
    assert_eq!(statement.marks[0].label, "Data");
}

#[test]
fn a_block_reward_comes_from_new_coins() {
    let raw = json!({"txid":"t","vin":[{"coinbase":"00"}],
        "vout":[plain(0, 300_010_000, "RM"),
                {"n":1,"valueSat":0,"scriptPubKey":{"type":"cryptocondition","feepool":{"version":1}}}]});
    let statement = state(&raw, &Context::default());
    assert_eq!(statement.parties[0].kind, PartyKind::NewCoins);
    assert_eq!(amount(&statement.parties[0].net, None), -300_010_000);
    assert_eq!(amount(&statement.parties[1].net, None), 300_010_000);
    assert_eq!(statement.closing_kind, ClosingKind::None);
    assert_eq!(statement.marks[0].label, "Fee Pool Checkpoint");
    assert!(!statement.marks[0].spent);
    assert_balanced(&statement);
}

#[test]
fn the_reserve_that_passes_through_an_import_is_carried() {
    let deposit = |n: u64, sats: i64, values: Value| {
        json!({"n":n,"valueSat":sats,"scriptPubKey":{"type":"cryptocondition","addresses":["RDEP"],
            "reservedeposit":{"version":1,"currencyvalues":values,"controllingcurrencyid":"iPOOL"}}})
    };
    let import = |n: u64| {
        json!({"n":n,"valueSat":0,"scriptPubKey":{"type":"cryptocondition","addresses":["RIMP"],
            "crosschainimport":{"version":1,"importcurrencyid":"iPOOL"}}})
    };
    let source = json!({"txid":"dep","vout":[
        deposit(0, 1_000, json!({})), deposit(1, 0, json!({"iTOK": 5.0})), import(2)]});
    let raw = json!({"txid":"t","vin":[
        {"txid":"dep","vout":0},{"txid":"dep","vout":1},{"txid":"dep","vout":2}],
        "vout":[import(0), plain(1, 400, "RB"),
            {"n":2,"valueSat":0,"scriptPubKey":{"type":"cryptocondition","addresses":["RB"],
                "reserveoutput":{"version":1,"currencyvalues":{"iTOK": 2.0}}}},
            deposit(3, 590, json!({})), deposit(4, 0, json!({"iTOK": 3.0}))]});
    let statement = state(&raw, &context_with(&[source]));
    assert!(statement.complete);
    assert_eq!(statement.columns, [None, Some("iTOK".to_string())]);
    assert_eq!(statement.parties.len(), 2);
    let deposits = &statement.parties[0];
    assert_eq!(deposits.kind, PartyKind::ReserveDeposits);
    assert_eq!(deposits.currency_id.as_deref(), Some("iPOOL"));
    assert_eq!(amount(&deposits.net, None), -410);
    assert_eq!(amount(&deposits.net, Some("iTOK")), -200_000_000);
    assert_eq!(amount(&deposits.carried, None), 590);
    assert_eq!(amount(&deposits.carried, Some("iTOK")), 300_000_000);
    let receiver = &statement.parties[1];
    assert_eq!(receiver.outputs, [1, 2]);
    assert_eq!(amount(&receiver.net, Some("iTOK")), 200_000_000);
    assert_eq!(statement.closing_kind, ClosingKind::InputsLessOutputs);
    assert_eq!(amount(&statement.closing, None), 10);
    let labels: Vec<(&str, bool)> = statement
        .marks
        .iter()
        .map(|mark| (mark.label.as_str(), mark.spent))
        .collect();
    assert_eq!(labels, [("Import", true), ("Import", false)]);
    assert_balanced(&statement);
}

#[test]
fn the_shielded_pool_is_a_party_with_the_public_total() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0,"valueSat":300_010_000,"address":"RA"}],
        "vout":[],"vShieldedOutput":[{}],"valueBalanceZat":-300_000_000});
    let statement = state(&raw, &Context::default());
    assert!(statement.complete);
    let pool = statement.parties.iter().find(|party| party.kind == PartyKind::ShieldedPool).unwrap();
    assert_eq!(amount(&pool.net, None), 300_000_000);
    assert_eq!(pool.outputs, [0]);
    assert_eq!(statement.closing_kind, ClosingKind::Fee);
    assert_eq!(amount(&statement.closing, None), 10_000);
    assert_balanced(&statement);
}

#[test]
fn a_reserve_transfer_gives_its_route() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0,"valueSat":700_000_000,"address":"RA"}],
        "vout":[{"n":0,"valueSat":650_020_000,"scriptPubKey":{"type":"cryptocondition","addresses":["RT"],
            "reservetransfer":{"version":1,"currencyvalues":{NATIVE: 6.5},"convert":true,
                "feecurrencyid":NATIVE,"fees":0.0002,"destinationcurrencyid":"iTO","via":"iVIA",
                "destination":{"address":"RD","type":2}}}},
            plain(1, 49_970_000, "RA")]});
    let statement = state(&raw, &Context::default());
    assert_eq!(statement.routes.len(), 1);
    let route = &statement.routes[0];
    assert_eq!(amount(&route.amounts, None), 650_000_000);
    assert_eq!((route.via.as_deref(), route.to_currency.as_deref()), (Some("iVIA"), Some("iTO")));
    assert!(route.conversion);
    assert_eq!(route.destination.as_deref(), Some("RD"));
    let waiting = statement.parties.iter().find(|party| party.kind == PartyKind::WaitingTransfers).unwrap();
    assert_eq!(amount(&waiting.net, None), 650_020_000);
    assert_eq!(statement.closing_kind, ClosingKind::InputsLessOutputs);
    assert_balanced(&statement);
}

#[test]
fn a_missing_input_value_makes_the_statement_incomplete() {
    let raw = json!({"txid":"t","vin":[{"txid":"a","vout":0}], "vout":[plain(0, 100, "RB")]});
    let statement = state(&raw, &Context::default());
    assert!(!statement.complete);
    assert_eq!(statement.closing_kind, ClosingKind::None);
    assert!(statement.closing.is_empty());
}

#[test]
fn a_transaction_with_no_expiry_height_does_not_expire() {
    assert_eq!(expiry_height(&json!({"expiryheight": 0})), None);
    assert_eq!(expiry_height(&json!({})), None);
    assert_eq!(expiry_height(&json!({"expiryheight": 4_336_416_u64})), Some(4_336_416));
}

/// A confirmed transaction, an unknown one, and the limit of one request.
#[tokio::test]
#[ignore]
async fn live_waiting_transactions() {
    let explorer = super::super::super::tests::live_explorer();
    let payment = "78ee5ec530126584f88fec33a4b561cc30c433ea5e20d851b992118ca8de730b";
    let unknown = "00000000000000000000000000000000000000000000000000000000000000aa";
    let found = explorer
        .waiting_transactions(vec![payment.into(), unknown.into()])
        .await;
    assert_eq!(found.len(), 2);
    let first = &found[0];
    println!(
        "tags {:?} expiry {:?} height {:?} parties {}",
        first.tags,
        first.expiry_height,
        first.height,
        first.statement.as_ref().map_or(0, |statement| statement.parties.len())
    );
    assert!(first.found);
    assert_eq!(first.height, Some(4_335_986));
    assert!(first.block_hash.is_some());
    assert_eq!(first.statement.as_ref().unwrap().parties.len(), 3);
    assert_balanced(first.statement.as_ref().unwrap());
    assert!(!found[1].found);
    assert!(found[1].statement.is_none());
    let many = vec![payment.to_string(); MAX_WAITING + 5];
    assert_eq!(explorer.waiting_transactions(many).await.len(), MAX_WAITING);
}

/// The eleven transactions of the design study, on the chain.
#[tokio::test]
#[ignore]
async fn live_statements_balance() {
    let explorer = super::super::super::tests::live_explorer();
    let cases = [
        ("payment", "78ee5ec530126584f88fec33a4b561cc30c433ea5e20d851b992118ca8de730b", 3, 1),
        ("block reward", "9bcd56b3615e256ec224d4d84e95ef25a68c432e4f48d25861f23f34cbfc2c22", 2, 1),
        ("stake", "92dd2967f0b5aefbd2ba787ecbad43468e003c3d78a10565c2948f5149123773", 1, 0),
        ("import", "2203780a4b337f4b5a9877a4e3113791cb884447f33a29be58240b0576362846", 7, 4),
        ("import 2", "148de4dc6e1a6ed8cd8d1c036def9ee7daac56e3f03ef025baf0ac6e3c892e9e", 6, 3),
        ("export", "1fb59142e4c5472e3e703d61c41dd6b5f2e9d0de0c1a28b3548d798b82ac6461", 2, 5),
        ("transfer", "db222dacdf8a1c2d07a9f6cbb2b30ed187b1955714dc2808cdd878a95328c18d", 2, 2),
        ("finalization", "011bb872d92bc2120e7eea09ee91837c55c2149f83533057730fe62fee8edd39", 1, 1),
        ("payout", "d42db0c064bd01f6b1939b24f4707e46cde3a15098789e4189c591d7f1e022d0", 293, 1),
        ("sweep", "3dcaa306d5ebe4b968c73ca834b862a6bdbf6a8295d07cd7c88685782ed4950e", 2, 1),
        ("shielded", "732d2c5c268eb2cd37b26ad95394b64be850266b2921ceeed2eb456d59f57847", 2, 1),
    ];
    for (name, txid, parties, columns) in cases {
        let tx = explorer.transaction(txid.into()).await.unwrap();
        let statement = &tx.statement;
        println!(
            "{name}: {} parties, columns {:?}, closing {:?} {:?}, {} marks, {} routes",
            statement.parties.len(),
            statement.columns.len(),
            statement.closing_kind,
            statement.closing,
            statement.marks.len(),
            statement.routes.len()
        );
        for party in statement.parties.iter().take(4) {
            println!("   {:?} {:?} net {:?} carried {}", party.kind, party.addresses.first(), party.net, party.carried.len());
        }
        assert!(statement.complete, "{name}");
        assert_eq!(statement.parties.len(), parties, "{name}");
        assert_eq!(statement.columns.len(), columns, "{name}");
        assert_balanced(statement);
    }
}
