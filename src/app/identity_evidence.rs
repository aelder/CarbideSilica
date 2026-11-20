use super::*;
use crate::cache::identity::{EvidenceEdge, EvidenceRecord};

const MAX_VERSION_TRANSACTIONS: usize = 8;
const MAX_PREVOUTS: usize = 24;
const MAX_COMMITMENT_PREVOUTS: usize = 8;
const MAX_REVERSE_ADDRESSES: usize = 4;
const MAX_REVERSE_RESULTS: usize = 12;

async fn load_evidence_transaction(
    config: &Config,
    client: &RpcClient,
    txid: &str,
) -> Result<TransactionRecord> {
    let params = serde_json::to_string(&[json!(txid), json!(1)])?;
    if let Some((raw, source)) = cached_rpc_response(config, "getrawtransaction", &params).await?
        && raw.get("blockhash").is_some()
    {
        let mut record = transaction_record(txid, raw);
        record.source_id = Some(source);
        return Ok(record);
    }
    let output = client.get_raw_transaction(txid).await?;
    anyhow::ensure!(
        output.result.get("txid").and_then(Value::as_str) == Some(txid),
        "evidence source returned a different transaction ID"
    );
    let mut record = transaction_record(txid, output.result);
    record.source_id = Some(output.source_id);
    Ok(record)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum EvidenceTarget {
    Identity(String),
    Address(String),
    Transaction(String),
}

#[derive(Debug, Clone)]
pub(super) struct EvidenceLink {
    pub label: String,
    pub detail: String,
    pub target: EvidenceTarget,
    pub source: String,
}

#[derive(Debug, Clone, Default)]
pub(super) struct IdentityEvidence {
    pub registration: Option<String>,
    pub links: Vec<EvidenceLink>,
    pub notes: Vec<String>,
    pub transactions: Vec<TransactionRecord>,
    pub retained_anchor: Option<(String, Option<String>)>,
}

pub(super) fn to_record(
    system: &str,
    identity: &str,
    version_txid: &str,
    version_height: u64,
    report: &IdentityEvidence,
) -> Option<EvidenceRecord> {
    let version = report.transactions.iter().find(|transaction| {
        transaction.summary.txid == version_txid
            && transaction.block_hash.is_some()
            && transaction.source_id.is_some()
    });
    let (source_id, block_hash) = if let Some(version) = version {
        (version.source_id.clone()?, version.block_hash.clone())
    } else {
        report.retained_anchor.clone()?
    };
    let edges = report
        .links
        .iter()
        .map(|link| {
            let (target_kind, target_id) = match &link.target {
                EvidenceTarget::Identity(id) => ("identity", id),
                EvidenceTarget::Address(id) => ("address", id),
                EvidenceTarget::Transaction(id) => ("transaction", id),
            };
            EvidenceEdge {
                relation: link.label.clone(),
                target_kind: target_kind.into(),
                target_id: target_id.clone(),
                detail: link.detail.clone(),
                evidence: link.source.clone(),
            }
        })
        .collect();
    Some(EvidenceRecord {
        system: system.into(),
        identity: identity.into(),
        version_txid: version_txid.into(),
        version_height,
        source_id,
        block_hash,
        registration: report.registration.clone(),
        notes: report.notes.clone(),
        edges,
        observed_at: 0,
        membership: "unverified".into(),
    })
}

pub(super) fn from_record(record: EvidenceRecord) -> IdentityEvidence {
    let retained_anchor = Some((record.source_id.clone(), record.block_hash.clone()));
    let mut notes = record.notes;
    notes.push(format!(
        "Retained evidence from {} at {}; block membership {}. Refreshing.",
        record.source_id, record.observed_at, record.membership
    ));
    let links = record
        .edges
        .into_iter()
        .filter_map(|edge| {
            let target = match edge.target_kind.as_str() {
                "identity" => EvidenceTarget::Identity(edge.target_id),
                "address" => EvidenceTarget::Address(edge.target_id),
                "transaction" => EvidenceTarget::Transaction(edge.target_id),
                _ => return None,
            };
            Some(EvidenceLink {
                label: edge.relation,
                detail: edge.detail,
                target,
                source: edge.evidence,
            })
        })
        .collect();
    IdentityEvidence {
        registration: record.registration,
        links,
        notes,
        transactions: Vec::new(),
        retained_anchor,
    }
}

#[cfg(test)]
mod retained_report_tests {
    use super::*;

    #[test]
    fn historical_expansion_can_resave_a_retained_report_without_live_transactions() {
        let record = EvidenceRecord {
            system: "iSystem".into(),
            identity: "iIdentity".into(),
            version_txid: "version".into(),
            version_height: 10,
            source_id: "local".into(),
            block_hash: Some("block".into()),
            registration: None,
            notes: Vec::new(),
            edges: Vec::new(),
            observed_at: 0,
            membership: "unverified".into(),
        };
        let report = from_record(record);
        let resaved = to_record("iSystem", "iIdentity", "version", 10, &report).unwrap();
        assert_eq!(resaved.source_id, "local");
        assert_eq!(resaved.block_hash.as_deref(), Some("block"));
    }
}

pub(super) fn merge_historical(target: &mut IdentityEvidence, retained: &IdentityEvidence) {
    for link in &retained.links {
        if link.label.starts_with("Historical shared") {
            add_link(
                target,
                &link.label,
                link.detail.clone(),
                link.target.clone(),
                link.source.clone(),
            );
        }
    }
    for note in &retained.notes {
        if note.starts_with("Historical reverse window") && !target.notes.contains(note) {
            target.notes.push(note.clone());
        }
    }
}

fn script_at(transaction: &Value, index: u64) -> Option<&Value> {
    transaction
        .get("vout")?
        .as_array()?
        .iter()
        .find(|output| output.get("n").and_then(Value::as_u64) == Some(index))?
        .get("scriptPubKey")
}

fn reservation(transaction: &Value) -> Option<(&Value, u64)> {
    transaction
        .get("vout")?
        .as_array()?
        .iter()
        .find_map(|output| {
            let script = output.get("scriptPubKey")?;
            let reservation = script
                .get("identityreservation")
                .or_else(|| script.get("identityadvancedreservation"))
                .or_else(|| script.get("identityregistration"))?;
            Some((reservation, output.get("n")?.as_u64()?))
        })
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn script_addresses(script: &Value) -> Vec<String> {
    script
        .get("addresses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn transparent_funding_script(script: &Value) -> bool {
    matches!(
        script.get("type").and_then(Value::as_str),
        Some("pubkeyhash" | "pubkey" | "scripthash" | "multisig")
    )
}

fn add_link(
    report: &mut IdentityEvidence,
    label: &str,
    detail: String,
    target: EvidenceTarget,
    source: String,
) {
    if !report
        .links
        .iter()
        .any(|link| link.label == label && link.target == target && link.source == source)
    {
        report.links.push(EvidenceLink {
            label: label.to_string(),
            detail,
            target,
            source,
        });
    }
}

/// Identify a locally created identity output in the import's generated region,
/// corroborated by the matching identity-export transfer in supplemental data.
fn add_identity_import(report: &mut IdentityEvidence, transaction: &Value, seed: &str) {
    let Some(txid) = transaction.get("txid").and_then(Value::as_str) else {
        return;
    };
    let Some(outputs) = transaction.get("vout").and_then(Value::as_array) else {
        return;
    };
    let mut matches = Vec::new();
    for (position, output) in outputs.iter().enumerate() {
        let Some(import) = output
            .pointer("/scriptPubKey/crosschainimport")
            .filter(|value| value.is_object())
        else {
            continue;
        };
        // A system-import companion repeats the same batch; use its main import.
        if import["flags"]
            .as_u64()
            .is_some_and(|flags| flags & 0x40 != 0)
        {
            continue;
        }
        let (Some(source_system), Some(export_txid), Some(source_height)) = (
            import["sourcesystemid"].as_str(),
            import["exporttxid"].as_str(),
            import["sourceheight"].as_u64(),
        ) else {
            continue;
        };
        let Ok(import_index) = u32::try_from(position) else {
            continue;
        };
        let Some((start, generated)) =
            crate::defi::generated_region(transaction, import, import_index)
        else {
            continue;
        };
        for (offset, generated_output) in generated.iter().enumerate() {
            let Some(identity) = generated_output.pointer("/scriptPubKey/identityprimary") else {
                continue;
            };
            if identity["identityaddress"].as_str() != Some(seed) {
                continue;
            }
            let corroborated = outputs[..start]
                .iter()
                .filter_map(|output| output.pointer("/scriptPubKey/crosschainexport"))
                .filter(|export| export["issupplemental"].as_bool() == Some(true))
                .filter_map(|export| export["transfers"].as_array())
                .flatten()
                .any(|transfer| {
                    transfer["flags"]
                        .as_u64()
                        .is_some_and(|flags| flags & 0x1000 != 0)
                        && transfer.pointer("/destination/identity") == Some(identity)
                });
            if !corroborated {
                continue;
            }
            let index = generated_output["n"]
                .as_u64()
                .unwrap_or((start + offset) as u64);
            matches.push((
                index,
                source_system,
                source_height,
                export_txid,
                import_index,
            ));
        }
    }
    // More than one distinct source batch is ambiguous; do not choose one.
    if matches.len() != 1 {
        return;
    }
    let (index, source_system, source_height, export_txid, import_index) = matches[0];
    if report.registration.is_none() {
        report.registration = Some(format!("Imported identity at output {index}"));
    }
    add_link(
        report,
        "Identity import transaction",
        format!("identity output {index}; source {source_system} at height {source_height}"),
        EvidenceTarget::Transaction(txid.into()),
        format!(
            "{txid}:{index}; import {txid}:{import_index}; source system {source_system}; export {export_txid}"
        ),
    );
    let note = "Imported identity; earlier versions may exist on the source system";
    if !report.notes.iter().any(|item| item == note) {
        report.notes.push(note.into());
    }
}

fn add_registration(
    report: &mut IdentityEvidence,
    transaction: &Value,
    prevouts: &HashMap<String, Value>,
) -> Option<(String, u64)> {
    let txid = transaction.get("txid")?.as_str()?;
    let (reservation, reservation_index) = reservation(transaction)?;
    let name = text_field(reservation, "name").unwrap_or_else(|| "unknown".to_string());
    report.registration = Some(format!(
        "{name}@  tx {}:{}",
        short_id(txid),
        reservation_index
    ));
    add_link(
        report,
        "Registration transaction",
        format!("reservation output {reservation_index}"),
        EvidenceTarget::Transaction(txid.to_string()),
        format!("{txid}:{reservation_index}"),
    );
    for (field, label) in [
        ("parent", "Parent"),
        ("referral", "Referral"),
        ("nameid", "Name ID"),
    ] {
        if let Some(address) = text_field(reservation, field) {
            add_link(
                report,
                label,
                format!("reservation {field}"),
                EvidenceTarget::Identity(address),
                format!("{txid}:{reservation_index}"),
            );
        }
    }
    if let Some(script) = script_at(transaction, reservation_index) {
        for address in script_addresses(script)
            .into_iter()
            .filter(|address| address.starts_with('R'))
        {
            add_link(
                report,
                "Reservation output destination",
                "decoded reservation output destination".to_string(),
                EvidenceTarget::Address(address),
                format!("{txid}:{reservation_index}"),
            );
        }
    }
    let mut commitment = None;
    for input in transaction
        .get("vin")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(source_txid), Some(index)) = (
            input.get("txid").and_then(Value::as_str),
            input.get("vout").and_then(Value::as_u64),
        ) else {
            continue;
        };
        let Some(script) = prevouts
            .get(source_txid)
            .and_then(|source| script_at(source, index))
        else {
            continue;
        };
        let Some(hash) = script
            .get("commitmenthash")
            .or_else(|| script.get("identitycommitment"))
        else {
            continue;
        };
        commitment = Some((source_txid.to_string(), index));
        add_link(
            report,
            "Name commitment",
            format!(
                "hash {}",
                hash.get("hash")
                    .and_then(Value::as_str)
                    .unwrap_or("unavailable")
            ),
            EvidenceTarget::Transaction(source_txid.to_string()),
            format!("{source_txid}:{index}"),
        );
        for address in script_addresses(script) {
            let target = if address.starts_with('i') {
                EvidenceTarget::Identity(address)
            } else {
                EvidenceTarget::Address(address)
            };
            add_link(
                report,
                "Commitment control destination",
                "decoded commitment output destination; control role may vary".to_string(),
                target,
                format!("{source_txid}:{index}"),
            );
        }
        break;
    }
    if commitment.is_none() {
        report
            .notes
            .push("Consumed name commitment unavailable from fetched prevouts".to_string());
    }
    commitment
}

fn add_funding(
    report: &mut IdentityEvidence,
    transaction: &Value,
    prevouts: &HashMap<String, Value>,
    excluded: Option<(&str, u64)>,
    label: &str,
) {
    let Some(txid) = transaction.get("txid").and_then(Value::as_str) else {
        return;
    };
    for (input_index, input) in transaction
        .get("vin")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        let (Some(source_txid), Some(index)) = (
            input.get("txid").and_then(Value::as_str),
            input.get("vout").and_then(Value::as_u64),
        ) else {
            continue;
        };
        if excluded == Some((source_txid, index)) {
            continue;
        }
        let Some(output) = prevouts.get(source_txid).and_then(|source| {
            source
                .get("vout")?
                .as_array()?
                .iter()
                .find(|output| output.get("n").and_then(Value::as_u64) == Some(index))
        }) else {
            continue;
        };
        let Some(script) = output.get("scriptPubKey") else {
            continue;
        };
        if !transparent_funding_script(script) {
            continue;
        }
        let value = output
            .get("value")
            .map(Value::to_string)
            .unwrap_or_else(|| "unknown".to_string());
        for address in script_addresses(script)
            .into_iter()
            .filter(|address| address.starts_with('R'))
        {
            add_link(
                report,
                label,
                format!("input {input_index}, {value} native units; transparent candidate"),
                EvidenceTarget::Address(address),
                format!("{txid} input {input_index} <- {source_txid}:{index}"),
            );
        }
    }
}

