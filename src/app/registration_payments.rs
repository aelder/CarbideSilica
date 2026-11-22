use super::*;
use crate::defi::{address_payload, amount_sats, issuer_instruction};
use crate::witnesses::condition_data;
use sha2::{Digest, Sha256};

const VRSC: &str = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct Key {
    txid: String,
    block: BlockHash,
    source: String,
}

#[derive(Debug, Clone)]
pub(super) enum Status {
    Loading,
    Unverified,
    Verified(Report),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Payment {
    output: usize,
    recipient: String,
    amount: i128,
}

#[derive(Debug, Clone)]
pub(super) struct Report {
    issuer: String,
    referrals: Vec<Payment>,
    charge: Option<Payment>,
    budget: i128,
    fee: i128,
}

struct Registration<'a> {
    identity: &'a Value,
    reservation: usize,
    commitment: String,
    referrer: Option<&'a str>,
    referrals: Vec<Payment>,
    charge: Option<Payment>,
    budget: i128,
}

#[derive(Clone, Copy)]
struct Policy<'a> {
    issuer: &'a str,
    full: i128,
    levels: usize,
    native_minimum: i128,
    advanced: bool,
}

impl Policy<'_> {
    fn per_referral(self) -> i128 {
        self.full / (self.levels as i128 + 2)
    }

    fn price(self, referred: bool) -> i128 {
        if referred {
            self.full * (self.levels as i128 + 1) / (self.levels as i128 + 2)
        } else {
            self.full
        }
    }

    fn supported_height(self, height: u64) -> bool {
        if self.advanced {
            (2_549_420..4_141_448).contains(&height)
        } else {
            (1_053_660..1_796_400).contains(&height)
        }
    }
}

fn digest_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .rev()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn identity_key(address: &str) -> Option<Vec<u8>> {
    let (version, payload) = address_payload(address)?;
    if version != 102 {
        return None;
    }
    let mut key = vec![4];
    key.extend(payload);
    Some(key)
}

fn out(tx: &Value, index: usize) -> Option<&Value> {
    transaction_output(tx, index).filter(|out| out["n"].as_u64() == Some(index as u64))
}

fn confirmed(tx: &Value, txid: &str) -> Option<u64> {
    if tx["txid"].as_str() != Some(txid)
        || tx["confirmations"].as_u64().filter(|&n| n > 0).is_none()
    {
        return None;
    }
    tx["height"].as_u64()
}

fn native(output: &Value) -> Option<i128> {
    let amount = i128::from(i64::try_from(amount_sats(&output["value"])?).ok()?);
    if let Some(explicit) = native_value_zat(output)
        && explicit != amount
    {
        return None;
    }
    Some(amount)
}

fn visible_value(tx: &Value) -> bool {
    ["vShieldedSpend", "vShieldedOutput", "vjoinsplit"]
        .iter()
        .all(|field| tx[*field].as_array().is_some_and(Vec::is_empty))
        && amount_sats(&tx["valueBalance"]) == Some(0)
}

fn sole_identity(script: &Value, eval: u8) -> Option<&str> {
    let [address] = script["addresses"].as_array()?.as_slice() else {
        return None;
    };
    let address = address.as_str()?;
    let (keys, _) = condition_data(script, eval)?;
    (script["type"] == "cryptocondition"
        && script["reqSigs"] == 1
        && script["spendableoutput"] == true
        && keys.as_slice() == [identity_key(address)?])
    .then_some(address)
}

fn token_amount(script: &Value, issuer: &str) -> Option<i128> {
    let values = script["reserveoutput"]["currencyvalues"].as_object()?;
    if values.len() != 1 {
        return None;
    }
    amount_sats(values.get(issuer)?)
}

