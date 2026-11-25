use super::*;

/// Answers requests from samples, keyed by method and JSON parameters.
#[derive(Default)]
struct Fake {
    answers: HashMap<String, Value>,
    /// Errors by method, for requests that have no sample.
    errors: HashMap<String, String>,
}

impl Fake {
    fn on(mut self, method: &str, params: Value, answer: Value) -> Self {
        self.answers.insert(format!("{method} {params}"), answer);
        self
    }

    fn fails(mut self, method: &str, message: &str) -> Self {
        self.errors.insert(method.into(), message.into());
        self
    }

    fn transaction(self, tx: Value) -> Self {
        let txid = tx["txid"].as_str().unwrap().to_string();
        self.on("getrawtransaction", json!([txid, 1]), tx)
    }
}

impl Rpc for Fake {
    fn call(&self, method: &str, params: Vec<Value>) -> impl Future<Output = Answer> + Send {
        let key = format!("{method} {}", Value::Array(params));
        let answer = match self.answers.get(&key) {
            Some(value) => Ok((value.clone(), "sample".to_string())),
            None => Err(self
                .errors
                .get(method)
                .cloned()
                .unwrap_or_else(|| format!("no sample for {key}"))),
        };
        std::future::ready(answer)
    }
}

fn hash(digit: char) -> String {
    std::iter::repeat_n(digit, 64).collect()
}

// ------------------------------------------------------------- timelock

#[test]
fn timelock_has_three_meanings() {
    let vault =
        |flags: u64, timelock: u64| json!({"version": 3, "flags": flags, "timelock": timelock});
    assert_eq!(
        timelock_meaning(&json!({"version": 1, "flags": 2, "timelock": 50}), Some(10)),
        (Timelock::Unlocked, "unlocked".into())
    );
    assert_eq!(
        timelock_meaning(&vault(2, 1440), Some(10)),
        (
            Timelock::Delay { blocks: 1440 },
            "locked (1440 blocks unlock delay)".into()
        )
    );
    assert_eq!(
        timelock_meaning(&vault(2, 1), Some(10)).1,
        "locked (1 block unlock delay)"
    );
    assert_eq!(
        timelock_meaning(&vault(0, 5000), Some(4000)),
        (
            Timelock::UnlocksAt { height: 5000 },
            "locked (unlocks at height 5000)".into()
        )
    );
    // Once the chain passes the height, the identity is unlocked.
    assert_eq!(
        timelock_meaning(&vault(0, 5000), Some(6000)).0,
        Timelock::Unlocked
    );
    // Without a known tip, a height is still reported as locked.
    assert_eq!(
        timelock_meaning(&vault(0, 5000), None).0,
        Timelock::UnlocksAt { height: 5000 }
    );
    // A revoked identity is not timelocked.
    assert_eq!(
        timelock_meaning(&vault(0x8002, 9), Some(1)).0,
        Timelock::Unlocked
    );
}

// ------------------------------------------------------ version reconstruction

fn history_sample() -> Value {
    let identity = |keys: Value, revocation: &str, flags: u64| {
        json!({"version": 3, "identityaddress": "iSeed", "name": "seed", "parent": "iVRSC",
            "systemid": "iVRSC", "primaryaddresses": keys, "minimumsignatures": 1,
            "revocationauthority": revocation, "recoveryauthority": "iSeed",
            "flags": flags, "timelock": 0})
    };
    json!({"history": [
        {"height": 300, "blockhash": "b300", "output": {"txid": hash('3'), "voutnum": 0},
         "identity": identity(json!(["RNew"]), "iGuard", 0x8000)},
        {"height": 100, "blockhash": "b100", "output": {"txid": hash('1'), "voutnum": 0},
         "identity": identity(json!(["ROld"]), "iSeed", 0)},
        {"height": 200, "blockhash": "b200", "output": {"txid": hash('2'), "voutnum": 1},
         "identity": identity(json!(["ROld"]), "iGuard", 4)},
    ]})
}

#[test]
fn rebuilds_each_version_as_it_stood() {
    let versions = versions_from_history(&history_sample(), "iSeed");
    let heights: Vec<u64> = versions
        .iter()
        .map(|version| version.block_height)
        .collect();
    assert_eq!(heights, [100, 200, 300]);
    assert_eq!(versions[1].raw["txid"], hash('2'));
    assert_eq!(versions[1].raw["vout"], 1);
    assert_eq!(versions[1].raw["blockhash"], "b200");
    assert_eq!(versions[0].raw["status"], "active");
    assert_eq!(versions[2].raw["status"], "revoked");

    let mut states = Vec::new();
    let mut previous = None;
    for version in &versions {
        states.push(identity_state(&version.raw, previous, Some(1000), "seed@").unwrap());
        previous = Some(&version.raw["identity"]);
    }
    assert_eq!(states[0].changes, ["Registered"]);
    assert!(states[0].revocation_is_self && states[0].recovery_is_self);
    assert!(!states[0].tokenized_control);
    assert_eq!(states[1].changes, ["Revocation authority changed"]);
    assert_eq!(states[1].revocation_authority.as_deref(), Some("iGuard"));
    assert!(!states[1].revocation_is_self);
    assert!(states[1].tokenized_control);
    assert_eq!(states[1].output, Some(1));
    assert_eq!(states[2].primary_addresses, ["RNew"]);
    assert!(states[2].revoked);
    assert_eq!(states[2].status, "revoked");
    assert!(
        states[2]
            .changes
            .iter()
            .any(|change| change.starts_with("Signing keys changed"))
    );
    assert!(states[2].changes.iter().any(|change| change == "Revoked"));
    assert!(states[2].json.contains("RNew"));
}

#[test]
fn a_zero_transaction_id_is_not_a_link() {
    let raw = json!({"blockheight": 5, "txid": hash('0'), "status": "active",
        "identity": {"identityaddress": "iSeed", "flags": 0}});
    assert_eq!(
        identity_state(&raw, None, None, "seed@").unwrap().txid,
        None
    );
}

