//! Transactions and blocks: detail, transfer traces, witnesses and links.
//!
//! Part of the bridge's Swift-facing API. See `ffi.rs` for the shared types and helpers.
//!
//! Much of the transaction overview lives in the terminal's `src/app.rs`, where it reads
//! the terminal's object store. The pure parts are ported here with a comment naming the
//! source function. The store is replaced by `Context`, which holds the previous outputs
//! and the marketplace facts that this module requests for one transaction.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use super::*;
use crate::domain::{MarketplaceTag, OutputTag, OutputTagKind, TransactionTag};
use crate::world::{OutpointRef, SystemId};
use anyhow::Context as _;

/// At most this many source transactions are requested to describe a transaction's inputs.
const MAX_SOURCE_TRANSACTIONS: usize = 64;
/// At most this many notarization outputs get a status line.
const MAX_NOTARIZATION_CONTEXTS: usize = 2;
/// At most this many distinct heights are dated in one JSON document.
const MAX_JSON_DATES: usize = 48;
/// Inputs before this many confirmations cannot stake (Carbide's `Stake age pending`).
const STAKE_AGE: u64 = 150;

// --------------------------------------------------------------- records

/// An output on a named system.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Outpoint {
    pub system_id: String,
    pub txid: String,
    pub output: u32,
}

impl From<&OutpointRef> for Outpoint {
    fn from(outpoint: &OutpointRef) -> Self {
        Self {
            system_id: outpoint.system_id.to_string(),
            txid: outpoint.txid.clone(),
            output: outpoint.vout,
        }
    }
}

impl Outpoint {
    fn reference(&self) -> Option<OutpointRef> {
        Some(OutpointRef {
            system_id: SystemId::try_from(self.system_id.clone()).ok()?,
            txid: self.txid.clone(),
            vout: self.output,
        })
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TransactionInput {
    /// The position in Carbide's list: transparent inputs first, then shielded spends.
    pub index: u32,
    pub coinbase: bool,
    pub previous_txid: Option<String>,
    pub previous_output: Option<u32>,
    pub addresses: Vec<String>,
    /// Native value in satoshis. None when it is hidden or the source did not report it.
    pub value_sats: Option<i64>,
    /// "Sapling" or "Sprout" for a shielded spend.
    pub shielded_pool: Option<String>,
    /// True when a shielded value is derived from the Sapling value balance.
    pub derived_value: bool,
    /// Value-bearing token amounts, keyed by currency ID.
    pub reserve_amounts: Vec<CurrencyAmount>,
    pub sequence: Option<u64>,
    pub script_type: Option<String>,
    /// Carbide's annotations, such as "Auto-Arbitrage Input".
    pub annotations: Vec<String>,
    /// The block time and height of the transaction that created the spent output.
    pub source_block_time: Option<u64>,
    pub source_height: Option<u64>,
    /// Blocks between the spent output's block and this transaction's block.
    pub confirmations_when_spent: Option<u64>,
}

/// An output tag with what it refers to.
#[derive(Debug, Clone, uniffi::Record)]
pub struct OutputTagInfo {
    /// Carbide's tag label, without a currency name.
    pub label: String,
    /// The currency a DeFi tag names, shown after the label.
    pub currency_id: Option<String>,
    /// True when the tag opens the transfer trace for this output.
    pub traceable: bool,
    /// The output the tag opens, for finalizations and evidence.
    pub target: Option<Outpoint>,
}

/// What an output reference points to.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum ReferenceTarget {
    Currency {
        id: String,
    },
    Destination {
        address: String,
        system_id: Option<String>,
    },
    Output {
        outpoint: Outpoint,
    },
    /// A block committed to by a historical notarization.
    Block {
        hash: String,
        height: u32,
    },
    /// Unrecognised data. `script_body` is true when the pushes did not decode and the
    /// bytes are the script after `OP_RETURN`.
    Bytes {
        hex: String,
        script_body: bool,
    },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct OutputReference {
    pub label: String,
    pub target: ReferenceTarget,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TransactionOutput {
    /// The output number. Shielded outputs follow the transparent ones.
    pub index: u32,
    pub value_sats: Option<i64>,
    pub addresses: Vec<String>,
    pub script_type: Option<String>,
    pub tags: Vec<String>,
    pub spent_txid: Option<String>,
    /// The daemon's `reserve_balance`, keyed by currency name or ID.
    pub currency_values: Vec<CurrencyAmount>,
    /// Unrecognised OP_RETURN data, as hex.
    pub data_hex: Option<String>,
    pub json: String,
    pub shielded_pool: Option<String>,
    pub derived_value: bool,
    /// The daemon's `spendableoutput`: true for a spendable output type, false for a
    /// protocol output type.
    pub spendable: Option<bool>,
    /// Value-bearing token amounts, keyed by currency ID, as Carbide counts them.
    pub reserve_amounts: Vec<CurrencyAmount>,
    pub tag_details: Vec<OutputTagInfo>,
    pub annotations: Vec<String>,
    /// Protocol routing labels: `label` is the address, `value` is Carbide's description.
    pub routing: Vec<Field>,
    pub references: Vec<OutputReference>,
    pub reference_notes: Vec<String>,
    /// "Immature" and "Stake age pending".
    pub badges: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TransactionStatus {
    Confirmed,
    Unconfirmed,
    /// In a block whose confirmations the source did not establish.
    ConfirmationsUnverified,
    /// In a block that is not on the active chain. Mempool membership is not checked.
    OrphanedInclusion,
}

/// A side of a transaction's value: all inputs or all outputs.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ValueTotal {
    /// Carbide's label for the native part, such as `1.00000000 VRSC known + hidden`.
    pub label: String,
    /// The sum of the known native values. None when no value is known.
    pub sats: Option<i64>,
    /// A shielded value is hidden.
    pub hidden: bool,
    /// A shielded value is derived from the value balance.
    pub derived: bool,
    pub coinbase: bool,
    /// A transparent value is missing from the source.
    pub remaining_unknown: bool,
    pub reserves: Vec<CurrencyAmount>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum FeeKind {
    Known,
    /// A value is missing, so the fee cannot be calculated.
    Unknown,
    /// Protocol outputs account for value elsewhere. Carbide does not reconstruct the fee.
    ProtocolAccounting,
    /// A coinbase has no fee.
    NotApplicable,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct ValueAccounting {
    pub input_count: u32,
    pub output_count: u32,
    pub inputs: ValueTotal,
    pub outputs: ValueTotal,
    pub fee_kind: FeeKind,
    pub fee_sats: Option<i64>,
    /// Carbide's fee text.
    pub fee_label: String,
    /// Shown for protocol transactions: transparent inputs minus outputs plus the net
    /// shielded flow. None when a value is missing.
    pub public_native_delta: Option<i64>,
    pub show_public_delta: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TransactionDetail {
    pub txid: String,
    pub block_hash: Option<String>,
    pub height: Option<u64>,
    pub confirmations: Option<i64>,
    pub time: Option<u64>,
    pub size: Option<u64>,
    pub version: Option<i64>,
    pub expiry_height: Option<u64>,
    pub tags: Vec<String>,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub private_inputs: u32,
    pub private_outputs: u32,
    /// Outputs from which a transfer can be followed.
    pub trace_roots: Vec<TraceRoot>,
    /// Outputs carrying notarization evidence whose signers can be investigated.
    pub witness_roots: Vec<TraceRoot>,
    /// References from this transaction's outputs to other outputs. Statuses come from
    /// `protocol_links`.
    pub links: Vec<ProtocolLink>,
    pub json: String,
    pub status: TransactionStatus,
    /// Carbide's status text, such as `confirmed (12)`.
    pub status_text: String,
    /// The block time, when the transaction is in a block.
    pub block_time: Option<u64>,
    pub locktime: Option<u64>,
    pub in_mempool: bool,
    /// True when a public API supplied the transaction.
    pub from_public_api: bool,
    pub accounting: ValueAccounting,
    /// Who ended with more and who ended with less, in each currency.
    pub statement: statement::TransactionStatement,
    /// Carbide's shielded lines, such as `Sapling 1.00000000 VRSC shielded -> transparent`.
    pub shielded: Vec<String>,
    /// Identity registration lines.
    pub registration: Vec<String>,
    /// `Notarization output N: confirmed in VRSC view`.
    pub notarization_context: Vec<String>,
    /// The transaction has a fee pool checkpoint output.
    pub fee_pool_checkpoint: bool,
    /// The native currency's name.
    pub coin: String,
    pub provenance: Provenance,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct BlockDetail {
    pub row: BlockRow,
    pub previous_hash: Option<String>,
    pub next_hash: Option<String>,
    pub confirmations: Option<i64>,
    pub fields: Vec<Field>,
    pub transactions: Vec<TransactionRow>,
    /// The block as the daemon reported it, with transactions reduced to IDs.
    pub json: String,
    /// Carbide's block type label, such as "Staked" or "Mined".
    pub production: String,
    /// How many transaction bodies were loaded for the tags.
    pub bodies_loaded: u32,
    /// The tags of the block's transactions together, without Coinbase and Stake.
    pub combined_tags: Vec<String>,
    /// "Chain tip", "Active" or "Orphaned". None when the source did not say.
    pub chain_status: Option<String>,
    pub on_active_chain: bool,
    /// The active chain's block at this height, when this block is not on it.
    pub active_hash: Option<String>,
    pub provenance: Provenance,
}

/// An output of a transaction from which a transfer can be followed.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraceRoot {
    pub output: u32,
    pub label: String,
}

/// One output along a traced transfer.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraceStep {
    pub system_id: String,
    pub txid: String,
    pub output: u32,
    pub label: String,
    pub detail: String,
}

/// A conversion's net amounts and the import batch's named prices.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraceRate {
    /// "Principal" or "Forward budget".
    pub purpose: String,
    pub net_input: CurrencyAmount,
    pub output: CurrencyAmount,
    /// Output units per input unit, in satoshis of the output currency.
    pub rate_sats: Option<i64>,
    /// True when the division is exact.
    pub exact: bool,
    pub prices: Vec<TracePrice>,
}

/// A price field of the import batch. These are builder fields, not market quotes.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TracePrice {
    pub currency: String,
    pub field: String,
    pub value: f64,
}

/// The pool state an import executed against, and its linked previous notarization.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PoolInspection {
    pub pool: String,
    pub import: Outpoint,
    pub height: Option<u64>,
    pub previous: Option<Outpoint>,
}

/// One leg of a transfer: an instruction and, when found, what it produced.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraceLeg {
    pub source_system: String,
    pub source_txid: String,
    pub source_output: u32,
    /// Carbide's own wording for how far this leg got.
    pub stage: String,
    pub confidence: Confidence,
    pub principal: Vec<CurrencyAmount>,
    /// False when Carbide could not read the principal.
    pub principal_known: bool,
    pub pool: Option<String>,
    pub conversion_fee: Option<CurrencyAmount>,
    pub transfer_fee_budget: Option<CurrencyAmount>,
    pub forwarding_budget: Option<CurrencyAmount>,
    pub forwarding_conversion_fee: Option<CurrencyAmount>,
    /// "Received", "Forwarded" or "Refunded" when a result output was replayed exactly.
    pub result: Option<String>,
    pub result_amounts: Vec<CurrencyAmount>,
    pub result_system: Option<String>,
    pub result_txid: Option<String>,
    pub result_output: Option<u32>,
    /// The fee budget carried into the next instruction.
    pub next_fee_budget: Option<CurrencyAmount>,
    pub burned: Option<CurrencyAmount>,
    pub batch_burned: Option<CurrencyAmount>,
    pub proceeds: Vec<CurrencyAmount>,
    pub proceeds_output: Option<Outpoint>,
    pub import_height: Option<u64>,
    /// The system that executed the import.
    pub execution_system: Option<String>,
    pub import_output: Option<Outpoint>,
    pub rates: Vec<TraceRate>,
    /// Every currency this leg names.
    pub currencies: Vec<String>,
    pub pool_inspection: Option<PoolInspection>,
}

/// How much of a chain was searched for a transfer's import.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TraceSearch {
    pub system_id: String,
    pub first: u64,
    pub last: u64,
    pub tip: u64,
    pub windows: u32,
    pub more_older: bool,
}

/// The import height found for one leg, kept when another leg is searched again.
#[derive(Debug, Clone, uniffi::Record)]
pub struct KnownImport {
    pub root: Outpoint,
    pub height: u64,
}

/// A stage of a transfer that the evidence reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum TraceStage {
    Sent,
    Exported,
    Imported,
    /// A result output that the replay matched: received, refunded, forwarded or burned.
    Result,
}

/// A stage of a transfer with the block that holds it. The page puts the stages on one
/// axis of blocks, so that a wait is a length.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TraceMoment {
    pub stage: TraceStage,
    pub system_id: String,
    pub txid: String,
    pub output: u32,
    /// None when the transaction is not in a block, or is on a system that this
    /// computer does not have.
    pub height: Option<u64>,
    pub time: Option<u64>,
}

/// The export batch that took a transfer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct TraceBatch {
    /// The number of transfers in the export.
    pub size: u32,
    /// The position of this transfer in the batch, from 0.
    pub place: u32,
}

/// A transfer followed from its source output as far as the evidence reaches.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TransferTrace {
    /// One plain statement of the outcome so far.
    pub verdict: String,
    pub confidence: Confidence,
    pub legs: Vec<TraceLeg>,
    pub steps: Vec<TraceStep>,
    pub notes: Vec<String>,
    /// Why the trace stopped, when it did not reach a result.
    pub stop: Option<String>,
    pub search: Option<TraceSearch>,
    /// True while the search continues. Set on intermediate reports.
    pub searching: bool,
    /// The leg the search applies to.
    pub search_root: Option<Outpoint>,
    pub import_heights: Vec<KnownImport>,
    /// The stages that the evidence reached, in the order of the trail.
    pub moments: Vec<TraceMoment>,
    /// The export batch of the first leg.
    pub batch: Option<TraceBatch>,
    /// Other outputs of the import that went to the destination address. The replay
    /// tells the result from them.
    pub candidates: u32,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum TraceRequest {
    /// The import window after the export.
    Initial,
    /// The 1,000-block window that ends at a destination height.
    WindowEndingAt { height: u64 },
    /// Up to 32 older windows, starting below a height.
    BackwardFrom { height: u64 },
}

/// What to search, and for which leg. Other legs keep their known import heights.
#[derive(Debug, Clone, uniffi::Record)]
pub struct TracePlan {
    pub request: TraceRequest,
    pub target: Option<Outpoint>,
    pub known_imports: Vec<KnownImport>,
}

