//! Identities and addresses.
//!
//! Part of the bridge's Swift-facing API. See `ffi.rs` for the shared types and helpers.
//!
//! The terminal keeps this logic in `src/app.rs` and in `src/app/identity_evidence.rs`,
//! `src/app/registration_payments.rs` and `src/app/token_control.rs`, where it is tied to the
//! terminal's own state. The pure parts are ported here. Each section names its source.
//! Evidence links are stored with Carbide's own record, `crate::cache::identity`.

use super::*;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::cache::identity::{EvidenceEdge, EvidenceRecord, MAX_EDGES};
use crate::defi::{address_payload, amount_sats, generated_region, issuer_instruction};
use crate::witnesses::condition_data;

// --------------------------------------------------------------- records

/// The three meanings of an identity's timelock field.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Timelock {
    Unlocked,
    /// Locked. An unlock request takes effect after this many blocks.
    Delay {
        blocks: u64,
    },
    /// Locked until this block height.
    UnlocksAt {
        height: u64,
    },
}

/// One version of an identity, as it stood after the update at `height`.
#[derive(Debug, Clone, uniffi::Record)]
pub struct IdentityState {
    pub height: u64,
    pub time: Option<u64>,
    pub block_hash: Option<String>,
    /// False when the block of this update is not on the active chain. `None` when not checked.
    pub on_active_chain: Option<bool>,
    pub txid: Option<String>,
    pub output: Option<u32>,
    pub name: String,
    pub identity_address: String,
    /// The node's word: "active" or "revoked".
    pub status: String,
    pub format_version: Option<u64>,
    pub flags: u64,
    pub defines_currency: bool,
    pub tokenized_control: bool,
    pub revoked: bool,
    pub timelock: Timelock,
    pub timelock_value: u64,
    /// Carbide's own words for the timelock.
    pub timelock_label: String,
    pub parent: Option<String>,
    pub system_id: Option<String>,
    pub primary_addresses: Vec<String>,
    pub minimum_signatures: Option<u64>,
    pub revocation_authority: Option<String>,
    pub recovery_authority: Option<String>,
    pub revocation_is_self: bool,
    pub recovery_is_self: bool,
    pub private_address: Option<String>,
    /// What changed since the version before.
    pub changes: Vec<String>,
    pub json: String,
}

/// Whether a block is on the active chain now.
#[derive(Debug, Clone, uniffi::Record)]
pub struct BlockActivity {
    pub hash: String,
    /// `None` when no source answered.
    pub active: Option<bool>,
}

/// Every version of an identity that the source reports.
#[derive(Debug, Clone, uniffi::Record)]
pub struct IdentityTimeline {
    pub identity_address: String,
    pub name: String,
    pub fully_qualified_name: Option<String>,
    /// Oldest first. The last entry is the current version.
    pub versions: Vec<IdentityState>,
    pub history_complete: bool,
    pub history_note: Option<String>,
    /// True when the block of the current version is no longer on the active chain.
    pub stale: bool,
    pub tip_height: Option<u64>,
    pub source: DataSource,
    /// Where the current version came from, and whether it is a stored observation.
    pub provenance: Provenance,
}