#[test]
fn offers_drop_expired_entries() {
    let result = json!({"identity": [
        {"offer": {"txid": "t1", "blockexpiry": 900,
            "offer": {"identityid": "iSeed"}, "accept": {"iVRSC": 10.0}}},
        {"offer": {"txid": "t2", "blockexpiry": 2000,
            "offer": {"iVRSC": 5.0}, "accept": {"identityid": "iSeed"}}},
        {"offer": {"txid": "t2", "blockexpiry": 2000,
            "offer": {"iVRSC": 5.0}, "accept": {"identityid": "iSeed"}}},
    ]});
    let badges = offer_badges(&result, "iSeed", Some(1000));
    assert!(badges.buy);
    assert!(!badges.sell, "the sale offer expired at 900");
    assert_eq!((badges.open, badges.expired), (1, 1));
    let early = offer_badges(&result, "iSeed", Some(100));
    assert!(early.buy && early.sell);
}

// ------------------------------------------------------------ referral walk

fn registration_tx(txid: &str, name: &str, referral: Option<&str>) -> Value {
    json!({"txid": txid, "vin": [], "vout": [
        {"n": 0, "scriptPubKey": {"identityreservation":
            {"name": name, "nameid": format!("i{name}"), "parent": "iVRSC",
             "referral": referral.unwrap_or("")}}}
    ]})
}

/// Each identity in `chain` was referred by the next one.
fn referral_fake(chain: &[&str]) -> Fake {
    let mut fake = Fake::default();
    for (position, identity) in chain.iter().enumerate() {
        let txid = format!("{:0>64}", position + 1);
        fake = fake
            .on(
                "getidentityhistory",
                json!([identity]),
                json!({"history": [{"output": {"txid": txid}}]}),
            )
            .transaction(registration_tx(
                &txid,
                identity,
                chain.get(position + 1).copied(),
            ));
    }
    fake
}

#[tokio::test]
async fn referral_walk_stops_after_four_hops() {
    let fake = referral_fake(&["iR1", "iR2", "iR3", "iR4", "iR5", "iR6"]);
    let mut report = Evidence::default();
    add_referral_chain(&mut report, &fake, Some("iR1".into())).await;
    let ancestors: Vec<_> = report
        .links
        .iter()
        .map(|link| (link.detail.clone(), link.target.clone()))
        .collect();
    assert_eq!(ancestors.len(), 4);
    assert_eq!(ancestors[0].1, EvidenceTarget::Identity("iR2".into()));
    assert!(ancestors[0].0.starts_with("depth 1 via iR1"));
    assert_eq!(ancestors[3].1, EvidenceTarget::Identity("iR5".into()));
    assert!(
        report
            .notes
            .contains(&"Referral ancestry capped at four hops".to_string())
    );
}

#[tokio::test]
async fn referral_walk_detects_a_cycle() {
    let fake = referral_fake(&["iR1", "iR2"])
        // iR2 names iR1 as its referrer, which closes the loop.
        .transaction(registration_tx(&format!("{:0>64}", 2), "iR2", Some("iR1")));
    let mut report = Evidence::default();
    add_referral_chain(&mut report, &fake, Some("iR1".into())).await;
    assert_eq!(report.links.len(), 2);
    assert!(
        report
            .notes
            .contains(&"Referral chain contains a cycle".to_string())
    );
}

#[tokio::test]
async fn referral_walk_reports_a_missing_history() {
    let fake = referral_fake(&["iR1"]).transaction(registration_tx(
        &format!("{:0>64}", 1),
        "iR1",
        Some("iGone"),
    ));
    let mut report = Evidence::default();
    add_referral_chain(&mut report, &fake, Some("iR1".into())).await;
    assert_eq!(report.links.len(), 1);
    assert!(
        report
            .notes
            .contains(&"Referral ancestry unavailable beyond iGone".to_string())
    );
}

// ------------------------------------------------------------- evidence

#[tokio::test]
async fn evidence_separates_registration_commitment_and_funding() {
    let registration = json!({"txid": hash('a'), "blockhash": "block", "vin": [
            {"txid": hash('f'), "vout": 0}, {"txid": hash('c'), "vout": 1}],
        "vout": [{"n": 0, "scriptPubKey": {
            "identityreservation": {"name": "seed", "nameid": "iSeed", "parent": "iParent",
                "referral": "", "salt": "salt"},
            "addresses": ["RSigner", "iSeed"]}}]});
    let funding = json!({"txid": hash('f'), "vout": [{"n": 0, "value": 100,
        "scriptPubKey": {"type": "pubkeyhash", "addresses": ["RFunder"]}}]});
    let commitment = json!({"txid": hash('c'), "vin": [], "vout": [{"n": 1, "scriptPubKey": {
        "commitmenthash": {"hash": "abc"}, "addresses": ["RControl"]}}]});
    let fake = Fake::default()
        .transaction(registration)
        .transaction(funding)
        .transaction(commitment)
        .fails(
            "getidentitieswithaddress",
            "getidentitieswithaddress requires -idindex=1",
        );
    let version = VersionRecord {
        identity_address: "iSeed".into(),
        block_height: 10,
        raw: json!({"txid": hash('a'), "identity": {"identityaddress": "iSeed",
            "primaryaddresses": ["RSigner"], "revocationauthority": "iSeed",
            "recoveryauthority": "iSeed"}}),
    };
    let report = load_evidence(&fake, &[version], "iSystem").await;
    assert_eq!(report.anchor, Some(("sample".into(), Some("block".into()))));
    assert_eq!(
        report.registration.as_deref(),
        Some("seed@  tx aaaaaa...aaaaaa:0")
    );
    let find = |label: &str| report.links.iter().find(|link| link.label == label);
    assert_eq!(
        find("Registration funding candidate").unwrap().target,
        EvidenceTarget::Address("RFunder".into())
    );
    assert_eq!(
        find("Commitment control destination").unwrap().target,
        EvidenceTarget::Address("RControl".into())
    );
    assert_eq!(find("Name commitment").unwrap().detail, "hash abc");
    assert!(
        find("Registration transaction")
            .unwrap()
            .source
            .starts_with("iSystem ")
    );
    // The commitment input is not funding.
    assert!(
        !report
            .links
            .iter()
            .any(|link| link.label.contains("funding")
                && link.target == EvidenceTarget::Address("RControl".into()))
    );
    assert_eq!(
        link_confidence("Registration funding candidate"),
        Confidence::Candidate
    );
    assert_eq!(
        link_confidence("Registration transaction"),
        Confidence::Observed
    );
}