fn add_reverse_results(
    report: &mut IdentityEvidence,
    values: Vec<Value>,
    authority: &str,
    field: &str,
    source: &str,
    seed: &str,
) {
    let count = values.len();
    let mut seen = HashSet::new();
    for identity in values.into_iter().take(MAX_REVERSE_RESULTS) {
        let Some(address) = identity.get("identityaddress").and_then(Value::as_str) else {
            continue;
        };
        if address == seed
            || identity.get(field).and_then(Value::as_str) != Some(authority)
            || !seen.insert(address.to_string())
        {
            continue;
        }
        let name = identity
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(address);
        let outpoint = identity
            .get("txout")
            .map(Value::to_string)
            .unwrap_or_else(|| "unknown outpoint".to_string());
        add_link(
            report,
            &format!("Shared {field}"),
            format!("{name}@; active identity output {outpoint}; {source}"),
            EvidenceTarget::Identity(address.to_string()),
            format!("{source} {field}={authority}"),
        );
    }
    if count > MAX_REVERSE_RESULTS {
        report.notes.push(format!(
            "{field} reverse lookup displayed first {MAX_REVERSE_RESULTS} of {count} results"
        ));
    }
}

pub(super) async fn expand_historical(
    config: &Config,
    report: &mut IdentityEvidence,
    versions: &[IdentityVersionRecord],
    from_height: u64,
    to_height: u64,
) -> Result<()> {
    anyhow::ensure!(
        from_height <= to_height && to_height - from_height < 1_000,
        "historical reverse lookup must fit one 1,000-block window"
    );
    anyhow::ensure!(
        report.links.len() < crate::cache::identity::MAX_EDGES,
        "identity evidence link cap reached"
    );
    let Some(seed) = versions.first().map(|v| v.identity_address.as_str()) else {
        anyhow::bail!("identity version unavailable");
    };
    let client = RpcClient::for_config(config)?
        .noninteractive()
        .probe_identity_index();
    let mut queries = Vec::new();
    for (field, recovery) in [("recoveryauthority", true), ("revocationauthority", false)] {
        let mut values = BTreeSet::new();
        for version in versions {
            if let Some(value) = version.raw_json["identity"][field].as_str()
                && value.starts_with('i')
            {
                values.insert(value.to_string());
            }
        }
        for authority in values.into_iter().take(MAX_REVERSE_ADDRESSES) {
            queries.push((field, recovery, authority));
        }
    }
    let mut failures = 0;
    for (field, recovery, authority) in queries {
        match client
            .get_historical_identities_with_authority(&authority, recovery, from_height, to_height)
            .await
        {
            Ok(output) => {
                let source = format!(
                    "{} {field}={authority} fromheight={from_height} toheight={to_height} unspent=false",
                    output.source_id
                );
                let count = output.result.len();
                for candidate in output.result.into_iter().take(MAX_REVERSE_RESULTS) {
                    let Some(other) = candidate["identityaddress"].as_str() else {
                        continue;
                    };
                    if other == seed || candidate[field].as_str() != Some(&authority) {
                        continue;
                    }
                    let outpoint = candidate
                        .get("txout")
                        .map(Value::to_string)
                        .unwrap_or_else(|| "unknown outpoint".into());
                    add_link(
                        report,
                        &format!("Historical shared {field}"),
                        format!("identity output {outpoint}; window {from_height}-{to_height}"),
                        EvidenceTarget::Identity(other.into()),
                        source.clone(),
                    );
                }
                if count > MAX_REVERSE_RESULTS {
                    report.notes.push(format!("Historical {field}={authority}: first {MAX_REVERSE_RESULTS} of {count} results"));
                }
            }
            Err(error) => {
                failures += 1;
                tracing::debug!(%error, "historical identity association omitted");
            }
        }
    }
    let mut addresses = BTreeSet::new();
    for version in versions {
        for address in version.raw_json["identity"]["primaryaddresses"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if address.starts_with('R') {
                addresses.insert(address.to_string());
            }
        }
    }
    for address in addresses.into_iter().take(MAX_REVERSE_ADDRESSES) {
        match client
            .get_historical_identities_with_address(&address, from_height, to_height)
            .await
        {
            Ok(output) => {
                let source = format!(
                    "{} getidentitieswithaddress address={address} fromheight={from_height} toheight={to_height} unspent=false",
                    output.source_id
                );
                let count = output.result.len();
                for candidate in output.result.into_iter().take(MAX_REVERSE_RESULTS) {
                    let Some(other) = candidate["identityaddress"].as_str() else {
                        continue;
                    };
                    if other == seed
                        || !candidate["primaryaddresses"]
                            .as_array()
                            .is_some_and(|values| {
                                values.iter().any(|value| value.as_str() == Some(&address))
                            })
                    {
                        continue;
                    }
                    let outpoint = candidate
                        .get("txout")
                        .map(Value::to_string)
                        .unwrap_or_else(|| "unknown outpoint".into());
                    add_link(
                        report,
                        "Historical shared primary address",
                        format!("identity output {outpoint}; window {from_height}-{to_height}"),
                        EvidenceTarget::Identity(other.into()),
                        source.clone(),
                    );
                }
                if count > MAX_REVERSE_RESULTS {
                    report.notes.push(format!("Historical address {address}: first {MAX_REVERSE_RESULTS} of {count} results"));
                }
            }
            Err(error) => {
                failures += 1;
                tracing::debug!(%error, "historical address association omitted");
            }
        }
    }
    if report.links.len() > crate::cache::identity::MAX_EDGES {
        report.links.truncate(crate::cache::identity::MAX_EDGES);
        report
            .notes
            .push("Identity evidence link cap reached; further matches were omitted".into());
    }
    report.notes.push(format!(
        "Historical reverse window {from_height}-{to_height}; {failures} unavailable queries"
    ));
    Ok(())
}

