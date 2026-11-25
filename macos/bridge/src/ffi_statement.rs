//! The statement by party: who ended with more and who ended with less, in each currency.
//!
//! This is arithmetic on Carbide's own flows. All inputs and outputs of one address are
//! one party, the reserve deposits of a currency are one party, and the transfers that
//! wait for an export are one party. An amount that a party gave and received again is
//! not a change: it is reported apart, as carried. The chain does not record which input
//! paid which output, and the statement does not say that it does.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{
    Context, FeeKind, Flow, FlowKind, ShieldedAccounting, ValueAccounting, clamp_sats,
    decimal_value_to_zat, text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PartyKind {
    /// One address, or the addresses of one output script.
    Address,
    /// The reserve deposits that a currency controls.
    ReserveDeposits,
    /// Reserve transfers that wait for an export.
    WaitingTransfers,
    /// The block reward: new coins and the fees of the block.
    NewCoins,
    /// The shielded pools. The total is public. The recipients are not.
    ShieldedPool,
    /// Value in an output that has no address.
    NoAddress,
}

/// An amount of one currency.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StatementAmount {
    /// The currency ID. None for the chain's own coin.
    pub currency_id: Option<String>,
    pub sats: i64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct StatementParty {
    pub kind: PartyKind,
    /// Empty for new coins and for the shielded pool.
    pub addresses: Vec<String>,
    /// The currency that controls the reserve deposits.
    pub currency_id: Option<String>,
    /// The net change in each currency: positive is received, negative is given. No
    /// entry is zero.
    pub net: Vec<StatementAmount>,
    /// Amounts that the party gave and received again in this transaction.
    pub carried: Vec<StatementAmount>,
    /// The `index` of each input and output of the party in the transaction's lists.
    pub inputs: Vec<u32>,
    pub outputs: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ClosingKind {
    /// Each column sums to zero without a closing row, or a value is missing.
    None,
    /// Carbide gives a fee for this transaction.
    Fee,
    /// Protocol outputs account for value elsewhere. The row is the inputs less the
    /// outputs, and it is not a fee.
    InputsLessOutputs,
}

/// An input or output that carries no value: its meaning is its kind.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StatementMark {
    pub label: String,
    /// The currency that Carbide's tag names.
    pub currency_id: Option<String>,
    /// True for an input that this transaction spends.
    pub spent: bool,
    /// The `index` in the input list or the output list.
    pub index: u32,
}