#[tokio::test]
async fn historical_lookup_says_when_the_source_has_no_identity_index() {
    let fake = Fake::default()
        .fails(
            "getidentitieswithaddress",
            "getidentitieswithaddress requires -idindex=1 when starting the daemon",
        )
        .fails(
            "getidentitieswithrevocation",
            "getidentitieswithrevocation requires -idindex=1",
        );
    let version = VersionRecord {
        identity_address: "iSeed".into(),
        block_height: 10,
        raw: json!({"identity": {"primaryaddresses": ["RKey"], "revocationauthority": "iGuard"}}),
    };
    let mut report = Evidence::default();
    let outcome = expand_historical(&fake, &mut report, &[version], 1, 1000).await;
    assert!(outcome.index_missing);
    assert_eq!((outcome.requests, outcome.failures), (2, 2));
    assert_eq!(
        report.notes.last().unwrap(),
        "Historical reverse window 1-1000; 2 unavailable queries"
    );
}

#[tokio::test]
async fn historical_links_persist_with_their_version_and_block() {
    let fake = Fake::default().on(
        "getidentitieswithaddress",
        json!([{"address": "RKey", "fromheight": 1, "toheight": 1000, "unspent": false}]),
        json!([
            {"identityaddress": "iOther", "primaryaddresses": ["RKey"], "txout": {"txid": "t", "voutnum": 0}},
            {"identityaddress": "iSeed", "primaryaddresses": ["RKey"]},
            {"identityaddress": "iUnrelated", "primaryaddresses": ["RElse"]},
        ]),
    );
    let version = VersionRecord {
        identity_address: "iSeed".into(),
        block_height: 10,
        raw: json!({"identity": {"primaryaddresses": ["RKey"]}}),
    };
    let mut report = Evidence {
        anchor: Some(("local".into(), Some("block-a".into()))),
        ..Default::default()
    };
    let outcome = expand_historical(&fake, &mut report, &[version], 1, 1000).await;
    assert!(!outcome.index_missing);
    assert_eq!(report.links.len(), 1);
    assert_eq!(report.links[0].label, "Historical shared primary address");
    assert_eq!(
        report.links[0].target,
        EvidenceTarget::Identity("iOther".into())
    );

    let record = to_record("iSystem", "iSeed", "version", 10, &report).unwrap();
    assert_eq!(record.block_hash.as_deref(), Some("block-a"));
    assert_eq!(record.edges[0].target_kind, "identity");
    // A later live refresh keeps the historical links.
    let retained = from_record(record);
    let mut fresh = Evidence::default();
    merge_historical(&mut fresh, &retained);
    assert_eq!(fresh.links.len(), 1);
    assert!(fresh.notes[0].starts_with("Historical reverse window 1-1000"));
    // Without an anchor there is nothing to store it against.
    assert!(to_record("iSystem", "iSeed", "version", 10, &Evidence::default()).is_none());
}

#[test]
fn shared_funding_needs_the_same_destination_in_two_identities() {
    let mut left = Evidence::default();
    let mut right = Evidence::default();
    add_link(
        &mut left,
        "Registration funding candidate",
        "input 0".into(),
        EvidenceTarget::Address("RFunder".into()),
        "left:0".into(),
    );
    add_link(
        &mut right,
        "Update funding candidate",
        "input 1".into(),
        EvidenceTarget::Address("RFunder".into()),
        "right:1".into(),
    );
    add_link(
        &mut right,
        "Reservation output destination",
        "output".into(),
        EvidenceTarget::Address("RSigner".into()),
        "right:2".into(),
    );
    connect_shared_evidence("iLeft", &mut left, "iRight", &mut right);
    assert!(left.links.iter().any(|link| link.label == "Shared funding"
        && link.target == EvidenceTarget::Identity("iRight".into())
        && link.source == "left:0 | right:1"));
    assert!(
        !left
            .links
            .iter()
            .any(|link| link.source.contains("right:2"))
    );
}

#[test]
fn reverse_authority_uses_the_matching_field_and_excludes_the_identity() {
    let mut report = Evidence::default();
    add_reverse_results(
        &mut report,
        vec![
            json!({"identityaddress": "iSeed", "name": "Seed", "recoveryauthority": "iAuthority"}),
            json!({"identityaddress": "iOther", "name": "Other", "recoveryauthority": "iAuthority"}),
            json!({"identityaddress": "iWrong", "name": "Wrong", "recoveryauthority": "iDifferent"}),
        ],
        "iAuthority",
        "recoveryauthority",
        "local",
        "iSeed",
    );
    assert_eq!(report.links.len(), 1);
    assert_eq!(
        report.links[0].target,
        EvidenceTarget::Identity("iOther".into())
    );
}

// --------------------------------------------------- registration payments
//
// Builders from the tests in `src/app/registration_payments.rs`. Currency identities stand
// in for synthetic parties.

const ISSUER: &str = "iHog9UCTrn95qpUBFCZ7kKz7qWdMA8MQ6N";
const REFERRER: &str = "i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X";

fn push(bytes: &[u8], script: &mut Vec<u8>) {
    if bytes.len() >= 76 {
        script.push(76);
    }
    script.push(bytes.len() as u8);
    script.extend(bytes);
}

