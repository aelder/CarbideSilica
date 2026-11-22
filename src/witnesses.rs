//! Bounded, read-only evidence traversal. Decoding is not signature verification.
use crate::{
    cache::Cache,
    config::Config,
    rpc::RpcClient,
    world::{OutpointRef, SystemId},
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

const MAX_TRANSACTIONS: usize = 64;
const MAX_NODES: usize = 256;
const MAX_DEPTH: usize = 32;
const MAX_MULTIPART_BYTES: usize = 2 * 1024 * 1024;
const SIGNATURE_TYPE: &str = "iP1QT5ee7EP63WSrfjiMFc1dVJVSAy85cT";

#[derive(Debug, Clone, Serialize)]
pub struct Origin {
    pub output: OutpointRef,
    pub source: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct Signature {
    pub identity: String,
    pub height: Option<u64>,
    pub confirmed: bool,
    pub system: SystemId,
    pub target: OutpointRef,
    pub key_signatures: BTreeSet<String>,
    pub origins: Vec<Origin>,
}
#[derive(Debug, Clone, Default, Serialize)]
pub struct Report {
    pub signatures: Vec<Signature>,
    pub finalizations: Vec<String>,
    pub issues: Vec<String>,
    pub transactions: usize,
    pub outputs: usize,
    pub multipart_sets: usize,
}

pub fn roots(transaction: &Value) -> Vec<(u32, String)> {
    transaction["vout"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, o)| {
            let script = &o["scriptPubKey"];
            [
                ("earnednotarization", "Earned notarization"),
                ("acceptednotarization", "Accepted notarization"),
                ("finalizenotarization", "Finalization"),
                ("notaryevidence", "Evidence"),
            ]
            .into_iter()
            .find(|(key, _)| script.get(key).is_some())
            .map(|(_, label)| (i as u32, label.into()))
        })
        .collect()
}

fn reference(system: &SystemId, value: &Value, carrier: &OutpointRef) -> Option<OutpointRef> {
    let id = value["txid"].as_str()?;
    if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let n = value
        .get("voutnum")
        .or_else(|| value.get("vout"))?
        .as_u64()?;
    let null = id.bytes().all(|b| b == b'0');
    Some(OutpointRef {
        system_id: if null {
            carrier.system_id.clone()
        } else {
            system.clone()
        },
        txid: if null {
            carrier.txid.clone()
        } else {
            id.into()
        },
        vout: u32::try_from(n).ok()?,
    })
}

fn collect_signatures(value: &Value, origins: &[Origin], report: &mut Report, depth: usize) {
    if depth > MAX_DEPTH {
        report
            .issues
            .push("Nested evidence depth limit reached".into());
        return;
    }
    let Some(objects) = value.get("chainobjects").and_then(Value::as_array) else {
        report
            .issues
            .push("Evidence chain objects are absent or unavailable".into());
        return;
    };
    for object in objects {
        let body = &object["value"];
        if object["vdxftype"].as_str() == Some(SIGNATURE_TYPE) {
            let parsed = (|| -> Option<(SystemId, OutpointRef, bool)> {
                let system = SystemId::try_from(body["systemid"].as_str()?.to_owned()).ok()?;
                let target = reference(&system, &body["output"], &origins.first()?.output)?;
                Some((system, target, body["confirmed"].as_bool()?))
            })();
            let Some((system, target, confirmed)) = parsed else {
                report
                    .issues
                    .push("Signature object has incomplete target metadata".into());
                continue;
            };
            let Some(signers) = body["signatures"].as_object() else {
                report
                    .issues
                    .push("Signature object has no signer map".into());
                continue;
            };
            for (identity, signature) in signers {
                let height = signature["blockheight"].as_u64();
                let keys: BTreeSet<String> = signature["signatures"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                if height.is_none() || keys.is_empty() {
                    report
                        .issues
                        .push(format!("Incomplete signature entry for {identity}"));
                }
                if let Some(existing) = report.signatures.iter_mut().find(|s| {
                    s.identity == *identity
                        && s.height == height
                        && s.confirmed == confirmed
                        && s.system == system
                        && s.target == target
                }) {
                    existing.key_signatures.extend(keys);
                    for origin in origins {
                        if !existing
                            .origins
                            .iter()
                            .any(|o| o.output == origin.output && o.source == origin.source)
                        {
                            existing.origins.push(origin.clone());
                        }
                    }
                } else {
                    report.signatures.push(Signature {
                        identity: identity.clone(),
                        height,
                        confirmed,
                        system: system.clone(),
                        target: target.clone(),
                        key_signatures: keys,
                        origins: origins.to_vec(),
                    });
                }
            }
        } else if body.get("chainobjects").is_some() {
            collect_signatures(body, origins, report, depth + 1);
        }
    }
}

fn source_label(config: &Config, source: &str) -> String {
    if source == config.daemon_cache_key() {
        return format!(
            "{} daemon {}:{}",
            config.chain.name, config.daemon.host, config.daemon.port
        );
    }
    config
        .daemon
        .api_pool
        .endpoints
        .iter()
        .find(|e| crate::rpc::source_id("public-api", &e.url) == source)
        .map(|e| format!("API {}", e.url))
        .unwrap_or_else(|| source.to_owned())
}

struct Fetcher {
    config: Config,
    client: RpcClient,
    transactions: HashMap<String, (Value, String)>,
    failed: HashMap<String, String>,
    cancelled: bool,
    requests: usize,
    refresh: bool,
}
impl Fetcher {
    async fn get(&mut self, txid: &str) -> Result<(Value, String)> {
        ensure!(!self.cancelled, "Investigation cancelled");
        if let Some(error) = self.failed.get(txid) {
            bail!("{error}");
        }
        if let Some(value) = self.transactions.get(txid) {
            return Ok(value.clone());
        }
        ensure!(
            self.transactions.len() + self.failed.len() < MAX_TRANSACTIONS,
            "Transaction limit ({MAX_TRANSACTIONS}) reached"
        );
        let params = json!([txid, 1]).to_string();
        let cache = Cache::open(&self.config.cache_path)?;
        let mut sources = vec![self.config.daemon_cache_key()];
        sources.extend(
            self.config
                .daemon
                .api_pool
                .endpoints
                .iter()
                .map(|e| crate::rpc::source_id("public-api", &e.url)),
        );
        for source in sources.into_iter().filter(|_| !self.refresh) {
            if let Some(raw) = cache
                .raw_rpc_response_async(
                    &source,
                    &self.config.chain_cache_key(),
                    "getrawtransaction",
                    &params,
                )
                .await?
            {
                let tx: Value = serde_json::from_str::<Value>(&raw)?["result"].take();
                if tx["txid"].as_str() == Some(txid) {
                    let source = source_label(&self.config, &source);
                    self.transactions
                        .insert(txid.into(), (tx.clone(), source.clone()));
                    return Ok((tx, source));
                }
            }
        }
        ensure!(self.requests < 128, "Investigation RPC limit reached");
        self.requests += 1;
        let output = match self.client.get_raw_transaction(txid).await {
            Ok(output) => output,
            Err(error) => {
                let message = error.to_string();
                self.cancelled = message.to_lowercase().contains("cancel");
                self.failed.insert(txid.into(), message);
                return Err(error);
            }
        };
        ensure!(
            output.result["txid"].as_str() == Some(txid),
            "Source returned a different transaction"
        );
        let source = source_label(&self.config, &output.source_id);
        self.transactions
            .insert(txid.into(), (output.result.clone(), source.clone()));
        Ok((output.result, source))
    }
}

/// A selected carrier owns all evidence input/output references. A signature's
/// foreign target is retained as context, never looked up on the carrier daemon.
pub async fn investigate(
    config: Config,
    root: OutpointRef,
    seed: Value,
    source: String,
    refresh: bool,
) -> Report {
    let mut report = Report::default();
    if config.chain.system_id.get() != Some(&root.system_id) {
        report
            .issues
            .push("Investigation source does not match the output's system".into());
        return report;
    }
    let client = match RpcClient::for_config(&config) {
        Ok(c) => c,
        Err(e) => {
            report.issues.push(e.to_string());
            return report;
        }
    };
    let source = source_label(&config, &source);
    let mut fetch = Fetcher {
        config,
        client,
        transactions: if refresh {
            HashMap::new()
        } else {
            HashMap::from([(root.txid.clone(), (seed, source))])
        },
        failed: HashMap::new(),
        cancelled: false,
        requests: 0,
        refresh,
    };
    let mut queue = VecDeque::from([(root, 0usize)]);
    let mut seen = HashSet::new();
    while let Some((node, depth)) = queue.pop_front() {
        if fetch.cancelled {
            report
                .issues
                .push("Investigation cancelled; remaining evidence not fetched".into());
            break;
        }
        if seen.contains(&node) {
            continue;
        }
        if depth > MAX_DEPTH || seen.len() >= MAX_NODES {
            report
                .issues
                .push("Evidence traversal limit reached; investigation incomplete".into());
            break;
        }
        seen.insert(node.clone());
        let (transaction, source) = match fetch.get(&node.txid).await {
            Ok(value) => value,
            Err(error) => {
                let error = format!("{}:{}: {error}", node.txid, node.vout);
                let cancelled = error.to_lowercase().contains("cancel");
                report.issues.push(error);
                if cancelled {
                    break;
                }
                continue;
            }
        };
        let Some(output) = transaction["vout"]
            .as_array()
            .and_then(|v| v.get(node.vout as usize))
        else {
            report
                .issues
                .push(format!("Missing output {}:{}", node.txid, node.vout));
            continue;
        };
        report.outputs += 1;
        let script = &output["scriptPubKey"];
        if let Some(finalization) = script.get("finalizenotarization") {
            report.finalizations.push(format!(
                "{}:{} — {} (transaction confirmations: {}; source: {})",
                node.txid,
                node.vout,
                finalization["status"].as_str().unwrap_or("unknown"),
                transaction["confirmations"],
                source
            ));
            if finalization["evidenceoutputs"]
                .as_array()
                .is_some_and(|v| v.len() > MAX_NODES)
            {
                report.issues.push("Evidence output limit reached".into());
            }
            if let Some(target) = reference(&node.system_id, &finalization["output"], &node) {
                if let Some(line) = report.finalizations.last_mut() {
                    line.push_str(&format!("; target {}:{}", target.txid, target.vout));
                }
            } else {
                report
                    .issues
                    .push("Finalization target outpoint is absent".into());
            }
            for index in finalization["evidenceoutputs"]
                .as_array()
                .into_iter()
                .flatten()
                .take(MAX_NODES)
            {
                if let Some(index) = index.as_u64().and_then(|n| u32::try_from(n).ok()) {
                    queue.push_back((
                        OutpointRef {
                            vout: index,
                            ..node.clone()
                        },
                        depth + 1,
                    ));
                } else {
                    report.issues.push("Invalid evidence output index".into());
                }
            }
            let mut input_parts = Vec::new();
            let inputs = finalization["evidenceinputs"].as_array();
            if inputs.is_some_and(|inputs| inputs.len() > MAX_NODES) {
                report.issues.push("Evidence input limit reached".into());
            }
            for index in inputs.into_iter().flatten().take(MAX_NODES) {
                let input = index
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .and_then(|n| transaction["vin"].as_array()?.get(n));
                if let Some(target) =
                    input.and_then(|value| reference(&node.system_id, value, &node))
                {
                    match fetch.get(&target.txid).await {
                        Ok((tx, src))
                            if tx["vout"][target.vout as usize]["scriptPubKey"]["notaryevidence"]
                                ["type"]
                                == 2 =>
                        {
                            let first = fragment(
                                &tx["vout"][target.vout as usize]["scriptPubKey"]["notaryevidence"],
                            )
                            .is_ok_and(|f| f.index == 0);
                            if first {
                                finish_input_parts(
                                    &mut fetch,
                                    &mut input_parts,
                                    &mut seen,
                                    &mut report,
                                )
                                .await;
                            }
                            input_parts.push((target, tx, src));
                        }
                        _ => {
                            finish_input_parts(
                                &mut fetch,
                                &mut input_parts,
                                &mut seen,
                                &mut report,
                            )
                            .await;
                            queue.push_back((target, depth + 1));
                        }
                    }
                } else {
                    finish_input_parts(&mut fetch, &mut input_parts, &mut seen, &mut report).await;
                    report
                        .issues
                        .push("Evidence input index or outpoint is absent".into());
                }
            }
            finish_input_parts(&mut fetch, &mut input_parts, &mut seen, &mut report).await;
        } else if let Some(evidence) = script.get("notaryevidence") {
            if evidence["type"] == 2 {
                let mut group = Vec::new();
                // Starting in the middle is explicitly incomplete; don't guess
                // which earlier fragments belong to this set.
                for index in node.vout as usize..transaction["vout"].as_array().map_or(0, Vec::len)
                {
                    let ev = &transaction["vout"][index]["scriptPubKey"]["notaryevidence"];
                    if ev["type"] != 2 {
                        break;
                    }
                    if index > node.vout as usize && fragment(ev).is_ok_and(|f| f.index == 0) {
                        break;
                    }
                    let point = OutpointRef {
                        vout: index as u32,
                        ..node.clone()
                    };
                    if !seen.contains(&point) && seen.len() >= MAX_NODES {
                        report.issues.push(
                            "Evidence output limit reached while assembling fragments".into(),
                        );
                        break;
                    }
                    seen.insert(point.clone());
                    group.push((point, transaction.clone(), source.clone()));
                }
                decode_parts(&mut fetch, &group, &mut report).await;
            } else {
                collect_signatures(
                    &evidence["evidence"],
                    &[Origin {
                        output: node.clone(),
                        source: source.clone(),
                    }],
                    &mut report,
                    0,
                );
            }
        } else if script.get("earnednotarization").is_some()
            || script.get("acceptednotarization").is_some()
        {
            for (index, out) in transaction["vout"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let f = &out["scriptPubKey"]["finalizenotarization"];
                if reference(&node.system_id, &f["output"], &node).as_ref() == Some(&node) {
                    queue.push_back((
                        OutpointRef {
                            vout: index as u32,
                            ..node.clone()
                        },
                        depth + 1,
                    ));
                }
            }
        } else {
            report.issues.push(format!(
                "{}:{} is not notarization evidence or a finalization",
                node.txid, node.vout
            ));
        }
        // Follow only finalizations for this same target in the spending tx.
        // Spending a pending finalization can finalize a different notarization.
        if let Some(spender) = output["spentTxId"].as_str() {
            let target = script
                .get("finalizenotarization")
                .and_then(|f| reference(&node.system_id, &f["output"], &node))
                .unwrap_or(node.clone());
            match fetch.get(spender).await {
                Ok((spent, _)) => {
                    let carrier = OutpointRef {
                        txid: spender.into(),
                        vout: 0,
                        system_id: node.system_id.clone(),
                    };
                    for (index, out) in spent["vout"].as_array().into_iter().flatten().enumerate() {
                        if reference(
                            &node.system_id,
                            &out["scriptPubKey"]["finalizenotarization"]["output"],
                            &carrier,
                        )
                        .as_ref()
                            == Some(&target)
                        {
                            queue.push_back((
                                OutpointRef {
                                    vout: index as u32,
                                    ..carrier.clone()
                                },
                                depth + 1,
                            ));
                        }
                    }
                }
                Err(e) => report
                    .issues
                    .push(format!("Spending transaction {spender} unavailable: {e}")),
            }
        }
    }
    report.transactions = fetch.transactions.len();
    report.outputs = seen.len();
    report.signatures.sort_by(|a, b| {
        (
            &a.identity,
            a.height,
            a.confirmed,
            &a.system,
            &a.target.txid,
            a.target.vout,
        )
            .cmp(&(
                &b.identity,
                b.height,
                b.confirmed,
                &b.system,
                &b.target.txid,
                b.target.vout,
            ))
    });
    report.issues.sort();
    report.issues.dedup();
    report
}

struct Fragment {
    index: u64,
    total: usize,
    start: usize,
    data: Vec<u8>,
}
fn unhex(hex: &str) -> Result<Vec<u8>> {
    ensure!(
        hex.len().is_multiple_of(2) && hex.len() <= MAX_MULTIPART_BYTES * 2,
        "Invalid or oversized evidence hex"
    );
    hex.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|s| Ok(u8::from_str_radix(std::str::from_utf8(s)?, 16)?))
        .collect()
}
struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(n)
            .context("Evidence length overflow")?;
        let bytes = self
            .bytes
            .get(self.position..end)
            .context("Truncated evidence data")?;
        self.position = end;
        Ok(bytes)
    }
    fn varint(&mut self) -> Result<u64> {
        let mut n = 0u64;
        for _ in 0..10 {
            let b = self.take(1)?[0];
            n = n
                .checked_mul(128)
                .and_then(|n| n.checked_add((b & 127) as u64))
                .context("Evidence integer overflow")?;
            if b & 128 == 0 {
                return Ok(n);
            }
            n = n.checked_add(1).context("Evidence integer overflow")?;
        }
        bail!("Oversized evidence integer")
    }
    fn compact(&mut self) -> Result<usize> {
        let b = self.take(1)?[0];
        let value = match b {
            253 => u16::from_le_bytes(self.take(2)?.try_into()?) as u64,
            254 => u32::from_le_bytes(self.take(4)?.try_into()?) as u64,
            255 => u64::from_le_bytes(self.take(8)?.try_into()?),
            _ => b as u64,
        };
        Ok(usize::try_from(value)?)
    }
}
fn fragment(evidence: &Value) -> Result<Fragment> {
    ensure!(evidence["type"] == 2, "Not multipart evidence");
    let objects = evidence["evidence"]["chainobjects"]
        .as_array()
        .context("No multipart object")?;
    ensure!(
        objects.len() == 1,
        "Multipart output must contain one data object"
    );
    let data = unhex(
        objects[0]["value"]["hex"]
            .as_str()
            .context("Multipart data hex absent")?,
    )?;
    let mut r = Reader {
        bytes: &data,
        position: 0,
    };
    // CEvidenceData writes its version twice, then type and VARINT descriptor.
    ensure!(
        r.varint()? == 1 && r.varint()? == 1 && r.varint()? == 2,
        "Unsupported multipart descriptor"
    );
    let index = r.varint()?;
    let total = usize::try_from(r.varint()?)?;
    let start = usize::try_from(r.varint()?)?;
    ensure!(total <= MAX_MULTIPART_BYTES, "Multipart byte limit reached");
    let len = r.compact()?;
    let payload = r.take(len)?.to_vec();
    ensure!(r.position == data.len(), "Trailing multipart bytes");
    Ok(Fragment {
        index,
        total,
        start,
        data: payload,
    })
}
fn assemble(parts: &[Value]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut total = None;
    for (index, evidence) in parts.iter().enumerate() {
        let f = fragment(evidence)?;
        ensure!(
            f.index == index as u64 && f.start == bytes.len(),
            "Missing or out-of-order multipart fragment"
        );
        ensure!(
            total.is_none_or(|n| n == f.total),
            "Multipart lengths disagree"
        );
        total = Some(f.total);
        ensure!(
            bytes.len().saturating_add(f.data.len()) <= f.total,
            "Multipart data exceeds declared length"
        );
        bytes.extend(f.data);
    }
    ensure!(total == Some(bytes.len()), "Multipart evidence incomplete");
    Ok(bytes)
}
fn pushes(bytes: &[u8]) -> Result<Vec<Vec<u8>>> {
    let mut r = Reader { bytes, position: 0 };
    let mut values = Vec::new();
    while r.position < bytes.len() {
        values.push(read_push(&mut r)?.to_vec());
    }
    Ok(values)
}