/// A pool's supply or one reserve before and after an import.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PoolChange {
    /// "Supply", or the reserve currency's ID.
    pub label: String,
    pub is_supply: bool,
    pub before: Option<f64>,
    pub after: Option<f64>,
    pub change: Option<f64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PoolSnapshot {
    pub output: Outpoint,
    pub height: Option<u64>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct PoolComparison {
    pub previous: Option<PoolSnapshot>,
    pub imported: PoolSnapshot,
    pub note: Option<String>,
    pub changes: Vec<PoolChange>,
    /// More than 12 reserves. The list shows the first 12.
    pub more_reserves: bool,
}

/// A reference between this transaction's outputs and another output (Carbide's F3).
#[derive(Debug, Clone, uniffi::Record)]
pub struct ProtocolLink {
    pub label: String,
    /// The output of this transaction that carries the reference, or for a reverse
    /// reference, the output of this transaction that is referenced.
    pub output: u32,
    pub target_system: String,
    pub target_txid: String,
    pub target_output: u32,
    /// The decoded field, such as `crosschainimport.exporttxid`.
    pub field: String,
    /// True for a reference from another transaction to this one.
    pub reverse: bool,
    /// Carbide's resolution status. None until the counterpart is checked.
    pub status: Option<String>,
    /// Both sides resolved.
    pub resolved: bool,
    /// The two sides disagree.
    pub problem: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct LinkReport {
    pub links: Vec<ProtocolLink>,
    /// How many reverse references the cache held.
    pub reverse_count: u32,
    /// Why reverse references are not available, when they are not.
    pub reverse_unavailable: Option<String>,
}

/// An output that holds a signature's evidence.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WitnessOrigin {
    pub output: Outpoint,
    pub source: String,
}

/// Signatures by one identity over one target, as found in notarization evidence.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WitnessSignature {
    pub identity: String,
    pub height: Option<u64>,
    /// True for a confirming statement, false for a rejecting one.
    pub confirms: bool,
    pub system: String,
    pub target_system: String,
    pub target_txid: String,
    pub target_output: u32,
    pub key_count: u32,
    pub key_signatures: Vec<String>,
    pub origins: Vec<WitnessOrigin>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct WitnessReport {
    pub signatures: Vec<WitnessSignature>,
    pub finalizations: Vec<String>,
    /// Gaps and limits reached, in Carbide's words.
    pub issues: Vec<String>,
    pub transactions_checked: u32,
    pub outputs_checked: u32,
    pub multipart_sets: u32,
    /// The decoded report as JSON: root, status and report.
    pub json: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum JsonLinkKind {
    Transaction,
    Block,
    Address,
    Identity,
    Currency,
}

/// What the JSON view can do with one value: open it, name it or date it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct JsonMark {
    /// A JSON pointer, such as `/vout/0/scriptPubKey/addresses/0`.
    pub pointer: String,
    pub link: Option<JsonLinkKind>,
    pub target: Option<String>,
    /// True when a name can replace the value. `identityaddress` stays raw.
    pub nameable: bool,
    pub date: Option<u64>,
    /// True for a future height: the date is an estimate.
    pub estimated: bool,
}

// --------------------------------------------------------------- explorer

#[uniffi::export(async_runtime = "tokio")]
impl Explorer {
    /// A transaction with the terminal's overview: status, value accounting, fee, shielded
    /// flows, annotations and maturity.
    pub async fn transaction(&self, txid: String) -> Outcome<TransactionDetail> {
        let params = [Value::from(txid.trim()), Value::from(1)];
        let output = self
            .observed(
                "getrawtransaction",
                &params,
                self.primary.get_raw_transaction(txid.trim()).await,
            )
            .await?;
        let provenance = output.provenance.clone();
        let raw = output.result;
        let from_public_api = output.source_id.starts_with("public-api:");
        let txid = text(&raw["txid"]).unwrap_or_else(|| txid.trim().to_string());

        // The chain's view: tip, block and mempool, through Carbide's chain state.
        let mut chain = crate::chain_state::ChainState::default();
        let mut order = 0;
        let mut observe =
            |chain: &mut crate::chain_state::ChainState, method: &str, result: Value| {
                order += 1;
                chain.observe(crate::chain_state::RpcObservation {
                    authorization: None,
                    order,
                    method: method.into(),
                    params: Vec::new(),
                    result,
                });
            };
        if let Ok(info) = self.settle(self.client.get_info().await) {
            observe(&mut chain, "getinfo", info.result);
        }
        let block_hash = text(&raw["blockhash"]);
        let mut orphaned = false;
        if let Some(hash) = &block_hash {
            if let Ok(header) = self.settle(
                self.client
                    .call_with_raw::<Value>("getblockheader", &[Value::from(hash.as_str())])
                    .await,
            ) {
                orphaned = header.result["confirmations"].as_i64() == Some(-1);
                observe(&mut chain, "getblockheader", header.result);
            }
        } else if let Ok(pool) = self.settle(
            self.client
                .call_with_raw::<Value>("getrawmempool", &[])
                .await,
        ) {
            observe(&mut chain, "getrawmempool", pool.result);
        }
        observe(&mut chain, "getrawtransaction", raw.clone());

        if let Some(system) = self.config.chain.system_id.get() {
            remember_transaction(system, &raw);
        }
        let context = self.context(&raw).await;
        let coin = chain
            .chain_name
            .clone()
            .unwrap_or_else(|| self.config.chain.name.clone());
        let native = chain
            .chain_id
            .clone()
            .or_else(|| self.config.chain.system_id.get().map(ToString::to_string));
        let maturity = self.maturity(&chain, &raw).await;
        let depth = block_hash
            .as_deref()
            .and_then(|hash| chain.confirmations(hash))
            .or_else(|| chain.in_mempool(&txid).then_some(0));
        let confirmations =
            depth_for_status(&chain, block_hash.as_deref(), raw["confirmations"].as_i64());
        let in_mempool = chain.in_mempool(&txid);
        let status = transaction_status(in_mempool, confirmations, block_hash.is_some(), orphaned);

        let notarization_context = self.notarization_context(&raw, &coin).await;
        self.forgive();

        let height = raw["height"].as_u64();
        let flows = transaction_flows(&raw, &context, native.as_deref());
        let accounting = value_accounting(&raw, &flows, &context, &coin);
        let statement =
            statement::statement(&raw, &flows, &context, native.as_deref(), &accounting);
        let observing = self.config.chain.system_id.get().cloned();
        let mut detail = TransactionDetail {
            block_hash: block_hash.clone(),
            height,
            confirmations: raw["confirmations"].as_i64(),
            time: raw["blocktime"].as_u64().or_else(|| raw["time"].as_u64()),
            size: raw["size"]
                .as_u64()
                .or_else(|| raw["vsize"].as_u64())
                .or_else(|| raw["hex"].as_str().map(|hex| hex.len() as u64 / 2)),
            version: raw["version"].as_i64(),
            expiry_height: raw["expiryheight"].as_u64(),
            tags: display_tags(&raw, &context),
            inputs: Vec::new(),
            outputs: Vec::new(),
            private_inputs: crate::domain::private_input_count(&raw) as u32,
            private_outputs: crate::domain::private_output_count(&raw) as u32,
            trace_roots: crate::defi::roots(&raw)
                .into_iter()
                .map(|(output, label)| TraceRoot { output, label })
                .collect(),
            witness_roots: crate::witnesses::roots(&raw)
                .into_iter()
                .map(|(output, label)| TraceRoot { output, label })
                .collect(),
            links: observing
                .as_ref()
                .map(|system| crate::relationships::transaction_links(system, &raw))
                .unwrap_or_default()
                .into_iter()
                .map(|link| protocol_link(&link, false, None))
                .collect(),
            json: pretty(&without(&raw, "hex")),
            txid: txid.clone(),
            status: status.0,
            status_text: status.1,
            block_time: block_hash
                .as_ref()
                .and(raw["blocktime"].as_u64().or_else(|| raw["time"].as_u64()))
                .filter(|time| *time > 0),
            locktime: raw["locktime"].as_u64(),
            in_mempool,
            from_public_api,
            accounting,
            statement,
            shielded: shielded_lines(&ShieldedAccounting::from_raw(&raw), &coin),
            registration: registration_lines(&raw),
            notarization_context,
            fee_pool_checkpoint: raw["vout"].as_array().is_some_and(|outputs| {
                outputs
                    .iter()
                    .any(|output| output["scriptPubKey"]["feepool"].is_object())
            }),
            coin: coin.clone(),
            provenance,
        };
        for flow in &flows {
            match flow.kind {
                FlowKind::Input => detail.inputs.push(input_record(flow, &context, height)),
                FlowKind::Output => {
                    let badges = output_badges(&raw, flow, depth, maturity, height);
                    detail.outputs.push(output_record(&raw, flow, badges));
                }
            }
        }
        Ok(detail)
    }

    /// Forward and reverse protocol references with Carbide's resolution status. Each
    /// counterpart is requested from the chain it is on, when that chain is configured
    /// on this computer. Reverse references come from the persistent cache and from the
    /// transactions this session already loaded, as in the terminal.
    pub async fn protocol_links(&self, txid: String) -> Outcome<LinkReport> {
        let carrier = self
            .settle(self.client.get_raw_transaction(txid.trim()).await)?
            .result;
        let system = self
            .config
            .chain
            .system_id
            .get()
            .cloned()
            .ok_or_else(|| fail("The system ID of the chain is not known."))?;
        remember_transaction(&system, &carrier);
        let mut links = crate::relationships::transaction_links(&system, &carrier);
        let mut reverse_unavailable = None;
        match &self.cache {
            Some(cache) => match cache
                .reverse_links(self.config.chain_cache_key(), txid.trim().to_string())
                .await
            {
                Ok(found) => links.extend(found),
                Err(error) => {
                    reverse_unavailable = Some(format!("The cache did not answer: {error:#}"))
                }
            },
            None => {
                reverse_unavailable = Some(
                    self.cache_problem
                        .clone()
                        .unwrap_or_else(|| "The cache is not open.".into()),
                )
            }
        }
        links.extend(session_reverse_links(&system, txid.trim()));
        let links = sorted_links(links);
        let reverse_count = links
            .iter()
            .filter(|link| link.field.starts_with("reverse:"))
            .count() as u32;

        let mut counterparts: HashMap<(String, String), Option<Value>> = HashMap::new();
        for link in &links {
            let key = (link.target.system_id.to_string(), link.target.txid.clone());
            if counterparts.contains_key(&key) {
                continue;
            }
            let found = match self.client_for(&link.target.system_id) {
                Some(client) => client
                    .get_raw_transaction(&link.target.txid)
                    .await
                    .ok()
                    .map(|output| output.result),
                None => None,
            };
            if let Some(found) = &found {
                remember_transaction(&link.target.system_id, found);
            }
            counterparts.insert(key, found);
        }
        self.forgive();
        let records = links
            .iter()
            .map(|link| {
                let reverse = link.field.starts_with("reverse:");
                let status = if self.client_for(&link.target.system_id).is_none() {
                    "No connection configured"
                } else {
                    let target = counterparts
                        .get(&(link.target.system_id.to_string(), link.target.txid.clone()))
                        .and_then(Option::as_ref);
                    crate::relationships::link_status(
                        link,
                        &system,
                        &carrier,
                        &link.target.system_id,
                        target,
                    )
                };
                protocol_link(link, reverse, Some(status))
            })
            .collect();
        Ok(LinkReport {
            links: records,
            reverse_count,
            reverse_unavailable,
        })
    }

    /// Follows a transfer from one output through export, import and its result, using
    /// Carbide's DeFi trail. While it runs, `trace_progress(token)` returns the trail so far.
    pub async fn trace_transfer(
        &self,
        txid: String,
        output: u32,
        plan: TracePlan,
        token: u64,
    ) -> Outcome<TransferTrace> {
        let seed = self
            .settle(self.client.get_raw_transaction(txid.trim()).await)?
            .result;
        let system_id = self
            .config
            .chain
            .system_id
            .get()
            .cloned()
            .ok_or_else(|| fail("The system ID of the chain is not known."))?;
        let root = OutpointRef {
            system_id,
            txid: txid.trim().to_string(),
            vout: output,
        };
        let plan = crate::defi::SearchPlan {
            request: match plan.request {
                TraceRequest::Initial => crate::defi::ImportSearch::Initial,
                TraceRequest::WindowEndingAt { height } => {
                    crate::defi::ImportSearch::WindowEndingAt(height)
                }
                TraceRequest::BackwardFrom { height } => {
                    crate::defi::ImportSearch::BackwardFrom(height)
                }
            },
            target: plan.target.as_ref().and_then(Outpoint::reference),
            known_imports: plan
                .known_imports
                .iter()
                .filter_map(|known| Some((known.root.reference()?, known.height)))
                .collect(),
        };
        let seed_copy = seed.clone();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let relay = tokio::spawn(async move {
            while let Some(trail) = receiver.recv().await {
                progress_slots().insert(token, transfer_trace(trail));
            }
        });
        let trail =
            crate::defi::investigate(self.all_configs(), root, seed, plan, Some(sender)).await;
        let _ = relay.await;
        leg_slots().insert(format!("{}:{output}", txid.trim()), trail.legs.clone());
        progress_slots().remove(&token);
        let mut trace = transfer_trace(trail);
        self.place_moments(&mut trace, &seed_copy).await;
        self.forgive();
        Ok(trace)
    }

    /// The trail so far for a running `trace_transfer`. None before the first report.
    pub fn trace_progress(&self, token: u64) -> Option<TransferTrace> {
        progress_slots().get(&token).cloned()
    }

    /// Compares a pool's state at an import with the linked previous pool notarization.
    /// `leg` is a leg of the last trace of this output. The difference covers the whole
    /// pool, so it includes other transfers and fees.
    pub async fn pool_comparison(
        &self,
        txid: String,
        output: u32,
        leg: u32,
    ) -> Outcome<PoolComparison> {
        let summary = leg_slots()
            .get(&format!("{}:{output}", txid.trim()))
            .and_then(|legs| legs.get(leg as usize).cloned())
            .ok_or_else(|| fail("Trace the transfer again. The leg is no longer available."))?;
        let comparison = crate::defi::pool::inspect(self.all_configs(), summary).await?;
        self.forgive();
        Ok(pool_changes(&comparison))
    }

    /// Who signed the notarization evidence in one output, using Carbide's witness
    /// investigation. It reports reachable evidence; it does not verify signatures or quorum.
    /// `refresh` requests every reachable transaction again instead of reusing answers.
    pub async fn witnesses(
        &self,
        txid: String,
        output: u32,
        refresh: bool,
    ) -> Outcome<WitnessReport> {
        let seed = self.settle(self.client.get_raw_transaction(txid.trim()).await)?;
        let system_id = self
            .config
            .chain
            .system_id
            .get()
            .cloned()
            .ok_or_else(|| fail("The system ID of the chain is not known."))?;
        let root = OutpointRef {
            system_id,
            txid: txid.trim().to_string(),
            vout: output,
        };
        let report = crate::witnesses::investigate(
            self.config.noninteractive(),
            root.clone(),
            seed.result,
            seed.source_id,
            refresh,
        )
        .await;
        self.forgive();
        Ok(witness_report(&root, report))
    }

    /// `id` is a height or a block hash.
    pub async fn block(&self, id: String) -> Outcome<BlockDetail> {
        let selector = match id.trim().parse::<u64>() {
            Ok(height) => BlockId::Height(height),
            Err(_) => BlockId::Hash(id.trim().to_string()),
        };
        let params = [block_selector(&selector)];
        let mut output = self
            .observed(
                "getblock",
                &params,
                fetch_block(&self.primary, selector).await,
            )
            .await?;
        let mut raw = result_of(&output.response_json);

        // The block lists transaction IDs. Fetch the bodies, which carry the
        // tags and totals, for as many as is reasonable in one view.
        let mut tasks = tokio::task::JoinSet::new();
        for (index, tx) in output
            .result
            .tx
            .iter()
            .take(MAX_BLOCK_TX_BODIES)
            .enumerate()
        {
            let client = self.client.clone();
            let txid = tx.txid.clone();
            tasks.spawn(async move { (index, client.get_raw_transaction(&txid).await) });
        }
        let mut bodies_loaded = 0;
        while let Some(joined) = tasks.join_next().await {
            let (index, body) = joined.context("transaction request stopped")?;
            if let Ok(body) = self.settle(body) {
                output.result.tx[index].raw = Some(body.result);
                bodies_loaded += 1;
            }
        }
        let block = &output.result;
        let transactions: Vec<TransactionRow> = block
            .tx
            .iter()
            .map(|tx| TransactionRow {
                txid: tx.txid.clone(),
                tags: labels(tx),
                input_count: count(tx.raw.as_ref().map(|raw| &raw["vin"])),
                output_count: count(tx.raw.as_ref().map(|raw| &raw["vout"])),
                output_sats: tx.raw.as_ref().and_then(|raw| {
                    raw["vout"]
                        .as_array()
                        .map(|outputs| outputs.iter().filter_map(sats).sum())
                }),
            })
            .collect();
        // Tags that need source transactions use only bodies this block or this session
        // already loaded. No request is made for them.
        let session = self.config.chain.system_id.get().map(|system| {
            for tx in &block.tx {
                if let Some(raw) = &tx.raw {
                    remember_transaction(system, raw);
                }
            }
            session_transactions(system)
        });
        let combined_tags = combined_block_tags(
            &block
                .tx
                .iter()
                .filter_map(|tx| tx.raw.as_ref())
                .map(|raw| {
                    let context = loaded_context(
                        raw,
                        session.as_ref().unwrap_or(&HashMap::new()),
                        self.config.chain.system_id.get().map(ToString::to_string),
                    );
                    TransactionRow {
                        txid: String::new(),
                        tags: display_tags(raw, &context),
                        input_count: 0,
                        output_count: 0,
                        output_sats: None,
                    }
                })
                .collect::<Vec<_>>(),
        );
        let mut fields = Vec::new();
        for (label, key) in [
            ("Merkle root", "merkleroot"),
            ("Final Sapling root", "finalsaplingroot"),
            ("Difficulty", "difficulty"),
            ("Bits", "bits"),
            ("Nonce", "nonce"),
            ("Version", "version"),
            ("Chain work", "chainwork"),
            ("Chain stake", "chainstake"),
            ("Segment ID", "segid"),
        ] {
            if let Some(value) = display(&raw[key]) {
                fields.push(Field {
                    label: label.into(),
                    value,
                });
            }
        }
        let confirmations = raw["confirmations"].as_i64();
        let longest = self
            .settle(self.client.get_info().await)
            .ok()
            .and_then(|info| info.result["longestchain"].as_u64());
        let chain_status = block_chain_status(confirmations, block.height, longest);
        let on_active_chain = confirmations.is_some_and(|count| count > 0);
        let active_hash = if confirmations.is_some_and(|count| count < 0) {
            self.settle(
                self.client
                    .call_with_raw::<String>("getblockhash", &[Value::from(block.height)])
                    .await,
            )
            .ok()
            .map(|output| output.result)
            .filter(|hash| hash != &block.hash)
        } else {
            None
        };
        if let Some(list) = raw.get_mut("tx").and_then(Value::as_array_mut) {
            for tx in list {
                if let Some(txid) = tx["txid"].as_str() {
                    *tx = Value::from(txid);
                }
            }
        }
        Ok(BlockDetail {
            row: block_row(block),
            previous_hash: block.previousblockhash.clone(),
            next_hash: block.nextblockhash.clone(),
            confirmations,
            fields,
            transactions,
            json: pretty(&raw),
            production: block.production_label(),
            bodies_loaded,
            combined_tags,
            chain_status: chain_status.map(str::to_string),
            on_active_chain,
            active_hash,
            provenance: output.provenance.clone(),
        })
    }

    /// Links and dates for the values in a JSON document. Heights are dated from the
    /// chain they belong to: recorded times for past blocks and estimates for future ones.
    /// `dates` false skips the date requests.
    pub async fn json_marks(&self, json: String, dates: bool) -> Vec<JsonMark> {
        let Ok(root) = serde_json::from_str::<Value>(&json) else {
            return Vec::new();
        };
        let mut marks = json_links(&root);
        if !dates {
            return marks;
        }
        let targets = date_targets(&root);
        let own = self.config.chain.system_id.get().map(ToString::to_string);
        let mut wanted: BTreeMap<(String, u64), Option<(u64, bool)>> = BTreeMap::new();
        for (_, target) in &targets {
            if let DateTarget::Height { system, height } = target {
                let system = if system.is_empty() {
                    own.clone().unwrap_or_default()
                } else {
                    system.clone()
                };
                if wanted.len() < MAX_JSON_DATES {
                    wanted.entry((system, *height)).or_insert(None);
                }
            }
        }
        self.date_heights(&mut wanted).await;
        self.forgive();
        for (pointer, target) in targets {
            let date = match target {
                DateTarget::Time(time) => Some((time, false)),
                DateTarget::Height { system, height } => {
                    let system = if system.is_empty() {
                        own.clone().unwrap_or_default()
                    } else {
                        system
                    };
                    wanted.get(&(system, height)).copied().flatten()
                }
            };
            let Some((time, estimated)) = date else {
                continue;
            };
            match marks.iter_mut().find(|mark| mark.pointer == pointer) {
                Some(mark) => {
                    mark.date = Some(time);
                    mark.estimated = estimated;
                }
                None => marks.push(JsonMark {
                    pointer,
                    link: None,
                    target: None,
                    nameable: false,
                    date: Some(time),
                    estimated,
                }),
            }
        }
        marks
    }
}

impl Explorer {
    /// A client for a configured chain with this system ID.
    fn client_for(&self, system: &SystemId) -> Option<RpcClient> {
        if self.config.chain.system_id.get() == Some(system) {
            return Some(self.client.clone());
        }
        let config = self
            .other_chains
            .iter()
            .find(|config| config.chain.system_id.get() == Some(system))?;
        RpcClient::for_config(&config.noninteractive())
            .ok()
            .map(RpcClient::noninteractive)
    }

    /// The source transactions of the inputs, and the marketplace facts that need them.
    async fn context(&self, raw: &Value) -> Context {
        let mut wanted: Vec<String> = Vec::new();
        for input in raw["vin"].as_array().into_iter().flatten() {
            if let Some(txid) = text(&input["txid"])
                && !wanted.contains(&txid)
                && wanted.len() < MAX_SOURCE_TRANSACTIONS
            {
                wanted.push(txid);
            }
        }
        let mut context = Context {
            native: self.config.chain.system_id.get().map(ToString::to_string),
            ..Context::default()
        };
        let fetched = self.fetch_transactions(&wanted).await;
        if let Some(system) = self.config.chain.system_id.get() {
            for source in fetched.values() {
                remember_transaction(system, source);
            }
        }
        context.sources = fetched;
        context.complete = wanted.len()
            == raw["vin"].as_array().map_or(0, |inputs| {
                inputs
                    .iter()
                    .filter_map(|input| text(&input["txid"]))
                    .collect::<HashSet<_>>()
                    .len()
            });

        // Marketplace: an input that spends output 0 of an offer's funding transaction
        // closes or takes the offer. The offer is proven by the proof transaction that
        // spends output 1 of the funding transaction.
        let mut proofs = Vec::new();
        for source in context.sources.values() {
            if marketplace_funding_shape(source)
                && let Some(proof) = text(&source["vout"][1]["spentTxId"])
            {
                proofs.push(proof);
            }
        }
        // A proof in this transaction names its funding transactions through output 1.
        if contains_marketplace_offer_proof(raw) {
            context.proof_here = raw["vin"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|input| input["vout"].as_u64() == Some(1))
                .filter_map(|input| text(&input["txid"]))
                .any(|funding| {
                    context
                        .sources
                        .get(&funding)
                        .is_some_and(marketplace_funding_shape)
                });
        }
        proofs.sort();
        proofs.dedup();
        proofs.truncate(8);
        for (txid, proof) in self.fetch_transactions(&proofs).await {
            context.proofs.insert(txid, proof);
        }
        context
    }

    async fn fetch_transactions(&self, txids: &[String]) -> HashMap<String, Value> {
        let mut tasks = tokio::task::JoinSet::new();
        for txid in txids {
            let client = self.client.clone();
            let txid = txid.clone();
            tasks.spawn(async move { (txid.clone(), client.get_raw_transaction(&txid).await) });
        }
        let mut found = HashMap::new();
        while let Some(Ok((txid, result))) = tasks.join_next().await {
            if let Ok(output) = self.settle(result) {
                found.insert(txid, output.result);
            }
        }
        found
    }

    /// Coinbase maturity for this chain: 100 on Verus, else from the chain's block time.
    async fn maturity(&self, chain: &crate::chain_state::ChainState, raw: &Value) -> Option<u64> {
        let coinbase = raw["vin"]
            .as_array()
            .is_some_and(|inputs| inputs.iter().any(|input| input.get("coinbase").is_some()));
        let name = chain
            .chain_name
            .clone()
            .unwrap_or_else(|| self.config.chain.name.clone())
            .trim_end_matches('@')
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if matches!(name.as_str(), "vrsc" | "vrsctest") {
            return Some(100);
        }
        if !coinbase {
            return None;
        }
        let id = chain.chain_id.clone()?;
        let definition = self
            .settle(self.client.get_currency(&id).await)
            .ok()?
            .result;
        crate::chain_state::coinbase_maturity(
            definition["blocktime"].as_u64()?,
            name == "chips777.vrsctest",
        )
    }

    /// The status line Carbide shows for a notarization opened from the notarization
    /// browser, here for each notarization output of the transaction.
    async fn notarization_context(&self, raw: &Value, observer: &str) -> Vec<String> {
        let mut lines = Vec::new();
        let txid = raw["txid"].as_str().unwrap_or_default();
        for (index, output) in raw["vout"].as_array().into_iter().flatten().enumerate() {
            if lines.len() >= MAX_NOTARIZATION_CONTEXTS {
                break;
            }
            let script = &output["scriptPubKey"];
            let Some(currency) = script
                .get("earnednotarization")
                .or_else(|| script.get("acceptednotarization"))
                .and_then(|notarization| text(&notarization["currencyid"]))
            else {
                continue;
            };
            let vout = output["n"].as_u64().unwrap_or(index as u64);
            let data = self
                .settle(
                    self.client
                        .call_with_raw::<Value>("getnotarizationdata", &[Value::from(currency)])
                        .await,
                )
                .ok()
                .map(|output| output.result);
            lines.push(format!(
                "Notarization output {vout}: {} in {observer} view",
                notarization_status(data.as_ref(), txid, vout)
            ));
        }
        lines
    }

    /// Fills each wanted (system, height) with a time and whether it is an estimate.
    async fn date_heights(&self, wanted: &mut BTreeMap<(String, u64), Option<(u64, bool)>>) {
        let own = self.config.chain.system_id.get().map(ToString::to_string);
        let mut configs: HashMap<String, Config> = HashMap::new();
        for config in self.all_configs() {
            if let Some(id) = config.chain.system_id.get() {
                configs.entry(id.to_string()).or_insert(config);
            }
        }
        let systems: BTreeSet<String> = wanted.keys().map(|(system, _)| system.clone()).collect();
        for system in systems {
            let Some(config) = configs.get(&system) else {
                continue;
            };
            let client = if Some(&system) == own.as_ref() {
                self.client.clone()
            } else {
                match RpcClient::for_config(&config.noninteractive()) {
                    Ok(client) => client.noninteractive(),
                    Err(_) => continue,
                }
            };
            let tip = self
                .settle(client.get_info().await)
                .ok()
                .and_then(|info| info.result["blocks"].as_u64());
            let heights: Vec<u64> = wanted
                .keys()
                .filter(|(key, _)| key == &system)
                .map(|(_, height)| *height)
                .collect();
            let mut calibration = None;
            let mut tasks = tokio::task::JoinSet::new();
            for height in heights {
                if tip.is_some_and(|tip| height > tip) {
                    if calibration.is_none() {
                        calibration = Some(crate::block_forecast::calibrate(config).await.ok());
                    }
                    if let Some(Some(calibration)) = &calibration
                        && let Ok(time) = calibration.time_at_height(height)
                    {
                        wanted.insert((system.clone(), height), Some((time.max(0) as u64, true)));
                    }
                    continue;
                }
                let client = client.clone();
                tasks.spawn(async move {
                    let block = fetch_block(&client, BlockId::Height(height)).await;
                    (height, block.ok().map(|output| output.result.time))
                });
            }
            while let Some(Ok((height, time))) = tasks.join_next().await {
                if let Some(time) = time {
                    wanted.insert((system.clone(), height), Some((time, false)));
                }
            }
        }
    }
}

/// The legs of the last trace of each output, for `pool_comparison`.
fn leg_slots() -> std::sync::MutexGuard<'static, HashMap<String, Vec<crate::defi::LegSummary>>> {
    static SLOTS: LazyLock<Mutex<HashMap<String, Vec<crate::defi::LegSummary>>>> =
        LazyLock::new(Default::default);
    SLOTS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Transactions this process has loaded, by system and transaction ID: the app's version of
/// the terminal's session store, which its reverse reference scan reads.
const MAX_SESSION_TRANSACTIONS: usize = 4096;

fn session_slots() -> std::sync::MutexGuard<
    'static,
    (
        HashMap<(String, String), Value>,
        std::collections::VecDeque<(String, String)>,
    ),
> {
    static SLOTS: LazyLock<
        Mutex<(
            HashMap<(String, String), Value>,
            std::collections::VecDeque<(String, String)>,
        )>,
    > = LazyLock::new(Default::default);
    SLOTS.lock().unwrap_or_else(|e| e.into_inner())
}

fn remember_transaction(system: &SystemId, raw: &Value) {
    let Some(txid) = raw["txid"].as_str() else {
        return;
    };
    let key = (system.to_string(), txid.to_string());
    let mut slots = session_slots();
    let (store, order) = &mut *slots;
    if store.insert(key.clone(), raw.clone()).is_none() {
        order.push_back(key);
        while order.len() > MAX_SESSION_TRANSACTIONS {
            if let Some(oldest) = order.pop_front() {
                store.remove(&oldest);
            }
        }
    }
}

#[cfg(test)]
fn forget_session() {
    let mut slots = session_slots();
    slots.0.clear();
    slots.1.clear();
}

/// The loaded transactions of one system, by transaction ID.
fn session_transactions(system: &SystemId) -> HashMap<String, Value> {
    session_slots()
        .0
        .iter()
        .filter(|((id, _), _)| id == system.as_ref())
        .map(|((_, txid), raw)| (txid.clone(), raw.clone()))
        .collect()
}

/// Port of the scan in `session_relationships` in `src/app.rs`: every loaded transaction on
/// every system that references the target. It never starts a scan or a request.
fn session_reverse_links(system: &SystemId, txid: &str) -> Vec<crate::relationships::Link> {
    let carriers: Vec<(String, Value)> = session_slots()
        .0
        .iter()
        .filter(|((carrier_system, carrier), _)| {
            !(carrier_system == system.as_ref() && carrier == txid)
        })
        .map(|((carrier_system, _), raw)| (carrier_system.clone(), raw.clone()))
        .collect();
    let mut links = Vec::new();
    for (carrier_system, raw) in carriers {
        let Ok(carrier_system) = SystemId::try_from(carrier_system) else {
            continue;
        };
        links.extend(crate::relationships::reverse_links(
            system,
            txid,
            &carrier_system,
            &raw,
        ));
    }
    links
}

/// A context for tags from transactions that are already loaded, without requests.
fn loaded_context(raw: &Value, loaded: &HashMap<String, Value>, native: Option<String>) -> Context {
    let mut context = Context {
        native,
        ..Context::default()
    };
    for input in raw["vin"].as_array().into_iter().flatten() {
        if let Some(txid) = input["txid"].as_str()
            && let Some(source) = loaded.get(txid)
        {
            context.sources.insert(txid.to_string(), source.clone());
            if let Some(proof) = source["vout"][1]["spentTxId"].as_str()
                && let Some(proof_raw) = loaded.get(proof)
            {
                context.proofs.insert(proof.to_string(), proof_raw.clone());
            }
        }
    }
    context.proof_here = contains_marketplace_offer_proof(raw)
        && raw["vin"].as_array().into_iter().flatten().any(|input| {
            input["vout"].as_u64() == Some(1)
                && input["txid"]
                    .as_str()
                    .and_then(|txid| context.sources.get(txid))
                    .is_some_and(marketplace_funding_shape)
        });
    context.complete = raw["vin"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|input| input["txid"].as_str())
        .all(|txid| context.sources.contains_key(txid));
    context
}

fn progress_slots() -> std::sync::MutexGuard<'static, HashMap<u64, TransferTrace>> {
    static SLOTS: LazyLock<Mutex<HashMap<u64, TransferTrace>>> = LazyLock::new(Default::default);
    SLOTS.lock().unwrap_or_else(|e| e.into_inner())
}

