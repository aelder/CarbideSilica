//! The sentence of a statement: what a transaction did, in words (`StatementWords` in
//! `TransactionStatement.swift`). The Mempool page shows it for each waiting transaction
//! and for each transaction that left with a block.
//!
//! The Swift app has one `StatementWords` for the Transaction page and the Mempool page.
//! The Transaction page of the Linux port is in another folder, so this module holds the
//! part that the Mempool page uses: the names, the amounts and the sentence.

use std::collections::HashMap;
use std::rc::Rc;

use crate::bridge::api::{self, ClosingKind, PartyKind, StatementAmount, StatementParty, TransactionStatement};
use crate::format;
use crate::model::AppModel;

use super::text::{capital, list};

/// More receiving addresses than this are counted, not named (`StatementLayout.maxReceivers`).
const MAX_RECEIVERS: usize = 8;

/// What the words need from the model: the names of IDs and the chain's own coin.
#[derive(Clone, Debug, Default)]
pub struct Names {
    pub coin: String,
    pub chain_id: Option<String>,
    pub shows_names: bool,
    pub names: HashMap<String, String>,
}

impl Names {
    pub fn of(model: &Rc<AppModel>) -> Self {
        let coin = model.coin();
        let state = model.state();
        Self {
            coin,
            chain_id: state.status.as_ref().and_then(|s| s.chain_id.clone()),
            shows_names: crate::display::shows_names(),
            names: state.names.clone(),
        }
    }

    /// The name of a currency ID: the coin, the name without its "@", or a short ID
    /// (`AppModel.currencyName`).
    pub fn currency_name(&self, id: &str) -> String {
        if Some(id) == self.chain_id.as_deref() || id == self.coin {
            return self.coin.clone();
        }
        if !self.shows_names {
            return format::short(id, 8, 6);
        }
        match self.names.get(id) {
            Some(name) => name.strip_suffix('@').unwrap_or(name).to_string(),
            None => format::short(id, 8, 6),
        }
    }

    /// An identity or currency ID as the "Show Names for IDs" setting says
    /// (`AppModel.displayID`).
    pub fn display_id(&self, id: &str) -> String {
        if self.shows_names
            && let Some(name) = self.names.get(id)
        {
            return name.clone();
        }
        format::short(id, 8, 6)
    }
}

pub struct Words<'a> {
    pub statement: &'a TransactionStatement,
    pub coin: &'a str,
    pub tags: &'a [String],
    pub coinbase: bool,
    pub fee_pool: bool,
    pub names: &'a Names,
}

impl<'a> Words<'a> {
    /// The words of a transaction that the Mempool page read. None when it has no statement.
    pub fn of(detail: &'a api::WaitingTransaction, names: &'a Names) -> Option<Self> {
        Some(Self {
            statement: detail.statement.as_ref()?,
            coin: &detail.coin,
            tags: &detail.tags,
            coinbase: detail.coinbase,
            fee_pool: detail.fee_pool_checkpoint,
            names,
        })
    }

    fn currency(&self, id: Option<&str>) -> String {
        id.map(|id| self.names.currency_name(id)).unwrap_or_else(|| self.coin.to_string())
    }

    fn amount(&self, amount: &StatementAmount) -> String {
        format!("{} {}", format::coins(amount.sats.saturating_abs()), self.currency(amount.currency_id.as_deref()))
    }

    /// A party in the middle of a sentence.
    fn name(&self, party: &StatementParty) -> String {
        match party.kind {
            PartyKind::Address => {
                party.addresses.first().map(|a| self.names.display_id(a)).unwrap_or_else(|| "an address".into())
            }
            PartyKind::ReserveDeposits => party
                .currency_id
                .as_deref()
                .map(|id| format!("the reserve deposits of {}", self.names.currency_name(id)))
                .unwrap_or_else(|| "the reserve deposits".into()),
            PartyKind::WaitingTransfers => "the waiting transfers".into(),
            PartyKind::NewCoins => "new coins and fees".into(),
            PartyKind::ShieldedPool => "the shielded pool".into(),
            PartyKind::NoAddress => "an output with no address".into(),
        }
    }