fn association_kind(label: &str) -> Option<&'static str> {
    match label {
        "Commitment control destination" => Some("commitment control"),
        "Commitment funding candidate"
        | "Registration funding candidate"
        | "Update funding candidate" => Some("funding"),
        _ => None,
    }
}

async fn add_referral_chain(
    report: &mut IdentityEvidence,
    config: &Config,
    client: &RpcClient,
    first: Option<String>,
) {
    let mut next = first;
    let mut visited = HashSet::new();
    for depth in 1..=4 {
        let Some(address) = next.take() else { return };
        if !visited.insert(address.clone()) {
            report
                .notes
                .push("Referral chain contains a cycle".to_string());
            return;
        }
        let Ok(history) = client.get_identity_history(&address).await else {
            report
                .notes
                .push(format!("Referral ancestry unavailable beyond {address}"));
            return;
        };
        let Some(first_version) = history
            .result
            .get("history")
            .and_then(Value::as_array)
            .and_then(|versions| versions.first())
        else {
            return;
        };
        let Some(txid) = first_version
            .get("output")
            .and_then(|output| output.get("txid"))
            .and_then(Value::as_str)
        else {
            return;
        };
        let Ok(transaction) = load_evidence_transaction(config, client, txid).await else {
            return;
        };
        let Some((reservation, index)) = reservation(&transaction.raw_json) else {
            return;
        };
        next = text_field(reservation, "referral");
        if let Some(ancestor) = &next {
            add_link(
                report,
                "Referral ancestor",
                format!("depth {depth} via {address}; reservation output {index}"),
                EvidenceTarget::Identity(ancestor.clone()),
                format!("{txid}:{index}"),
            );
        }
        report.transactions.push(transaction);
    }
    if next.is_some() {
        report
            .notes
            .push("Referral ancestry capped at four hops".to_string());
    }
}