// ------------------------------------------------------- transaction overview

/// What the terminal's object store supplies to the overview, for one transaction.
#[derive(Debug, Default)]
struct Context {
    /// The chain's own currency ID.
    native: Option<String>,
    /// Source transactions of the inputs, by transaction ID.
    sources: HashMap<String, Value>,
    /// All source transactions were found.
    complete: bool,
    /// Proof transactions of marketplace offers funded by a source transaction.
    proofs: HashMap<String, Value>,
    /// This transaction is an offer proof for a funding transaction it spends.
    proof_here: bool,
}

impl Context {
    /// Port of `resolved_prevout` in `src/app.rs`.
    fn prevout(&self, input: &Value) -> Option<&Value> {
        let txid = input.get("txid")?.as_str()?;
        let index = usize::try_from(input.get("vout")?.as_u64()?).ok()?;
        self.sources.get(txid)?.get("vout")?.as_array()?.get(index)
    }

    /// A source transaction is an offer when a proof transaction proves it.
    /// Port of `ObjectStore::marketplace_offer` without the `getoffers` listing.
    fn offer(&self, funding: &Value) -> bool {
        marketplace_funding_shape(funding)
            && funding["vout"][1]["spentTxId"]
                .as_str()
                .and_then(|proof| self.proofs.get(proof))
                .is_some_and(contains_marketplace_offer_proof)
    }