fn reservation_payload<'a>(
    script: &'a Value,
    reservation: &'a Value,
    policy: Policy<'_>,
    id: &str,
) -> Option<(String, Option<&'a str>)> {
    let (keys, payload) = condition_data(script, if policy.advanced { 10 } else { 18 })?;
    if keys.as_slice() != [identity_key(id)?] {
        return None;
    }
    let body = if policy.advanced {
        if reservation["version"] != 1 || reservation["parent"] != policy.issuer {
            return None;
        }
        payload.strip_prefix(&[1, 0, 0, 0])?
    } else {
        payload.as_slice()
    };
    let (&len, body) = body.split_first()?;
    if len == 0 || len >= 253 {
        return None;
    }
    let (name, body) = body.split_at_checked(usize::from(len))?;
    if name != reservation["name"].as_str()?.as_bytes() {
        return None;
    }
    let body = if policy.advanced {
        body.strip_prefix(identity_key(policy.issuer)?.get(1..)?)?
    } else {
        body
    };
    let (referral, salt) = body.split_at_checked(20)?;
    if salt.len() != 32 {
        return None;
    }
    let decoded_referrer = reservation["referral"].as_str()?;
    let referrer = if referral == [0; 20] {
        // Older daemon decoders rendered a null identity as a Base58 address.
        if !decoded_referrer.is_empty() && identity_key(decoded_referrer)?.get(1..)? != referral {
            return None;
        }
        None
    } else {
        if identity_key(decoded_referrer)?.get(1..)? != referral {
            return None;
        }
        Some(decoded_referrer)
    };
    let commitment = digest_hex(&Sha256::digest(Sha256::digest(payload)));
    Some((commitment, referrer))
}

fn registration<'a>(tx: &'a Value, policy: Policy<'_>) -> Option<Registration<'a>> {
    let outputs = tx["vout"].as_array()?;
    let primary = out(tx, 0)?;
    let script = &primary["scriptPubKey"];
    let identity = script.get("identityprimary")?;
    let id = identity["identityaddress"].as_str()?;
    let (keys, _) = condition_data(script, 14)?;
    if keys.as_slice() != [identity_key(id)?]
        || identity["version"] != if policy.advanced { 3 } else { 1 }
        || identity["parent"] != policy.issuer
        || identity["systemid"] != VRSC
        || identity["flags"].as_u64()? & 0x8000 != 0
        || native(primary) != Some(0)
    {
        return None;
    }
    let mut reservations = outputs
        .iter()
        .filter(|out| out["scriptPubKey"].get("identityreservation").is_some());
    let reserved = reservations.next()?;
    let index = usize::try_from(reserved["n"].as_u64()?).ok()?;
    let start = if policy.advanced { 2 } else { 1 };
    let script = &reserved["scriptPubKey"];
    let reservation = script.get("identityreservation")?;
    if reservations.next().is_some()
        || outputs
            .iter()
            .skip(1)
            .any(|out| out["scriptPubKey"].get("identityprimary").is_some())
        || out(tx, index) != Some(reserved)
        || index < start
        || index - start > policy.levels
        || reservation["name"] != identity["name"]
        || reservation["nameid"] != id
        || reservation["parent"] != policy.issuer
        || native(reserved) != Some(0)
    {
        return None;
    }
    let (commitment, referrer) = reservation_payload(script, reservation, policy, id)?;
    let mut referrals = Vec::new();
    for index in start..index {
        let output = out(tx, index)?;
        let script = &output["scriptPubKey"];
        let (recipient, amount) = if policy.advanced {
            if native(output)? != 0 {
                return None;
            }
            (
                sole_identity(script, 9)?,
                token_amount(script, policy.issuer)?,
            )
        } else {
            if script["evalnone"] != "valid" {
                return None;
            }
            (sole_identity(script, 0)?, native(output)?)
        };
        if amount != policy.per_referral() {
            return None;
        }
        referrals.push(Payment {
            output: index,
            recipient: recipient.into(),
            amount,
        });
    }
    if referrals.first().map(|payment| payment.recipient.as_str()) != referrer {
        return None;
    }
    let (charge, budget) = if policy.advanced {
        let output = out(tx, 1)?;
        let (amount, budget) = issuer_instruction(
            &output["scriptPubKey"]["reservetransfer"],
            policy.issuer,
            VRSC,
        )?;
        if amount
            != policy.price(referrer.is_some()) - policy.per_referral() * referrals.len() as i128
            || native(output)? != budget
            || condition_data(&output["scriptPubKey"], 8).is_none()
        {
            return None;
        }
        (
            Some(Payment {
                output: 1,
                recipient: policy.issuer.into(),
                amount,
            }),
            budget,
        )
    } else {
        (None, 0)
    };
    Some(Registration {
        identity,
        reservation: index,
        commitment,
        referrer,
        referrals,
        charge,
        budget,
    })
}