pub(super) fn connect_shared_evidence(
    left_address: &str,
    left: &mut IdentityEvidence,
    right_address: &str,
    right: &mut IdentityEvidence,
) {
    let left_evidence = left.links.clone();
    let right_evidence = right.links.clone();
    let mut added = 0;
    for left_link in &left_evidence {
        let (Some(left_kind), EvidenceTarget::Address(left_target)) =
            (association_kind(&left_link.label), &left_link.target)
        else {
            continue;
        };
        for right_link in &right_evidence {
            let (Some(right_kind), EvidenceTarget::Address(right_target)) =
                (association_kind(&right_link.label), &right_link.target)
            else {
                continue;
            };
            if left_kind != right_kind || left_target != right_target {
                continue;
            }
            let detail = format!(
                "shared {left_kind} destination {left_target}; inspect both source outpoints"
            );
            let source = format!("{} | {}", left_link.source, right_link.source);
            add_link(
                left,
                &format!("Shared {left_kind}"),
                detail.clone(),
                EvidenceTarget::Identity(right_address.to_string()),
                source.clone(),
            );
            add_link(
                right,
                &format!("Shared {left_kind}"),
                detail,
                EvidenceTarget::Identity(left_address.to_string()),
                source,
            );
            added += 1;
            if added == MAX_REVERSE_RESULTS {
                left.notes.push(
                    "Shared evidence comparison capped; inspect source transactions for more"
                        .to_string(),
                );
                right.notes.push(
                    "Shared evidence comparison capped; inspect source transactions for more"
                        .to_string(),
                );
                return;
            }
        }
    }
}