    /// Port of `ObjectStore::marketplace_tags` for the transaction being shown.
    fn marketplace_tags(&self, raw: &Value) -> Vec<MarketplaceTag> {
        let mut tags = Vec::new();
        let txid = raw["txid"].as_str();
        if self.proof_here {
            tags.push(MarketplaceTag::OfferProof);
        }
        let spends_offer = raw["vin"].as_array().into_iter().flatten().any(|input| {
            input["vout"].as_u64() == Some(0)
                && input["txid"]
                    .as_str()
                    .and_then(|funding| self.sources.get(funding))
                    .is_some_and(|funding| {
                        funding["vout"][0]["spentTxId"].as_str() == txid && self.offer(funding)
                    })
        });
        if spends_offer {
            let count = raw["vin"].as_array().map_or(0, Vec::len);
            tags.push(if count > 1 {
                MarketplaceTag::OfferTaken
            } else {
                MarketplaceTag::OfferClosed
            });
        }
        tags
    }
}

/// Port of `marketplace_funding_shape` in `src/app.rs`.
fn marketplace_funding_shape(raw: &Value) -> bool {
    let Some(script) = raw["vout"][0]["scriptPubKey"].as_object() else {
        return false;
    };
    script.contains_key("identityprimary") || script.contains_key("commitmenthash")
}

/// Port of `contains_marketplace_offer_proof` in `src/app.rs`.
fn contains_marketplace_offer_proof(raw: &Value) -> bool {
    raw["vout"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|output| output["scriptPubKey"]["hex"].as_str())
        .any(op_return_starts_with_transaction_proof)
}

/// Port of `op_return_starts_with_transaction_proof` in `src/app.rs`.
fn op_return_starts_with_transaction_proof(script_hex: &str) -> bool {
    script_hex
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("6a"))
        && extract_op_return_payload(script_hex)
            .is_some_and(|payload| payload.starts_with(&[5, 0, 0, 0, 3]))
}

/// Port of `extract_op_return_payload` in `src/app.rs`.
fn extract_op_return_payload(script_hex: &str) -> Option<Vec<u8>> {
    let script = decode_hex(script_hex)?;
    if script.first() != Some(&0x6a) {
        return None;
    }
    let (&opcode, remainder) = script.get(1..).and_then(|value| value.split_first())?;
    let (size, offset): (usize, usize) = match opcode {
        0x01..=0x4b => (usize::from(opcode), 0),
        0x4c => (usize::from(*remainder.first().unwrap_or(&0)), 1),
        0x4d if remainder.len() >= 2 => (
            usize::from(u16::from_le_bytes([remainder[0], remainder[1]])),
            2,
        ),
        _ => return None,
    };
    Some(remainder.get(offset..offset.saturating_add(size))?.to_vec())
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(value.get(index..index + 2)?, 16).ok())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Port of `stakeguard_enforcement` in `src/app.rs`.
fn stakeguard_enforcement(raw: &Value, context: &Context) -> bool {
    contains_stakeguard_cheat_evidence(raw)
        && raw["vin"].as_array().into_iter().flatten().any(|input| {
            context
                .prevout(input)
                .is_some_and(|output| output["scriptPubKey"].get("stakeguard").is_some())
        })
}

/// Port of `contains_stakeguard_cheat_evidence` in `src/app.rs`.
fn contains_stakeguard_cheat_evidence(raw: &Value) -> bool {
    let Some(hex) = raw["vout"]
        .as_array()
        .and_then(|outputs| outputs.last())
        .and_then(|output| output["scriptPubKey"]["hex"].as_str())
    else {
        return false;
    };
    let Some(inner) = extract_op_return_payload(hex) else {
        return false;
    };
    inner.first() == Some(&0x53) && inner.get(1).is_some_and(|op| matches!(op, 0x01..=0x4e))
}

/// Port of `import_auto_arbitrage_input_indexes` in `src/app.rs`.
fn auto_arbitrage_inputs(raw: &Value, context: &Context) -> HashSet<usize> {
    let mut indexes = HashSet::new();
    let mut found_import = false;
    for (index, input) in raw["vin"].as_array().into_iter().flatten().enumerate() {
        let Some(prevout) = context.prevout(input) else {
            continue;
        };
        if prevout.pointer("/scriptPubKey/crosschainimport").is_some() {
            found_import = true;
            continue;
        }
        if found_import
            && prevout
                .pointer("/scriptPubKey/acceptednotarization")
                .is_some()
        {
            break;
        }
        if found_import
            && prevout
                .pointer("/scriptPubKey/reservetransfer/arbitrageonly")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        {
            indexes.insert(index);
        }
    }
    indexes
}

/// Port of `decimal_value_to_zat` in `src/app.rs`.
fn decimal_value_to_zat(value: &Value) -> Option<i128> {
    if let Some(value) = value.as_str() {
        return decimal_string_to_zat(value);
    }
    if value.is_number() {
        return decimal_string_to_zat(&value.to_string());
    }
    value
        .get("valueZat")
        .and_then(json_i128)
        .or_else(|| value.get("value").and_then(decimal_value_to_zat))
        .or_else(|| value.get("amount").and_then(decimal_value_to_zat))
}

/// Port of `decimal_string_to_zat` in `src/app.rs`.
fn decimal_string_to_zat(value: &str) -> Option<i128> {
    let value = value.trim();
    let (mantissa, exponent) = value
        .split_once(['e', 'E'])
        .map_or(Some((value, 0)), |(m, e)| Some((m, e.parse::<i32>().ok()?)))?;
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
        let divisor = 10_i128.checked_pow(u32::try_from(scale.checked_neg()?).ok()?)?;
        if digits % divisor != 0 {
            return None;
        }
        digits / divisor
    };
    Some(if negative { -amount } else { amount })
}

fn json_i128(value: &Value) -> Option<i128> {
    value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_u64().map(i128::from))
}

/// Port of `native_value_zat` in `src/app.rs`.
fn native_value_zat(value: &Value) -> Option<i128> {
    ["valueZat", "valueSat", "satoshis"]
        .iter()
        .find_map(|key| value.get(key).and_then(json_i128))
}

/// Port of `format_native_zat` in `src/app.rs`.
fn format_native_zat(zat: i128) -> String {
    let sign = if zat < 0 { "-" } else { "" };
    let magnitude = zat.unsigned_abs();
    format!(
        "{sign}{}.{:08}",
        magnitude / 100_000_000,
        magnitude % 100_000_000
    )
}

fn coins(zat: i128) -> f64 {
    zat as f64 / 100_000_000.0
}

fn clamp_sats(zat: i128) -> Option<i64> {
    i64::try_from(zat).ok()
}

fn case_insensitive_field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value
        .as_object()?
        .iter()
        .find_map(|(candidate, value)| candidate.eq_ignore_ascii_case(key).then_some(value))
}

fn case_insensitive_array<'a>(value: &'a Value, key: &str) -> Option<&'a Vec<Value>> {
    case_insensitive_field(value, key).and_then(Value::as_array)
}

fn shielded_amount_field(value: &Value, zat_key: &str, decimal_key: &str) -> Option<i128> {
    case_insensitive_field(value, zat_key)
        .map(json_i128)
        .unwrap_or_else(|| {
            case_insensitive_field(value, decimal_key).and_then(decimal_value_to_zat)
        })
}

/// Port of `ShieldedAccounting` in `src/app.rs`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ShieldedAccounting {
    sapling_present: bool,
    sapling_value_balance_zat: i128,
    sapling_complete: bool,
    sprout_present: bool,
    sprout_complete: bool,
    sprout_to_transparent_zat: i128,
    transparent_to_sprout_zat: i128,
    complete: bool,
}

impl ShieldedAccounting {
    fn from_raw(raw: &Value) -> Self {
        let sapling_fields_valid = ["vshieldedspend", "vshieldedoutput"]
            .iter()
            .all(|key| case_insensitive_field(raw, key).is_none_or(Value::is_array));
        let sprout_fields_valid =
            case_insensitive_field(raw, "vjoinsplit").is_none_or(Value::is_array);
        let sapling_present = !sapling_fields_valid
            || case_insensitive_array(raw, "vshieldedspend").is_some_and(|items| !items.is_empty())
            || case_insensitive_array(raw, "vshieldedoutput")
                .is_some_and(|items| !items.is_empty());
        let sapling_value = case_insensitive_field(raw, "valueBalanceZat")
            .map(json_i128)
            .or_else(|| case_insensitive_field(raw, "valueBalance").map(decimal_value_to_zat))
            .unwrap_or(if sapling_present { None } else { Some(0) })
            .filter(|value| (i128::from(i64::MIN)..=i128::from(i64::MAX)).contains(value));

        let joinsplits = case_insensitive_array(raw, "vjoinsplit");
        let sprout_present =
            !sprout_fields_valid || joinsplits.is_some_and(|items| !items.is_empty());
        let mut sprout_to_transparent_zat = 0_i128;
        let mut transparent_to_sprout_zat = 0_i128;
        let mut sprout_complete = sprout_fields_valid;
        for joinsplit in joinsplits.into_iter().flatten() {
            let vpub_new = shielded_amount_field(joinsplit, "vpub_newZat", "vpub_new");
            let vpub_old = shielded_amount_field(joinsplit, "vpub_oldZat", "vpub_old");
            match (vpub_new, vpub_old) {
                (Some(vpub_new), Some(vpub_old)) if vpub_new >= 0 && vpub_old >= 0 => {
                    sprout_to_transparent_zat = sprout_to_transparent_zat
                        .checked_add(vpub_new)
                        .unwrap_or_else(|| {
                            sprout_complete = false;
                            0
                        });
                    transparent_to_sprout_zat = transparent_to_sprout_zat
                        .checked_add(vpub_old)
                        .unwrap_or_else(|| {
                            sprout_complete = false;
                            0
                        });
                }
                _ => sprout_complete = false,
            }
        }
        Self {
            sapling_present,
            sapling_value_balance_zat: sapling_value.unwrap_or_default(),
            sapling_complete: sapling_value.is_some() && sapling_fields_valid,
            sprout_present,
            sprout_complete,
            sprout_to_transparent_zat,
            transparent_to_sprout_zat,
            complete: sapling_value.is_some() && sapling_fields_valid && sprout_complete,
        }
    }

    fn present(self) -> bool {
        self.sapling_present || self.sprout_present
    }