/// What a reserve transfer asks for. The result is in a later import.
#[derive(Debug, Clone, uniffi::Record)]
pub struct StatementRoute {
    pub output: u32,
    pub amounts: Vec<StatementAmount>,
    /// The currency that the conversion goes through.
    pub via: Option<String>,
    pub to_currency: Option<String>,
    pub conversion: bool,
    pub destination: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct TransactionStatement {
    /// The currencies with a change, the chain's own coin first. None is that coin.
    pub columns: Vec<Option<String>>,
    /// The parties that gave, then the parties that received, then the parties with no
    /// change.
    pub parties: Vec<StatementParty>,
    /// What closes each column: the amount that the inputs have and the outputs do not.
    /// A negative amount is in the outputs and not in the inputs.
    pub closing: Vec<StatementAmount>,
    pub closing_kind: ClosingKind,
    pub marks: Vec<StatementMark>,
    pub routes: Vec<StatementRoute>,
    /// False when a value is missing. The columns then do not sum to zero.
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Key {
    NewCoins,
    Deposit(String),
    Transfer(Vec<String>),
    Address(Vec<String>),
    NoAddress,
    Pool,
}

type Amounts = BTreeMap<Option<String>, i128>;

#[derive(Debug)]
struct Draft {
    key: Key,
    kind: PartyKind,
    addresses: Vec<String>,
    currency_id: Option<String>,
    given: Amounts,
    received: Amounts,
    inputs: Vec<u32>,
    outputs: Vec<u32>,
}

/// The output script that a flow stands for: the output itself, or the output that an
/// input spends.
fn script<'a>(raw: &'a Value, context: &'a Context, flow: &Flow) -> Option<&'a Value> {
    let output = match flow.kind {
        FlowKind::Input => {
            let input = raw["vin"].as_array()?.get(flow.index)?;
            input.get("prevout").or_else(|| context.prevout(input))?
        }
        FlowKind::Output => raw["vout"]
            .as_array()?
            .iter()
            .enumerate()
            .find(|(index, output)| {
                output["n"].as_u64().unwrap_or(*index as u64) == flow.index as u64
            })?
            .1,
    };
    output.get("scriptPubKey")
}

/// The name of an input or output that carries no value. Carbide's tag when it has one.
fn mark_label(flow: &Flow, script: Option<&Value>, spent: bool) -> (String, Option<String>) {
    let tags = if spent {
        script
            .map(|script| crate::domain::output_tags(&serde_json::json!({ "scriptPubKey": script })))
            .unwrap_or_default()
    } else {
        flow.tags.clone()
    };
    if let Some(tag) = tags.first() {
        return (tag.kind.label().to_string(), tag.currency_id.clone());
    }
    let known = [
        ("feepool", "Fee Pool Checkpoint"),
        ("crosschainexport", "Export"),
        ("crosschainimport", "Import"),
        ("identityprimary", "Identity"),
        ("identityreservation", "Identity Reservation"),
        ("identityadvancedreservation", "Identity Reservation"),
        ("stakeguard", "StakeGuard"),
    ];
    let label = script
        .and_then(Value::as_object)
        .and_then(|keys| {
            known
                .iter()
                .find(|(key, _)| keys.contains_key(*key))
                .map(|(_, label)| *label)
        })
        .or_else(|| {
            (flow.script_type.as_deref() == Some("nulldata")).then_some("Data")
        })
        .unwrap_or("Output Without Value");
    (label.to_string(), None)
}

fn amounts_of(flow: &Flow) -> Amounts {
    let mut amounts = Amounts::new();
    if let Some(value) = flow.value_zat.filter(|value| *value != 0) {
        amounts.insert(None, value);
    }
    for (currency, amount) in &flow.reserve_amounts {
        if *amount != 0 {
            *amounts.entry(Some(currency.clone())).or_default() += amount;
        }
    }
    amounts
}

fn list(amounts: &Amounts) -> Vec<StatementAmount> {
    amounts
        .iter()
        .filter(|(_, amount)| **amount != 0)
        .map(|(currency, amount)| StatementAmount {
            currency_id: currency.clone(),
            // An amount outside 64 bits is not possible on the chain. Saturate to be safe.
            sats: clamp_sats(*amount).unwrap_or(if *amount < 0 { i64::MIN } else { i64::MAX }),
        })
        .collect()
}

fn route(output: u32, payload: &Value, native: Option<&str>) -> StatementRoute {
    let currency = |id: Option<String>| id.filter(|id| Some(id.as_str()) != native);
    let mut amounts = Amounts::new();
    for (id, amount) in payload["currencyvalues"].as_object().into_iter().flatten() {
        if let Some(amount) = decimal_value_to_zat(amount) {
            *amounts.entry(currency(Some(id.clone()))).or_default() += amount;
        }
    }
    StatementRoute {
        output,
        amounts: list(&amounts),
        via: text(&payload["via"]),
        to_currency: text(&payload["destinationcurrencyid"]),
        conversion: payload["convert"].as_bool() == Some(true),
        destination: text(&payload["destination"]["address"]),
    }
}

/// The statement of a transaction, from Carbide's flows and value accounting.
pub(super) fn statement(
    raw: &Value,
    flows: &[Flow],
    context: &Context,
    native: Option<&str>,
    accounting: &ValueAccounting,
) -> TransactionStatement {
    let mut drafts: Vec<Draft> = Vec::new();
    let mut marks = Vec::new();
    let mut routes = Vec::new();
    let mut complete = accounting.fee_kind != FeeKind::Unknown;
    let mut private_inputs = Vec::new();
    let mut private_outputs = Vec::new();
    let coinbase = flows.iter().any(|flow| flow.coinbase);

    for flow in flows {
        let spent = flow.kind == FlowKind::Input;
        let index = flow.index as u32;
        if flow.private.is_some() {
            if spent { &mut private_inputs } else { &mut private_outputs }.push(index);
            continue;
        }
        if flow.coinbase {
            continue;
        }
        let script = script(raw, context, flow);
        if spent && flow.value_zat.is_none() {
            // The source of this input is not available: its value is not known.
            complete = false;
            continue;
        }
        let transfer = script.and_then(|script| script.get("reservetransfer"));
        if !spent && let Some(payload) = transfer.filter(|payload| payload.is_object()) {
            routes.push(route(index, payload, native));
        }
        let amounts = amounts_of(flow);
        if amounts.is_empty() {
            let (label, currency_id) = mark_label(flow, script, spent);
            marks.push(StatementMark { label, currency_id, spent, index });
            continue;
        }
        let deposit = script.and_then(|script| script.get("reservedeposit"));
        let (key, kind, currency_id) = if let Some(deposit) = deposit.filter(|d| d.is_object()) {
            let controller = text(&deposit["controllingcurrencyid"]);
            let id = controller
                .clone()
                .or_else(|| flow.addresses.first().cloned())
                .unwrap_or_default();
            (Key::Deposit(id), PartyKind::ReserveDeposits, controller)
        } else if transfer.is_some_and(Value::is_object) {
            (Key::Transfer(flow.addresses.clone()), PartyKind::WaitingTransfers, None)
        } else if flow.addresses.is_empty() {
            (Key::NoAddress, PartyKind::NoAddress, None)
        } else {
            (Key::Address(flow.addresses.clone()), PartyKind::Address, None)
        };
        let position = drafts.iter().position(|draft| draft.key == key).unwrap_or_else(|| {
            drafts.push(Draft {
                key,
                kind,
                addresses: flow.addresses.clone(),
                currency_id,
                given: Amounts::new(),
                received: Amounts::new(),
                inputs: Vec::new(),
                outputs: Vec::new(),
            });
            drafts.len() - 1
        });
        let draft = &mut drafts[position];
        let side = if spent { &mut draft.given } else { &mut draft.received };
        for (currency, amount) in amounts {
            *side.entry(currency).or_default() += amount;
        }
        if spent { &mut draft.inputs } else { &mut draft.outputs }.push(index);
    }

    // The shielded pools: the public total, not the recipients.
    if !private_inputs.is_empty() || !private_outputs.is_empty() {
        let mut draft = Draft {
            key: Key::Pool,
            kind: PartyKind::ShieldedPool,
            addresses: Vec::new(),
            currency_id: None,
            given: Amounts::new(),
            received: Amounts::new(),
            inputs: private_inputs,
            outputs: private_outputs,
        };
        match ShieldedAccounting::from_raw(raw).net_to_transparent_zat() {
            Some(net) if net > 0 => {
                draft.given.insert(None, net);
            }
            Some(net) if net < 0 => {
                draft.received.insert(None, -net);
            }
            Some(_) => {}
            None => complete = false,
        }
        drafts.push(draft);
    }

    // What the inputs have and the outputs do not, in each currency.
    let mut remainder = Amounts::new();
    for draft in &drafts {
        for (currency, amount) in &draft.given {
            *remainder.entry(currency.clone()).or_default() += amount;
        }
        for (currency, amount) in &draft.received {
            *remainder.entry(currency.clone()).or_default() -= amount;
        }
    }
    remainder.retain(|_, amount| *amount != 0);

    // A block reward makes the coins that its outputs have.
    if coinbase {
        let made: Amounts = remainder
            .iter()
            .filter(|(_, amount)| **amount < 0)
            .map(|(currency, amount)| (currency.clone(), -amount))
            .collect();
        if !made.is_empty() {
            remainder.retain(|_, amount| *amount > 0);
            drafts.insert(
                0,
                Draft {
                    key: Key::NewCoins,
                    kind: PartyKind::NewCoins,
                    addresses: Vec::new(),
                    currency_id: None,
                    given: made,
                    received: Amounts::new(),
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                },
            );
        }
    }

    let (closing, closing_kind) = if !complete || remainder.is_empty() {
        (Vec::new(), ClosingKind::None)
    } else if accounting.fee_kind == FeeKind::Known {
        (list(&remainder), ClosingKind::Fee)
    } else {
        (list(&remainder), ClosingKind::InputsLessOutputs)
    };

    let mut parties: Vec<StatementParty> = drafts
        .into_iter()
        .map(|draft| {
            let mut net = Amounts::new();
            let mut carried = Amounts::new();
            for (currency, given) in &draft.given {
                *net.entry(currency.clone()).or_default() -= given;
                if let Some(received) = draft.received.get(currency) {
                    carried.insert(currency.clone(), *given.min(received));
                }
            }
            for (currency, received) in &draft.received {
                *net.entry(currency.clone()).or_default() += received;
            }
            StatementParty {
                kind: draft.kind,
                addresses: draft.addresses,
                currency_id: draft.currency_id,
                net: list(&net),
                carried: list(&carried),
                inputs: draft.inputs,
                outputs: draft.outputs,
            }
        })
        .collect();
    // Givers, then receivers, then parties with no change. The order in each group is the
    // order of the transaction.
    parties.sort_by_key(|party| {
        if party.net.iter().any(|amount| amount.sats < 0) {
            0
        } else if party.net.is_empty() {
            2
        } else {
            1
        }
    });

    let mut columns: Vec<Option<String>> = Vec::new();
    for amount in parties.iter().flat_map(|party| &party.net).chain(&closing) {
        if !columns.contains(&amount.currency_id) {
            columns.push(amount.currency_id.clone());
        }
    }
    if let Some(position) = columns.iter().position(Option::is_none) {
        columns.remove(position);
        columns.insert(0, None);
    }

    TransactionStatement {
        columns,
        parties,
        closing,
        closing_kind,
        marks,
        routes,
        complete,
    }
}

#[cfg(test)]
#[path = "ffi_statement_tests.rs"]
mod tests;