fn ordered_referrals(
    registration: &Registration<'_>,
    ancestor: &Registration<'_>,
    levels: usize,
) -> bool {
    let Some(referrer) = registration.referrer else {
        return false;
    };
    ancestor.identity["identityaddress"] == referrer
        && registration
            .referrals
            .iter()
            .map(|payment| payment.recipient.as_str())
            .eq(std::iter::once(referrer)
                .chain(
                    ancestor
                        .referrals
                        .iter()
                        .map(|payment| payment.recipient.as_str()),
                )
                .take(levels))
}

fn fee(
    tx: &Value,
    reg: &Registration<'_>,
    policy: Policy<'_>,
    parents: &[Value],
) -> Option<(u64, i128)> {
    if !visible_value(tx) {
        return None;
    }
    let inputs = tx["vin"].as_array()?;
    let height = tx["height"].as_u64()?;
    if inputs.len() != parents.len() {
        return None;
    }
    let mut native_in = 0_i128;
    let mut tokens_in = 0_i128;
    let mut commitment_height = None;
    let mut seen = HashSet::new();
    for (input, parent) in inputs.iter().zip(parents) {
        let id = input["txid"].as_str()?;
        let index = usize::try_from(input["vout"].as_u64()?).ok()?;
        let prior = confirmed(parent, id)?;
        if prior >= height || !seen.insert((id, index)) {
            return None;
        }
        let output = out(parent, index)?;
        let value = native(output)?;
        if native_value_zat(input).is_some_and(|explicit| explicit != value) {
            return None;
        }
        native_in = native_in.checked_add(value)?;
        let script = &output["scriptPubKey"];
        if let Some(commitment) = script.get("commitmenthash") {
            let (_, payload) = condition_data(script, 17)?;
            if payload.len() != 32
                || value != 0
                || commitment["hash"] != reg.commitment
                || digest_hex(&payload) != reg.commitment
                || commitment_height.replace(prior).is_some()
            {
                return None;
            }
        } else if policy.advanced {
            sole_identity(script, 9)?;
            tokens_in = tokens_in.checked_add(token_amount(script, policy.issuer)?)?;
        } else if !matches!(
            script["type"].as_str(),
            Some("pubkeyhash" | "pubkey" | "scripthash" | "multisig")
        ) {
            return None;
        }
    }

    // The reservation ends the payment layout. A later change output may
    // use the same destination as a referral.
    let outputs = tx["vout"].as_array()?;
    let mut native_out = 0_i128;
    let mut change_tokens = 0_i128;
    for index in 0..outputs.len() {
        let output = out(tx, index)?;
        native_out = native_out.checked_add(native(output)?)?;
        if index > reg.reservation {
            let script = &output["scriptPubKey"];
            if policy.advanced {
                sole_identity(script, 9)?;
                change_tokens = change_tokens.checked_add(token_amount(script, policy.issuer)?)?;
            } else if !matches!(
                script["type"].as_str(),
                Some("pubkeyhash" | "pubkey" | "scripthash" | "multisig")
            ) {
                return None;
            }
        }
    }
    let referral_total = policy.per_referral() * reg.referrals.len() as i128;
    let miner_fee = native_in.checked_sub(native_out)?;
    let minimum = if let Some(charge) = &reg.charge {
        if tokens_in.checked_sub(change_tokens)? != charge.amount.checked_add(referral_total)? {
            return None;
        }
        policy.native_minimum
    } else {
        policy.price(reg.referrer.is_some()) - referral_total
    };
    (miner_fee >= minimum).then_some((commitment_height?, miner_fee))
}

async fn fetch(client: &RpcClient, source: &str, method: &str, params: &[Value]) -> Option<Value> {
    client.call_on(source, method, params).await.ok()
}