    fn net_to_transparent_zat(self) -> Option<i128> {
        if !self.complete {
            return None;
        }
        self.sapling_value_balance_zat
            .checked_add(self.sprout_to_transparent_zat)?
            .checked_sub(self.transparent_to_sprout_zat)
    }
}

/// Port of `pool_transfer_label` in `src/app.rs`.
fn pool_transfer_label(pool: &str, net_to_transparent_zat: i128, currency: &str) -> String {
    match net_to_transparent_zat.cmp(&0) {
        std::cmp::Ordering::Greater => format!(
            "{pool} {} {currency} shielded -> transparent",
            format_native_zat(net_to_transparent_zat)
        ),
        std::cmp::Ordering::Less => format!(
            "{pool} {} {currency} transparent -> shielded",
            format_native_zat(-net_to_transparent_zat)
        ),
        std::cmp::Ordering::Equal => format!("{pool} public net 0 (individual values unknown)"),
    }
}

/// Port of `shielded_accounting_spans` in `src/app.rs`, as plain lines.
fn shielded_lines(accounting: &ShieldedAccounting, currency: &str) -> Vec<String> {
    if !accounting.present() {
        return Vec::new();
    }
    let mut parts = Vec::new();
    if accounting.sapling_present && !accounting.sapling_complete {
        parts.push("Sapling public balance unavailable".to_string());
    } else if accounting.sapling_present {
        parts.push(pool_transfer_label(
            "Sapling",
            accounting.sapling_value_balance_zat,
            currency,
        ));
    }
    if accounting.sprout_present && !accounting.sprout_complete {
        parts.push("Sprout public balance unavailable".to_string());
    } else if accounting.sprout_present {
        if accounting.transparent_to_sprout_zat > 0 {
            parts.push(format!(
                "Sprout {} {currency} transparent -> shielded",
                format_native_zat(accounting.transparent_to_sprout_zat)
            ));
        }
        if accounting.sprout_to_transparent_zat > 0 {
            parts.push(format!(
                "Sprout {} {currency} shielded -> transparent",
                format_native_zat(accounting.sprout_to_transparent_zat)
            ));
        }
        if accounting.transparent_to_sprout_zat == 0 && accounting.sprout_to_transparent_zat == 0 {
            parts.push("Sprout public net 0 (individual values unknown)".to_string());
        }
    }
    if !accounting.complete {
        parts.push("accounting unavailable".to_string());
    }
    parts
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowKind {
    Input,
    Output,
}

/// Port of `TransactionFlow` in `src/app.rs`.
#[derive(Debug, Clone)]
struct Flow {
    kind: FlowKind,
    index: usize,
    addresses: Vec<String>,
    value_zat: Option<i128>,
    reserve_amounts: BTreeMap<String, i128>,
    source_txid: Option<String>,
    source_vout: Option<u64>,
    sequence: Option<u64>,
    script_type: Option<String>,
    spendable: Option<bool>,
    coinbase: bool,
    /// "Sapling" or "Sprout".
    private: Option<&'static str>,
    tags: Vec<OutputTag>,
    annotations: Vec<String>,
    references: crate::protocol::References,
    routing: BTreeMap<String, String>,
}

impl Flow {
    fn private(kind: FlowKind, index: usize, pool: &'static str, value: Option<i128>) -> Self {
        Self {
            kind,
            index,
            addresses: Vec::new(),
            value_zat: value,
            reserve_amounts: BTreeMap::new(),
            source_txid: None,
            source_vout: None,
            sequence: None,
            script_type: Some(pool.to_string()),
            spendable: None,
            coinbase: false,
            private: Some(pool),
            tags: Vec::new(),
            annotations: Vec::new(),
            references: Default::default(),
            routing: BTreeMap::new(),
        }
    }
}

/// Port of `monetary_reserve_amounts` in `src/app.rs`. Only value-bearing token payloads,
/// never embedded import or export accounting.
fn monetary_reserve_amounts(output: &Value, native: Option<&str>) -> BTreeMap<String, i128> {
    let mut amounts = BTreeMap::new();
    for kind in ["reserveoutput", "reservetransfer", "reservedeposit"] {
        let Some(payload) = output.pointer(&format!("/scriptPubKey/{kind}")) else {
            continue;
        };
        // Match CReserveTransfer::TotalCurrencyOut: minted principal has no
        // custody here, while both fee budgets are held in the fee currency.
        let minted = kind == "reservetransfer"
            && payload["flags"]
                .as_u64()
                .is_some_and(|flags| flags & 0x20 != 0);
        if !minted && let Some(values) = payload["currencyvalues"].as_object() {
            for (id, amount) in values {
                if let Some(amount) = decimal_value_to_zat(amount) {
                    *amounts.entry(id.clone()).or_default() += amount;
                }
            }
        }
        if kind == "reservetransfer" {
            let fee_currency = payload["feecurrencyid"].as_str().or(native);
            if let Some(id) = fee_currency {
                if let Some(fees) = decimal_value_to_zat(&payload["fees"]) {
                    *amounts.entry(id.to_owned()).or_default() += fees;
                }
                let destination = &payload["destination"];
                if destination["type"]
                    .as_u64()
                    .is_some_and(|kind| kind & 128 != 0)
                    && destination["gateway"]
                        .as_str()
                        .is_some_and(|id| !id.is_empty())
                    && let Some(fees) = decimal_value_to_zat(&destination["fees"])
                {
                    *amounts.entry(id.to_owned()).or_default() += fees;
                }
            }
        }
    }
    if let Some(native) = native {
        amounts.remove(native);
    }
    amounts
}

/// Port of `transaction_flows` in `src/app.rs`. Registration payment annotations are
/// left to the registration payments card, which reads the same transaction.
fn transaction_flows(raw: &Value, context: &Context, native: Option<&str>) -> Vec<Flow> {
    let mut flows = Vec::new();
    let auto_arbitrage = auto_arbitrage_inputs(raw, context);
    let marketplace = context.marketplace_tags(raw);
    let closes_offer = marketplace.contains(&MarketplaceTag::OfferClosed);
    for (index, input) in raw["vin"].as_array().into_iter().flatten().enumerate() {
        let prevout = input.get("prevout").or_else(|| context.prevout(input));
        let addresses = {
            let direct = crate::protocol::raw_addresses(input);
            if direct.is_empty() {
                prevout
                    .map(crate::protocol::raw_addresses)
                    .unwrap_or_default()
            } else {
                direct
            }
        };
        let value_source = if native_value_zat(input).is_some() || input.get("value").is_some() {
            input
        } else {
            prevout.unwrap_or(input)
        };
        let marketplace_cancellation = input["txid"]
            .as_str()
            .zip(input["vout"].as_u64())
            .is_some_and(|(txid, index)| {
                index == 0
                    && context
                        .sources
                        .get(txid)
                        .is_some_and(|funding| context.offer(funding))
            });
        let mut annotations = Vec::new();
        if marketplace_cancellation {
            annotations.push("Marketplace Offer Cancellation".to_string());
        }
        if auto_arbitrage.contains(&index) {
            annotations.push("Auto-Arbitrage Input".to_string());
        }
        flows.push(Flow {
            kind: FlowKind::Input,
            index,
            addresses,
            value_zat: native_value_zat(value_source),
            reserve_amounts: monetary_reserve_amounts(
                input.get("prevout").or(prevout).unwrap_or(input),
                native,
            ),
            source_txid: text(&input["txid"]),
            source_vout: input["vout"].as_u64(),
            sequence: input["sequence"].as_u64(),
            script_type: input
                .pointer("/prevout/scriptPubKey/type")
                .or_else(|| prevout.and_then(|output| output.pointer("/scriptPubKey/type")))
                .or_else(|| input.get("type"))
                .and_then(Value::as_str)
                .map(str::to_string),
            spendable: None,
            coinbase: input.get("coinbase").is_some(),
            private: None,
            tags: Vec::new(),
            annotations,
            references: Default::default(),
            routing: BTreeMap::new(),
        });
    }
    let transparent_inputs = flows.len();
    let sapling_inputs = case_insensitive_array(raw, "vshieldedspend").map_or(0, Vec::len);
    let sapling_outputs = case_insensitive_array(raw, "vshieldedoutput").map_or(0, Vec::len);
    let accounting = ShieldedAccounting::from_raw(raw);
    let derived_input = (sapling_inputs == 1 && sapling_outputs == 0)
        .then_some(accounting.sapling_value_balance_zat)
        .filter(|value| accounting.sapling_complete && *value >= 0);
    for offset in 0..sapling_inputs {
        let mut flow = Flow::private(
            FlowKind::Input,
            transparent_inputs + offset,
            "Sapling",
            derived_input,
        );
        if derived_input.is_some() {
            flow.annotations
                .push("Derived from Sapling valueBalance".to_string());
        }
        flows.push(flow);
    }
    let sprout_inputs = crate::domain::private_input_count(raw).saturating_sub(sapling_inputs);
    for offset in 0..sprout_inputs {
        flows.push(Flow::private(
            FlowKind::Input,
            transparent_inputs + sapling_inputs + offset,
            "Sprout",
            None,
        ));
    }

    let mut outpoint_links = BTreeMap::<u32, Vec<crate::relationships::Link>>::new();
    if let Some(system) = native.and_then(|id| SystemId::try_from(id.to_string()).ok()) {
        for link in crate::relationships::transaction_links(&system, raw) {
            outpoint_links
                .entry(link.carrier_vout)
                .or_default()
                .push(link);
        }
    }
    let timelock = crate::domain::emission_timelock(raw);
    let stake = crate::domain::is_stake_transaction(raw);
    let enforcement = stakeguard_enforcement(raw, context);
    let market_proof = marketplace.contains(&MarketplaceTag::OfferProof);
    let outputs = raw["vout"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    for (index, output) in outputs.iter().enumerate() {
        let script = output.get("scriptPubKey").unwrap_or(&Value::Null);
        let mut tags = crate::domain::output_tags(output);
        let addresses = crate::protocol::raw_addresses(output);
        let routing = addresses
            .iter()
            .filter_map(|address| {
                crate::protocol::routing_label(script, address)
                    .map(|label| (address.clone(), label))
            })
            .collect();
        let mut references = crate::protocol::references(output);
        for link in outpoint_links
            .get(&(index as u32))
            .into_iter()
            .flatten()
            .take(crate::protocol::MAX_LINKS.saturating_sub(references.links.len()))
        {
            references.links.push(crate::protocol::Link {
                label: link.label.clone(),
                target: crate::protocol::Target::Outpoint(link.target.clone()),
            });
        }
        let mut annotations = Vec::new();
        if closes_offer && script["type"].as_str() != Some("nulldata") {
            annotations.push("Marketplace Offer Refund".to_string());
        }
        if let Some(legacy) = crate::domain::legacy_notarization(output) {
            references.links.push(crate::protocol::Link {
                label: "Notarized VRSC block".into(),
                target: crate::protocol::Target::LegacyNotarizedBlock {
                    hash: legacy.block_hash,
                    height: legacy.height,
                },
            });
            annotations.push(format!(
                "Destination transaction {}",
                legacy.destination_txid
            ));
            annotations.push(format!("Merkle-of-Merkle-roots depth {}", legacy.depth));
            annotations.push(format!("Merkle-of-Merkle-roots {}", legacy.merkle_root));
        } else {
            let known_data = (stake && index == 1)
                || (enforcement && index + 1 == outputs.len())
                || timelock.is_some_and(|lock| index == lock.metadata_output_index)
                || (market_proof
                    && script["hex"]
                        .as_str()
                        .is_some_and(op_return_starts_with_transaction_proof));
            if !known_data && let Some(data) = crate::domain::arbitrary_op_return(output) {
                references.links.push(crate::protocol::Link {
                    label: "View OP_RETURN bytes".into(),
                    target: crate::protocol::Target::OpReturn(data),
                });
            }
        }
        if let Some(lock) = timelock
            && index == lock.locked_output_index
        {
            tags.push(OutputTag {
                kind: OutputTagKind::EmissionTimelock,
                currency_id: None,
            });
            annotations.push(format!("Unlock height {}", lock.unlock_height));
        }
        flows.push(Flow {
            kind: FlowKind::Output,
            index: output["n"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(index),
            addresses,
            value_zat: native_value_zat(output),
            reserve_amounts: monetary_reserve_amounts(output, native),
            source_txid: None,
            source_vout: None,
            sequence: None,
            script_type: text(&script["type"]),
            spendable: script["spendableoutput"].as_bool(),
            coinbase: false,
            private: None,
            tags,
            annotations,
            references,
            routing,
        });
    }
    let transparent_outputs = outputs.len();
    let derived_output = (sapling_outputs == 1 && sapling_inputs == 0)
        .then(|| -accounting.sapling_value_balance_zat)
        .filter(|value| accounting.sapling_complete && *value >= 0);
    for offset in 0..sapling_outputs {
        let mut flow = Flow::private(
            FlowKind::Output,
            transparent_outputs + offset,
            "Sapling",
            derived_output,
        );
        if derived_output.is_some() {
            flow.annotations
                .push("Derived from Sapling valueBalance".to_string());
        }
        if closes_offer {
            flow.annotations
                .push("Marketplace Offer Refund".to_string());
        }
        flows.push(flow);
    }
    let sprout_outputs = crate::domain::private_output_count(raw).saturating_sub(sapling_outputs);
    for offset in 0..sprout_outputs {
        let mut flow = Flow::private(
            FlowKind::Output,
            transparent_outputs + sapling_outputs + offset,
            "Sprout",
            None,
        );
        if closes_offer {
            flow.annotations
                .push("Marketplace Offer Refund".to_string());
        }
        flows.push(flow);
    }
    flows
}

/// Port of `protocol_value_accounting` in `src/app.rs`. These operations can introduce
/// implicit value or converted reserves, which decoded inputs and outputs do not expose.
fn protocol_value_accounting(raw: &Value, context: &Context) -> bool {
    let protocol_output = |output: &Value| {
        output["scriptPubKey"].as_object().is_some_and(|script| {
            [
                "crosschainimport",
                "crosschainexport",
                "currencydefinition",
                "reservetransfer",
                "reservedeposit",
                "feepool",
            ]
            .iter()
            .any(|key| script.contains_key(*key))
        })
    };
    raw["vout"]
        .as_array()
        .is_some_and(|outputs| outputs.iter().any(protocol_output))
        || raw["vin"].as_array().is_some_and(|inputs| {
            inputs.iter().any(|input| {
                context
                    .prevout(input)
                    .or_else(|| input.get("prevout"))
                    .is_some_and(protocol_output)
            })
        })
}

fn sum_known(flows: &[&Flow]) -> Option<i128> {
    flows
        .iter()
        .try_fold(0_i128, |total, flow| Some(total + flow.value_zat?))
}

/// Port of `transaction_side_value_label` in `src/app.rs`.
fn side_total(flows: &[&Flow], currency: &str, coinbase: bool) -> ValueTotal {
    let hidden = flows
        .iter()
        .any(|flow| flow.private.is_some() && flow.value_zat.is_none());
    let derived = flows
        .iter()
        .any(|flow| flow.private.is_some() && flow.value_zat.is_some());
    let known: Vec<&Flow> = flows
        .iter()
        .copied()
        .filter(|flow| flow.value_zat.is_some())
        .collect();
    let total = sum_known(&known);
    let mut reserves = BTreeMap::<String, i128>::new();
    for flow in flows {
        for (id, amount) in &flow.reserve_amounts {
            *reserves.entry(id.clone()).or_default() += amount;
        }
    }
    let label = if coinbase {
        TransactionTag::Coinbase.label().to_owned()
    } else {
        match total {
            None => "unknown".to_string(),
            Some(total) if hidden => {
                format!("{} {currency} known + hidden", format_native_zat(total))
            }
            Some(total) if derived => {
                format!("{} {currency} (partly derived)", format_native_zat(total))
            }
            Some(total) => format!("{} {currency}", format_native_zat(total)),
        }
    };
    ValueTotal {
        label,
        sats: total.and_then(clamp_sats),
        hidden,
        derived,
        coinbase,
        remaining_unknown: flows
            .iter()
            .any(|flow| flow.private.is_none() && !flow.coinbase && flow.value_zat.is_none()),
        reserves: reserves
            .into_iter()
            .filter(|(_, amount)| *amount != 0)
            .map(|(currency, amount)| CurrencyAmount {
                currency,
                amount: coins(amount),
            })
            .collect(),
    }
}

/// Port of `transaction_values_labels` in `src/app.rs`.
fn value_accounting(
    raw: &Value,
    flows: &[Flow],
    context: &Context,
    currency: &str,
) -> ValueAccounting {
    let inputs: Vec<&Flow> = flows.iter().filter(|f| f.kind == FlowKind::Input).collect();
    let outputs: Vec<&Flow> = flows
        .iter()
        .filter(|f| f.kind == FlowKind::Output)
        .collect();
    let coinbase = inputs.iter().any(|input| input.coinbase);
    let transparent_inputs: Vec<&Flow> = inputs
        .iter()
        .copied()
        .filter(|flow| flow.private.is_none())
        .collect();
    let transparent_outputs: Vec<&Flow> = outputs
        .iter()
        .copied()
        .filter(|flow| flow.private.is_none())
        .collect();
    let shielded_net = ShieldedAccounting::from_raw(raw).net_to_transparent_zat();
    let public_delta = sum_known(&transparent_inputs)
        .zip(sum_known(&transparent_outputs))
        .zip(shielded_net)
        .and_then(|((inputs, outputs), net)| inputs.checked_sub(outputs)?.checked_add(net));
    let protocol = protocol_value_accounting(raw, context);
    let (fee_kind, fee_sats, fee_label) = if coinbase {
        (FeeKind::NotApplicable, None, "n/a".to_string())
    } else if protocol {
        (
            FeeKind::ProtocolAccounting,
            None,
            "unknown (protocol accounting)".to_string(),
        )
    } else {
        match public_delta {
            Some(fee) => (
                FeeKind::Known,
                clamp_sats(fee),
                format!("{} {currency}", format_native_zat(fee)),
            ),
            None => (FeeKind::Unknown, None, "unknown".to_string()),
        }
    };
    ValueAccounting {
        input_count: inputs.len() as u32,
        output_count: outputs.len() as u32,
        inputs: side_total(&inputs, currency, coinbase),
        outputs: side_total(&outputs, currency, false),
        fee_kind,
        fee_sats,
        fee_label,
        public_native_delta: public_delta.and_then(clamp_sats),
        show_public_delta: protocol && !coinbase,
    }
}

/// Confirmations as Carbide counts them for the status line: the chain state's count when
/// it has one, the source's own count only when the chain state has no tip.
fn depth_for_status(
    chain: &crate::chain_state::ChainState,
    block_hash: Option<&str>,
    raw_confirmations: Option<i64>,
) -> i64 {
    block_hash
        .and_then(|hash| chain.confirmations(hash))
        .map(|count| count as i64)
        .unwrap_or_else(|| {
            if chain.height.is_none() && chain.reconciled.is_none() {
                raw_confirmations.unwrap_or_default()
            } else {
                0
            }
        })
}

/// Port of the status in `render_transaction_view` in `src/app.rs`. The "confirmed since"
/// form is a display choice and is made by the app from the block time.
fn transaction_status(
    in_mempool: bool,
    confirmations: i64,
    has_block: bool,
    orphaned: bool,
) -> (TransactionStatus, String) {
    if in_mempool {
        (TransactionStatus::Unconfirmed, "unconfirmed".into())
    } else if confirmations > 0 {
        (
            TransactionStatus::Confirmed,
            format!("confirmed ({confirmations})"),
        )
    } else if orphaned {
        (
            TransactionStatus::OrphanedInclusion,
            "orphaned inclusion; current mempool membership unverified".into(),
        )
    } else if has_block {
        (
            TransactionStatus::ConfirmationsUnverified,
            "confirmations unverified".into(),
        )
    } else {
        (TransactionStatus::Unconfirmed, "unconfirmed".into())
    }
}

/// Port of `output_status_badges` in `src/app.rs`. `depth` is the confirmation count, or
/// zero for a transaction in the mempool. `maturity` is the coinbase maturity.
fn output_badges(
    raw: &Value,
    flow: &Flow,
    depth: Option<u64>,
    maturity: Option<u64>,
    height: Option<u64>,
) -> Vec<String> {
    use crate::chain_state::StatusBadge;
    if flow.kind != FlowKind::Output || flow.private.is_some() {
        return Vec::new();
    }
    let Some(output) = raw["vout"].as_array().and_then(|outputs| {
        outputs
            .iter()
            .enumerate()
            .find(|(index, output)| {
                output["n"].as_u64().unwrap_or(*index as u64) == flow.index as u64
            })
            .map(|(_, output)| output)
    }) else {
        return Vec::new();
    };
    let monetary = flow.value_zat.is_some_and(|value| value > 0)
        || output["scriptPubKey"]["reserve_balance"]
            .as_object()
            .is_some_and(|balances| {
                balances
                    .values()
                    .any(|amount| amount.as_f64().is_some_and(|value| value > 0.0))
            });
    if !monetary || flow.script_type.as_deref() == Some("nulldata") {
        return Vec::new();
    }
    let Some(depth) = depth else {
        return Vec::new();
    };
    let coinbase = raw["vin"]
        .as_array()
        .is_some_and(|inputs| inputs.iter().any(|input| input.get("coinbase").is_some()));
    let mut badges = Vec::new();
    // PBaaS block-one allocations bypass ordinary coinbase maturity. Maturity is 100 on
    // Verus itself, so a block-one exception never applies there.
    if coinbase
        && (maturity == Some(100) || height != Some(1))
        && maturity.is_some_and(|maturity| depth < maturity)
    {
        badges.push(StatusBadge::Immature.label().to_string());
    }
    let stake_candidate = flow.value_zat.is_some_and(|value| value > 0)
        && match flow.script_type.as_deref() {
            Some("pubkey" | "pubkeyhash") => flow.spendable != Some(false),
            Some("cryptocondition") => flow.spendable == Some(true),
            _ => false,
        };
    if stake_candidate && depth < STAKE_AGE {
        badges.push(StatusBadge::StakeAgePending.label().to_string());
    }
    badges
}

/// Port of `transaction_tags` in `src/app.rs`: the summary's tags with DeFi tags last,
/// then the tags that need the inputs' sources and the marketplace facts.
fn display_tags(raw: &Value, context: &Context) -> Vec<String> {
    let summary = crate::domain::TransactionSummary {
        txid: raw["txid"].as_str().unwrap_or_default().to_string(),
        raw: Some(raw.clone()),
    };
    let mut tags = summary.tags();
    tags.sort_by_key(|tag| {
        matches!(
            tag,
            TransactionTag::Output(OutputTag {
                kind: OutputTagKind::CurrencyDefinition
                    | OutputTagKind::ReserveTransfer
                    | OutputTagKind::AutoArbitrage
                    | OutputTagKind::Conversion
                    | OutputTagKind::CrossChainTransfer
                    | OutputTagKind::Export
                    | OutputTagKind::Import,
                ..
            })
        )
    });
    if !consumed_token_burns(raw, context).is_empty() {
        tags.push(TransactionTag::ConsumptionBurn);
    }
    if stakeguard_enforcement(raw, context) {
        tags.push(TransactionTag::StakeGuardEnforcement);
    }
    if raw["vout"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|output| output.pointer("/scriptPubKey/crosschainimport").is_some())
        && !auto_arbitrage_inputs(raw, context).is_empty()
    {
        tags.push(TransactionTag::AutoArbitrageImport);
    }
    tags.extend(
        context
            .marketplace_tags(raw)
            .into_iter()
            .map(TransactionTag::Marketplace),
    );
    let mut labels: Vec<String> = Vec::new();
    for tag in tags {
        let label = tag.label().to_string();
        if !labels.contains(&label) {
            labels.push(label);
        }
    }
    labels
}

/// Port of `consumed_token_burns` in `src/app.rs`. A conservative proof for ordinary
/// transparent token consumption. Incomplete previous outputs prove nothing.
fn consumed_token_burns(raw: &Value, context: &Context) -> BTreeMap<String, i128> {
    let prove = || -> Option<BTreeMap<String, i128>> {
        if crate::domain::private_input_count(raw) > 0
            || crate::domain::private_output_count(raw) > 0
        {
            return None;
        }
        let mut totals = BTreeMap::<String, i128>::new();
        let inputs = raw.get("vin")?.as_array()?;
        let outputs = raw.get("vout")?.as_array()?;
        if inputs.is_empty() || outputs.is_empty() {
            return None;
        }
        let mut account = |output: &Value, sign: i128| -> Option<()> {
            let script = output.get("scriptPubKey")?.as_object()?;
            if script.iter().any(|(key, value)| {
                (value.is_object() && !matches!(key.as_str(), "reserveoutput" | "reserve_balance"))
                    || matches!(
                        key.as_str(),
                        "reservetransfer"
                            | "reservedeposit"
                            | "crosschainimport"
                            | "crosschainexport"
                            | "currencydefinition"
                            | "identityprimary"
                            | "identityreservation"
                            | "identityadvancedreservation"
                    )
            }) {
                return None;
            }
            if script.get("type").and_then(Value::as_str) == Some("cryptocondition")
                && !script.get("reserveoutput").is_some_and(Value::is_object)
                && script.get("evalnone").and_then(Value::as_str) != Some("valid")
            {
                return None;
            }
            if let Some(reserve) = script.get("reserveoutput") {
                for (id, value) in reserve.get("currencyvalues")?.as_object()? {
                    let amount = decimal_value_to_zat(value)?;
                    if amount < 0 {
                        return None;
                    }
                    let entry = totals.entry(id.clone()).or_default();
                    *entry = entry.checked_add(amount.checked_mul(sign)?)?;
                }
            }
            Some(())
        };
        for input in inputs {
            if input.get("coinbase").is_some() {
                return None;
            }
            account(context.prevout(input).or_else(|| input.get("prevout"))?, 1)?;
        }
        for output in outputs {
            account(output, -1)?;
        }
        if totals.values().any(|value| *value < 0) {
            return None;
        }
        totals.retain(|id, amount| *amount > 0 && context.native.as_deref() != Some(id.as_str()));
        Some(totals)
    };
    prove().unwrap_or_default()
}

/// Port of `transaction_registration_lines` in `src/app/identity_evidence.rs`. Names are
/// left as IDs; the app shows the name.
fn registration_lines(raw: &Value) -> Vec<String> {
    let Some((reservation, index)) =
        raw["vout"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|output| {
                let script = output.get("scriptPubKey")?;
                let reservation = script
                    .get("identityreservation")
                    .or_else(|| script.get("identityadvancedreservation"))
                    .or_else(|| script.get("identityregistration"))?;
                Some((reservation, output.get("n")?.as_u64()?))
            })
    else {
        return Vec::new();
    };
    let field = |key: &str| text(&reservation[key]);
    let mut lines = vec![format!(
        "Registers {}@ ({}) from reservation output {index}",
        field("name").unwrap_or_else(|| "unknown".into()),
        field("nameid").unwrap_or_else(|| "unknown".into())
    )];
    for (key, label) in [("parent", "Parent"), ("referral", "Referral")] {
        if let Some(value) = field(key) {
            lines.push(format!("{label}: {value}"));
        }
    }
    if let Some(script) = raw["vout"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|output| output["n"].as_u64() == Some(index))
        .map(|output| &output["scriptPubKey"])
    {
        let destinations: Vec<String> = strings(&script["addresses"])
            .into_iter()
            .filter(|address| address.starts_with('R'))
            .collect();
        if !destinations.is_empty() {
            lines.push(format!(
                "Reservation output destination: {}",
                destinations.join(", ")
            ));
        }
    }
    lines
}

/// The status of a notarization outpoint in `getnotarizationdata`. Port of
/// `TransactionContext::status_line` in `src/app/notarization_ui.rs`: only the last
/// confirmed notarization is "confirmed"; the others listed are "unconfirmed".
fn notarization_status(data: Option<&Value>, txid: &str, vout: u64) -> &'static str {
    let Some(data) = data else {
        return "status unavailable";
    };
    let rows = data["notarizations"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let matches = |entry: &Value| {
        entry["txid"].as_str() == Some(txid) && entry["vout"].as_u64() == Some(vout)
    };
    let confirmed = data["lastconfirmed"].as_i64().filter(|index| *index >= 0);
    if let Some(index) = confirmed
        && rows
            .iter()
            .find(|entry| entry["index"].as_i64() == Some(index))
            .or_else(|| rows.get(index as usize))
            .is_some_and(matches)
    {
        return "confirmed";
    }
    if rows.iter().any(matches) {
        "unconfirmed"
    } else {
        "status unavailable"
    }
}

/// Port of `tag_target` in `src/app.rs`.
fn tag_info(flow: &Flow, tag: &OutputTag) -> OutputTagInfo {
    let outpoint_link = |label: Option<&str>| {
        flow.references
            .links
            .iter()
            .find_map(|link| match &link.target {
                crate::protocol::Target::Outpoint(target)
                    if label.is_none_or(|label| link.label == label) =>
                {
                    Some(Outpoint::from(target))
                }
                _ => None,
            })
    };
    let target = match tag.kind {
        OutputTagKind::ExportFinalization(_) => {
            outpoint_link(Some("Export referenced by finalization"))
        }
        OutputTagKind::NotaryEvidence | OutputTagKind::NotarizationFinalization(_) => {
            outpoint_link(None)
        }
        _ => None,
    };
    let traceable = matches!(
        tag.kind,
        OutputTagKind::ReserveTransfer | OutputTagKind::CrossChainTransfer | OutputTagKind::Burn
    ) && flow.kind == FlowKind::Output
        && flow
            .tags
            .iter()
            .any(|tag| tag.kind == OutputTagKind::ReserveTransfer);
    // Port of `display_output_tag`: these tags name their currency.
    let named = matches!(
        tag.kind,
        OutputTagKind::CurrencyDefinition
            | OutputTagKind::AutoArbitrage
            | OutputTagKind::Conversion
            | OutputTagKind::Export
            | OutputTagKind::Import
    );
    OutputTagInfo {
        label: tag.kind.label().to_string(),
        currency_id: tag.currency_id.clone().filter(|_| named),
        traceable,
        target,
    }
}

fn reference_record(link: &crate::protocol::Link) -> OutputReference {
    use crate::protocol::Target;
    OutputReference {
        label: link.label.clone(),
        target: match &link.target {
            Target::Currency(id) => ReferenceTarget::Currency { id: id.clone() },
            Target::Destination { address, system } => ReferenceTarget::Destination {
                address: address.clone(),
                system_id: system.as_ref().map(ToString::to_string),
            },
            Target::Outpoint(target) => ReferenceTarget::Output {
                outpoint: Outpoint::from(target),
            },
            Target::LegacyNotarizedBlock { hash, height } => ReferenceTarget::Block {
                hash: hash.clone(),
                height: *height,
            },
            Target::OpReturn(data) => ReferenceTarget::Bytes {
                hex: hex(&data.bytes),
                script_body: data.script_body,
            },
        },
    }
}

fn amount_list(amounts: &BTreeMap<String, i128>) -> Vec<CurrencyAmount> {
    amounts
        .iter()
        .filter(|(_, amount)| **amount != 0)
        .map(|(currency, amount)| CurrencyAmount {
            currency: currency.clone(),
            amount: coins(*amount),
        })
        .collect()
}

fn input_record(flow: &Flow, context: &Context, height: Option<u64>) -> TransactionInput {
    let source = flow
        .source_txid
        .as_deref()
        .and_then(|txid| context.sources.get(txid));
    let source_height = source
        .filter(|source| source["blockhash"].is_string())
        .and_then(|source| source["height"].as_u64());
    TransactionInput {
        index: flow.index as u32,
        coinbase: flow.coinbase,
        previous_txid: flow.source_txid.clone(),
        previous_output: flow.source_vout.map(|vout| vout as u32),
        addresses: flow.addresses.clone(),
        value_sats: flow.value_zat.and_then(clamp_sats),
        shielded_pool: flow.private.map(str::to_string),
        derived_value: flow.private.is_some() && flow.value_zat.is_some(),
        reserve_amounts: amount_list(&flow.reserve_amounts),
        sequence: flow.sequence,
        script_type: flow.script_type.clone(),
        annotations: flow.annotations.clone(),
        source_block_time: source
            .filter(|source| source["blockhash"].is_string())
            .and_then(|source| source["blocktime"].as_u64())
            .filter(|time| *time > 0),
        source_height,
        // Port of `input_context_lines`: the spending block's height minus the source's.
        confirmations_when_spent: height
            .zip(source_height)
            .and_then(|(spent, created)| spent.checked_sub(created)),
    }
}

fn output_record(raw: &Value, flow: &Flow, badges: Vec<String>) -> TransactionOutput {
    let output = raw["vout"]
        .as_array()
        .and_then(|outputs| {
            outputs
                .iter()
                .enumerate()
                .find(|(index, output)| {
                    output["n"].as_u64().unwrap_or(*index as u64) == flow.index as u64
                })
                .map(|(_, output)| output)
        })
        .filter(|_| flow.private.is_none());
    let data = flow
        .references
        .links
        .iter()
        .find_map(|link| match &link.target {
            crate::protocol::Target::OpReturn(data) => Some(data),
            _ => None,
        });
    TransactionOutput {
        index: flow.index as u32,
        value_sats: flow.value_zat.and_then(clamp_sats),
        addresses: flow.addresses.clone(),
        script_type: flow.script_type.clone(),
        tags: flow
            .tags
            .iter()
            .map(|tag| tag.kind.label().to_string())
            .collect(),
        spent_txid: output.and_then(|output| text(&output["spentTxId"])),
        currency_values: output
            .map(|output| amounts(&output["scriptPubKey"]["reserve_balance"]))
            .unwrap_or_default(),
        data_hex: data.map(|data| hex(&data.bytes)),
        json: output.map(pretty).unwrap_or_default(),
        shielded_pool: flow.private.map(str::to_string),
        derived_value: flow.private.is_some() && flow.value_zat.is_some(),
        spendable: flow.spendable,
        reserve_amounts: amount_list(&flow.reserve_amounts),
        tag_details: flow.tags.iter().map(|tag| tag_info(flow, tag)).collect(),
        annotations: flow.annotations.clone(),
        routing: flow
            .routing
            .iter()
            .map(|(address, label)| Field {
                label: address.clone(),
                value: label.clone(),
            })
            .collect(),
        references: flow.references.links.iter().map(reference_record).collect(),
        reference_notes: flow.references.notes.clone(),
        badges,
    }
}

// -------------------------------------------------------------------- links

/// Port of the ordering in `session_relationships` in `src/app.rs`: forward links first,
/// then by output and field, without duplicates.
fn sorted_links(mut links: Vec<crate::relationships::Link>) -> Vec<crate::relationships::Link> {
    links.sort_by(|a, b| {
        (
            a.field.starts_with("reverse:"),
            a.carrier_vout,
            &a.field,
            &a.target.txid,
            a.target.vout,
        )
            .cmp(&(
                b.field.starts_with("reverse:"),
                b.carrier_vout,
                &b.field,
                &b.target.txid,
                b.target.vout,
            ))
    });
    let mut seen = HashSet::new();
    links.retain(|link| seen.insert((link.field.clone(), link.carrier_vout, link.target.clone())));
    links
}

fn protocol_link(
    link: &crate::relationships::Link,
    reverse: bool,
    status: Option<&str>,
) -> ProtocolLink {
    ProtocolLink {
        label: link.label.clone(),
        output: link.carrier_vout,
        target_system: link.target.system_id.to_string(),
        target_txid: link.target.txid.clone(),
        target_output: link.target.vout,
        field: link.field.clone(),
        reverse,
        status: status.map(str::to_string),
        resolved: status == Some("Both sides resolved"),
        problem: status.is_some_and(link_problem),
    }
}

/// Statuses in which the two sides disagree, as opposed to one side not being loaded.
fn link_problem(status: &str) -> bool {
    matches!(
        status,
        "Source discrepancy"
            | "Target transaction mismatch"
            | "Referenced output absent"
            | "Unexpected target output type"
            | "Export destination mismatch"
            | "Export source mismatch"
            | "Transfer reference mismatch"
    )
}

// -------------------------------------------------------------------- block

/// The tags of a block's transactions together, in first-seen order, without Coinbase and
/// Stake. Port of `block_tag_spans` in `src/app.rs`.
fn combined_block_tags(transactions: &[TransactionRow]) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    for tag in transactions.iter().flat_map(|row| &row.tags) {
        if tag != "Coinbase" && tag != "Stake" && !tags.contains(tag) {
            tags.push(tag.clone());
        }
    }
    tags
}

/// A block's place in the chain from the daemon's confirmation count: -1 means it is not
/// on the active chain.
fn block_chain_status(
    confirmations: Option<i64>,
    height: u64,
    longest: Option<u64>,
) -> Option<&'static str> {
    use crate::chain_state::StatusBadge;
    let confirmations = confirmations?;
    Some(if confirmations < 0 {
        StatusBadge::Orphaned.label()
    } else if confirmations == 1 && longest.is_none_or(|longest| longest == height) {
        StatusBadge::ChainTip.label()
    } else if confirmations > 0 {
        StatusBadge::Active.label()
    } else {
        StatusBadge::Unverified.label()
    })
}

// -------------------------------------------------------------------- trace

fn exact_amount(amount: &crate::defi::Amount) -> CurrencyAmount {
    CurrencyAmount {
        currency: amount.currency.clone(),
        amount: coins(amount.satoshis),
    }
}

/// Port of `summary_lines` in `src/app/defi_ui.rs`, as data.
fn trace_leg(leg: &crate::defi::LegSummary) -> TraceLeg {
    use crate::defi::{ReceiptKind, Stage};
    let confidence = match leg.stage {
        Stage::Received | Stage::Forwarded | Stage::Refunded | Stage::Burned => {
            Confidence::Verified
        }
        Stage::Unspent | Stage::BurnUnverified => Confidence::Unknown,
        _ => Confidence::Observed,
    };
    let receipt = leg.receipt.as_ref();
    let execution = leg.execution.as_ref();
    TraceLeg {
        source_system: leg.source.system_id.to_string(),
        source_txid: leg.source.txid.clone(),
        source_output: leg.source.vout,
        stage: leg.stage.label().to_string(),
        confidence,
        principal: leg.principal.iter().flatten().map(exact_amount).collect(),
        principal_known: leg.principal.is_some(),
        pool: leg.pool.clone(),
        conversion_fee: leg.conversion_fee.as_ref().map(exact_amount),
        transfer_fee_budget: leg.transfer_fee_budget.as_ref().map(exact_amount),
        forwarding_budget: leg.forwarding_budget.as_ref().map(exact_amount),
        forwarding_conversion_fee: leg.forwarding_conversion_fee.as_ref().map(exact_amount),
        result: receipt.map(|receipt| {
            match receipt.kind {
                ReceiptKind::Received => "Received",
                ReceiptKind::Forwarded => "Forwarded",
                ReceiptKind::Refunded => "Refunded",
            }
            .to_string()
        }),
        result_amounts: receipt
            .map(|receipt| receipt.amounts.iter().map(exact_amount).collect())
            .unwrap_or_default(),
        result_system: receipt.map(|receipt| receipt.output.system_id.to_string()),
        result_txid: receipt.map(|receipt| receipt.output.txid.clone()),
        result_output: receipt.map(|receipt| receipt.output.vout),
        next_fee_budget: receipt
            .and_then(|receipt| receipt.next_fee_budget.as_ref())
            .map(exact_amount),
        burned: leg
            .supply_burn
            .as_ref()
            .map(|burn| exact_amount(&burn.selected)),
        batch_burned: leg
            .supply_burn
            .as_ref()
            .map(|burn| exact_amount(&burn.batch_total)),
        proceeds: leg
            .proceeds
            .iter()
            .flat_map(|proceeds| proceeds.amounts.iter().map(exact_amount))
            .collect(),
        proceeds_output: leg
            .proceeds
            .as_ref()
            .map(|proceeds| Outpoint::from(&proceeds.output)),
        import_height: execution.and_then(|execution| execution.height),
        execution_system: execution.map(|execution| execution.import.system_id.to_string()),
        import_output: execution.map(|execution| Outpoint::from(&execution.import)),
        // Port of the rate lines in `summary_lines` in `src/app/defi_ui.rs`.
        rates: leg
            .execution_rates()
            .iter()
            .map(|rate| {
                let scaled = rate.output.satoshis.checked_mul(100_000_000);
                TraceRate {
                    purpose: rate.purpose.to_string(),
                    net_input: exact_amount(&rate.net_input),
                    output: exact_amount(&rate.output),
                    rate_sats: scaled
                        .and_then(|scaled| clamp_sats(scaled / rate.net_input.satoshis)),
                    exact: scaled.is_some_and(|scaled| scaled % rate.net_input.satoshis == 0),
                    prices: rate
                        .prices
                        .iter()
                        .map(|(currency, field, value)| TracePrice {
                            currency: currency.clone(),
                            field: field.to_string(),
                            value: coins(*value),
                        })
                        .collect(),
                }
            })
            .collect(),
        currencies: leg.currency_ids(),
        pool_inspection: execution
            .zip(leg.pool.as_ref())
            .map(|(execution, pool)| PoolInspection {
                pool: pool.clone(),
                import: Outpoint::from(&execution.import),
                height: execution.height,
                previous: execution.previous.as_ref().map(Outpoint::from),
            }),
    }
}

/// Carbide reports a stop with the failing request attached. Say the common cause plainly.
fn plain_stop(message: &str) -> String {
    if message.contains("-32601") || message.contains("Method not found") {
        return "This data source does not permit the necessary request. A fully synchronized \
                local node can complete the trace."
            .into();
    }
    message.to_string()
}

/// Reduces Carbide's trail to what the app draws, keeping its stage wording.
impl Explorer {
    /// Finds the block of each stage on this chain. A stage on another system has no
    /// height here: the import height of its leg is in the leg.
    async fn place_moments(&self, trace: &mut TransferTrace, seed: &Value) {
        let local = self.config.chain.system_id.get().map(ToString::to_string);
        let mut wanted: Vec<String> = Vec::new();
        for moment in &trace.moments {
            if Some(&moment.system_id) == local.as_ref() && !wanted.contains(&moment.txid) {
                wanted.push(moment.txid.clone());
            }
        }
        let mut found = self.fetch_transactions(&wanted).await;
        if let Some(txid) = seed["txid"].as_str() {
            found.entry(txid.to_string()).or_insert_with(|| seed.clone());
        }
        for moment in &mut trace.moments {
            if let Some(raw) = found.get(&moment.txid).filter(|raw| raw["blockhash"].is_string()) {
                moment.height = raw["height"].as_u64();
                moment.time = raw["blocktime"].as_u64().filter(|time| *time > 0);
            }
        }
    }
}

/// The stage that an entry of Carbide's trail stands for. Entries that are context, such
/// as the import notarization and the address candidates, are not a stage.
fn trace_stage(position: usize, label: &str) -> Option<TraceStage> {
    match label {
        _ if position == 0 => Some(TraceStage::Sent),
        "Export" => Some(TraceStage::Exported),
        "Import" => Some(TraceStage::Imported),
        "Replayed next-leg transfer" => Some(TraceStage::Sent),
        "Direct transfer result" => Some(TraceStage::Result),
        label if label.starts_with("Replayed ") => Some(TraceStage::Result),
        _ => None,
    }
}

/// The batch from the detail of Carbide's export entry: "source input 2; batch inputs
/// 1..12; ...". The end of the range is not in the batch.
fn trace_batch(detail: &str) -> Option<TraceBatch> {
    let after = |text: &str, key: &str| -> Option<String> {
        let rest = &text[text.find(key)? + key.len()..];
        Some(rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect())
    };
    let input: u32 = after(detail, "source input ")?.parse().ok()?;
    let range = after(detail, "batch inputs ")?;
    let (first, end) = range.split_once("..")?;
    let (first, end): (u32, u32) = (first.parse().ok()?, end.parse().ok()?);
    (end > first && input >= first && input < end).then_some(TraceBatch {
        size: end - first,
        place: input - first,
    })
}

fn transfer_trace(trail: crate::defi::Trail) -> TransferTrace {
    use crate::defi::Stage;
    let moments = trail
        .entries
        .iter()
        .enumerate()
        .filter_map(|(position, entry)| {
            let stage = trace_stage(position, &entry.label)?;
            Some(TraceMoment {
                stage,
                system_id: entry.output.system_id.to_string(),
                txid: entry.output.txid.clone(),
                output: entry.output.vout,
                height: None,
                time: None,
            })
        })
        .collect();
    let batch = trail
        .entries
        .iter()
        .find(|entry| entry.label == "Export")
        .and_then(|entry| trace_batch(&entry.detail));
    let candidates = trail
        .entries
        .iter()
        .filter(|entry| entry.label == "Destination-address candidate")
        .count() as u32;
    let legs: Vec<TraceLeg> = trail.legs.iter().map(trace_leg).collect();
    let (verdict, confidence) = match trail.legs.last().map(|leg| leg.stage) {
        Some(Stage::Received) => (
            "Received. The replay matches the result output.",
            Confidence::Verified,
        ),
        Some(Stage::Refunded) => (
            "Refunded. The full amount went back to the sender.",
            Confidence::Verified,
        ),
        Some(Stage::Forwarded) => ("Forwarded to the next system.", Confidence::Verified),
        Some(Stage::Burned) => (
            "Burned. The supply change is verified.",
            Confidence::Verified,
        ),
        Some(Stage::Imported | Stage::BurnUnverified) => (
            "Imported. The result output is not matched.",
            Confidence::Observed,
        ),
        Some(Stage::Exported) => (
            "Exported. The searched blocks do not contain the import.",
            Confidence::Observed,
        ),
        Some(Stage::Pending) => ("Not exported at this time.", Confidence::Observed),
        Some(Stage::Unconfirmed) => ("Not confirmed in a block.", Confidence::Observed),
        Some(Stage::Unspent) => ("Not included in an export.", Confidence::Unknown),
        Some(Stage::Observed) => (
            "Found at the source. No further data is available.",
            Confidence::Observed,
        ),
        None if !trail.entries.is_empty() => (
            "Carbide inspected this output. The evidence is listed below.",
            Confidence::Observed,
        ),
        None => ("No data is available for this output.", Confidence::Unknown),
    };
    TransferTrace {
        verdict: verdict.to_string(),
        confidence,
        legs,
        steps: trail
            .entries
            .into_iter()
            .map(|entry| TraceStep {
                system_id: entry.output.system_id.to_string(),
                txid: entry.output.txid,
                output: entry.output.vout,
                label: entry.label,
                detail: entry.detail,
            })
            .collect(),
        notes: trail.notes,
        stop: trail.stop.map(|stop| plain_stop(&stop.message)),
        search: trail.search.map(|search| TraceSearch {
            system_id: search.system,
            first: search.first,
            last: search.last,
            tip: search.tip,
            windows: search.windows,
            more_older: search.more_older,
        }),
        searching: trail.searching,
        search_root: trail.search_root.as_ref().map(Outpoint::from),
        import_heights: trail
            .import_heights
            .iter()
            .map(|(root, height)| KnownImport {
                root: Outpoint::from(root),
                height: *height,
            })
            .collect(),
        moments,
        batch,
        candidates,
    }
}

/// Port of `pool_lines` in `src/app/defi_ui.rs`, as data.
fn pool_changes(comparison: &crate::defi::pool::Comparison) -> PoolComparison {
    use crate::defi::pool::state_amount;
    let before = comparison.previous.as_ref().map(|snapshot| &snapshot.state);
    let after = &comparison.imported.state;
    let change =
        |label: String, is_supply: bool, old: Option<i128>, new: Option<i128>| PoolChange {
            label,
            is_supply,
            before: old.map(coins),
            after: new.map(coins),
            change: old
                .zip(new)
                .and_then(|(old, new)| new.checked_sub(old))
                .map(coins),
        };
    let mut changes = vec![change(
        "Supply".into(),
        true,
        before.and_then(|state| state_amount(&state["supply"])),
        state_amount(&after["supply"]),
    )];
    let reserves = |state: &Value| -> BTreeMap<String, Value> {
        state["reservecurrencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|reserve| {
                Some((
                    reserve["currencyid"].as_str()?.to_owned(),
                    reserve["reserves"].clone(),
                ))
            })
            .collect()
    };
    let old = before.map(&reserves).unwrap_or_default();
    let new = reserves(after);
    let ids: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
    for id in ids.iter().take(12) {
        changes.push(change(
            (*id).clone(),
            false,
            old.get(*id).and_then(state_amount),
            new.get(*id).and_then(state_amount),
        ));
    }
    let snapshot = |snapshot: &crate::defi::pool::Snapshot| PoolSnapshot {
        output: Outpoint::from(&snapshot.output),
        height: snapshot.height,
    };
    PoolComparison {
        previous: comparison.previous.as_ref().map(snapshot),
        imported: snapshot(&comparison.imported),
        note: comparison.note.clone(),
        changes,
        more_reserves: ids.len() > 12,
    }
}