    pub fn sentence(&self) -> String {
        let statement = self.statement;
        let parties = &statement.parties;
        let givers: Vec<&StatementParty> = parties.iter().filter(|p| p.net.iter().any(|a| a.sats < 0)).collect();
        let receivers: Vec<&StatementParty> =
            parties.iter().filter(|p| !p.net.is_empty() && !p.net.iter().any(|a| a.sats < 0)).collect();
        let addresses: Vec<&StatementParty> = receivers
            .iter()
            .copied()
            .filter(|p| matches!(p.kind, PartyKind::Address | PartyKind::NoAddress))
            .collect();
        let pool = parties.iter().find(|p| p.kind == PartyKind::ShieldedPool);
        let written: Vec<&api::StatementMark> = statement.marks.iter().filter(|m| !m.spent).collect();
        let one_column = statement.columns.len() == 1;
        let column: Option<&str> = statement.columns.first().and_then(|c| c.as_deref());
        let net = |party: &StatementParty| -> i64 {
            party.net.iter().find(|a| a.currency_id.as_deref() == column).map(|a| a.sats).unwrap_or(0)
        };
        let money = |sats: i64| format!("{} {}", format::coins(sats.saturating_abs()), self.currency(column));
        let mut parts: Vec<String> = Vec::new();

        if self.coinbase {
            let total: i64 = addresses.iter().map(|p| net(p)).sum();
            let to = if addresses.len() == 1 { self.name(addresses[0]) } else { format!("{} addresses", addresses.len()) };
            parts.push(format!("The block reward of {} went to {to}.", money(total)));
            if self.fee_pool {
                parts.push("One output records the fee pool.".into());
            }
            return parts.join(" ");
        }
        if self.tags.iter().any(|t| t == "Stake")
            && givers.is_empty()
            && receivers.is_empty()
            && let Some(owner) = parties.first()
            && let Some(stayed) = owner.carried.first()
        {
            return format!(
                "This transaction staked the block. {} stayed at {}. No value moved.",
                self.amount(stayed),
                self.name(owner)
            );
        }

        // The kind, from Carbide's tags.
        if let Some(mark) = written.iter().find(|m| m.label == "Import") {
            parts.push(match &mark.currency_id {
                Some(id) => format!("An import into {}.", self.names.currency_name(id)),
                None => "An import.".into(),
            });
        } else if let Some(mark) = written.iter().find(|m| m.label == "Export") {
            parts.push(match &mark.currency_id {
                Some(id) => format!("An export for {}.", self.names.currency_name(id)),
                None => "An export.".into(),
            });
        }

        let giver = if givers.len() == 1 { self.name(givers[0]) } else { format!("{} parties", givers.len()) };
        if !statement.routes.is_empty() {
            let conversions = statement.routes.iter().all(|r| r.conversion);
            let noun = if conversions { "conversion" } else { "transfer" };
            let count = statement.routes.len();
            let mut text = format!("{} started {count} {noun}{}", capital(&giver), if count == 1 { "" } else { "s" });
            if count <= 2 {
                let each: Vec<String> = statement
                    .routes
                    .iter()
                    .map(|route| {
                        let amounts: Vec<String> = route.amounts.iter().map(|a| self.amount(a)).collect();
                        let mut words = list(&amounts);
                        if let Some(to) = &route.to_currency {
                            words += &format!(" to {}", self.names.currency_name(to));
                        }
                        if let Some(via) = &route.via {
                            words += &format!(" through {}", self.names.currency_name(via));
                        }
                        words
                    })
                    .collect();
                text += &format!(": {}", list(&each));
            }
            parts.push(text + ".");
            parts.push(format!("The {} not in this transaction.", if count == 1 { "result is" } else { "results are" }));
        } else if !receivers.is_empty() && !givers.is_empty() {
            let mut clauses: Vec<String> = Vec::new();
            if one_column && addresses.len() <= 2 {
                clauses.extend(addresses.iter().map(|p| format!("{} to {}", money(net(p)), self.name(p))));
            } else if !addresses.is_empty() {
                let what = if one_column {
                    money(addresses.iter().map(|p| net(p)).sum())
                } else {
                    let names: Vec<String> = statement.columns.iter().map(|c| self.currency(c.as_deref())).collect();
                    list(&names)
                };
                let to = if addresses.len() == 1 { self.name(addresses[0]) } else { format!("{} addresses", addresses.len()) };
                clauses.push(format!("{what} to {to}"));
            }
            if let Some(pool) = pool
                && receivers.iter().any(|p| p.kind == PartyKind::ShieldedPool)
            {
                clauses.push(format!("{} into the shielded pool", money(net(pool))));
            }
            let verb = if addresses.is_empty() { "sent" } else { "paid" };
            parts.push(format!("{} {verb} {}.", capital(&giver), list(&clauses)));
            if let Some(pool) = pool {
                parts.push(
                    if net(pool) > 0 { "The recipient in the pool is hidden." } else { "The sender in the pool is hidden." }
                        .into(),
                );
            }
            if addresses.len() > MAX_RECEIVERS && one_column {
                let mut sorted: Vec<i64> = addresses.iter().map(|p| net(p)).collect();
                sorted.sort();
                parts.push(format!("Half of the addresses received less than {}.", money(sorted[sorted.len() / 2])));
            }
        } else if !written.is_empty() && !givers.is_empty() {
            let labels: Vec<String> = written.iter().map(|m| m.label.to_lowercase()).collect();
            parts.push(format!("{} wrote: {}.", capital(&giver), list(&labels)));
        } else if givers.is_empty() && receivers.is_empty() && parts.is_empty() {
            parts.push(if self.tags.is_empty() {
                "No value moved.".into()
            } else {
                format!("{}. No value moved.", self.tags.iter().take(3).cloned().collect::<Vec<_>>().join(", "))
            });
        }

        if statement.closing_kind == ClosingKind::Fee
            && let Some(fee) = statement.closing.iter().find(|a| a.currency_id.is_none())
        {
            parts.push(format!("The fee was {} {}.", format::coins(fee.sats), self.coin));
            if receivers.is_empty() && statement.routes.is_empty() && !givers.is_empty() {
                parts.push("No other value moved.".into());
            }
        }
        parts.join(" ")
    }
}