async fn verify(client: RpcClient, key: Key, height: u64) -> Option<Report> {
    let source = &key.source;
    let tx = fetch(
        &client,
        source,
        "getrawtransaction",
        &[key.txid.as_str().into(), 1.into()],
    )
    .await?;
    if confirmed(&tx, &key.txid) != Some(height) || tx["blockhash"] != key.block {
        return None;
    }
    let identity = &out(&tx, 0)?["scriptPubKey"]["identityprimary"];
    let issuer = identity["parent"].as_str()?;
    let advanced = issuer != VRSC;
    let root = fetch(&client, source, "getcurrency", &[Value::from(VRSC)]).await?;
    if root["currencyid"] != VRSC
        || root["systemid"] != VRSC
        || root["options"] != 264
        || root["proofprotocol"] != 1
        || amount_sats(&root["idregistrationfees"])? != 10_000_000_000
        || root["idreferrallevels"] != 3
    {
        return None;
    }
    let definition;
    let policy = if advanced {
        definition = fetch(&client, source, "getcurrency", &[Value::from(issuer)]).await?;
        let full = amount_sats(&definition["idregistrationfees"])?;
        let levels = usize::try_from(definition["idreferrallevels"].as_u64()?).ok()?;
        if definition["version"] != 1
            || definition["currencyid"] != issuer
            || definition["systemid"] != VRSC
            || definition["launchsystemid"] != VRSC
            || definition["options"] != 40
            || definition["proofprotocol"] != 1
            || !(1..=16).contains(&levels)
            || !(5..=i128::from(i64::MAX)).contains(&full)
        {
            return None;
        }
        Policy {
            issuer,
            advanced,
            full,
            levels,
            native_minimum: amount_sats(&root["idimportfees"])?,
        }
    } else {
        definition = root;
        Policy {
            issuer,
            advanced,
            full: 10_000_000_000,
            levels: 3,
            native_minimum: 0,
        }
    };
    if !policy.supported_height(height)
        || (advanced && definition["startblock"].as_u64()? >= height)
    {
        return None;
    }
    let reg = registration(&tx, policy)?;
    let inputs = tx["vin"].as_array()?;
    if !(1..=12).contains(&inputs.len()) {
        return None;
    }
    let mut parents = Vec::with_capacity(inputs.len());
    for input in inputs {
        let id = input["txid"].as_str().filter(|id| is_hash_like(id))?;
        parents.push(
            fetch(
                &client,
                source,
                "getrawtransaction",
                &[Value::from(id), 1.into()],
            )
            .await?,
        );
    }
    let (commit_height, fee) = fee(&tx, &reg, policy, &parents)?;
    if !policy.supported_height(commit_height) {
        return None;
    }
    if let Some(referrer) = reg.referrer {
        let lookup = commit_height.checked_sub(1)?;
        let historical = fetch(
            &client,
            source,
            "getidentity",
            &[referrer.into(), lookup.into()],
        )
        .await?;
        let history = fetch(
            &client,
            source,
            "getidentityhistory",
            &[referrer.into(), 0.into(), lookup.into()],
        )
        .await?;
        let entries = history["history"].as_array()?;
        let first = entries
            .iter()
            .min_by_key(|entry| entry["height"].as_u64().unwrap_or(u64::MAX))?;
        let id = first["output"]["txid"]
            .as_str()
            .filter(|id| is_hash_like(id))?;
        let original = fetch(&client, source, "getrawtransaction", &[id.into(), 1.into()]).await?;
        let original_height = confirmed(&original, id)?;
        let ancestor = registration(&original, policy)?;
        if first["identity"] != *ancestor.identity
            || first["output"]["voutnum"] != 0
            || first["height"].as_u64() != Some(original_height)
            || first["blockhash"] != original["blockhash"]
            || historical["status"] != "active"
            || historical["identity"]["identityaddress"] != referrer
            || historical["identity"]["flags"].as_u64()? & 0x8000 != 0
            || historical["identity"]["parent"] != issuer
            || historical["identity"]["systemid"] != VRSC
            || !entries.iter().any(|entry| {
                entry["identity"] == historical["identity"]
                    && entry["height"] == historical["blockheight"]
                    && entry["output"]["txid"] == historical["txid"]
                    && entry["output"]["voutnum"] == historical["vout"]
            })
            || !historical["blockheight"]
                .as_u64()
                .is_some_and(|h| (original_height..=lookup).contains(&h))
            || !policy.supported_height(original_height)
            || (advanced && definition["startblock"].as_u64()? >= original_height)
            || !ordered_referrals(&reg, &ancestor, policy.levels)
        {
            return None;
        }
    }
    Some(Report {
        issuer: issuer.into(),
        referrals: reg.referrals,
        charge: reg.charge,
        budget: reg.budget,
        fee,
    })
}