// ---------------------------------------------------------------- witnesses

fn witness_report(root: &OutpointRef, report: crate::witnesses::Report) -> WitnessReport {
    // The document the terminal shows with Tab: the root, the status and the report.
    let json = pretty(&serde_json::json!({
        "root": root,
        "status": "complete",
        "message": Value::Null,
        "report": &report,
    }));
    WitnessReport {
        signatures: report
            .signatures
            .into_iter()
            .map(|signature| WitnessSignature {
                key_count: signature.key_signatures.len() as u32,
                key_signatures: signature.key_signatures.into_iter().collect(),
                identity: signature.identity,
                height: signature.height,
                confirms: signature.confirmed,
                system: signature.system.to_string(),
                target_system: signature.target.system_id.to_string(),
                target_txid: signature.target.txid,
                target_output: signature.target.vout,
                origins: signature
                    .origins
                    .iter()
                    .map(|origin| WitnessOrigin {
                        output: Outpoint::from(&origin.output),
                        source: origin.source.clone(),
                    })
                    .collect(),
            })
            .collect(),
        finalizations: report.finalizations,
        issues: report.issues,
        transactions_checked: report.transactions as u32,
        outputs_checked: report.outputs as u32,
        multipart_sets: report.multipart_sets as u32,
        json,
    }
}