fn read_push<'a>(r: &mut Reader<'a>) -> Result<&'a [u8]> {
    let opcode = r.take(1)?[0];
    let length = match opcode {
        0..=75 => opcode as usize,
        76 => r.take(1)?[0] as usize,
        77 => u16::from_le_bytes(r.take(2)?.try_into()?) as usize,
        78 => u32::from_le_bytes(r.take(4)?.try_into()?) as usize,
        _ => bail!("Unexpected script opcode"),
    };
    r.take(length)
}

/// Read the first Verus v3 condition parameter set and its ordered data pushes.
pub(crate) fn condition_data(
    script: &serde_json::Value,
    eval: u8,
) -> Option<(Vec<Vec<u8>>, Vec<u8>)> {
    let bytes = unhex(script["hex"].as_str()?).ok()?;
    let mut r = Reader {
        bytes: &bytes,
        position: 0,
    };
    read_push(&mut r).ok()?;
    if r.take(1).ok()? != [0xcc] {
        return None;
    }
    let first = read_push(&mut r).ok()?;
    let mut drops = 1;
    while bytes.get(r.position).is_some_and(|&byte| byte != 0x75) {
        read_push(&mut r).ok()?;
        drops += 1;
    }
    if bytes.len() - r.position != drops || bytes[r.position..].iter().any(|&byte| byte != 0x75) {
        return None;
    }
    let params = pushes(first).ok()?;
    let &[3, code, quorum, count] = params.first()?.as_slice() else {
        return None;
    };
    if code != eval || quorum != 1 || count != 1 {
        return None;
    }
    Some((
        params.get(1..=usize::from(count))?.to_vec(),
        if eval == 0 {
            Vec::new()
        } else {
            params.get(usize::from(count) + 1)?.clone()
        },
    ))
}
fn push(bytes: &[u8], output: &mut Vec<u8>) {
    match bytes.len() {
        0..=75 => output.push(bytes.len() as u8),
        76..=255 => output.extend([76, bytes.len() as u8]),
        256..=65535 => {
            output.push(77);
            output.extend((bytes.len() as u16).to_le_bytes());
        }
        _ => {
            output.push(78);
            output.extend((bytes.len() as u32).to_le_bytes());
        }
    }
    output.extend(bytes);
}
fn decode_script(original: &str, evidence: &[u8]) -> Result<String> {
    let bytes = unhex(original)?;
    let mut r = Reader {
        bytes: &bytes,
        position: 0,
    };
    let op = r.take(1)?[0];
    let n = match op {
        0..=75 => op as usize,
        76 => r.take(1)?[0] as usize,
        77 => u16::from_le_bytes(r.take(2)?.try_into()?) as usize,
        _ => bail!("Unsupported condition push"),
    };
    r.take(n)?;
    ensure!(r.take(1)? == [0xcc], "Missing condition opcode");
    let prefix = &bytes[..r.position];
    ensure!(bytes.last() == Some(&0x75), "Missing parameter drop opcode");
    let outer = pushes(&bytes[r.position..bytes.len() - 1])?;
    ensure!(outer.len() == 1, "Unexpected condition parameter layout");
    let mut params = pushes(&outer[0])?;
    ensure!(
        params.len() >= 3 && params[0].len() == 4,
        "Incomplete condition parameters"
    );
    let keys = params[0][3] as usize;
    let index = 1 + keys;
    ensure!(index < params.len(), "Evidence payload absent");
    params[index] = evidence.to_vec();
    let mut body = Vec::new();
    for item in params {
        push(&item, &mut body);
    }
    let mut script = prefix.to_vec();
    push(&body, &mut script);
    script.push(0x75);
    Ok(script.iter().map(|b| format!("{b:02x}")).collect())
}
async fn finish_input_parts(
    fetch: &mut Fetcher,
    parts: &mut Vec<(OutpointRef, Value, String)>,
    seen: &mut HashSet<OutpointRef>,
    report: &mut Report,
) {
    if parts.is_empty() {
        return;
    }
    let new: HashSet<_> = parts
        .iter()
        .map(|(output, _, _)| output.clone())
        .filter(|output| !seen.contains(output))
        .collect();
    if seen.len() + new.len() > MAX_NODES {
        report
            .issues
            .push("Evidence output limit reached while assembling input fragments".into());
        parts.clear();
        return;
    }
    seen.extend(new);
    decode_parts(fetch, parts, report).await;
    parts.clear();
}