/// The sentence of a transaction that the page read, or None when it has none.
pub fn sentence(detail: Option<&api::WaitingTransaction>, names: &Names) -> Option<String> {
    Some(Words::of(detail?, names)?.sentence())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn party(kind: PartyKind, address: &str, net: &[i64]) -> StatementParty {
        StatementParty {
            kind,
            addresses: if address.is_empty() { vec![] } else { vec![address.to_string()] },
            currency_id: None,
            net: net.iter().map(|sats| StatementAmount { currency_id: None, sats: *sats }).collect(),
            carried: vec![],
            inputs: vec![],
            outputs: vec![],
        }
    }

    fn statement(parties: Vec<StatementParty>, fee: Option<i64>) -> TransactionStatement {
        TransactionStatement {
            columns: vec![None],
            parties,
            closing: fee.map(|sats| vec![StatementAmount { currency_id: None, sats }]).unwrap_or_default(),
            closing_kind: if fee.is_some() { ClosingKind::Fee } else { ClosingKind::None },
            marks: vec![],
            routes: vec![],
            complete: true,
        }
    }

    fn names() -> Names {
        Names { coin: "VRSC".into(), chain_id: None, shows_names: true, names: HashMap::new() }
    }

    #[test]
    fn a_payment() {
        let s = statement(
            vec![
                party(PartyKind::Address, "RSender0000000000000000000000000000", &[-150_010_000]),
                party(PartyKind::Address, "RReceiver000000000000000000000000000", &[150_000_000]),
            ],
            Some(10_000),
        );
        let names = names();
        let words = Words { statement: &s, coin: "VRSC", tags: &[], coinbase: false, fee_pool: false, names: &names };
        assert_eq!(
            words.sentence(),
            "RSender0…000000 paid 1.5 VRSC to RReceive…000000. The fee was 0.0001 VRSC."
        );
    }

    #[test]
    fn no_value_and_a_reward() {
        let names = names();
        let s = statement(vec![], None);
        let tags = vec!["Identity".to_string()];
        let words = Words { statement: &s, coin: "VRSC", tags: &tags, coinbase: false, fee_pool: false, names: &names };
        assert_eq!(words.sentence(), "Identity. No value moved.");
        let s = statement(vec![party(PartyKind::Address, "RMiner", &[600_000_000])], None);
        let words = Words { statement: &s, coin: "VRSC", tags: &[], coinbase: true, fee_pool: true, names: &names };
        assert_eq!(words.sentence(), "The block reward of 6 VRSC went to RMiner. One output records the fee pool.");
    }

    #[test]
    fn names_of_currencies() {
        let mut names = names();
        names.chain_id = Some("i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV".into());
        names.names.insert("iCurrency00000000000000000000000000".into(), "Bridge.vETH@".into());
        assert_eq!(names.currency_name("i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV"), "VRSC");
        assert_eq!(names.currency_name("iCurrency00000000000000000000000000"), "Bridge.vETH");
        assert_eq!(names.display_id("iCurrency00000000000000000000000000"), "Bridge.vETH@");
        names.shows_names = false;
        assert_eq!(names.currency_name("iCurrency00000000000000000000000000"), "iCurrenc…000000");
    }
}