fn condition(eval: u8, recipient: &str, data: &[u8]) -> String {
    let mut params = Vec::new();
    push(&[3, eval, 1, 1], &mut params);
    push(&identity_key(recipient).unwrap(), &mut params);
    if eval != 0 {
        push(data, &mut params);
    }
    let mut script = vec![1, 1, 0xcc];
    push(&params, &mut script);
    script.push(0x75);
    script.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn reserve_output(index: usize, recipient: &str, tokens: f64, native: f64) -> Value {
    json!({"n":index, "value":native, "scriptPubKey": {
        "type":"cryptocondition", "reqSigs":1, "spendableoutput":true,
        "addresses":[recipient], "hex":condition(9, recipient, &[1]),
        "reserveoutput":{"version":1,"currencyvalues":{ISSUER:tokens}}
    }})
}

fn registration_outputs(name: &str, subject: &str, first: Option<&str>) -> (Vec<Value>, Vec<u8>) {
    let mut payload = vec![1, 0, 0, 0, name.len() as u8];
    payload.extend(name.as_bytes());
    payload.extend_from_slice(&identity_key(ISSUER).unwrap()[1..]);
    if let Some(id) = first {
        payload.extend_from_slice(&identity_key(id).unwrap()[1..]);
    } else {
        payload.extend([0; 20]);
    }
    payload.extend([0x75; 32]);
    let digest = Sha256::digest(Sha256::digest(&payload)).to_vec();
    let mut outputs = vec![
        json!({"n":0, "value":0, "scriptPubKey":{
            "hex": condition(14, subject, &[1]), "identityprimary":{
                "version":3, "identityaddress":subject, "name":name,
                "parent":ISSUER, "systemid":VRSC, "flags":0
            }
        }}),
        json!({"n":1, "value":0.0002, "scriptPubKey":{
            "hex":condition(8, ISSUER, &[1]), "reservetransfer":{
                "version":1, "flags":129, "burnchangeprice":true,
                "currencyvalues":{ISSUER:if first.is_some() { 0.6 } else { 1.0 }},
                "feecurrencyid":VRSC, "fees":0.0002,
                "destinationcurrencyid":ISSUER, "destination":{"type":4, "address":ISSUER}
            }
        }}),
    ];
    if let Some(id) = first {
        outputs.push(reserve_output(2, id, 0.2, 0.0));
    }
    outputs.push(json!({"n":outputs.len(), "value":0, "scriptPubKey":{
        "hex":condition(10, subject, &payload), "identityreservation":{
            "version":1, "name":name, "nameid":subject,
            "referral":first.unwrap_or(""), "parent":ISSUER
        }
    }}));
    (outputs, digest)
}

/// A referred registration under an issuer, its parents, and the referrer's own registration.
fn payment_fake(charge: f64) -> (Fake, String, String) {
    let (mut outputs, digest) = registration_outputs("VRSC", VRSC, Some(REFERRER));
    outputs[1]["scriptPubKey"]["reservetransfer"]["currencyvalues"][ISSUER] = json!(charge);
    outputs.push(reserve_output(4, REFERRER, 1.2, 1.0));
    let (old, _) = registration_outputs("vETH", REFERRER, None);
    let (commit, funding, txid, block) = (hash('c'), hash('f'), hash('a'), hash('b'));
    let (original_id, original_block) = (hash('d'), hash('e'));
    let original = json!({"txid": original_id, "blockhash": original_block,
        "height": 3_000_000, "confirmations": 900_000, "vout": old});
    let ancestor_identity = original["vout"][0]["scriptPubKey"]["identityprimary"].clone();
    let commitment = json!({"txid": commit, "height": 4_000_000, "confirmations": 9,
    "vout": [{"n":0,"value":0,"scriptPubKey":{
        "hex":condition(17, ISSUER, &digest), "commitmenthash":{"hash":digest_hex(&digest)}
    }}]});
    let funding_tx = json!({"txid": funding, "height": 4_000_001, "confirmations": 7,
        "vout": [reserve_output(0, REFERRER, 2.0, 1.0202)]});
    let tx = json!({"txid": txid, "blockhash": block, "height": 4_000_002, "confirmations": 6,
        "vin": [{"txid": commit, "vout": 0}, {"txid": funding, "vout": 0}],
        "vout": outputs, "vShieldedSpend": [], "vShieldedOutput": [],
        "vjoinsplit": [], "valueBalance": 0});
    let fake = Fake::default()
        .transaction(tx)
        .transaction(commitment)
        .transaction(funding_tx)
        .transaction(original)
        .on(
            "getcurrency",
            json!([VRSC]),
            json!({"currencyid": VRSC, "systemid": VRSC,
            "options": 264, "proofprotocol": 1, "idregistrationfees": 100,
            "idreferrallevels": 3, "idimportfees": 0.02}),
        )
        .on(
            "getcurrency",
            json!([ISSUER]),
            json!({"version": 1, "currencyid": ISSUER,
            "systemid": VRSC, "launchsystemid": VRSC, "options": 40, "proofprotocol": 1,
            "idregistrationfees": 1.0, "idreferrallevels": 3, "startblock": 10}),
        )
        .on(
            "getidentity",
            json!([REFERRER, 3_999_999]),
            json!({"status": "active",
            "identity": ancestor_identity, "blockheight": 3_000_000,
            "txid": original_id, "vout": 0}),
        )
        .on(
            "getidentityhistory",
            json!([REFERRER, 0, 3_999_999]),
            json!({"history": [
            {"identity": ancestor_identity, "height": 3_000_000, "blockhash": original_block,
             "output": {"txid": original_id, "voutnum": 0}}]}),
        );
    (fake, txid, block)
}

#[tokio::test]
async fn verifies_registration_payments_against_the_issuer_rules() {
    let (fake, txid, block) = payment_fake(0.6);
    let report = verify_payments(&fake, &txid, &block, 4_000_002)
        .await
        .unwrap();
    assert_eq!(report.issuer, ISSUER);
    assert_eq!(report.fee, 2_000_000);
    assert_eq!(report.budget, 20_000);
    assert_eq!(report.annotation(1), Some("Issuer Charge"));
    assert_eq!(report.annotation(2), Some("Direct Referral Payment"));
    assert_eq!(
        report.annotation(4),
        None,
        "change to the referrer is not a referral"
    );

    let shown = verified_payments(&report, "VRSC".into());
    assert_eq!(shown.status, PaymentsStatus::Verified);
    assert_eq!(shown.referrals.len(), 1);
    assert_eq!(shown.referrals[0].recipient, REFERRER);
    assert_eq!(shown.referrals[0].annotation, "Direct Referral Payment");
    assert_eq!(shown.referral_total_sats, 20_000_000);
    assert_eq!(shown.charge.as_ref().unwrap().amount_sats, 60_000_000);
    assert_eq!(shown.total_issuer_sats, Some(80_000_000));
    assert_eq!(shown.total_native_sats, 2_020_000);
    assert!(shown.summary.starts_with("Referrals (1) 0.20000000"));
    assert!(
        shown
            .summary
            .ends_with(&format!("Total 0.80000000 {ISSUER} + 0.02020000 VRSC"))
    );
}

#[tokio::test]
async fn a_charge_that_breaks_the_rules_is_unverified() {
    let (fake, txid, block) = payment_fake(0.5);
    assert!(
        verify_payments(&fake, &txid, &block, 4_000_002)
            .await
            .is_none()
    );
    // The wrong block also fails: the evidence must be the block that was asked about.
    let (fake, txid, _) = payment_fake(0.6);
    assert!(
        verify_payments(&fake, &txid, &hash('9'), 4_000_002)
            .await
            .is_none()
    );
}

#[test]
fn ancestor_referrals_must_follow_the_referrer_chain() {
    let policy = Policy {
        issuer: ISSUER,
        full: 100_000_000,
        levels: 3,
        native_minimum: 2_000_000,
        advanced: true,
    };
    let (outputs, _) = registration_outputs("VRSC", VRSC, Some(REFERRER));
    let tx = json!({"vout": outputs});
    let reg = registration(&tx, policy).unwrap();
    let (old, _) = registration_outputs("vETH", REFERRER, None);
    let (wrong, _) = registration_outputs("vDEX", ISSUER, None);
    assert!(ordered_referrals(
        &reg,
        &registration(&json!({"vout": old}), policy).unwrap(),
        3
    ));
    assert!(!ordered_referrals(
        &reg,
        &registration(&json!({"vout": wrong}), policy).unwrap(),
        3
    ));
    assert!(payment_candidate(&tx));
    assert!(!payment_candidate(&json!({"vout": [{"scriptPubKey": {}}]})));
}

// ------------------------------------------------------------ token trail

const TOKEN: &str = "iToken";
const HOLDER: &str = "RXPzH3i4fZaaPFdkVHWKRDgXDqrFXErEGz";

fn token_out(n: u64, addresses: &[&str], signatures: u64) -> Value {
    json!({"n": n, "scriptPubKey": {"reqSigs": signatures, "addresses": addresses,
        "hex": "51", "reserveoutput": {"currencyvalues": {TOKEN: 0.00000001}}}})
}

fn definition_tx(txid: &str, with_token: bool) -> Value {
    let mut outputs = vec![json!({"n": 0, "scriptPubKey": {"currencydefinition":
        {"currencyid": TOKEN, "options": NFT_TOKEN}}})];
    if with_token {
        outputs.push(token_out(1, &[HOLDER], 1));
    }
    json!({"txid": txid, "height": 100, "vout": outputs})
}

fn definition(txid: &str) -> Value {
    json!({"currencyid": TOKEN, "options": NFT_TOKEN, "definitiontxid": txid,
        "definitiontxout": 0, "startblock": 150})
}

/// `txid` spends `previous` and puts the token in its output 0.
fn spend(fake: Fake, previous: (&str, u64), txid: &str, outputs: Vec<Value>) -> Fake {
    fake.on(
        "gettxout",
        json!([previous.0, previous.1, true]),
        Value::Null,
    )
    .on(
        "getspentinfo",
        json!([{"txid": previous.0, "index": previous.1}]),
        json!({"txid": txid, "index": 0}),
    )
    .transaction(
        json!({"txid": txid, "vin": [{"txid": previous.0, "vout": previous.1}],
            "vout": outputs}),
    )
}

#[tokio::test]
async fn token_trail_reaches_the_current_holder() {
    let (defined, next) = (hash('d'), hash('1'));
    let fake = Fake::default().transaction(definition_tx(&defined, true));
    let fake = spend(fake, (&defined, 1), &next, vec![token_out(0, &[HOLDER], 1)]).on(
        "gettxout",
        json!([next, 0, true]),
        token_out(0, &[HOLDER], 1),
    );
    let frontier = token_origin(&fake, TOKEN, "sample", &definition(&defined))
        .await
        .unwrap();
    assert_eq!(
        (frontier.current.txid.as_str(), frontier.current.index),
        (defined.as_str(), 1)
    );
    let result = token_result(follow_token(&fake, TOKEN, frontier).await);
    assert_eq!(
        result.state,
        TokenControlState::Unspent {
            holder: Some(HOLDER.into())
        }
    );
    assert_eq!(result.hops, 1);
    assert_eq!(result.frontier.unwrap().txid, next);
}

#[tokio::test]
async fn an_unspent_output_with_several_signers_has_no_single_holder() {
    let defined = hash('d');
    let fake = Fake::default()
        .transaction(definition_tx(&defined, true))
        .on(
            "gettxout",
            json!([defined, 1, true]),
            token_out(1, &[HOLDER, "RBQ8KjZ8Bk5DFwjr5pLDLBJGhE9MuXCxsp"], 2),
        );
    let frontier = token_origin(&fake, TOKEN, "sample", &definition(&defined))
        .await
        .unwrap();
    let result = token_result(follow_token(&fake, TOKEN, frontier).await);
    assert_eq!(result.state, TokenControlState::Unspent { holder: None });
}

#[tokio::test]
async fn token_trail_pauses_after_one_pass_and_resumes() {
    let ids: Vec<String> = (0..21).map(|i| format!("{:0>64x}", i + 1)).collect();
    let mut fake = Fake::default().transaction(definition_tx(&ids[0], true));
    fake = spend(
        fake,
        (&ids[0], 1),
        &ids[1],
        vec![token_out(0, &[HOLDER], 1)],
    );
    for pair in ids[1..].windows(2) {
        fake = spend(
            fake,
            (&pair[0], 0),
            &pair[1],
            vec![token_out(0, &[HOLDER], 1)],
        );
    }
    fake = fake.on(
        "gettxout",
        json!([ids[20], 0, true]),
        token_out(0, &[HOLDER], 1),
    );
    let frontier = token_origin(&fake, TOKEN, "sample", &definition(&ids[0]))
        .await
        .unwrap();
    let first = token_result(follow_token(&fake, TOKEN, frontier).await);
    assert_eq!(first.state, TokenControlState::Paused);
    assert_eq!(first.hops, HOPS_PER_PASS);
    let row = first.frontier.unwrap();
    assert_eq!(row.visited.len(), HOPS_PER_PASS as usize + 1);
    let second = token_result(follow_token(&fake, TOKEN, Frontier::from_row(&row)).await);
    assert_eq!(
        second.state,
        TokenControlState::Unspent {
            holder: Some(HOLDER.into())
        }
    );
    assert_eq!(second.hops, 20);
}

#[tokio::test]
async fn token_trail_names_why_it_stopped() {
    let (defined, next) = (hash('d'), hash('1'));
    // Two outputs carry the token: there is no unique continuation.
    let fake = spend(
        Fake::default().transaction(definition_tx(&defined, true)),
        (&defined, 1),
        &next,
        vec![token_out(0, &[HOLDER], 1), token_out(1, &[HOLDER], 1)],
    );
    let frontier = token_origin(&fake, TOKEN, "sample", &definition(&defined))
        .await
        .unwrap();
    assert_eq!(
        token_result(follow_token(&fake, TOKEN, frontier).await).state,
        TokenControlState::Unresolved {
            reason: "No unique on-chain continuation for this control token".into()
        }
    );

    // A spend that returns to an outpoint already followed is a loop.
    let looping = spend(
        Fake::default().transaction(definition_tx(&defined, true)),
        (&defined, 1),
        &next,
        vec![token_out(0, &[HOLDER], 1)],
    )
    .on("gettxout", json!([next, 0, true]), Value::Null)
    .on(
        "getspentinfo",
        json!([{"txid": next, "index": 0}]),
        json!({"txid": defined, "index": 0}),
    )
    .transaction(
        json!({"txid": defined, "vin": [{"txid": next, "vout": 0}], "vout": [
        {"n": 0, "scriptPubKey": {}}, token_out(1, &[HOLDER], 1)]}),
    );
    let frontier = Frontier::new("sample".into(), &defined, 1);
    assert_eq!(
        token_result(follow_token(&looping, TOKEN, frontier).await).state,
        TokenControlState::Unresolved {
            reason: "Control token path contains a loop".into()
        }
    );

    // The source cannot check the output at all.
    let frontier = Frontier::new("sample".into(), &hash('7'), 0);
    let result = token_result(follow_token(&Fake::default(), TOKEN, frontier).await);
    assert_eq!(
        result.state,
        TokenControlState::Unresolved {
            reason: "Source could not check the unspent output".into()
        }
    );
    assert!(
        result.frontier.is_some(),
        "a retry can start from the same place"
    );
}

#[tokio::test]
async fn token_minted_at_launch_is_found_in_the_launch_import() {
    let (defined, launch) = (hash('d'), hash('2'));
    let import = json!({"importcurrencyid": TOKEN, "numoutputs": 2});
    let record = json!({"importtxid": launch, "importvout": 0, "import": import,
        "importnotarization": {"launchcleared": true, "launchconfirmed": true,
            "currencystate": {"currencyid": TOKEN, "flags": 24}}});
    let fake = Fake::default()
        .transaction(definition_tx(&defined, false))
        .on("getblockcount", json!([]), json!(2000))
        .on("getimports", json!([TOKEN, 149, 1148]), json!([record]))
        .transaction(json!({"txid": launch, "vout": [
            {"n": 0, "scriptPubKey": {"crosschainimport": import}},
            {"n": 1, "scriptPubKey": {"addresses": [HOLDER]}},
            token_out(2, &[HOLDER], 1)]}));
    let frontier = token_origin(&fake, TOKEN, "sample", &definition(&defined))
        .await
        .unwrap();
    assert_eq!(
        (frontier.current.txid.as_str(), frontier.current.index),
        (launch.as_str(), 2)
    );

    let empty = Fake::default()
        .transaction(definition_tx(&defined, false))
        .on("getblockcount", json!([]), json!(2000))
        .on("getimports", json!([TOKEN, 149, 1148]), Value::Null);
    assert_eq!(
        token_origin(&empty, TOKEN, "sample", &definition(&defined))
            .await
            .unwrap_err(),
        "No confirmed control token launch found in the launch window"
    );
    let twice = Fake::default()
        .transaction(definition_tx(&defined, false))
        .on("getblockcount", json!([]), json!(2000))
        .on(
            "getimports",
            json!([TOKEN, 149, 1148]),
            json!([record, record]),
        );
    assert_eq!(
        token_origin(&twice, TOKEN, "sample", &definition(&defined))
            .await
            .unwrap_err(),
        "Control token launch is ambiguous"
    );
    let early = Fake::default()
        .transaction(definition_tx(&defined, false))
        .on("getblockcount", json!([]), json!(120));
    assert_eq!(
        token_origin(&early, TOKEN, "sample", &definition(&defined))
            .await
            .unwrap_err(),
        "Waiting for the currency launch"
    );
    let not_a_token = json!({"currencyid": TOKEN, "options": 0});
    assert_eq!(
        token_origin(&Fake::default(), TOKEN, "sample", &not_a_token)
            .await
            .unwrap_err(),
        "Control token definition could not be verified"
    );
}

// ------------------------------------------------------------- addresses

#[test]
fn address_flows_put_the_viewed_address_first() {
    let mut outputs: Vec<Value> = (0..7)
        .map(|n| json!({"n": n, "value": 1.5, "scriptPubKey": {"addresses": [format!("ROther{n}")]}}))
        .collect();
    outputs.push(json!({"n": 7, "value": 0.0, "spentTxId": "spender",
        "scriptPubKey": {"addresses": ["RViewed"], "reserveoutput":
            {"currencyvalues": {"iTBTC": 0.25}}}}));
    outputs.push(json!({"n": 8, "value": 2.0, "scriptPubKey": {"addresses": ["RViewed"]}}));
    let raw = json!({"txid": "tx1", "blocktime": 1700, "height": 5,
        "vin": [{"coinbase": "03"}, {"address": "RViewed", "valueSat": 250_000_000}],
        "vout": outputs,
        "vShieldedOutput": [{}, {}]});
    let unspent = HashSet::from([("tx1".to_string(), 8)]);
    let tile = address_tile(&raw, "RViewed", "VRSC", &unspent, &HashMap::new());
    assert_eq!(tile.time, Some(1700));
    assert_eq!(tile.inputs[0].addresses, ["RViewed"]);
    assert!(tile.inputs[0].viewed);
    assert_eq!(
        tile.inputs[0].amounts,
        [FlowAmount {
            currency: "VRSC".into(),
            sats: 250_000_000
        }]
    );
    assert!(tile.inputs[1].coinbase);
    // Viewed outputs first, then the two private outputs, then the rest.
    assert_eq!(tile.outputs[0].index, 7);
    assert_eq!(tile.outputs[0].status, FlowStatus::Spent);
    assert!(tile.outputs[0].amounts.contains(&FlowAmount {
        currency: "iTBTC".into(),
        sats: 25_000_000
    }));
    assert_eq!(tile.outputs[1].index, 8);
    assert_eq!(tile.outputs[1].status, FlowStatus::Unspent);
    assert!(tile.outputs[2].private && tile.outputs[3].private);
    assert_eq!(tile.outputs[2].index, 9);
    assert_eq!(tile.outputs[4].status, FlowStatus::Unknown);
    assert_eq!(tile.outputs.len(), ADDRESS_FLOW_LIMIT);
    assert_eq!(tile.more_outputs, 11 - ADDRESS_FLOW_LIMIT as u32);
    assert_eq!(tile.more_inputs, 0);
}

#[test]
fn balances_keep_spent_down_currencies_and_name_the_native_coin() {
    let response = json!({"balance": 150_000_000, "received": 900_000_000,
        "currencybalance": {VRSC: 1.5, "iTBTC": 0.5},
        "currencyreceived": {VRSC: 9.0, "iTBTC": 1.0, "iGone": 3.0},
        "currencynames": {"iTBTC": "tBTC.vETH", "iGone": "Gone"}});
    let balances = balance_amounts(&response, Some(VRSC), "VRSC");
    let names: Vec<_> = balances
        .iter()
        .map(|row| (row.name.as_str(), row.amount_sats))
        .collect();
    assert_eq!(
        names,
        [
            ("VRSC", 150_000_000),
            ("Gone", 0),
            ("tBTC.vETH", 50_000_000)
        ]
    );
    assert!(balances[0].native);

    // An older response with only the native fields.
    let old = json!({"balance": 7, "received": 7});
    let balances = balance_amounts(&old, Some(VRSC), "VRSC");
    assert_eq!(balances.len(), 1);
    assert_eq!(
        (balances[0].name.as_str(), balances[0].amount_sats),
        ("VRSC", 7)
    );
}

#[test]
fn primary_address_identities_are_named_and_sorted() {
    let found = primary_address_identities(
        "RKey",
        vec![
            json!({"identityaddress": "iB", "name": "bob", "primaryaddresses": ["RKey"]}),
            json!({"identityaddress": "iA", "name": "Alice", "primaryaddresses": ["RKey", "RX"]}),
            json!({"identityaddress": "iA", "name": "Alice", "primaryaddresses": ["RKey"]}),
            json!({"identityaddress": "iC", "name": "carol", "primaryaddresses": ["ROther"]}),
        ],
    );
    assert_eq!(
        found,
        [
            ("iA".to_string(), "Alice@".to_string()),
            ("iB".into(), "bob@".into())
        ]
    );
    assert!(
        index_message("getidentitieswithaddress requires -idindex=1").contains("identity index")
    );
}

#[test]
fn decimal_amounts_convert_exactly() {
    assert_eq!(decimal_to_sats(&json!(0.1)), Some(10_000_000));
    assert_eq!(decimal_to_sats(&json!("1e-8")), Some(1));
    assert_eq!(decimal_to_sats(&json!("-2.5")), Some(-250_000_000));
    assert_eq!(decimal_to_sats(&json!("0.000000001")), None);
}

#[test]
fn inputs_take_addresses_and_amounts_from_the_spent_output() {
    let previous = hash('e');
    let prevouts = HashMap::from([(
        previous.clone(),
        json!({"txid": previous, "vout": [
            {"n": 0, "value": 9.0, "scriptPubKey": {"addresses": ["RElse"]}},
            {"n": 1, "value": 0.5, "scriptPubKey": {"addresses": ["RViewed"],
                "reserveoutput": {"currencyvalues": {"iTBTC": 1.25}}}},
        ]}),
    )]);
    let raw = json!({"txid": "tx2", "blockhash": "blockA",
        "vin": [{"txid": previous, "vout": 1}, {"txid": hash('7'), "vout": 0, "address": "RSeen", "valueSat": 3}],
        "vout": []});
    let tile = address_tile(&raw, "RViewed", "VRSC", &HashSet::new(), &prevouts);
    assert_eq!(tile.block_hash.as_deref(), Some("blockA"));
    assert!(tile.inputs[0].viewed);
    assert!(tile.inputs[0].amounts.contains(&FlowAmount {
        currency: "iTBTC".into(),
        sats: 125_000_000
    }));
    assert!(tile.inputs[0].amounts.contains(&FlowAmount {
        currency: "VRSC".into(),
        sats: 50_000_000
    }));
    // An input whose earlier transaction did not load keeps its own fields.
    assert_eq!(tile.inputs[1].addresses, ["RSeen"]);
    assert_eq!(
        tile.inputs[1].amounts,
        [FlowAmount {
            currency: "VRSC".into(),
            sats: 3
        }]
    );
}

#[test]
fn prevout_requests_are_bounded_and_skip_coinbase() {
    let transactions = [
        json!({"vin": [{"coinbase": "03"}, {"txid": "a"}, {"txid": "b"}]}),
        json!({"vin": [{"txid": "a"}, {"txid": "c"}, {"txid": "d"}]}),
    ];
    assert_eq!(prevout_txids(transactions.iter(), 10), ["a", "b", "c", "d"]);
    assert_eq!(prevout_txids(transactions.iter(), 2), ["a", "b"]);
    assert_eq!(block_active(&json!({"confirmations": -1})), Some(false));
    assert_eq!(block_active(&json!({"confirmations": 0})), Some(true));
    assert_eq!(block_active(&json!({})), None);
}

// ------------------------------------------------------ stored observations

/// Records one JSON-RPC answer in the explorer's cache, as Carbide's client does.
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
                Network::Mainnet,
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

#[tokio::test]
async fn a_stored_identity_opens_when_no_source_answers() {
    let explorer = super::super::tests::explorer(None);
    let api = crate::rpc::source_id("public-api", "https://api.example.invalid");
    let history = history_sample();
    let current = json!({"identity": history["history"][0]["identity"], "status": "revoked",
        "blockheight": 300, "txid": hash('3'), "vout": 0, "friendlyname": "seed.VRSC@",
        "fullyqualifiedname": "seed.VRSC@"});
    record(
        &explorer,
        &api,
        "getidentity",
        json!(["seedstored@"]),
        current,
    )
    .await;
    record(
        &explorer,
        &api,
        "getidentityhistory",
        json!(["iSeed"]),
        history,
    )
    .await;
    // The test explorer's node has no credentials, so no source answers.
    let timeline = explorer
        .identity_timeline("seedstored@".into())
        .await
        .unwrap();
    assert!(timeline.provenance.retained);
    assert_eq!(timeline.provenance.source_id, api);
    assert!(timeline.provenance.failure.is_some());
    assert_eq!(timeline.versions.len(), 3);
    assert_eq!(timeline.versions[2].status, "revoked");
    assert_eq!(timeline.name, "seed@");
    // Nothing stored for another identity: the error stays.
    assert!(
        explorer
            .identity_timeline("nothing-stored@".into())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_stored_address_opens_when_no_source_answers() {
    let explorer = super::super::tests::explorer(None);
    let api = crate::rpc::source_id("public-api", "https://api.example.invalid");
    let address = "RStoredAddressForTheTest";
    record(
        &explorer,
        &api,
        "getaddressbalance",
        json!([{"addresses": [address], "friendlynames": true}]),
        json!({"balance": 150_000_000, "received": 150_000_000}),
    )
    .await;
    record(
        &explorer,
        &api,
        "getaddresstxids",
        json!([{"addresses": [address]}]),
        json!([hash('1'), hash('2')]),
    )
    .await;
    let overview = explorer.address_overview(address.into()).await.unwrap();
    assert!(overview.provenance.retained);
    assert_eq!(overview.transaction_count, 2);
    assert_eq!(overview.balances[0].amount_sats, 150_000_000);
    assert!(
        explorer
            .address_overview("RNothingStored".into())
            .await
            .is_err()
    );
}

// ------------------------------------------------------------- live tests

#[tokio::test]
#[ignore]
async fn live_identity_timeline() {
    let explorer = super::super::tests::live_explorer();
    let timeline = explorer.identity_timeline("vETH@".into()).await.unwrap();
    println!(
        "{} versions, complete {}, stale {}",
        timeline.versions.len(),
        timeline.history_complete,
        timeline.stale
    );
    for version in &timeline.versions {
        println!(
            "  {} {:?} {} {:?} {:?}",
            version.height, version.time, version.status, version.timelock, version.changes
        );
    }
    assert!(timeline.versions.len() >= 7);
    let latest = timeline.versions.last().unwrap();
    assert!(latest.defines_currency);
    assert_eq!(latest.on_active_chain, Some(true));
    assert!(!timeline.provenance.retained);
    let hash = latest.block_hash.clone().unwrap();
    let checked = explorer.blocks_active(vec![hash, "00".repeat(32)]).await;
    println!("block checks: {checked:?}");
    assert_eq!(checked[0].active, Some(true));
    assert_eq!(checked[1].active, None);
}

#[tokio::test]
#[ignore]
async fn live_identity_offers() {
    let explorer = super::super::tests::live_explorer();
    let offers = explorer
        .identity_offers("i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X".into())
        .await;
    println!("offers: {offers:?}");
    assert!(offers.is_ok());
}

#[tokio::test]
#[ignore]
async fn live_identity_evidence() {
    let explorer = super::super::tests::live_explorer();
    let report = explorer.identity_evidence("vETH@".into()).await.unwrap();
    println!(
        "registration: {:?}, saved {}",
        report.registration, report.saved
    );
    for link in &report.links {
        println!(
            "  {} | {} | {} | {:?}",
            link.label, link.detail, link.target, link.confidence
        );
    }
    for note in &report.notes {
        println!("  note: {note}");
    }
    assert!(report.source_id.is_some());
}

#[tokio::test]
#[ignore]
async fn live_identity_historical_evidence() {
    let explorer = super::super::tests::live_explorer();
    let result = explorer
        .identity_historical_evidence("vETH@".into(), 3_000_000)
        .await
        .unwrap();
    println!(
        "window {}-{}: {} found, {} of {} unavailable, index missing {}",
        result.from_height,
        result.to_height,
        result.found,
        result.unavailable,
        result.requests,
        result.index_missing
    );
    for note in &result.report.notes {
        println!("  note: {note}");
    }
}

#[tokio::test]
#[ignore]
async fn live_registration_payments() {
    let explorer = super::super::tests::live_explorer();
    let timeline = explorer.identity_timeline("vETH@".into()).await.unwrap();
    let txid = timeline.versions[0].txid.clone().unwrap();
    let payments = explorer.registration_payments(txid).await.unwrap();
    println!("payments: {payments:?}");
}

#[tokio::test]
#[ignore]
async fn live_token_control() {
    let explorer = super::super::tests::live_explorer();
    // Set CARBIDE_TEST_TOKEN_IDENTITY to an identity with tokenized control.
    let id = std::env::var("CARBIDE_TEST_TOKEN_IDENTITY")
        .unwrap_or_else(|_| "i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X".into());
    let mut result = explorer.token_control(id.clone(), None).await.unwrap();
    println!("first pass: {result:?}");
    while result.state == TokenControlState::Paused {
        result = explorer
            .token_control(id.clone(), result.frontier.clone())
            .await
            .unwrap();
        println!("next pass: {result:?}");
    }
}

#[tokio::test]
#[ignore]
async fn live_address_pages() {
    let explorer = super::super::tests::live_explorer();
    let address = "RXPzH3i4fZaaPFdkVHWKRDgXDqrFXErEGz".to_string();
    let overview = explorer.address_overview(address.clone()).await.unwrap();
    println!(
        "{} transactions, first {:?}, latest {:?}, balances {:?}",
        overview.transaction_count, overview.first_time, overview.latest_time, overview.balances
    );
    let page = explorer
        .address_activity(address.clone(), 0, 5)
        .await
        .unwrap();
    for tile in &page.tiles {
        println!(
            "  {} {:?} in {} out {} (+{} more)",
            tile.txid,
            tile.time,
            tile.inputs.len(),
            tile.outputs.len(),
            tile.more_outputs
        );
    }
    assert!(page.tiles.len() + page.missing.len() <= 5);
    assert!(!overview.provenance.retained);
    // vETH@'s own address has ordinary inputs, filled from the outputs they spend.
    let identity = explorer
        .address_activity("i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X".into(), 0, 3)
        .await
        .unwrap();
    for tile in page.tiles.iter().chain(&identity.tiles) {
        for input in tile.inputs.iter().filter(|input| !input.coinbase) {
            println!(
                "    input #{} {:?} {:?}",
                input.index, input.addresses, input.amounts
            );
        }
    }
    match explorer.identities_with_primary_address(address).await {
        Ok(found) => println!("primary address for {} identities", found.identities.len()),
        Err(error) => println!("primary address request: {error}"),
    }
}