pub(super) fn candidate(raw: &Value) -> bool {
    raw["vout"].as_array().is_some_and(|outputs| {
        outputs.iter().any(|o| {
            o["scriptPubKey"].get("identityreservation").is_some()
                || o["scriptPubKey"]
                    .get("identityadvancedreservation")
                    .is_some()
        })
    })
}

pub(super) fn request(
    state: &mut AppState,
    config: &Config,
    sender: &UnboundedSender<AppEvent>,
    txid: &str,
) {
    if config.chain.testnet
        || config
            .chain
            .system_id
            .get()
            .is_none_or(|id| id.as_ref() != VRSC)
    {
        return;
    }
    let Some(tx) = state.store.transaction(txid) else {
        return;
    };
    let Some((block, source, height)) = tx
        .block_hash
        .clone()
        .zip(tx.source_id.clone())
        .and_then(|(block, source)| tx.block_height.map(|height| (block, source, height)))
    else {
        return;
    };
    if !((1_053_660..1_796_400).contains(&height) || (2_549_420..4_141_448).contains(&height))
        || !candidate(&tx.raw_json)
    {
        return;
    }
    let key = Key {
        txid: txid.into(),
        block,
        source,
    };
    if state.store.registration_payments.contains_key(&key) {
        return;
    }
    let mut config = config.noninteractive();
    config.daemon.background = true;
    let Ok(client) = RpcClient::for_config(&config) else {
        return;
    };
    let request = Arc::new(());
    state
        .store
        .registration_payments
        .insert(key.clone(), (request.clone(), Status::Loading));
    let sender = sender.clone();
    tokio::spawn(async move {
        let report =
            tokio::time::timeout(Duration::from_secs(12), verify(client, key.clone(), height))
                .await
                .ok()
                .flatten();
        let _ = sender.send(AppEvent::RegistrationPayments {
            key,
            request,
            report,
        });
    });
}

fn status<'a>(raw: &Value, store: &'a ObjectStore) -> Option<&'a Status> {
    let txid = raw["txid"].as_str()?;
    let tx = store.transaction(txid)?;
    let key = Key {
        txid: txid.into(),
        block: tx.block_hash.clone()?,
        source: tx.source_id.clone()?,
    };
    store
        .registration_payments
        .get(&key)
        .map(|(_, status)| status)
}

pub(super) fn report<'a>(raw: &Value, store: &'a ObjectStore) -> Option<&'a Report> {
    let Some(Status::Verified(report)) = status(raw, store) else {
        return None;
    };
    Some(report)
}

impl Report {
    pub(super) fn annotation(&self, index: usize) -> Option<&'static str> {
        if self
            .charge
            .as_ref()
            .is_some_and(|charge| charge.output == index)
        {
            return Some("Issuer Charge");
        }
        self.referrals
            .iter()
            .position(|referral| referral.output == index)
            .map(|position| {
                if position == 0 {
                    "Direct Referral Payment"
                } else {
                    "Ancestor Referral Payment"
                }
            })
    }
}