// --------------------------------------------------------------------- JSON

fn looks_like_identity_address(value: &str) -> bool {
    value.starts_with('i')
        && value.len() == 34
        && value.chars().all(|ch| ch.is_ascii_alphanumeric())
}

fn looks_like_transparent_address(value: &str) -> bool {
    value.starts_with('R')
        && value.len() == 34
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() && !matches!(ch, '0' | 'O' | 'I' | 'l'))
}

fn is_hash_like(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Keys whose i-address value names a currency or a system, not an identity.
fn currency_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("currency")
        || key.ends_with("systemid")
        || matches!(
            key.as_str(),
            "systemid" | "launchsystemid" | "parent" | "via" | "exportto" | "gatewayid" | "gateway"
        )
}

/// Port of `json_link` in `src/app.rs`, extended to transparent addresses and to currency
/// IDs. `parent_key` is the member that holds the value, for array items.
fn json_link(label: &str, parent_key: &str, value: &str) -> Option<JsonLinkKind> {
    if looks_like_identity_address(value) {
        let key = if label.parse::<usize>().is_ok() {
            parent_key
        } else {
            label
        };
        return Some(if currency_key(key) && key != "parent" {
            JsonLinkKind::Currency
        } else {
            JsonLinkKind::Identity
        });
    }
    if value.ends_with('@') && value.len() > 1 {
        return Some(JsonLinkKind::Identity);
    }
    if looks_like_transparent_address(value) {
        return Some(JsonLinkKind::Address);
    }
    if !is_hash_like(value) {
        return None;
    }
    let label = label.to_ascii_lowercase();
    // A field named as a transaction ID holds one. Proof and content hashes are never
    // turned into transaction IDs.
    if label.ends_with("txid") {
        Some(JsonLinkKind::Transaction)
    } else if matches!(
        label.as_str(),
        "blockhash" | "previousblockhash" | "nextblockhash" | "bestblockhash"
    ) {
        Some(JsonLinkKind::Block)
    } else {
        None
    }
}