/// Open marketplace offers that involve an identity. Expired offers are left out.
#[derive(Debug, Clone, uniffi::Record)]
pub struct IdentityOffers {
    /// Someone offers to buy this identity.
    pub buy: bool,
    /// This identity is offered for sale.
    pub sell: bool,
    pub open: u32,
    pub expired: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum EvidenceTargetKind {
    Identity,
    Address,
    Transaction,
}

/// One association from Carbide's registration and association evidence.
#[derive(Debug, Clone, uniffi::Record)]
pub struct EvidenceLinkRow {
    /// Carbide's label, for example "Registration funding candidate".
    pub label: String,
    pub detail: String,
    pub kind: EvidenceTargetKind,
    pub target: String,
    /// The outpoint or request that supports the link.
    pub source: String,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct IdentityEvidenceReport {
    /// For example "alice@  tx 1a2b3c...d4e5f6:2", or "Imported identity at output 3".
    pub registration: Option<String>,
    pub imported: bool,
    pub links: Vec<EvidenceLinkRow>,
    pub notes: Vec<String>,
    /// The identity version the evidence belongs to.
    pub version_txid: Option<String>,
    pub version_height: u64,
    /// The source and block that anchor the stored evidence.
    pub source_id: Option<String>,
    pub block_hash: Option<String>,
    /// True when the live refresh failed and the links come only from stored evidence.
    pub retained_only: bool,
    /// True when the evidence was stored with its version and source block.
    pub saved: bool,
    pub history_complete: bool,
}

/// The result of a historical reverse request over one window of blocks.
#[derive(Debug, Clone, uniffi::Record)]
pub struct HistoricalEvidence {
    pub report: IdentityEvidenceReport,
    pub from_height: u64,
    pub to_height: u64,
    /// Requests that the source did not answer.
    pub unavailable: u32,
    pub requests: u32,
    /// True when the source said that it has no identity index.
    pub index_missing: bool,
    /// Historical links found in this window.
    pub found: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum PaymentsStatus {
    /// The transaction does not register an identity.
    NotRegistration,
    Unverified {
        reason: String,
    },
    Verified,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PaymentRow {
    pub output: u32,
    pub recipient: String,
    pub currency: String,
    pub amount_sats: i64,
    /// "Direct Referral Payment", "Ancestor Referral Payment" or "Issuer Charge".
    pub annotation: String,
}

/// Registration payments, checked against the issuer's rules as Carbide checks them.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RegistrationPayments {
    pub status: PaymentsStatus,
    pub issuer: Option<String>,
    pub native_currency: String,
    pub referrals: Vec<PaymentRow>,
    pub referral_total_sats: i64,
    pub charge: Option<PaymentRow>,
    pub budget_sats: i64,
    pub fee_sats: i64,
    /// What was paid in the issuer's currency (referrals and charge), when there is a charge.
    pub total_issuer_sats: Option<i64>,
    pub total_native_sats: i64,
    /// Carbide's one-line summary.
    pub summary: String,
}

/// Where a control-token trail stopped, so a later request can continue from it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TokenFrontier {
    pub source: String,
    pub txid: String,
    pub output: u64,
    /// Outpoints already followed, as "txid:n".
    pub visited: Vec<String>,
    pub hops: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum TokenControlState {
    /// The hop limit for one pass was reached. Continue from the frontier.
    Paused,
    /// The trail stopped. The reason is Carbide's.
    Unresolved { reason: String },
    /// The token is in an unspent output. `holder` is `None` when that output has no single
    /// holder address.
    Unspent { holder: Option<String> },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TokenControl {
    pub state: TokenControlState,
    pub frontier: Option<TokenFrontier>,
    pub hops: u32,
    pub hops_per_pass: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AddressBalance {
    pub currency_id: String,
    pub name: String,
    pub amount_sats: i64,
    pub native: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AddressOverview {
    pub address: String,
    pub balances: Vec<AddressBalance>,
    pub balance_error: Option<String>,
    pub transaction_count: u64,
    pub first_txid: Option<String>,
    pub latest_txid: Option<String>,
    pub first_time: Option<u64>,
    pub latest_time: Option<u64>,
    pub history_error: Option<String>,
    pub json: String,
    /// Where the balances came from, and whether they are a stored observation.
    pub provenance: Provenance,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PrimaryAddressIdentities {
    pub identities: Vec<NameEntry>,
    pub source: DataSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FlowStatus {
    Unspent,
    Spent,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct FlowAmount {
    pub currency: String,
    pub sats: i64,
}

/// One input or output of a transaction on an address page.
#[derive(Debug, Clone, uniffi::Record)]
pub struct AddressFlow {
    pub index: u32,
    pub addresses: Vec<String>,
    pub amounts: Vec<FlowAmount>,
    pub coinbase: bool,
    pub private: bool,
    /// True when the viewed address is one of `addresses`.
    pub viewed: bool,
    pub status: FlowStatus,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AddressTile {
    pub txid: String,
    pub time: Option<u64>,
    pub height: Option<u64>,
    pub block_hash: Option<String>,
    pub confirmations: Option<i64>,
    pub tags: Vec<String>,
    pub inputs: Vec<AddressFlow>,
    pub outputs: Vec<AddressFlow>,
    /// Flows past Carbide's limit of six per column.
    pub more_inputs: u32,
    pub more_outputs: u32,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct AddressActivityPage {
    pub total: u64,
    pub offset: u64,
    pub tiles: Vec<AddressTile>,
    /// Transactions in this page that did not load.
    pub missing: Vec<String>,
}

// ------------------------------------------------------------- constants

/// Carbide's `ADDRESS_FLOW_LIMIT`.
const ADDRESS_FLOW_LIMIT: usize = 6;
const MAX_ACTIVITY_PAGE: u64 = 25;
const MAX_WALK_VERSIONS: usize = 256;
const MAX_VERSION_TIMES: usize = 60;
/// Earlier transactions fetched for one page of address tiles, to fill input amounts.
const MAX_PREVOUT_TRANSACTIONS: usize = 40;
const MAX_BLOCK_CHECKS: usize = 64;
const VRSC: &str = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
/// From `token_control.rs`.
const HOPS_PER_PASS: u32 = 16;
const RPC_TIMEOUT: Duration = Duration::from_secs(8);

// ------------------------------------------------------------- explorer

#[uniffi::export(async_runtime = "tokio")]
impl Explorer {
    /// Every version of an identity, each complete as it stood, oldest first.
    pub async fn identity_timeline(&self, name: String) -> Outcome<IdentityTimeline> {
        let loaded = self.identity_versions(name.trim(), true).await?;
        let tip = self.tip_height().await.ok();
        let mut versions = Vec::with_capacity(loaded.versions.len());
        let mut previous: Option<&Value> = None;
        for version in &loaded.versions {
            if let Some(state) = identity_state(&version.raw, previous, tip, &loaded.name) {
                versions.push(state);
            }
            previous = Some(&version.raw["identity"]);
        }
        // Block times and whether each block is still on the active chain.
        let mut tasks = tokio::task::JoinSet::new();
        let skip = versions.len().saturating_sub(MAX_VERSION_TIMES);
        for (index, version) in versions.iter().enumerate().skip(skip) {
            let selector = match &version.block_hash {
                Some(hash) => Value::from(hash.clone()),
                None => Value::from(version.height.to_string()),
            };
            let client = self.client.clone();
            tasks.spawn(async move {
                (
                    index,
                    client.call_with_raw::<Value>("getblock", &[selector]).await,
                )
            });
        }
        while let Some(Ok((index, block))) = tasks.join_next().await {
            if let Ok(block) = self.settle(block) {
                let block = block.result;
                versions[index].time = block["time"].as_u64();
                if versions[index].block_hash.is_none() {
                    versions[index].block_hash = text(&block["hash"]);
                }
                versions[index].on_active_chain =
                    block["confirmations"].as_i64().map(|count| count >= 0);
            }
        }
        let stale = versions
            .last()
            .is_some_and(|latest| latest.on_active_chain == Some(false));
        Ok(IdentityTimeline {
            identity_address: loaded.identity_address,
            name: loaded.name,
            fully_qualified_name: loaded.fully_qualified_name,
            versions,
            history_complete: loaded.complete,
            history_note: loaded.note,
            stale,
            tip_height: tip,
            source: source_of(&loaded.source),
            provenance: loaded.provenance,
        })
    }

    /// Whether each block is on the active chain now. A page calls this when the tip changes,
    /// to find a version or a transaction that a reorganization removed.
    pub async fn blocks_active(&self, hashes: Vec<String>) -> Vec<BlockActivity> {
        let mut tasks = tokio::task::JoinSet::new();
        for (index, hash) in hashes.iter().take(MAX_BLOCK_CHECKS).enumerate() {
            let client = self.client.clone();
            let hash = hash.clone();
            tasks.spawn(async move {
                let block = client
                    .call_with_raw::<Value>("getblock", &[Value::from(hash)])
                    .await;
                (index, block)
            });
        }
        let mut found: Vec<BlockActivity> = hashes
            .iter()
            .take(MAX_BLOCK_CHECKS)
            .map(|hash| BlockActivity {
                hash: hash.clone(),
                active: None,
            })
            .collect();
        while let Some(Ok((index, block))) = tasks.join_next().await {
            if let Ok(block) = self.settle(block) {
                found[index].active = block_active(&block.result);
            }
        }
        found
    }

    /// Whether there are open offers to buy or sell this identity.
    pub async fn identity_offers(&self, identity_address: String) -> Outcome<IdentityOffers> {
        let id = identity_address.trim();
        let output = self.settle(self.client.get_offers(id, false).await)?;
        let tip = self.tip_height().await.ok();
        Ok(offer_badges(&output.result, id, tip))
    }

    /// Carbide's registration and association evidence for the current version.
    pub async fn identity_evidence(&self, name: String) -> Outcome<IdentityEvidenceReport> {
        let loaded = self.identity_versions(name.trim(), false).await?;
        Ok(self.current_evidence(&loaded).await?.1)
    }

    /// Carbide's `v`: identities that shared a known authority or primary address with this
    /// identity during the 1,000 blocks ending at `end_height`. Needs the identity index.
    pub async fn identity_historical_evidence(
        &self,
        name: String,
        end_height: u64,
    ) -> Outcome<HistoricalEvidence> {
        let loaded = self.identity_versions(name.trim(), false).await?;
        let (mut report, _) = self.current_evidence(&loaded).await?;
        let to_height = end_height;
        let from_height = end_height.saturating_sub(REVERSE_WINDOW - 1);
        if report.links.len() >= MAX_EDGES {
            return Err(fail("The identity evidence link cap is reached."));
        }
        let before = report.links.len();
        let rpc = Open {
            explorer: self,
            client: self.client.clone().probe_identity_index(),
        };
        let mut newest_first = loaded.versions.clone();
        newest_first.reverse();
        let outcome =
            expand_historical(&rpc, &mut report, &newest_first, from_height, to_height).await;
        let found = report.links.len().saturating_sub(before) as u32;
        let saved = self.save_evidence(&loaded, &report).await;
        remember_evidence(&self.evidence_key(&loaded.identity_address), &report);
        let mut row = evidence_row(&report, &loaded, false, saved);
        if outcome.index_missing {
            row.notes.push(
                "The source has no identity index. It cannot answer historical reverse requests."
                    .into(),
            );
        }
        Ok(HistoricalEvidence {
            report: row,
            from_height,
            to_height,
            unavailable: outcome.failures,
            requests: outcome.requests,
            index_missing: outcome.index_missing,
            found,
        })
    }

    /// The payments in an identity registration, verified against the issuer's rules.
    pub async fn registration_payments(&self, txid: String) -> Outcome<RegistrationPayments> {
        let txid = txid.trim().to_string();
        let output = self.settle(self.client.get_raw_transaction(&txid).await)?;
        let tx = output.result;
        let native = self.config.chain.name.clone();
        if !payment_candidate(&tx) {
            return Ok(unverified_payments(PaymentsStatus::NotRegistration, native));
        }
        let unverified = |reason: &str| {
            unverified_payments(
                PaymentsStatus::Unverified {
                    reason: reason.into(),
                },
                self.config.chain.name.clone(),
            )
        };
        if self.config.chain.testnet
            || self
                .config
                .chain
                .system_id
                .get()
                .is_none_or(|id| id.as_ref() != VRSC)
        {
            return Ok(unverified(
                "Carbide checks registration payments only on VRSC mainnet.",
            ));
        }
        let (Some(block), Some(height)) = (text(&tx["blockhash"]), tx["height"].as_u64()) else {
            return Ok(unverified("The transaction is not in a block yet."));
        };
        if !(1_053_660..1_796_400).contains(&height) && !(2_549_420..4_141_448).contains(&height) {
            return Ok(unverified(
                "Carbide has no fee rules for registrations at this height.",
            ));
        }
        let rpc = Pinned {
            client: self.client.clone(),
            source: output.source_id,
        };
        let report = tokio::time::timeout(
            Duration::from_secs(12),
            verify_payments(&rpc, &txid, &block, height),
        )
        .await
        .ok()
        .flatten();
        Ok(match report {
            Some(report) => verified_payments(&report, native),
            None => unverified(
                "The payments do not match the issuer's rules, or the evidence is not available.",
            ),
        })
    }

    /// Follows an identity's control token. Pass the frontier of an earlier result to continue.
    pub async fn token_control(
        &self,
        identity_address: String,
        resume: Option<TokenFrontier>,
    ) -> Outcome<TokenControl> {
        let id = identity_address.trim().to_string();
        let frontier = match resume {
            Some(frontier) => Frontier::from_row(&frontier),
            None => {
                let response = tokio::time::timeout(
                    RPC_TIMEOUT,
                    self.client
                        .call_with_raw::<Value>("getcurrency", &[json!(id)]),
                )
                .await
                .ok()
                .map(|result| self.settle(result));
                let Some(Ok(response)) = response else {
                    return Ok(token_result(Status::Unresolved(
                        None,
                        "Currency definition unavailable",
                    )));
                };
                // Keep the definition, spend links and UTXO check on the same source.
                let rpc = Pinned {
                    client: self.client.clone(),
                    source: response.source_id.clone(),
                };
                match token_origin(&rpc, &id, &response.source_id, &response.result).await {
                    Ok(frontier) => frontier,
                    Err(reason) => return Ok(token_result(Status::Unresolved(None, reason))),
                }
            }
        };
        let rpc = Pinned {
            client: self.client.clone(),
            source: frontier.source.clone(),
        };
        Ok(token_result(follow_token(&rpc, &id, frontier).await))
    }

    /// The address header: balances, transaction count, and first and latest times.
    pub async fn address_overview(&self, address: String) -> Outcome<AddressOverview> {
        let address = address.trim().to_string();
        // The main request of the page: it can ask for an API, and a stored observation
        // answers when no source does.
        let answer = self
            .observed(
                "getaddressbalance",
                &[json!({"addresses": [&address], "friendlynames": true})],
                self.primary.get_address_balance(&address).await,
            )
            .await?;
        let provenance = answer.provenance;
        let balance = Some(answer.result);
        let balance_error: Option<String> = None;
        let (txids, history_error) = match self.address_txids(&address).await {
            Ok(txids) => (txids, None),
            Err(error) => (Vec::new(), Some(readable(&error))),
        };
        let native_id = self.config.chain.system_id.get().map(|id| id.to_string());
        let balances = balance
            .as_ref()
            .map(|response| {
                balance_amounts(response, native_id.as_deref(), &self.config.chain.name)
            })
            .unwrap_or_default();
        let first_txid = txids.first().cloned();
        let latest_txid = txids.last().cloned();
        let (first_time, latest_time) = tokio::join!(
            self.transaction_time(first_txid.as_deref()),
            self.transaction_time(latest_txid.as_deref())
        );
        Ok(AddressOverview {
            balances,
            balance_error,
            transaction_count: txids.len() as u64,
            first_txid,
            latest_txid,
            first_time,
            latest_time,
            history_error,
            json: balance.as_ref().map(pretty).unwrap_or_default(),
            provenance,
            address,
        })
    }

    /// Identities that have this address as a primary address now. Needs the identity index.
    pub async fn identities_with_primary_address(
        &self,
        address: String,
    ) -> Outcome<PrimaryAddressIdentities> {
        let address = address.trim().to_string();
        let client = self.client.clone().probe_identity_index();
        let output = self
            .settle(client.get_identities_with_address(&address).await)
            .map_err(|error| fail(&index_message(&readable(&error))))?;
        Ok(PrimaryAddressIdentities {
            identities: primary_address_identities(&address, output.result)
                .into_iter()
                .map(|(id, name)| NameEntry { id, name })
                .collect(),
            source: source_of(&output.source_id),
        })
    }

    /// One page of an address's transactions, newest first, as input and output flows.
    pub async fn address_activity(
        &self,
        address: String,
        offset: u64,
        count: u64,
    ) -> Outcome<AddressActivityPage> {
        let address = address.trim().to_string();
        let txids = self.address_txids(&address).await?;
        let total = txids.len() as u64;
        let count = count.clamp(1, MAX_ACTIVITY_PAGE);
        let page: Vec<String> = txids
            .iter()
            .rev()
            .skip(offset as usize)
            .take(count as usize)
            .cloned()
            .collect();
        let unspent: HashSet<(String, u64)> =
            match self.settle(self.client.get_address_utxos(&address).await) {
                Ok(output) => output
                    .result
                    .iter()
                    .filter_map(|utxo| {
                        Some((
                            text(&utxo["txid"])?,
                            utxo["outputIndex"].as_u64().or(utxo["vout"].as_u64())?,
                        ))
                    })
                    .collect(),
                Err(_) => HashSet::new(),
            };
        let mut tasks = tokio::task::JoinSet::new();
        for (index, txid) in page.iter().enumerate() {
            let client = self.client.clone();
            let txid = txid.clone();
            tasks.spawn(async move { (index, client.get_raw_transaction(&txid).await) });
        }
        let mut loaded: Vec<Option<Value>> = vec![None; page.len()];
        while let Some(Ok((index, result))) = tasks.join_next().await {
            loaded[index] = match self.settle(result) {
                Ok(output) => Some(output.result),
                Err(_) => self
                    .retained::<Value>("getrawtransaction", &[json!(page[index]), json!(1)], None)
                    .await
                    .map(|answer| answer.result),
            };
        }
        // The earlier outputs that these inputs spend, for their addresses and every
        // currency amount, as the terminal shows them once it has loaded them.
        let wanted = prevout_txids(loaded.iter().flatten(), MAX_PREVOUT_TRANSACTIONS);
        let mut tasks = tokio::task::JoinSet::new();
        for txid in wanted {
            let client = self.client.clone();
            tasks.spawn(async move { (txid.clone(), client.get_raw_transaction(&txid).await) });
        }
        let mut prevouts: HashMap<String, Value> = HashMap::new();
        while let Some(Ok((txid, result))) = tasks.join_next().await {
            if let Ok(output) = self.settle(result)
                && output.result["txid"].as_str() == Some(txid.as_str())
            {
                prevouts.insert(txid, output.result);
            }
        }
        let mut tiles = Vec::new();
        let mut missing = Vec::new();
        for (txid, raw) in page.iter().zip(loaded) {
            match raw {
                Some(raw) => tiles.push(address_tile(
                    &raw,
                    &address,
                    &self.config.chain.name,
                    &unspent,
                    &prevouts,
                )),
                None => missing.push(txid.clone()),
            }
        }
        Ok(AddressActivityPage {
            total,
            offset,
            tiles,
            missing,
        })
    }
}

// ------------------------------------------------- loading identity versions

/// One version as a `getidentity`-shaped value: `identity`, `txid`, `vout`, `blockheight`,
/// `status`, and `blockhash` when known.
#[derive(Debug, Clone)]
struct VersionRecord {
    identity_address: String,
    block_height: u64,
    raw: Value,
}

struct LoadedVersions {
    identity_address: String,
    name: String,
    fully_qualified_name: Option<String>,
    /// Oldest first.
    versions: Vec<VersionRecord>,
    complete: bool,
    note: Option<String>,
    source: String,
    provenance: Provenance,
}

impl Explorer {
    /// `primary` is true for the identity page's own request, which can ask for an API.
    async fn identity_versions(&self, name: &str, primary: bool) -> Result<LoadedVersions> {
        let client = if primary { &self.primary } else { &self.client };
        let current = self
            .observed(
                "getidentity",
                &[Value::from(name)],
                client.get_identity(name, None).await,
            )
            .await?;
        let source = current.source_id.clone();
        let provenance = current.provenance;
        let raw = current.result;
        let identity_address = text(&raw["identity"]["identityaddress"])
            .context("getidentity result omitted identityaddress")?;
        let display = text(&raw["friendlyname"])
            .or_else(|| text(&raw["fullyqualifiedname"]))
            .or_else(|| text(&raw["identity"]["name"]).map(|name| format!("{name}@")))
            .unwrap_or_else(|| name.to_string());
        let display = self.short_name(&display);
        let fully_qualified_name = text(&raw["fullyqualifiedname"]);

        let history = self
            .observed(
                "getidentityhistory",
                &[Value::from(identity_address.as_str())],
                self.client.get_identity_history(&identity_address).await,
            )
            .await;
        let (mut versions, mut complete, mut note) = match history {
            Ok(history) => {
                let versions = versions_from_history(&history.result, &identity_address);
                (versions, true, None)
            }
            Err(_) => (Vec::new(), false, None),
        };
        if versions.is_empty() && !provenance.retained {
            // Carbide's own walk: each version, then the version before its height.
            let (walked, walk_complete) = self.walk_versions(&identity_address, &raw).await;
            versions = walked;
            complete = walk_complete;
            if !complete {
                note = Some("The source did not return every earlier version.".into());
            }
        }
        // The current version carries the node's status.
        let current_height = raw["blockheight"].as_u64();
        match versions.last_mut() {
            Some(last) if Some(last.block_height) == current_height => {
                last.raw["status"] = raw["status"].clone();
            }
            _ => {
                if let Some(height) = current_height {
                    versions.push(VersionRecord {
                        identity_address: identity_address.clone(),
                        block_height: height,
                        raw: raw.clone(),
                    });
                }
            }
        }
        Ok(LoadedVersions {
            identity_address,
            name: display,
            fully_qualified_name,
            versions,
            complete,
            note,
            source,
            provenance,
        })
    }

    /// An address's transaction IDs, oldest first, or the stored list when no source answers.
    async fn address_txids(&self, address: &str) -> Result<Vec<String>> {
        Ok(self
            .observed(
                "getaddresstxids",
                &[json!({"addresses": [address]})],
                self.client.get_address_txids(address).await,
            )
            .await?
            .result)
    }

    /// Port of the loop in `load_identity_detail` (`src/app.rs`).
    async fn walk_versions(&self, identity: &str, current: &Value) -> (Vec<VersionRecord>, bool) {
        let mut newest_first = vec![current.clone()];
        let mut seen = HashSet::new();
        let mut complete = false;
        let mut height = current["blockheight"].as_u64();
        if let Some(height) = height {
            seen.insert(height);
        }
        while newest_first.len() < MAX_WALK_VERSIONS {
            let Some(previous) = height.and_then(|height| height.checked_sub(1)) else {
                complete = true;
                break;
            };
            match self.client.get_identity(identity, Some(previous)).await {
                Ok(output) => {
                    let Some(found) = output.result["blockheight"].as_u64() else {
                        break;
                    };
                    if !seen.insert(found) {
                        complete = true;
                        break;
                    }
                    height = Some(found);
                    newest_first.push(output.result);
                }
                Err(error) => {
                    complete = crate::rpc::identity_not_found(&error);
                    break;
                }
            }
        }
        let versions = newest_first
            .into_iter()
            .rev()
            .filter_map(|raw| {
                Some(VersionRecord {
                    identity_address: identity.to_string(),
                    block_height: raw["blockheight"].as_u64()?,
                    raw,
                })
            })
            .collect();
        (versions, complete)
    }

    async fn transaction_time(&self, txid: Option<&str>) -> Option<u64> {
        let tx = self
            .settle(self.client.get_raw_transaction(txid?).await)
            .ok()?
            .result;
        tx["blocktime"].as_u64().or_else(|| tx["time"].as_u64())
    }

    fn evidence_system(&self) -> String {
        self.config
            .chain
            .system_id
            .get()
            .map(ToString::to_string)
            .unwrap_or_else(|| self.config.chain.name.clone())
    }

    fn evidence_key(&self, identity: &str) -> String {
        format!("{}|{identity}", self.evidence_system())
    }

    /// Loads the evidence for the current version: stored evidence first, then a live
    /// refresh. Historical links found earlier are kept, as in Carbide.
    async fn current_evidence(
        &self,
        loaded: &LoadedVersions,
    ) -> Result<(Evidence, IdentityEvidenceReport)> {
        let current = loaded
            .versions
            .last()
            .context("identity version unavailable")?;
        let version_txid = text(&current.raw["txid"]).unwrap_or_default();
        let key = self.evidence_key(&loaded.identity_address);
        let retained = self
            .retained_evidence(loaded, &version_txid, current.block_height)
            .await;
        let mut newest_first = loaded.versions.clone();
        newest_first.reverse();
        let rpc = Open {
            explorer: self,
            client: self.client.clone(),
        };
        let mut report = load_evidence(&rpc, &newest_first, &self.evidence_system()).await;
        if !loaded.complete {
            report
                .notes
                .push("Registration evidence awaits complete identity history".into());
        }
        let live_failed = report.anchor.is_none() && report.links.is_empty();
        if let Some(retained) = &retained {
            if live_failed {
                report = retained.clone();
            } else {
                merge_historical(&mut report, retained);
                if report.anchor.is_none() {
                    report.anchor = retained.anchor.clone();
                }
            }
        }
        // Compare funding and commitment destinations with identities seen this session.
        connect_with_session(&key, &mut report);
        let saved = self.save_evidence(loaded, &report).await;
        remember_evidence(&key, &report);
        let row = evidence_row(&report, loaded, live_failed && retained.is_some(), saved);
        Ok((report, row))
    }

    async fn retained_evidence(
        &self,
        loaded: &LoadedVersions,
        version_txid: &str,
        version_height: u64,
    ) -> Option<Evidence> {
        let cache = self.evidence_cache()?;
        let record = cache
            .identity_evidence(
                self.evidence_system(),
                loaded.identity_address.clone(),
                version_txid.to_string(),
                version_height,
            )
            .await
            .ok()??;
        // An anchor block that left the active chain invalidates the stored links.
        if let Some(hash) = &record.block_hash
            && let Ok(block) = self.settle(
                self.client
                    .call_with_raw::<Value>("getblock", &[Value::from(hash.clone())])
                    .await,
            )
            && block.result["confirmations"]
                .as_i64()
                .is_some_and(|n| n < 0)
        {
            return None;
        }
        Some(from_record(record))
    }

    async fn save_evidence(&self, loaded: &LoadedVersions, report: &Evidence) -> bool {
        let Some(current) = loaded.versions.last() else {
            return false;
        };
        let Some(version_txid) = text(&current.raw["txid"]) else {
            return false;
        };
        let Some(record) = to_record(
            &self.evidence_system(),
            &loaded.identity_address,
            &version_txid,
            current.block_height,
            report,
        ) else {
            return false;
        };
        let Some(cache) = self.evidence_cache() else {
            return false;
        };
        cache.record_identity_evidence(record).await.is_ok()
    }

    fn evidence_cache(&self) -> Option<crate::cache::Cache> {
        let cache = crate::cache::Cache::open(&self.config.cache_path).ok()?;
        cache
            .bind_network(Network::from_testnet(self.config.chain.testnet))
            .ok()?;
        Some(cache)
    }
}

/// Turns `getidentityhistory` entries into `getidentity`-shaped versions, oldest first.
fn versions_from_history(history: &Value, identity_address: &str) -> Vec<VersionRecord> {
    let mut versions: Vec<VersionRecord> = history["history"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let height = entry["height"].as_u64()?;
            let identity = entry.get("identity")?.clone();
            let flags = identity["flags"].as_u64().unwrap_or(0);
            let mut raw = json!({
                "identity": identity,
                "blockheight": height,
                // CIdentity::FLAG_REVOKED
                "status": if flags & 0x8000 != 0 { "revoked" } else { "active" },
            });
            if let Some(txid) = text(&entry["output"]["txid"]) {
                raw["txid"] = txid.into();
            }
            if let Some(vout) = entry["output"]["voutnum"].as_u64() {
                raw["vout"] = vout.into();
            }
            if let Some(hash) = text(&entry["blockhash"]) {
                raw["blockhash"] = hash.into();
            }
            Some(VersionRecord {
                identity_address: identity_address.to_string(),
                block_height: height,
                raw,
            })
        })
        .collect();
    versions.sort_by_key(|version| version.block_height);
    versions.dedup_by_key(|version| version.block_height);
    versions
}

/// The whole identity at one version, from a `getidentity`-shaped value.
fn identity_state(
    raw: &Value,
    previous: Option<&Value>,
    tip: Option<u64>,
    name: &str,
) -> Option<IdentityState> {
    let identity = raw.get("identity")?;
    let identity_address = text(&identity["identityaddress"])?;
    let flags = identity["flags"].as_u64().unwrap_or(0);
    let (timelock, timelock_label) = timelock_meaning(identity, tip);
    let revocation = text(&identity["revocationauthority"]);
    let recovery = text(&identity["recoveryauthority"]);
    Some(IdentityState {
        height: raw["blockheight"].as_u64()?,
        time: None,
        block_hash: text(&raw["blockhash"]),
        on_active_chain: None,
        txid: text(&raw["txid"]).filter(|txid| !txid.chars().all(|ch| ch == '0')),
        output: raw["vout"].as_u64().map(|n| n as u32),
        name: name.to_string(),
        status: text(&raw["status"]).unwrap_or_else(|| "unknown".into()),
        format_version: identity["version"].as_u64(),
        flags,
        // CIdentity flags: 1 active currency, 4 tokenized control, 0x8000 revoked.
        defines_currency: flags & 1 != 0,
        tokenized_control: flags & 4 != 0,
        revoked: flags & 0x8000 != 0,
        timelock,
        timelock_value: identity["timelock"].as_u64().unwrap_or(0),
        timelock_label,
        parent: text(&identity["parent"]),
        system_id: text(&identity["systemid"]),
        primary_addresses: strings(&identity["primaryaddresses"]),
        minimum_signatures: identity["minimumsignatures"].as_u64(),
        revocation_is_self: revocation.as_deref() == Some(identity_address.as_str()),
        recovery_is_self: recovery.as_deref() == Some(identity_address.as_str()),
        revocation_authority: revocation,
        recovery_authority: recovery,
        private_address: text(&identity["privateaddress"]),
        changes: identity_changes(previous, identity),
        json: pretty(raw),
        identity_address,
    })
}

/// Port of `identity_timelock_label` (`src/app.rs`).
fn timelock_meaning(identity: &Value, chain_height: Option<u64>) -> (Timelock, String) {
    const VERSION_VAULT: u64 = 2;
    const FLAG_LOCKED: u64 = 0x2;
    const FLAG_REVOKED: u64 = 0x8000;

    let version = identity["version"].as_u64().unwrap_or_default();
    let flags = identity["flags"].as_u64().unwrap_or_default();
    let unlock_after = identity["timelock"].as_u64().unwrap_or_default();

    if version < VERSION_VAULT || flags & FLAG_REVOKED != 0 {
        return (Timelock::Unlocked, "unlocked".into());
    }
    if flags & FLAG_LOCKED != 0 {
        let unit = if unlock_after == 1 { "block" } else { "blocks" };
        return (
            Timelock::Delay {
                blocks: unlock_after,
            },
            format!("locked ({unlock_after} {unit} unlock delay)"),
        );
    }
    if unlock_after > 0 && chain_height.is_none_or(|height| unlock_after >= height) {
        return (
            Timelock::UnlocksAt {
                height: unlock_after,
            },
            format!("locked (unlocks at height {unlock_after})"),
        );
    }
    (Timelock::Unlocked, "unlocked".into())
}

/// Port of `parse_marketplace_offers` and `identity_offer_labels` (`src/app.rs`).
fn offer_badges(result: &Value, identity: &str, tip: Option<u64>) -> IdentityOffers {
    fn visit<'a>(value: &'a Value, offers: &mut Vec<(&'a Value, &'a Value, u64, &'a str)>) {
        match value {
            Value::Array(values) => values.iter().for_each(|value| visit(value, offers)),
            Value::Object(fields) => {
                if let Some(details) = fields.get("offer").and_then(Value::as_object)
                    && let Some(txid) = details.get("txid").and_then(Value::as_str)
                    && let (Some(offered), Some(accepted)) =
                        (details.get("offer"), details.get("accept"))
                {
                    let expiry = details
                        .get("blockexpiry")
                        .and_then(Value::as_u64)
                        .unwrap_or_default();
                    offers.push((offered, accepted, expiry, txid));
                    return;
                }
                fields.values().for_each(|value| visit(value, offers));
            }
            _ => {}
        }
    }
    let mut offers = Vec::new();
    visit(result, &mut offers);
    let mut seen = HashSet::new();
    offers.retain(|offer| seen.insert(offer.3));
    let mut badges = IdentityOffers {
        buy: false,
        sell: false,
        open: 0,
        expired: 0,
    };
    for (offered, accepted, expiry, _) in offers {
        if tip.is_some_and(|height| expiry != 0 && expiry <= height) {
            badges.expired += 1;
            continue;
        }
        let sell = offered["identityid"].as_str() == Some(identity);
        let buy = accepted["identityid"].as_str() == Some(identity);
        badges.sell |= sell;
        badges.buy |= buy;
        if sell || buy {
            badges.open += 1;
        }
    }
    badges
}

// ---------------------------------------------- requests behind a seam
//
// The ported logic asks for data through `Rpc`, so the tests can answer from samples.

type Answer = std::result::Result<(Value, String), String>;

trait Rpc: Sync {
    /// The result and the source that answered, or the source's error message.
    fn call(&self, method: &str, params: Vec<Value>) -> impl Future<Output = Answer> + Send;
}

async fn get<R: Rpc>(rpc: &R, method: &str, params: Vec<Value>) -> Option<Value> {
    rpc.call(method, params).await.ok().map(|(value, _)| value)
}

/// Every request goes to one source, as Carbide's `call_on`.
struct Pinned {
    client: RpcClient,
    source: String,
}

impl Rpc for Pinned {
    fn call(&self, method: &str, params: Vec<Value>) -> impl Future<Output = Answer> + Send {
        let client = self.client.clone();
        let source = self.source.clone();
        let method = method.to_string();
        async move {
            match tokio::time::timeout(RPC_TIMEOUT, client.call_on(&source, &method, &params)).await
            {
                Ok(Ok(value)) => Ok((value, source)),
                Ok(Err(error)) => Err(readable(&error)),
                Err(_) => Err(format!("{method} did not answer in time")),
            }
        }
    }
}

/// Requests through the explorer's normal source order.
struct Open<'a> {
    explorer: &'a Explorer,
    client: RpcClient,
}

impl Rpc for Open<'_> {
    fn call(&self, method: &str, params: Vec<Value>) -> impl Future<Output = Answer> + Send {
        let method = method.to_string();
        async move {
            let result = self
                .client
                .call_nullable_with_raw::<Value>(&method, &params)
                .await;
            match self.explorer.settle(result) {
                Ok(output) => Ok((output.result.unwrap_or(Value::Null), output.source_id)),
                Err(error) => Err(readable(&error)),
            }
        }
    }
}

fn is_hash_like(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn short_id(value: &str) -> String {
    if value.len() <= 12 {
        value.to_string()
    } else {
        format!("{}...{}", &value[..6], &value[value.len() - 6..])
    }
}

/// A daemon's refusal of an identity-index request, in plain words.
fn index_message(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("idindex") || lower.contains("identity reverse index") {
        "The source has no identity index. Start the node with -idindex=1, or enable a public API \
         in Settings."
            .into()
    } else {
        error.to_string()
    }
}

// ---------------------------------------------------- identity evidence
//
// Ported from `src/app/identity_evidence.rs`.

const MAX_VERSION_TRANSACTIONS: usize = 8;
const MAX_PREVOUTS: usize = 24;
const MAX_COMMITMENT_PREVOUTS: usize = 8;
const MAX_REVERSE_ADDRESSES: usize = 4;
const MAX_REVERSE_RESULTS: usize = 12;
const MAX_REFERRAL_HOPS: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
enum EvidenceTarget {
    Identity(String),
    Address(String),
    Transaction(String),
}

#[derive(Debug, Clone)]
struct EvidenceLink {
    label: String,
    detail: String,
    target: EvidenceTarget,
    source: String,
}

#[derive(Debug, Clone, Default)]
struct Evidence {
    registration: Option<String>,
    links: Vec<EvidenceLink>,
    notes: Vec<String>,
    /// The source and block of the version transaction.
    anchor: Option<(String, Option<String>)>,
}

fn to_record(
    system: &str,
    identity: &str,
    version_txid: &str,
    version_height: u64,
    report: &Evidence,
) -> Option<EvidenceRecord> {
    let (source_id, block_hash) = report.anchor.clone()?;
    let edges = report
        .links
        .iter()
        .take(MAX_EDGES)
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

fn from_record(record: EvidenceRecord) -> Evidence {
    let anchor = Some((record.source_id.clone(), record.block_hash.clone()));
    let mut notes = record.notes;
    notes.push(format!(
        "Retained evidence from {} at {}; block membership {}.",
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
    Evidence {
        registration: record.registration,
        links,
        notes,
        anchor,
    }
}

fn merge_historical(target: &mut Evidence, retained: &Evidence) {
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
    strings(&script["addresses"])
}

fn transparent_funding_script(script: &Value) -> bool {
    matches!(
        script.get("type").and_then(Value::as_str),
        Some("pubkeyhash" | "pubkey" | "scripthash" | "multisig")
    )
}

fn add_link(
    report: &mut Evidence,
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

fn is_import_of(report: &Evidence, txid: &str) -> bool {
    report.links.iter().any(|link| {
        link.label == "Identity import transaction"
            && link.target == EvidenceTarget::Transaction(txid.into())
    })
}

/// Identify a locally created identity output in the import's generated region,
/// corroborated by the matching identity-export transfer in supplemental data.
fn add_identity_import(report: &mut Evidence, transaction: &Value, seed: &str) {
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
        let Some((start, generated)) = generated_region(transaction, import, import_index) else {
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
    report: &mut Evidence,
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
    report: &mut Evidence,
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
    report: &mut Evidence,
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

struct HistoricalOutcome {
    requests: u32,
    failures: u32,
    index_missing: bool,
}

/// Port of `expand_historical`. `versions` is newest first.
async fn expand_historical<R: Rpc>(
    rpc: &R,
    report: &mut Evidence,
    versions: &[VersionRecord],
    from_height: u64,
    to_height: u64,
) -> HistoricalOutcome {
    let mut outcome = HistoricalOutcome {
        requests: 0,
        failures: 0,
        index_missing: false,
    };
    let Some(seed) = versions.first().map(|v| v.identity_address.clone()) else {
        return outcome;
    };
    let failed = |outcome: &mut HistoricalOutcome, error: &str| {
        outcome.failures += 1;
        let lower = error.to_ascii_lowercase();
        if lower.contains("idindex") || lower.contains("identity reverse index") {
            outcome.index_missing = true;
        }
    };
    let mut queries = Vec::new();
    for (field, recovery) in [("recoveryauthority", true), ("revocationauthority", false)] {
        let mut values = BTreeSet::new();
        for version in versions {
            if let Some(value) = version.raw["identity"][field].as_str()
                && value.starts_with('i')
            {
                values.insert(value.to_string());
            }
        }
        for authority in values.into_iter().take(MAX_REVERSE_ADDRESSES) {
            queries.push((field, recovery, authority));
        }
    }
    for (field, recovery, authority) in queries {
        let method = if recovery {
            "getidentitieswithrecovery"
        } else {
            "getidentitieswithrevocation"
        };
        outcome.requests += 1;
        let params = vec![json!({
            "identityid": authority, "fromheight": from_height,
            "toheight": to_height, "unspent": false,
        })];
        match rpc.call(method, params).await {
            Ok((value, source_id)) => {
                let results = value.as_array().cloned().unwrap_or_default();
                let source = format!(
                    "{source_id} {field}={authority} fromheight={from_height} toheight={to_height} unspent=false"
                );
                let count = results.len();
                for candidate in results.into_iter().take(MAX_REVERSE_RESULTS) {
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
                    report.notes.push(format!(
                        "Historical {field}={authority}: first {MAX_REVERSE_RESULTS} of {count} results"
                    ));
                }
            }
            Err(error) => failed(&mut outcome, &error),
        }
    }
    let mut addresses = BTreeSet::new();
    for version in versions {
        for address in version.raw["identity"]["primaryaddresses"]
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
        outcome.requests += 1;
        let params = vec![json!({
            "address": address, "fromheight": from_height,
            "toheight": to_height, "unspent": false,
        })];
        match rpc.call("getidentitieswithaddress", params).await {
            Ok((value, source_id)) => {
                let results = value.as_array().cloned().unwrap_or_default();
                let source = format!(
                    "{source_id} getidentitieswithaddress address={address} fromheight={from_height} toheight={to_height} unspent=false"
                );
                let count = results.len();
                for candidate in results.into_iter().take(MAX_REVERSE_RESULTS) {
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
                    report.notes.push(format!(
                        "Historical address {address}: first {MAX_REVERSE_RESULTS} of {count} results"
                    ));
                }
            }
            Err(error) => failed(&mut outcome, &error),
        }
    }
    if report.links.len() > MAX_EDGES {
        report.links.truncate(MAX_EDGES);
        report
            .notes
            .push("Identity evidence link cap reached; further matches were omitted".into());
    }
    let failures = outcome.failures;
    report.notes.push(format!(
        "Historical reverse window {from_height}-{to_height}; {failures} unavailable queries"
    ));
    outcome
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

/// Port of `add_referral_chain`: at most four hops, and a repeated identity stops the walk.
async fn add_referral_chain<R: Rpc>(report: &mut Evidence, rpc: &R, first: Option<String>) {
    let mut next = first;
    let mut visited = HashSet::new();
    for depth in 1..=MAX_REFERRAL_HOPS {
        let Some(address) = next.take() else { return };
        if !visited.insert(address.clone()) {
            report
                .notes
                .push("Referral chain contains a cycle".to_string());
            return;
        }
        let Some(history) = get(rpc, "getidentityhistory", vec![json!(address)]).await else {
            report
                .notes
                .push(format!("Referral ancestry unavailable beyond {address}"));
            return;
        };
        let Some(first_version) = history
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
        let Some((transaction, _)) = evidence_transaction(rpc, txid).await else {
            return;
        };
        let Some((reservation, index)) = reservation(&transaction) else {
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
    }
    if next.is_some() {
        report
            .notes
            .push("Referral ancestry capped at four hops".to_string());
    }
}

/// Port of `connect_shared_evidence`.
fn connect_shared_evidence(
    left_address: &str,
    left: &mut Evidence,
    right_address: &str,
    right: &mut Evidence,
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
                let note =
                    "Shared evidence comparison capped; inspect source transactions for more";
                left.notes.push(note.to_string());
                right.notes.push(note.to_string());
                return;
            }
        }
    }
}

/// Evidence loaded this session, by "system|identity", so funding shared between two
/// identities can be shown, as the terminal does for identities it has open.
fn session_evidence() -> &'static Mutex<HashMap<String, Evidence>> {
    static STORE: OnceLock<Mutex<HashMap<String, Evidence>>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

fn connect_with_session(key: &str, report: &mut Evidence) {
    let mut store = session_evidence().lock().unwrap_or_else(|e| e.into_inner());
    let (system, identity) = key.split_once('|').unwrap_or(("", key));
    for (other_key, other) in store.iter_mut() {
        let Some((other_system, other_identity)) = other_key.split_once('|') else {
            continue;
        };
        if other_key == key || other_system != system {
            continue;
        }
        connect_shared_evidence(identity, report, other_identity, other);
    }
}

fn remember_evidence(key: &str, report: &Evidence) {
    session_evidence()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key.to_string(), report.clone());
}

/// A transaction from the source, with a check that it is the one requested.
async fn evidence_transaction<R: Rpc>(rpc: &R, txid: &str) -> Option<(Value, String)> {
    let (value, source) = rpc
        .call("getrawtransaction", vec![json!(txid), json!(1)])
        .await
        .ok()?;
    (value["txid"].as_str() == Some(txid)).then_some((value, source))
}

fn source_transaction_ids(raw: &Value) -> Vec<String> {
    let mut txids = raw["vin"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|input| input["txid"].as_str())
        .map(str::to_string)
        .collect::<Vec<_>>();
    txids.sort();
    txids.dedup();
    txids
}

/// Port of `identity_evidence::load`. `versions` is newest first.
async fn load_evidence<R: Rpc>(rpc: &R, versions: &[VersionRecord], system: &str) -> Evidence {
    let mut report = Evidence::default();
    let Some(current) = versions.first().cloned() else {
        return report;
    };
    let seed = current.identity_address.clone();
    let mut ordered = versions.to_vec();
    ordered.sort_by_key(|version| version.block_height);
    for pair in ordered.windows(2).take(MAX_REVERSE_RESULTS) {
        let version = &pair[0];
        let next_height = pair[1].block_height;
        let Some(identity) = version.raw.get("identity") else {
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
        ordered.dedup_by_key(|version| version.block_height);
    }
    let mut transactions: HashMap<String, Value> = HashMap::new();
    for version in &ordered {
        let Some(txid) = version.raw.get("txid").and_then(Value::as_str) else {
            continue;
        };
        if txid.chars().all(|ch| ch == '0') || transactions.contains_key(txid) {
            continue;
        }
        if let Some((transaction, source)) = evidence_transaction(rpc, txid).await {
            if txid == current.raw["txid"].as_str().unwrap_or_default() {
                report.anchor = Some((source, text(&transaction["blockhash"])));
            }
            transactions.insert(txid.to_string(), transaction);
        }
    }
    for transaction in transactions.values() {
        add_identity_import(&mut report, transaction, &seed);
    }
    let mut prevout_ids = Vec::new();
    let mut seen_prevouts = HashSet::new();
    for version in &ordered {
        let Some(transaction) = version
            .raw
            .get("txid")
            .and_then(Value::as_str)
            .and_then(|txid| transactions.get(txid))
        else {
            continue;
        };
        if is_import_of(&report, transaction["txid"].as_str().unwrap_or_default()) {
            continue; // Import protocol inputs are not registration funding.
        }
        for txid in transaction["vin"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|input| input.get("txid").and_then(Value::as_str))
        {
            if seen_prevouts.insert(txid.to_string()) {
                prevout_ids.push(txid.to_string());
            }
        }
    }
    if prevout_ids.len() > MAX_PREVOUTS {
        report.notes.push(format!(
            "Prevout inspection limited to {MAX_PREVOUTS} transactions"
        ));
    }
    let mut prevouts: HashMap<String, Value> = HashMap::new();
    for txid in prevout_ids.into_iter().take(MAX_PREVOUTS) {
        if let Some((transaction, _)) = evidence_transaction(rpc, &txid).await {
            prevouts.insert(txid, transaction);
        }
    }
    if let Some(registration) = ordered
        .first()
        .and_then(|version| version.raw.get("txid"))
        .and_then(Value::as_str)
        .and_then(|txid| transactions.get(txid))
    {
        let registration_txid = registration["txid"].as_str().unwrap_or_default();
        let commitment = add_registration(&mut report, registration, &prevouts);
        let first_referral =
            reservation(registration).and_then(|(value, _)| text_field(value, "referral"));
        add_referral_chain(&mut report, rpc, first_referral).await;
        if !is_import_of(&report, registration_txid) {
            add_funding(
                &mut report,
                registration,
                &prevouts,
                commitment
                    .as_ref()
                    .map(|(txid, index)| (txid.as_str(), *index)),
                if reservation(registration).is_some() {
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
            for source_txid in source_transaction_ids(commit_tx)
                .into_iter()
                .take(MAX_COMMITMENT_PREVOUTS)
            {
                if let Some((source, _)) = evidence_transaction(rpc, &source_txid).await {
                    commit_prevouts.insert(source_txid, source);
                }
            }
            add_funding(
                &mut report,
                commit_tx,
                &commit_prevouts,
                None,
                "Commitment funding candidate",
            );
        }
    }
    for version in ordered.iter().skip(1) {
        if let Some(transaction) = version
            .raw
            .get("txid")
            .and_then(Value::as_str)
            .and_then(|txid| transactions.get(txid))
        {
            add_funding(
                &mut report,
                transaction,
                &prevouts,
                None,
                "Update funding candidate",
            );
        }
    }

    let identity = current.raw.get("identity").unwrap_or(&Value::Null);
    for (field, recovery) in [("recoveryauthority", true), ("revocationauthority", false)] {
        let Some(authority) = identity
            .get(field)
            .and_then(Value::as_str)
            .filter(|address| address.starts_with('i'))
        else {
            continue;
        };
        let method = if recovery {
            "getidentitieswithrecovery"
        } else {
            "getidentitieswithrevocation"
        };
        // Optional: a source without the identity index answers with an error.
        if let Ok((value, source)) = rpc
            .call(
                method,
                vec![json!({"identityid": authority, "unspent": true})],
            )
            .await
        {
            add_reverse_results(
                &mut report,
                value.as_array().cloned().unwrap_or_default(),
                authority,
                field,
                &source,
                &seed,
            );
        }
    }
    let primary = identity["primaryaddresses"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|address| address.starts_with('R'))
        .take(MAX_REVERSE_ADDRESSES)
        .map(str::to_string)
        .collect::<Vec<_>>();
    for address in primary {
        let Ok((value, source)) = rpc
            .call(
                "getidentitieswithaddress",
                vec![json!({"address": address, "unspent": true})],
            )
            .await
        else {
            continue;
        };
        let results = value.as_array().cloned().unwrap_or_default();
        let count = results.len();
        for candidate in results.into_iter().take(MAX_REVERSE_RESULTS) {
            let Some(other) = candidate.get("identityaddress").and_then(Value::as_str) else {
                continue;
            };
            if other == seed
                || !candidate["primaryaddresses"]
                    .as_array()
                    .is_some_and(|addresses| {
                        addresses
                            .iter()
                            .any(|value| value.as_str() == Some(address.as_str()))
                    })
            {
                continue;
            }
            add_link(
                &mut report,
                "Shared primary address",
                format!("active identity; {source}"),
                EvidenceTarget::Identity(other.to_string()),
                format!("{source} getidentitieswithaddress address={address}"),
            );
        }
        if count > MAX_REVERSE_RESULTS {
            report.notes.push(format!(
                "Primary address {address}: showing first {MAX_REVERSE_RESULTS} of {count} results"
            ));
        }
    }
    for link in &mut report.links {
        link.source = format!("{system} {}", link.source);
    }
    report
}

fn link_confidence(label: &str) -> Confidence {
    if label.contains("candidate") || label == "Commitment control destination" {
        Confidence::Candidate
    } else {
        Confidence::Observed
    }
}

fn evidence_row(
    report: &Evidence,
    loaded: &LoadedVersions,
    retained_only: bool,
    saved: bool,
) -> IdentityEvidenceReport {
    let current = loaded.versions.last();
    IdentityEvidenceReport {
        registration: report.registration.clone(),
        imported: report
            .links
            .iter()
            .any(|link| link.label == "Identity import transaction"),
        links: report
            .links
            .iter()
            .map(|link| {
                let (kind, target) = match &link.target {
                    EvidenceTarget::Identity(id) => (EvidenceTargetKind::Identity, id.clone()),
                    EvidenceTarget::Address(id) => (EvidenceTargetKind::Address, id.clone()),
                    EvidenceTarget::Transaction(id) => {
                        (EvidenceTargetKind::Transaction, id.clone())
                    }
                };
                EvidenceLinkRow {
                    confidence: link_confidence(&link.label),
                    label: link.label.clone(),
                    detail: link.detail.clone(),
                    kind,
                    target,
                    source: link.source.clone(),
                }
            })
            .collect(),
        notes: report.notes.clone(),
        version_txid: current.and_then(|version| text(&version.raw["txid"])),
        version_height: current.map_or(0, |version| version.block_height),
        source_id: report.anchor.as_ref().map(|(source, _)| source.clone()),
        block_hash: report.anchor.as_ref().and_then(|(_, hash)| hash.clone()),
        retained_only,
        saved,
        history_complete: loaded.complete,
    }
}

// ------------------------------------------------- registration payments
//
// Ported from `src/app/registration_payments.rs`.

#[derive(Debug, Clone, PartialEq, Eq)]
struct Payment {
    output: usize,
    recipient: String,
    amount: i128,
}

#[derive(Debug, Clone)]
struct PaymentReport {
    issuer: String,
    referrals: Vec<Payment>,
    charge: Option<Payment>,
    budget: i128,
    fee: i128,
}

impl PaymentReport {
    fn annotation(&self, index: usize) -> Option<&'static str> {
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
    tx["vout"]
        .as_array()?
        .get(index)
        .filter(|out| out["n"].as_u64() == Some(index as u64))
}

fn confirmed(tx: &Value, txid: &str) -> Option<u64> {
    if tx["txid"].as_str() != Some(txid)
        || tx["confirmations"].as_u64().filter(|&n| n > 0).is_none()
    {
        return None;
    }
    tx["height"].as_u64()
}

/// Port of `native_value_zat` (`src/app.rs`).
fn native_value_zat(value: &Value) -> Option<i128> {
    ["valueZat", "valueSat", "satoshis"].iter().find_map(|key| {
        let value = value.get(key)?;
        value
            .as_i64()
            .map(i128::from)
            .or_else(|| value.as_u64().map(i128::from))
    })
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

fn payment_fee(
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

/// Port of `registration_payments::verify`. Every request goes to the transaction's source.
async fn verify_payments<R: Rpc>(
    rpc: &R,
    txid: &str,
    block: &str,
    height: u64,
) -> Option<PaymentReport> {
    let tx = get(rpc, "getrawtransaction", vec![txid.into(), 1.into()]).await?;
    if confirmed(&tx, txid) != Some(height) || tx["blockhash"] != block {
        return None;
    }
    let identity = &out(&tx, 0)?["scriptPubKey"]["identityprimary"];
    let issuer = identity["parent"].as_str()?;
    let advanced = issuer != VRSC;
    let root = get(rpc, "getcurrency", vec![Value::from(VRSC)]).await?;
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
        definition = get(rpc, "getcurrency", vec![Value::from(issuer)]).await?;
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
        parents.push(get(rpc, "getrawtransaction", vec![Value::from(id), 1.into()]).await?);
    }
    let (commit_height, fee) = payment_fee(&tx, &reg, policy, &parents)?;
    if !policy.supported_height(commit_height) {
        return None;
    }
    if let Some(referrer) = reg.referrer {
        let lookup = commit_height.checked_sub(1)?;
        let historical = get(rpc, "getidentity", vec![referrer.into(), lookup.into()]).await?;
        let history = get(
            rpc,
            "getidentityhistory",
            vec![referrer.into(), 0.into(), lookup.into()],
        )
        .await?;
        let entries = history["history"].as_array()?;
        let first = entries
            .iter()
            .min_by_key(|entry| entry["height"].as_u64().unwrap_or(u64::MAX))?;
        let id = first["output"]["txid"]
            .as_str()
            .filter(|id| is_hash_like(id))?;
        let original = get(rpc, "getrawtransaction", vec![id.into(), 1.into()]).await?;
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
    Some(PaymentReport {
        issuer: issuer.into(),
        referrals: reg.referrals,
        charge: reg.charge,
        budget: reg.budget,
        fee,
    })
}

/// Port of `registration_payments::candidate`.
fn payment_candidate(raw: &Value) -> bool {
    raw["vout"].as_array().is_some_and(|outputs| {
        outputs.iter().any(|o| {
            o["scriptPubKey"].get("identityreservation").is_some()
                || o["scriptPubKey"]
                    .get("identityadvancedreservation")
                    .is_some()
        })
    })
}

fn sats_i64(amount: i128) -> i64 {
    i64::try_from(amount).unwrap_or(i64::MAX)
}

fn unverified_payments(status: PaymentsStatus, native: String) -> RegistrationPayments {
    let summary = match &status {
        PaymentsStatus::NotRegistration => String::new(),
        _ => "Unverified".into(),
    };
    RegistrationPayments {
        status,
        issuer: None,
        native_currency: native,
        referrals: Vec::new(),
        referral_total_sats: 0,
        charge: None,
        budget_sats: 0,
        fee_sats: 0,
        total_issuer_sats: None,
        total_native_sats: 0,
        summary,
    }
}

/// The figures of Carbide's `registration_payments::lines`. `native` is the chain's ticker.
fn verified_payments(report: &PaymentReport, native: String) -> RegistrationPayments {
    let row = |payment: &Payment, currency: &str| PaymentRow {
        output: payment.output as u32,
        recipient: payment.recipient.clone(),
        currency: currency.to_string(),
        amount_sats: sats_i64(payment.amount),
        annotation: report
            .annotation(payment.output)
            .unwrap_or_default()
            .to_string(),
    };
    // Without a charge (VRSC root registrations) referrals are paid in the native coin.
    let referral_currency = if report.charge.is_some() {
        report.issuer.as_str()
    } else {
        VRSC
    };
    let referrals: i128 = report.referrals.iter().map(|payment| payment.amount).sum();
    let total_native = report.fee
        + report.budget
        + if report.charge.is_none() {
            referrals
        } else {
            0
        };
    let total_issuer = report
        .charge
        .as_ref()
        .map(|charge| sats_i64(referrals + charge.amount));
    let coins = |sats: i128| {
        let sign = if sats < 0 { "-" } else { "" };
        let magnitude = sats.unsigned_abs();
        format!(
            "{sign}{}.{:08}",
            magnitude / 100_000_000,
            magnitude % 100_000_000
        )
    };
    let mut details = vec![format!(
        "Referrals ({}) {} {}",
        report.referrals.len(),
        coins(referrals),
        if report.charge.is_some() {
            report.issuer.as_str()
        } else {
            native.as_str()
        }
    )];
    let mut total = format!("{} {native}", coins(total_native));
    if let Some(charge) = &report.charge {
        details.push(format!(
            "Issuer charge {} {}",
            coins(charge.amount),
            report.issuer
        ));
        details.push(format!(
            "Native instruction budget {} {native}",
            coins(report.budget)
        ));
        total = format!(
            "{} {} + {total}",
            coins(referrals + charge.amount),
            report.issuer
        );
    }
    details.push(format!("Transaction fee {} {native}", coins(report.fee)));
    details.push(format!("Total {total}"));
    RegistrationPayments {
        status: PaymentsStatus::Verified,
        issuer: Some(report.issuer.clone()),
        referrals: report
            .referrals
            .iter()
            .map(|payment| row(payment, referral_currency))
            .collect(),
        referral_total_sats: sats_i64(referrals),
        charge: report
            .charge
            .as_ref()
            .map(|charge| row(charge, &report.issuer)),
        budget_sats: sats_i64(report.budget),
        fee_sats: sats_i64(report.fee),
        total_issuer_sats: total_issuer,
        total_native_sats: sats_i64(total_native),
        summary: details.join("  ·  "),
        native_currency: native,
    }
}

// -------------------------------------------------------- tokenized control
//
// Ported from `src/app/token_control.rs`.

const NFT_TOKEN: u64 = 0x800;
const LAUNCH_CLEAR_CONFIRMED: u64 = 8 | 16;
const LAUNCH_CLEAR_COMPLETE: u64 = LAUNCH_CLEAR_CONFIRMED | 32;
const LAUNCH_LOOKUP_BLOCKS: u64 = 1000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct OutPoint {
    txid: String,
    index: u64,
}

#[derive(Debug, Clone)]
struct Frontier {
    source: String,
    current: OutPoint,
    visited: HashSet<OutPoint>,
    hops: usize,
}

impl Frontier {
    fn new(source: String, txid: &str, index: u64) -> Self {
        let current = OutPoint {
            txid: txid.into(),
            index,
        };
        Self {
            source,
            visited: HashSet::from([current.clone()]),
            current,
            hops: 0,
        }
    }

    fn from_row(row: &TokenFrontier) -> Self {
        let current = OutPoint {
            txid: row.txid.clone(),
            index: row.output,
        };
        let mut visited: HashSet<OutPoint> = row
            .visited
            .iter()
            .filter_map(|point| {
                let (txid, index) = point.rsplit_once(':')?;
                Some(OutPoint {
                    txid: txid.into(),
                    index: index.parse().ok()?,
                })
            })
            .collect();
        visited.insert(current.clone());
        Self {
            source: row.source.clone(),
            current,
            visited,
            hops: row.hops as usize,
        }
    }

    fn row(&self) -> TokenFrontier {
        let mut visited: Vec<String> = self
            .visited
            .iter()
            .map(|point| format!("{}:{}", point.txid, point.index))
            .collect();
        visited.sort();
        TokenFrontier {
            source: self.source.clone(),
            txid: self.current.txid.clone(),
            output: self.current.index,
            visited,
            hops: self.hops as u32,
        }
    }
}

#[derive(Debug, Clone)]
enum Status {
    Paused(Frontier),
    Unresolved(Option<Frontier>, &'static str),
    Unspent(Frontier, Option<String>),
}

fn token_result(status: Status) -> TokenControl {
    let (state, frontier) = match status {
        Status::Paused(frontier) => (TokenControlState::Paused, Some(frontier)),
        Status::Unresolved(frontier, reason) => (
            TokenControlState::Unresolved {
                reason: reason.into(),
            },
            frontier,
        ),
        Status::Unspent(frontier, holder) => {
            (TokenControlState::Unspent { holder }, Some(frontier))
        }
    };
    TokenControl {
        state,
        hops: frontier.as_ref().map_or(0, |frontier| frontier.hops as u32),
        frontier: frontier.map(|frontier| frontier.row()),
        hops_per_pass: HOPS_PER_PASS,
    }
}

async fn token_transaction<R: Rpc>(rpc: &R, txid: &str) -> Option<Value> {
    let tx = get(rpc, "getrawtransaction", vec![json!(txid), json!(1)]).await?;
    (tx["txid"].as_str() == Some(txid)).then_some(tx)
}

fn indexed_output(tx: &Value, index: u64) -> Option<&Value> {
    let output = tx["vout"].get(usize::try_from(index).ok()?)?;
    (output["n"].as_u64() == Some(index)).then_some(output)
}

fn token_value<'a>(script: &'a Value, currency: &str) -> Option<&'a Value> {
    ["reserveoutput", "reservetransfer", "reservedeposit"]
        .into_iter()
        .find_map(|key| script[key]["currencyvalues"].get(currency))
}

fn has_control_token(script: &Value, currency: &str) -> bool {
    token_value(script, currency).and_then(amount_sats) == Some(1)
}

fn unique_token_output(outputs: &[Value], currency: &str) -> Option<u64> {
    let mut candidates = outputs.iter().filter(|out| {
        token_value(&out["scriptPubKey"], currency)
            .is_some_and(|value| amount_sats(value) != Some(0))
    });
    let candidate = candidates.next()?;
    if candidates.next().is_some() || !has_control_token(&candidate["scriptPubKey"], currency) {
        return None;
    }
    candidate["n"].as_u64()
}

fn token_holder_address(script: &Value, currency: &str) -> Option<String> {
    if amount_sats(&script["reserveoutput"]["currencyvalues"][currency]) != Some(1)
        || script["reqSigs"].as_u64() != Some(1)
    {
        return None;
    }
    let [address] = script["addresses"].as_array()?.as_slice() else {
        return None;
    };
    let address = address.as_str()?;
    let (prefix, _) = address_payload(address)?;
    matches!(prefix, 0 | 5 | 60 | 85 | 102).then(|| address.to_owned())
}

fn launch_import(record: &Value, currency: &str) -> bool {
    let notarization = &record["importnotarization"];
    notarization["launchcleared"].as_bool() == Some(true)
        && notarization["launchconfirmed"].as_bool() == Some(true)
        && notarization["prelaunch"].as_bool() != Some(true)
        && notarization["refunding"].as_bool() != Some(true)
        && notarization["currencystate"]["currencyid"].as_str() == Some(currency)
        && matches!(
            notarization["currencystate"]["flags"].as_u64(),
            Some(LAUNCH_CLEAR_CONFIRMED | LAUNCH_CLEAR_COMPLETE)
        )
        && record["import"]["importcurrencyid"].as_str() == Some(currency)
}

/// Port of `token_control::origin`, from the definition the source returned.
async fn token_origin<R: Rpc>(
    rpc: &R,
    identity: &str,
    source: &str,
    definition: &Value,
) -> std::result::Result<Frontier, &'static str> {
    let source = source.to_string();
    if definition["currencyid"].as_str() != Some(identity)
        || !definition["options"]
            .as_u64()
            .is_some_and(|f| f & NFT_TOKEN != 0)
    {
        return Err("Control token definition could not be verified");
    }
    let txid = definition["definitiontxid"]
        .as_str()
        .filter(|txid| is_hash_like(txid))
        .ok_or("Currency definition outpoint unavailable")?;
    let index = definition["definitiontxout"]
        .as_u64()
        .ok_or("Currency definition outpoint unavailable")?;
    let tx = token_transaction(rpc, txid)
        .await
        .ok_or("Currency definition transaction unavailable")?;
    let defined = &indexed_output(&tx, index).ok_or("Currency definition output unavailable")?["scriptPubKey"]
        ["currencydefinition"];
    if defined["currencyid"].as_str() != Some(identity)
        || !defined["options"]
            .as_u64()
            .is_some_and(|f| f & NFT_TOKEN != 0)
    {
        return Err("Control token definition outpoint could not be verified");
    }
    if let Some(index) = tx["vout"]
        .as_array()
        .and_then(|outputs| unique_token_output(outputs, identity))
        && indexed_output(&tx, index).is_some()
    {
        return Ok(Frontier::new(source, txid, index));
    }
    let definition_height = match tx["height"].as_u64() {
        Some(height) if height > 0 => height,
        _ => {
            let hash = tx["blockhash"]
                .as_str()
                .filter(|hash| is_hash_like(hash))
                .ok_or("Waiting for the currency definition to confirm")?;
            get(rpc, "getblockheader", vec![json!(hash)])
                .await
                .and_then(|header| header["height"].as_u64())
                .ok_or("Currency definition height unavailable")?
        }
    };
    let first = definition_height.max(
        definition["startblock"]
            .as_u64()
            .unwrap_or_default()
            .saturating_sub(1),
    );
    let best = get(rpc, "getblockcount", vec![])
        .await
        .and_then(|value| value.as_u64())
        .ok_or("Source chain height unavailable")?;
    if best < first {
        return Err("Waiting for the currency launch");
    }
    let last = first.saturating_add(LAUNCH_LOOKUP_BLOCKS - 1).min(best);
    let imports = get(
        rpc,
        "getimports",
        vec![json!(identity), json!(first), json!(last)],
    )
    .await
    .ok_or("Currency launch imports unavailable")?;
    let records = if imports.is_null() {
        &[][..]
    } else {
        imports
            .as_array()
            .ok_or("Currency launch imports unavailable")?
            .as_slice()
    };
    let mut candidates = records
        .iter()
        .filter(|record| launch_import(record, identity));
    let record = candidates
        .next()
        .ok_or("No confirmed control token launch found in the launch window")?;
    if candidates.next().is_some() {
        return Err("Control token launch is ambiguous");
    }
    let txid = record["importtxid"]
        .as_str()
        .filter(|txid| is_hash_like(txid))
        .ok_or("Currency launch outpoint unavailable")?;
    let index = record["importvout"]
        .as_u64()
        .and_then(|index| u32::try_from(index).ok())
        .ok_or("Currency launch outpoint unavailable")?;
    let tx = token_transaction(rpc, txid)
        .await
        .ok_or("Currency launch transaction unavailable")?;
    let import = &indexed_output(&tx, index.into())
        .ok_or("Currency launch outpoint unavailable")?["scriptPubKey"]["crosschainimport"];
    if import != &record["import"] {
        return Err("Currency launch outpoint could not be verified");
    }
    let (_, outputs) =
        generated_region(&tx, import, index).ok_or("Currency launch outputs unavailable")?;
    let index = unique_token_output(outputs, identity)
        .ok_or("No unique control token output in the currency launch")?;
    let output = indexed_output(&tx, index).ok_or("Control token launch output unavailable")?;
    if !has_control_token(&output["scriptPubKey"], identity) {
        return Err("Control token launch output could not be verified");
    }
    Ok(Frontier::new(source, txid, index))
}

fn verify_successor(
    tx: &Value,
    current: &OutPoint,
    input_index: Option<u64>,
    currency: &str,
) -> Option<OutPoint> {
    let mut matching = tx["vin"]
        .as_array()?
        .iter()
        .enumerate()
        .filter(|(_, input)| {
            input["txid"].as_str() == Some(current.txid.as_str())
                && input["vout"].as_u64() == Some(current.index)
        });
    let (index, _) = matching.next()?;
    if matching.next().is_some() || input_index.is_some_and(|expected| index as u64 != expected) {
        return None;
    }
    let next = unique_token_output(tx["vout"].as_array()?, currency)?;
    indexed_output(tx, next)?;
    Some(OutPoint {
        txid: tx["txid"].as_str()?.to_owned(),
        index: next,
    })
}

enum Step {
    Unspent(Option<String>),
    Spent(OutPoint),
}

async fn unspent_or_successor<R: Rpc>(
    rpc: &R,
    currency: &str,
    frontier: &Frontier,
) -> std::result::Result<Step, &'static str> {
    let point = &frontier.current;
    let response = get(
        rpc,
        "gettxout",
        vec![json!(point.txid), json!(point.index), json!(true)],
    )
    .await
    .ok_or("Source could not check the unspent output")?;
    if !response.is_null() {
        let script = &response["scriptPubKey"];
        if has_control_token(script, currency) {
            return Ok(Step::Unspent(token_holder_address(script, currency)));
        }
        // Some daemon versions omit decoded reserve data from gettxout.
        let tx = token_transaction(rpc, &point.txid)
            .await
            .ok_or("Unspent token transaction unavailable")?;
        let decoded = &indexed_output(&tx, point.index)
            .ok_or("Unspent token output unavailable")?["scriptPubKey"];
        if !has_control_token(decoded, currency)
            || script["hex"]
                .as_str()
                .is_none_or(|hex| hex.is_empty() || decoded["hex"].as_str() != Some(hex))
        {
            return Err("Unspent token output could not be verified");
        }
        return Ok(Step::Unspent(token_holder_address(decoded, currency)));
    }
    let tx = token_transaction(rpc, &point.txid).await;
    let metadata = tx.as_ref().and_then(|tx| indexed_output(tx, point.index));
    let spend =
        match metadata.and_then(|out| out["spentTxId"].as_str().filter(|id| is_hash_like(id))) {
            Some(txid) => (
                txid.to_owned(),
                metadata.and_then(|out| out["spentIndex"].as_u64()),
            ),
            None => {
                let indexed = get(
                    rpc,
                    "getspentinfo",
                    vec![json!({"txid": point.txid, "index": point.index})],
                )
                .await
                .ok_or("Source could not identify the spending transaction")?;
                let txid = indexed["txid"]
                    .as_str()
                    .filter(|txid| is_hash_like(txid))
                    .ok_or("Source could not identify the spending transaction")?;
                (txid.to_owned(), indexed["index"].as_u64())
            }
        };
    let tx = token_transaction(rpc, &spend.0)
        .await
        .ok_or("Spending transaction unavailable")?;
    let next = verify_successor(&tx, point, spend.1, currency)
        .ok_or("No unique on-chain continuation for this control token")?;
    Ok(Step::Spent(next))
}

/// Port of `token_control::follow`: one pass of at most `HOPS_PER_PASS` hops.
async fn follow_token<R: Rpc>(rpc: &R, currency: &str, mut frontier: Frontier) -> Status {
    for _ in 0..HOPS_PER_PASS {
        match unspent_or_successor(rpc, currency, &frontier).await {
            Ok(Step::Unspent(address)) => return Status::Unspent(frontier, address),
            Ok(Step::Spent(outpoint)) => {
                if !frontier.visited.insert(outpoint.clone()) {
                    return Status::Unresolved(
                        Some(frontier),
                        "Control token path contains a loop",
                    );
                }
                frontier.current = outpoint;
                frontier.hops += 1;
            }
            Err(message) => return Status::Unresolved(Some(frontier), message),
        }
    }
    Status::Paused(frontier)
}

// ------------------------------------------------------------- addresses
//
// Ported from `address_balance_amounts`, `parse_identity_associations` and
// `address_tile_flows` (`src/app.rs`).

/// Port of `decimal_value_to_zat` (`src/app.rs`).
fn decimal_to_sats(value: &Value) -> Option<i128> {
    if let Some(text) = value.as_str() {
        return decimal_string_to_sats(text);
    }
    if value.is_number() {
        return decimal_string_to_sats(&value.to_string());
    }
    value
        .get("valueZat")
        .and_then(|value| value.as_i64().map(i128::from))
        .or_else(|| value.get("value").and_then(decimal_to_sats))
        .or_else(|| value.get("amount").and_then(decimal_to_sats))
}

fn decimal_string_to_sats(value: &str) -> Option<i128> {
    let value = value.trim();
    let (mantissa, exponent) = match value.split_once(['e', 'E']) {
        Some((m, e)) => (m, e.parse::<i32>().ok()?),
        None => (value, 0),
    };
    let (negative, unsigned) = mantissa
        .strip_prefix('-')
        .map_or((false, mantissa), |v| (true, v));
    let (whole, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    if whole.is_empty()
        || !whole.chars().all(|ch| ch.is_ascii_digit())
        || !fraction.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{whole}{fraction}").parse::<i128>().ok()?;
    let scale = 8_i32
        .checked_add(exponent)?
        .checked_sub(i32::try_from(fraction.len()).ok()?)?;
    let amount = if scale >= 0 {
        digits.checked_mul(10_i128.checked_pow(u32::try_from(scale).ok()?)?)?
    } else {
        let divisor = 10_i128.checked_pow(scale.unsigned_abs())?;
        if digits % divisor != 0 {
            return None;
        }
        digits / divisor
    };
    Some(if negative { -amount } else { amount })
}

/// Every currency the address holds or has received, with the chain's own coin first.
fn balance_amounts(
    response: &Value,
    native_id: Option<&str>,
    native_name: &str,
) -> Vec<AddressBalance> {
    let names = response.get("currencynames").and_then(Value::as_object);
    let parse = |key: &str| -> BTreeMap<String, i128> {
        response
            .get(key)
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(id, value)| decimal_to_sats(value).map(|amount| (id.clone(), amount)))
            .collect()
    };
    let received = parse("currencyreceived");
    let mut amounts = parse("currencybalance");
    // The daemon can omit spent-down currencies from its sparse balance map.
    for (id, amount) in &received {
        if *amount > 0 {
            amounts.entry(id.clone()).or_insert(0);
        }
    }
    let json_sats = |key: &str| {
        response
            .get(key)
            .and_then(|value| value.as_i64().map(i128::from))
    };
    let native_balance = json_sats("balance");
    let native_received = json_sats("received").unwrap_or(0);
    let name_of = |id: &str| {
        if native_id == Some(id) {
            return native_name.to_string();
        }
        names
            .and_then(|names| names.get(id))
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string()
    };
    let mut rows: Vec<AddressBalance> = Vec::new();
    for (id, amount) in &amounts {
        if *amount != 0 || received.get(id).is_some_and(|value| *value > 0) {
            rows.push(AddressBalance {
                currency_id: id.clone(),
                name: name_of(id),
                amount_sats: sats_i64(*amount),
                native: native_id == Some(id.as_str()),
            });
        }
    }
    let has_native = rows
        .iter()
        .any(|row| row.native || row.name.eq_ignore_ascii_case(native_name));
    if let Some(native_balance) = native_balance
        && !has_native
        && (native_balance != 0 || native_received > 0)
    {
        // Older responses can omit names. Infer only one unambiguous native
        // entry, and never relabel an unrelated zero reserve balance.
        if native_id.is_none() {
            let candidates: Vec<String> = amounts
                .iter()
                .filter(|(id, amount)| {
                    **amount == native_balance
                        && name_of(id) == **id
                        && (native_balance != 0 || received.get(*id) == Some(&native_received))
                })
                .map(|(id, _)| id.clone())
                .collect();
            if let [only] = candidates.as_slice() {
                rows.retain(|row| &row.currency_id != only);
            }
        }
        rows.push(AddressBalance {
            currency_id: native_id.unwrap_or(native_name).to_string(),
            name: native_name.to_string(),
            amount_sats: sats_i64(native_balance),
            native: true,
        });
    }
    for row in &mut rows {
        if row.name.eq_ignore_ascii_case(native_name) {
            row.native = true;
        }
    }
    rows.sort_by(|left, right| {
        right.native.cmp(&left.native).then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
    });
    rows
}

/// Identities whose primary addresses include `address`, named and sorted.
fn primary_address_identities(address: &str, values: Vec<Value>) -> Vec<(String, String)> {
    let mut identities = values
        .into_iter()
        .filter(|identity| {
            identity["primaryaddresses"]
                .as_array()
                .is_some_and(|addresses| {
                    addresses
                        .iter()
                        .any(|value| value.as_str() == Some(address))
                })
        })
        .filter_map(|identity| {
            let id = identity["identityaddress"].as_str()?.to_string();
            let name = identity["name"].as_str()?.trim_end_matches('@');
            Some((id, format!("{name}@")))
        })
        .collect::<Vec<_>>();
    let mut seen = HashSet::new();
    identities.retain(|(id, _)| seen.insert(id.clone()));
    identities.sort_by(|left, right| {
        left.1
            .to_ascii_lowercase()
            .cmp(&right.1.to_ascii_lowercase())
            .then_with(|| left.0.cmp(&right.0))
    });
    identities
}

/// Port of `addresses_in` (`src/app.rs`).
fn addresses_in(value: &Value) -> Vec<String> {
    fn collect(value: &Value, addresses: &mut Vec<String>) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    if key.eq_ignore_ascii_case("address") {
                        if let Some(address) = child.as_str() {
                            addresses.push(address.to_string());
                        }
                    } else if key.eq_ignore_ascii_case("addresses") {
                        if let Some(items) = child.as_array() {
                            addresses
                                .extend(items.iter().filter_map(Value::as_str).map(str::to_string));
                        }
                    } else {
                        collect(child, addresses);
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| collect(item, addresses)),
            _ => {}
        }
    }
    let mut addresses = Vec::new();
    collect(value, &mut addresses);
    let mut seen = HashSet::new();
    addresses.retain(|address| seen.insert(address.clone()));
    addresses
}

/// Port of `output_currency_amounts` (`src/app.rs`).
fn flow_amounts(output: &Value, native_currency: &str) -> Vec<FlowAmount> {
    fn collect(value: &Value, amounts: &mut BTreeMap<String, i128>) {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let normalized = key.to_ascii_lowercase().replace('_', "");
                    if matches!(
                        normalized.as_str(),
                        "reservevalues" | "currencyvalues" | "reserveamounts"
                    ) && let Some(values) = child.as_object()
                    {
                        for (currency, amount) in values {
                            if let Some(amount) = decimal_to_sats(amount) {
                                *amounts.entry(currency.clone()).or_default() += amount;
                            }
                        }
                    } else {
                        collect(child, amounts);
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| collect(item, amounts)),
            _ => {}
        }
    }
    let mut amounts = BTreeMap::new();
    let native = native_value_zat(output).or_else(|| output.get("value").and_then(decimal_to_sats));
    if let Some(native) = native {
        amounts.insert(native_currency.to_string(), native);
    }
    collect(output, &mut amounts);
    amounts
        .into_iter()
        .map(|(currency, sats)| FlowAmount {
            currency,
            sats: sats_i64(sats),
        })
        .collect()
}

/// Port of `address_tile_flows`: the viewed address first, then private flows, then others.
/// `unspent` holds the viewed address's unspent outputs.
fn address_tile(
    raw: &Value,
    viewed: &str,
    native_currency: &str,
    unspent: &HashSet<(String, u64)>,
    prevouts: &HashMap<String, Value>,
) -> AddressTile {
    let txid = text(&raw["txid"]).unwrap_or_default();
    let mut inputs: Vec<AddressFlow> = raw["vin"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(index, input)| {
            // As the terminal's `resolved_prevout`: the spent output when it is loaded.
            let source = spent_output(input, prevouts).unwrap_or(input);
            let addresses = addresses_in(source);
            AddressFlow {
                index: index as u32,
                viewed: addresses.iter().any(|address| address == viewed),
                amounts: flow_amounts(source, native_currency),
                coinbase: input.get("coinbase").is_some(),
                private: false,
                status: FlowStatus::Spent,
                addresses,
            }
        })
        .collect();
    let transparent_inputs = inputs.len();
    for offset in 0..crate::domain::private_input_count(raw) {
        inputs.push(private_flow(transparent_inputs + offset));
    }
    let mut outputs: Vec<AddressFlow> = raw["vout"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(fallback, output)| {
            let index = output["n"].as_u64().unwrap_or(fallback as u64);
            let addresses = addresses_in(output);
            let status = if output
                .get("spentTxId")
                .is_some_and(|value| !value.is_null())
            {
                FlowStatus::Spent
            } else if unspent.contains(&(txid.clone(), index)) {
                FlowStatus::Unspent
            } else {
                FlowStatus::Unknown
            };
            AddressFlow {
                index: index as u32,
                viewed: addresses.iter().any(|address| address == viewed),
                amounts: flow_amounts(output, native_currency),
                coinbase: false,
                private: false,
                status,
                addresses,
            }
        })
        .collect();
    let transparent_outputs = outputs.len();
    for offset in 0..crate::domain::private_output_count(raw) {
        outputs.push(private_flow(transparent_outputs + offset));
    }
    let rank = |flow: &AddressFlow| {
        if flow.viewed {
            0
        } else if flow.private {
            1
        } else {
            2
        }
    };
    inputs.sort_by_key(rank);
    outputs.sort_by_key(rank);
    let more_inputs = inputs.len().saturating_sub(ADDRESS_FLOW_LIMIT) as u32;
    let more_outputs = outputs.len().saturating_sub(ADDRESS_FLOW_LIMIT) as u32;
    inputs.truncate(ADDRESS_FLOW_LIMIT);
    outputs.truncate(ADDRESS_FLOW_LIMIT);
    let summary = TransactionSummary {
        txid: txid.clone(),
        raw: Some(raw.clone()),
    };
    AddressTile {
        time: raw["blocktime"].as_u64().or_else(|| raw["time"].as_u64()),
        height: raw["height"].as_u64(),
        confirmations: raw["confirmations"].as_i64(),
        block_hash: text(&raw["blockhash"]),
        tags: labels(&summary),
        inputs,
        outputs,
        more_inputs,
        more_outputs,
        txid,
    }
}

/// Port of `resolved_prevout` (`src/app.rs`): the output an input spends, when its
/// transaction is loaded.
fn spent_output<'a>(input: &Value, prevouts: &'a HashMap<String, Value>) -> Option<&'a Value> {
    let txid = input.get("txid")?.as_str()?;
    let index = usize::try_from(input.get("vout")?.as_u64()?).ok()?;
    prevouts.get(txid)?.get("vout")?.as_array()?.get(index)
}

/// The transactions that the inputs of these tiles spend, at most `limit`, in order of first
/// use. Coinbase inputs spend nothing.
fn prevout_txids<'a>(transactions: impl Iterator<Item = &'a Value>, limit: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut txids = Vec::new();
    for transaction in transactions {
        for input in transaction["vin"].as_array().into_iter().flatten() {
            if let Some(txid) = input["txid"].as_str()
                && seen.insert(txid.to_string())
            {
                if txids.len() == limit {
                    return txids;
                }
                txids.push(txid.to_string());
            }
        }
    }
    txids
}

/// A block answer's place: on the active chain when its confirmations are not negative.
fn block_active(block: &Value) -> Option<bool> {
    block["confirmations"].as_i64().map(|count| count >= 0)
}

fn private_flow(index: usize) -> AddressFlow {
    AddressFlow {
        index: index as u32,
        addresses: Vec::new(),
        amounts: Vec::new(),
        coinbase: false,
        private: true,
        viewed: false,
        status: FlowStatus::Unknown,
    }
}

#[cfg(test)]
#[path = "ffi_identity_tests.rs"]
mod tests;