async fn decode_parts(
    fetch: &mut Fetcher,
    group: &[(OutpointRef, Value, String)],
    report: &mut Report,
) {
    let result = async {
        ensure!(!fetch.cancelled, "Investigation cancelled");
        let values: Vec<_> = group
            .iter()
            .map(|(p, tx, _)| tx["vout"][p.vout as usize]["scriptPubKey"]["notaryevidence"].clone())
            .collect();
        let bytes = assemble(&values)?;
        let (first, tx, _) = group.first().context("Empty multipart group")?;
        let hex = tx["vout"][first.vout as usize]["scriptPubKey"]["hex"]
            .as_str()
            .context("Original evidence script absent")?;
        let script = decode_script(hex, &bytes)?;
        let params = json!([script]);
        ensure!(fetch.requests < 128, "Investigation RPC limit reached");
        fetch.requests += 1;
        let output = fetch
            .client
            .call_with_raw::<Value>("decodescript", params.as_array().unwrap())
            .await?;
        let evidence = output
            .result
            .get("notaryevidence")
            .context("Daemon did not decode the assembled evidence")?;
        let origins = group
            .iter()
            .map(|(output, _, source)| Origin {
                output: output.clone(),
                source: source.clone(),
            })
            .collect::<Vec<_>>();
        collect_signatures(&evidence["evidence"], &origins, report, 0);
        Ok::<_, anyhow::Error>(())
    }
    .await;
    match result {
        Ok(()) => report.multipart_sets += 1,
        Err(e) => {
            if e.to_string().to_lowercase().contains("cancel") {
                fetch.cancelled = true;
            }
            report
                .issues
                .push(format!("Multipart evidence incomplete: {e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn system() -> SystemId {
        SystemId::try_from("i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV".to_owned()).unwrap()
    }
    #[test]
    fn real_witness_preserves_signing_height_target_and_provenance_without_counting_twice() {
        let tx: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/cross-chain/vrsc-witness.json"
        ))
        .unwrap();
        let origin = Origin {
            output: OutpointRef {
                system_id: system(),
                txid: tx["txid"].as_str().unwrap().into(),
                vout: 1,
            },
            source: "local VRSC".into(),
        };
        let evidence = &tx["vout"][1]["scriptPubKey"]["notaryevidence"]["evidence"];
        let mut report = Report::default();
        collect_signatures(evidence, std::slice::from_ref(&origin), &mut report, 0);
        collect_signatures(evidence, std::slice::from_ref(&origin), &mut report, 0);
        assert_eq!(report.signatures.len(), 1);
        let s = &report.signatures[0];
        assert_eq!(s.identity, "iByTJrnrwStZyV2qnLg5jipp12rnAPfCZZ");
        assert_eq!(s.height, Some(3083106));
        assert!(s.confirmed);
        assert_eq!(
            s.target.txid,
            "99cb138ed3e09a835acbb6673a6320ef41fc6b9563a94c8964ac295bd2ad513d"
        );
        assert_eq!(s.key_signatures.len(), 1);
        assert_eq!(s.origins.len(), 1);
        let mut changed = evidence.clone();
        changed["chainobjects"][1]["value"]["confirmed"] = json!(false);
        collect_signatures(&changed, std::slice::from_ref(&origin), &mut report, 0);
        changed["chainobjects"][1]["value"]["signatures"]["iByTJrnrwStZyV2qnLg5jipp12rnAPfCZZ"]["blockheight"] =
            json!(3083107);
        collect_signatures(&changed, &[origin], &mut report, 0);
        assert_eq!(report.signatures.len(), 3);
    }
    #[test]
    fn multipart_reassembles_and_preserves_condition_script_structure() {
        let outputs: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/cross-chain/vrsc-multipart.json"
        ))
        .unwrap();
        let parts: Vec<_> = outputs
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o["scriptPubKey"]["notaryevidence"].clone())
            .collect();
        let bytes = assemble(&parts).unwrap();
        assert_eq!(&bytes[..2], &[1, 1]);
        let decoded =
            decode_script(outputs[0]["scriptPubKey"]["hex"].as_str().unwrap(), &bytes).unwrap();
        assert!(decoded.ends_with("75"));
        assert!(decoded.len() > bytes.len() * 2);
        assert!(assemble(&parts[1..]).is_err());
        assert!(assemble(&parts[..1]).is_err());
        let mut reversed = parts.clone();
        reversed.reverse();
        assert!(assemble(&reversed).is_err());
    }
    #[test]
    fn malformed_fragments_and_scripts_fail_closed() {
        for hex in ["", "ff", "010102", "gg"] {
            let part = json!({"type":2,"evidence":{"chainobjects":[{"value":{"hex":hex}}]}});
            assert!(assemble(&[part]).is_err());
            assert!(decode_script(hex, &[1, 1]).is_err());
        }
    }
}