fn escape_pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// Every string value in the document that opens an object, with whether a name can be
/// shown for it. `identityaddress` and `fullyqualifiedname` stay raw, as in Carbide.
fn json_links(root: &Value) -> Vec<JsonMark> {
    fn walk(value: &Value, pointer: String, label: &str, parent: &str, marks: &mut Vec<JsonMark>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    walk(
                        child,
                        format!("{pointer}/{}", escape_pointer(key)),
                        key,
                        label,
                        marks,
                    );
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    walk(
                        child,
                        format!("{pointer}/{index}"),
                        &index.to_string(),
                        label,
                        marks,
                    );
                }
            }
            Value::String(text) => {
                let Some(link) = json_link(label, parent, text) else {
                    return;
                };
                let raw = label.eq_ignore_ascii_case("identityaddress")
                    || label.eq_ignore_ascii_case("fullyqualifiedname");
                marks.push(JsonMark {
                    pointer,
                    link: Some(link),
                    target: Some(text.clone()),
                    nameable: !raw
                        && matches!(link, JsonLinkKind::Identity | JsonLinkKind::Currency)
                        && looks_like_identity_address(text),
                    date: None,
                    estimated: false,
                });
            }
            _ => {}
        }
    }
    let mut marks = Vec::new();
    walk(root, String::new(), "", "", &mut marks);
    marks
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DateTarget {
    Time(u64),
    /// A height on a system. An empty system is the chain being viewed.
    Height {
        system: String,
        height: u64,
    },
}

fn date_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn date_number(object: &Value, names: &[&str]) -> Option<u64> {
    object.as_object()?.iter().find_map(|(name, value)| {
        names
            .contains(&date_key(name).as_str())
            .then(|| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
            .flatten()
    })
}

/// Port of `inclusion` in `src/app/json_dates.rs`, from the document alone.
fn inclusion(object: &Value) -> (Option<u64>, Option<u64>) {
    let height = date_number(object, &["height", "blockheight"]);
    let mut timestamp = date_number(object, &["blocktime"]);
    if object.get("hash").is_some() && object.get("txid").is_none() {
        timestamp = timestamp.or_else(|| date_number(object, &["time"]));
    }
    if height.is_some() && object.get("txid").is_none() {
        timestamp = timestamp.or_else(|| date_number(object, &["time"]));
    }
    (height, timestamp.filter(|t| *t > 0))
}

/// Port of `display` in `src/app/json_dates.rs`: what date, if any, a value at `pointer`
/// stands for. Relative delays stay counts. Values are never changed.
fn date_target(root: &Value, pointer: &str) -> Option<DateTarget> {
    if pointer
        .split('/')
        .any(|part| matches!(date_key(part).as_str(), "contentmap" | "contentmultimap"))
    {
        return None;
    }
    let value = root.pointer(pointer)?;
    let value = value.as_u64().or_else(|| value.as_str()?.parse().ok())?;
    let (parent, name) = pointer.rsplit_once('/')?;
    let name = date_key(&name.replace("~1", "/").replace("~0", "~"));
    let object = root.pointer(parent)?;
    let (own_height, own_time) = inclusion(object);
    let mut system = String::new();
    let height = match name.as_str() {
        "time"
        | "timestamp"
        | "ntime"
        | "blocktime"
        | "mediantime"
        | "tiptime"
        | "nextblocktime"
        | "lastblocktime"
        | "firstblocktime"
        | "creationtimestamp"
        | "registrationtimestamp"
        | "notarizedtime"
        | "timereceived"
        | "receivedtime"
        | "creationtime"
        | "registrationtime"
        | "firstseen"
        | "lastseen"
        | "lastsend"
        | "lastrecv"
        | "conntime" => {
            return valid_time(value).map(DateTarget::Time);
        }
        "confirmations" => {
            if value == 0 {
                return None;
            }
            if let Some(time) = own_time {
                return valid_time(time).map(DateTarget::Time);
            }
            own_height?
        }
        "locktime" if value >= 500_000_000 => return valid_time(value).map(DateTarget::Time),
        "locktime"
        | "expiryheight"
        | "unlockheight"
        | "launchheight"
        | "definitionheight"
        | "startheight"
        | "endheight"
        | "fromheight"
        | "toheight"
        | "proofheight"
        | "lastconfirmedheight"
        | "anchorheight" => {
            if value == 0 {
                return None;
            }
            value
        }
        "timelock" => {
            if value == 0
                || object["version"].as_u64().is_none_or(|v| v < 2)
                || object["flags"]
                    .as_u64()
                    .is_none_or(|flags| flags & 2 != 0 || flags & 0x8000 != 0)
            {
                return None;
            }
            value
        }
        "startblock" | "endblock" => {
            if value == 0 {
                return None;
            }
            system = object[if name == "startblock" {
                "launchsystemid"
            } else {
                "systemid"
            }]
            .as_str()?
            .to_string();
            value
        }
        "sourceheight" | "sourceheightstart" | "sourceheightend" => {
            system = object["sourcesystemid"].as_str()?.to_string();
            value
        }
        "height" | "blockheight" => value,
        "blocks" | "longestchain"
            if object.get("chainid").is_some() || object.get("bestblockhash").is_some() =>
        {
            value
        }
        _ => return None,
    };
    if system.is_empty()
        && own_height == Some(height)
        && let Some(time) = own_time
    {
        return valid_time(time).map(DateTarget::Time);
    }
    if system.is_empty()
        && own_height == Some(height)
        && own_time.is_none()
        && (object["hash"].is_string() || object["blockhash"].is_string())
    {
        // An explicit inclusion hash can identify another branch. A timestamp
        // fetched only by canonical height would not date that inclusion.
        return None;
    }
    Some(DateTarget::Height { system, height })
}

fn valid_time(time: u64) -> Option<u64> {
    chrono::DateTime::from_timestamp(i64::try_from(time).ok()?, 0).map(|_| time)
}

/// Every numeric value in the document that stands for a date.
fn date_targets(root: &Value) -> Vec<(String, DateTarget)> {
    fn walk(value: &Value, pointer: String, root: &Value, found: &mut Vec<(String, DateTarget)>) {
        match value {
            Value::Object(map) => {
                for (key, child) in map {
                    walk(
                        child,
                        format!("{pointer}/{}", escape_pointer(key)),
                        root,
                        found,
                    );
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    walk(child, format!("{pointer}/{index}"), root, found);
                }
            }
            Value::Number(_) | Value::String(_) => {
                if let Some(target) = date_target(root, &pointer) {
                    found.push((pointer, target));
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    walk(root, String::new(), root, &mut found);
    found
}

#[path = "ffi_statement.rs"]
// `pub(super)` lets `ffi::rust_api` name the statement records for a Rust caller.
pub(super) mod statement;

#[cfg(test)]
#[path = "ffi_transaction_tests.rs"]
mod tests;