pub(super) fn lines(raw: &Value, store: &ObjectStore, theme: &Theme) -> Vec<Line<'static>> {
    if !candidate(raw) {
        return Vec::new();
    }
    let heading = |text| {
        Line::from(vec![
            Span::styled("Registration payments  ", Style::default().fg(theme.accent)),
            Span::raw(text),
        ])
    };
    let report = match status(raw, store) {
        Some(Status::Verified(report)) => report,
        Some(Status::Loading) => return vec![heading("Checking historical evidence".into())],
        _ => return vec![heading("Unverified".into())],
    };
    let amount =
        |currency: &str, sats| currency_amount_label(currency_display_value(store, currency), sats);
    let referrals: i128 = report.referrals.iter().map(|payment| payment.amount).sum();
    let count = report.referrals.len();
    let mut details = vec![format!(
        "Referrals ({count}) {}",
        amount(&report.issuer, referrals)
    )];
    let mut total = amount(
        VRSC,
        report.fee
            + report.budget
            + if report.charge.is_none() {
                referrals
            } else {
                0
            },
    );
    if let Some(charge) = &report.charge {
        details.push(format!(
            "Issuer charge {}",
            amount(&report.issuer, charge.amount)
        ));
        details.push(format!(
            "Native instruction budget {}",
            amount(VRSC, report.budget)
        ));
        total = format!(
            "{} + {total}",
            amount(&report.issuer, referrals + charge.amount)
        );
    }
    details.push(format!("Transaction fee {}", amount(VRSC, report.fee)));
    details.push(format!("Total {total}"));
    vec![heading(details.join("  ·  "))]
}

#[cfg(test)]
mod tests {
    use super::*;

    // Currency identities stand in for synthetic parties to exercise the real
    // Base58 and script bindings without capturing anybody's registration.
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

    fn registration_outputs(
        name: &str,
        subject: &str,
        first: Option<&str>,
    ) -> (Vec<Value>, Vec<u8>) {
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

    #[test]
    fn token_registration_binds_historical_roles_and_excludes_same_address_change() {
        let policy = Policy {
            issuer: ISSUER,
            full: 100_000_000,
            levels: 3,
            native_minimum: 2_000_000,
            advanced: true,
        };
        let (mut outputs, digest) = registration_outputs("VRSC", VRSC, Some(REFERRER));
        outputs.push(reserve_output(4, REFERRER, 1.2, 1.0));
        let (old, _) = registration_outputs("vETH", REFERRER, None);
        let original = json!({"vout":old});
        let (wrong, _) = registration_outputs("vDEX", ISSUER, None);
        let other = json!({"vout":wrong});
        let commitment = json!({"txid":"commit", "height":4_000_000, "confirmations":9,
            "vout":[{"n":0,"value":0,"scriptPubKey":{
                "hex":condition(17, ISSUER, &digest), "commitmenthash":{"hash":digest_hex(&digest)}
            }}]
        });
        let funding = json!({"txid":"funding", "height":4_000_001, "confirmations":7,
            "vout":[reserve_output(0, REFERRER, 2.0, 1.0202)]
        });
        let id = "a".repeat(64);
        let block = "b".repeat(64);
        let tx = json!({"txid":id,"blockhash":block,"height":4_000_002,"confirmations":6,
            "vin":[{"txid":"commit","vout":0},{"txid":"funding","vout":0}],
            "vout":outputs, "vShieldedSpend":[], "vShieldedOutput":[],
            "vjoinsplit":[], "valueBalance":0
        });
        let reg = registration(&tx, policy).unwrap();
        assert!(ordered_referrals(
            &reg,
            &registration(&original, policy).unwrap(),
            3
        ));
        assert!(!ordered_referrals(
            &reg,
            &registration(&other, policy).unwrap(),
            3
        ));
        let parents = [commitment, funding];
        assert_eq!(
            fee(&tx, &reg, policy, &parents),
            Some((4_000_000, 2_000_000))
        );
        assert_eq!(fee(&tx, &reg, policy, &parents[..1]), None);

        let mut store = ObjectStore::default();
        let source = "synthetic";
        let mut record = transaction_record(&id, tx.clone());
        record.source_id = Some(source.into());
        store.insert_transaction(record);
        store.registration_payments.insert(
            Key {
                txid: id,
                block,
                source: source.into(),
            },
            (
                Arc::new(()),
                Status::Verified(Report {
                    issuer: ISSUER.into(),
                    referrals: reg.referrals,
                    charge: reg.charge,
                    budget: reg.budget,
                    fee: 2_000_000,
                }),
            ),
        );
        let report = report(&tx, &store).unwrap();
        assert_eq!(report.annotation(1), Some("Issuer Charge"));
        assert_eq!(report.annotation(2), Some("Direct Referral Payment"));
        assert_eq!(report.annotation(4), None);
    }
}