pub(super) fn transaction_registration_lines(
    transaction: &Value,
    store: &ObjectStore,
) -> Vec<String> {
    let Some((reservation, index)) = reservation(transaction) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    let name = text_field(reservation, "name").unwrap_or_else(|| "unknown".to_string());
    let id = text_field(reservation, "nameid").unwrap_or_else(|| "unknown".to_string());
    lines.push(format!(
        "Registers {name}@ ({id}) from reservation output {index}"
    ));
    for (field, label) in [("parent", "Parent"), ("referral", "Referral")] {
        if let Some(value) = text_field(reservation, field) {
            lines.push(format!(
                "{label}: {}",
                identity_display_value(store, &value, false)
            ));
        }
    }
    if let Some(script) = script_at(transaction, index) {
        let destinations = script_addresses(script)
            .into_iter()
            .filter(|address| address.starts_with('R'))
            .collect::<Vec<_>>();
        if !destinations.is_empty() {
            lines.push(format!(
                "Reservation output destination: {}",
                destinations.join(", ")
            ));
        }
    }
    lines
}

pub(super) async fn load(
    config: &Config,
    versions: Vec<IdentityVersionRecord>,
) -> Result<IdentityEvidence> {
    let mut report = IdentityEvidence::default();
    let Some(current) = versions.first().cloned() else {
        return Ok(report);
    };
    let client = RpcClient::for_config(config)?.noninteractive();
    let seed = current.identity_address.clone();
    let mut ordered = versions;
    ordered.sort_by_key(|version| version.block_height);
    for pair in ordered.windows(2).take(MAX_REVERSE_RESULTS) {
        let version = &pair[0];
        let next_height = pair[1].block_height;
        let Some(identity) = version.raw_json.get("identity") else {
            continue;
        };
        let source = format!(
            "getidentity version at height {} (through height {})",
            version.block_height,
            next_height.saturating_sub(1)
        );
        for (field, label) in [
            ("recoveryauthority", "Historical recovery authority"),
            ("revocationauthority", "Historical revocation authority"),
        ] {
            let Some(authority) = identity
                .get(field)
                .and_then(Value::as_str)
                .filter(|address| address.starts_with('i') && *address != seed)
            else {
                continue;
            };
            add_link(
                &mut report,
                label,
                format!(
                    "effective from {} through {}",
                    version.block_height,
                    next_height.saturating_sub(1)
                ),
                EvidenceTarget::Identity(authority.to_string()),
                source.clone(),
            );
        }
    }
    if ordered.len() > MAX_VERSION_TRANSACTIONS {
        report.notes.push(format!(
            "Funding inspection limited to the oldest and newest {} updates",
            MAX_VERSION_TRANSACTIONS
        ));
        ordered = ordered
            .iter()
            .take(MAX_VERSION_TRANSACTIONS / 2)
            .chain(ordered.iter().rev().take(MAX_VERSION_TRANSACTIONS / 2))
            .cloned()
            .collect();
        ordered.sort_by_key(|version| version.block_height);
    }
    let mut transactions = HashMap::new();
    for version in &ordered {
        let Some(txid) = version.raw_json.get("txid").and_then(Value::as_str) else {
            continue;
        };
        if txid.chars().all(|ch| ch == '0') || transactions.contains_key(txid) {
            continue;
        }
        if let Ok(transaction) = load_evidence_transaction(config, &client, txid).await {
            transactions.insert(txid.to_string(), transaction);
        }
    }
    for transaction in transactions.values() {
        add_identity_import(&mut report, &transaction.raw_json, &seed);
    }
    let mut prevout_ids = Vec::new();
    let mut seen_prevouts = HashSet::new();
    for version in &ordered {
        let Some(transaction) = version
            .raw_json
            .get("txid")
            .and_then(Value::as_str)
            .and_then(|txid| transactions.get(txid))
        else {
            continue;
        };
        if report.links.iter().any(|link| {
            link.label == "Identity import transaction"
                && link.target == EvidenceTarget::Transaction(transaction.summary.txid.clone())
        }) {
            continue; // Import protocol inputs are not registration funding.
        }
        for txid in transaction
            .raw_json
            .get("vin")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|input| input.get("txid").and_then(Value::as_str))
            .map(str::to_string)
        {
            if seen_prevouts.insert(txid.clone()) {
                prevout_ids.push(txid);
            }
        }
    }
    if prevout_ids.len() > MAX_PREVOUTS {
        report.notes.push(format!(
            "Prevout inspection limited to {MAX_PREVOUTS} transactions"
        ));
    }
    let mut prevouts = HashMap::new();
    for txid in prevout_ids.into_iter().take(MAX_PREVOUTS) {
        if let Ok(transaction) = load_evidence_transaction(config, &client, &txid).await {
            prevouts.insert(txid, transaction);
        }
    }
    if let Some(registration) = ordered
        .first()
        .and_then(|version| version.raw_json.get("txid"))
        .and_then(Value::as_str)
        .and_then(|txid| transactions.get(txid))
    {
        let previous = prevouts
            .iter()
            .map(|(txid, transaction)| (txid.clone(), transaction.raw_json.clone()))
            .collect::<HashMap<_, _>>();
        let commitment = add_registration(&mut report, &registration.raw_json, &previous);
        let first_referral = reservation(&registration.raw_json)
            .and_then(|(value, _)| text_field(value, "referral"));
        add_referral_chain(&mut report, config, &client, first_referral).await;
        if !report.links.iter().any(|link| {
            link.label == "Identity import transaction"
                && link.target == EvidenceTarget::Transaction(registration.summary.txid.clone())
        }) {
            add_funding(
                &mut report,
                &registration.raw_json,
                &previous,
                commitment
                    .as_ref()
                    .map(|(txid, index)| (txid.as_str(), *index)),
                if reservation(&registration.raw_json).is_some() {
                    "Registration funding candidate"
                } else {
                    "First local version funding candidate"
                },
            );
        }
        if let Some((commit_txid, _)) = commitment
            && let Some(commit_tx) = prevouts.get(&commit_txid)
        {
            let mut commit_prevouts = HashMap::new();
            for source_txid in source_transaction_ids(&commit_tx.raw_json)
                .into_iter()
                .take(MAX_COMMITMENT_PREVOUTS)
            {
                if let Ok(source) = load_evidence_transaction(config, &client, &source_txid).await {
                    report.transactions.push(source.clone());
                    commit_prevouts.insert(source_txid, source.raw_json);
                }
            }
            add_funding(
                &mut report,
                &commit_tx.raw_json,
                &commit_prevouts,
                None,
                "Commitment funding candidate",
            );
        }
    }
    for version in ordered.iter().skip(1) {
        if let Some(transaction) = version
            .raw_json
            .get("txid")
            .and_then(Value::as_str)
            .and_then(|txid| transactions.get(txid))
        {
            let previous = prevouts
                .iter()
                .map(|(txid, transaction)| (txid.clone(), transaction.raw_json.clone()))
                .collect::<HashMap<_, _>>();
            add_funding(
                &mut report,
                &transaction.raw_json,
                &previous,
                None,
                "Update funding candidate",
            );
        }
    }
    report.transactions.extend(transactions.into_values());
    report.transactions.extend(prevouts.into_values());

    let identity = current.raw_json.get("identity").unwrap_or(&Value::Null);
    for (field, recovery) in [("recoveryauthority", true), ("revocationauthority", false)] {
        let Some(authority) = identity
            .get(field)
            .and_then(Value::as_str)
            .filter(|address| address.starts_with('i'))
        else {
            continue;
        };
        match client
            .get_identities_with_authority(authority, recovery)
            .await
        {
            Ok(output) => add_reverse_results(
                &mut report,
                output.result,
                authority,
                field,
                &output.source_id,
                &seed,
            ),
            Err(error) => tracing::debug!(%error, "optional reverse identity association omitted"),
        }
    }
    let primary = identity
        .get("primaryaddresses")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|address| address.starts_with('R'))
        .take(MAX_REVERSE_ADDRESSES)
        .collect::<Vec<_>>();
    for address in primary {
        if let Ok(output) = client.get_identities_with_address(address).await {
            let count = output.result.len();
            for candidate in output.result.into_iter().take(MAX_REVERSE_RESULTS) {
                let Some(other) = candidate.get("identityaddress").and_then(Value::as_str) else {
                    continue;
                };
                if other == seed
                    || !candidate
                        .get("primaryaddresses")
                        .and_then(Value::as_array)
                        .is_some_and(|addresses| {
                            addresses
                                .iter()
                                .any(|value| value.as_str() == Some(address))
                        })
                {
                    continue;
                }
                add_link(
                    &mut report,
                    "Shared primary address",
                    format!("active identity; {}", output.source_id),
                    EvidenceTarget::Identity(other.to_string()),
                    format!(
                        "{} getidentitieswithaddress address={address}",
                        output.source_id
                    ),
                );
            }
            if count > MAX_REVERSE_RESULTS {
                report.notes.push(format!("Primary address {address}: showing first {MAX_REVERSE_RESULTS} of {count} results"));
            }
        }
    }
    let source_chain = config
        .chain
        .system_id
        .get()
        .map(ToString::to_string)
        .unwrap_or_else(|| config.chain.name.clone());
    for link in &mut report.links {
        link.source = format!("{source_chain} {}", link.source);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_identity_has_a_local_import_link_with_source_provenance() {
        let transaction: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/vdex-identity-import.json"
        ))
        .unwrap();
        let own = "iExBJfZYK7KREDpuhj6PzZBzqMAKaFg7d2";
        let mut report = IdentityEvidence::default();
        add_identity_import(&mut report, &transaction, own);
        assert_eq!(report.links.len(), 1);
        assert_eq!(report.links[0].label, "Identity import transaction");
        assert_eq!(
            report.links[0].target,
            EvidenceTarget::Transaction(transaction["txid"].as_str().unwrap().into())
        );
        assert!(
            report.links[0]
                .source
                .contains("a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4")
        );
        assert!(
            report.links[0]
                .source
                .contains("i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV")
        );
        assert!(report.registration.as_deref().unwrap().contains("Imported"));
        let mut unrelated = IdentityEvidence::default();
        add_identity_import(&mut unrelated, &transaction, "iUnrelated");
        assert!(unrelated.links.is_empty());
        let mut missing_transfer = transaction.clone();
        missing_transfer["vout"][4]["scriptPubKey"]["crosschainexport"]["transfers"] = json!([]);
        add_identity_import(&mut unrelated, &missing_transfer, own);
        assert!(unrelated.links.is_empty());
        let mut wrong_region = transaction.clone();
        wrong_region["vout"][0]["scriptPubKey"]["crosschainimport"]["numoutputs"] = json!(0);
        add_identity_import(&mut unrelated, &wrong_region, own);
        assert!(unrelated.links.is_empty());
    }

    #[test]
    fn registration_evidence_separates_reservation_commitment_and_funding() {
        let registration = json!({
            "txid": "registration", "vin": [
                {"txid": "funding", "vout": 0},
                {"txid": "commitment", "vout": 1}
            ],
            "vout": [{"n": 0, "scriptPubKey": {
                "identityreservation": {"name": "VRSC", "nameid": "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV", "parent": "iParent", "referral": "iReferral", "salt": "salt"},
                "addresses": ["RSigner", "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV"]
            }}]
        });
        let prevouts = HashMap::from([
            (
                "funding".to_string(),
                json!({"vout": [{"n": 0, "value": 100, "scriptPubKey": {"type": "pubkeyhash", "addresses": ["RFunder"]}}]}),
            ),
            (
                "commitment".to_string(),
                json!({"vout": [{"n": 1, "scriptPubKey": {
                    "commitmenthash": {"hash": "hash"}, "addresses": ["RControl"]
                }}]}),
            ),
        ]);
        let mut report = IdentityEvidence::default();
        let commitment = add_registration(&mut report, &registration, &prevouts);
        add_funding(
            &mut report,
            &registration,
            &prevouts,
            commitment
                .as_ref()
                .map(|(txid, index)| (txid.as_str(), *index)),
            "Registration funding candidate",
        );
        assert_eq!(
            report.registration.as_deref(),
            Some("VRSC@  tx registration:0")
        );
        assert!(report.links.iter().any(|link| link.label == "Referral"
            && link.target == EvidenceTarget::Identity("iReferral".into())));
        assert!(
            report
                .links
                .iter()
                .any(|link| link.label == "Reservation output destination"
                    && link.target == EvidenceTarget::Address("RSigner".into()))
        );
        assert!(
            report
                .links
                .iter()
                .any(|link| link.label == "Commitment control destination"
                    && link.target == EvidenceTarget::Address("RControl".into()))
        );
        assert!(
            report
                .links
                .iter()
                .any(|link| link.label == "Registration funding candidate"
                    && link.target == EvidenceTarget::Address("RFunder".into()))
        );
        assert!(
            !report
                .links
                .iter()
                .any(|link| link.label == "Registration funding candidate"
                    && link.target == EvidenceTarget::Address("RControl".into()))
        );
    }

    #[test]
    fn reverse_authority_uses_matching_decoded_field_and_excludes_seed() {
        let mut report = IdentityEvidence::default();
        add_reverse_results(
            &mut report,
            vec![
                json!({"identityaddress": "iSeed", "name": "Seed", "recoveryauthority": "iAuthority"}),
                json!({"identityaddress": "iOther", "name": "Other", "recoveryauthority": "iAuthority", "txout": {"txid": "evidence", "n": 1}}),
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
        assert!(
            report.links[0]
                .source
                .contains("recoveryauthority=iAuthority")
        );
    }

    #[test]
    fn shared_funding_requires_two_inspected_outpoints() {
        let mut left = IdentityEvidence::default();
        let mut right = IdentityEvidence::default();
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
}
