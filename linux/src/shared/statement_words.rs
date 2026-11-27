//! The words of a statement (`StatementText` and `StatementWords` in
//! `TransactionStatement.swift`, `MempoolText.span` in `MempoolView.swift`): amounts in
//! brief, lists in words, the names of parties and currencies, and the sentence that says
//! what the transaction did. The Transaction page and the Mempool page use the same words.

use std::collections::HashMap;

use crate::bridge::api::{ClosingKind, PartyKind, StatementAmount, StatementParty, TransactionStatement};
use crate::format;
use crate::model::{self, AppModel};

/// More receiving addresses than this are counted, not named (`StatementLayout.maxReceivers`).
pub const MAX_RECEIVERS: usize = 8;

/// An amount with as many decimals as its size needs. The full amount is in the help text.
pub fn brief(sats: i64) -> String {
    let magnitude = sats.unsigned_abs() as u128;
    let digits: u32 = if magnitude >= 100_000_000_000 {
        2
    } else if magnitude >= 100_000_000 {
        4
    } else {
        8
    };
    let divisor = 10u128.pow(8 - digits);
    let mut quotient = magnitude / divisor;
    let remainder = magnitude % divisor;
    // Halves go to the even neighbour, as the number formats of the Swift app do.
    if remainder * 2 > divisor || (remainder * 2 == divisor && quotient % 2 == 1) {
        quotient += 1;
    }
    let unit = 10u128.pow(digits);
    let whole = (quotient / unit).to_string();
    let fraction = format!("{:0width$}", quotient % unit, width = digits as usize);
    let fraction = fraction.trim_end_matches('0');
    if fraction.is_empty() { format::group(&whole) } else { format!("{}.{}", format::group(&whole), fraction) }
}

pub fn signed(sats: i64) -> String {
    format!("{}{}", if sats < 0 { "−" } else { "+" }, brief(sats))
}

/// "a", "a and b", "a, b and c".
pub fn list(parts: &[String]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        n => format!("{} and {}", parts[..n - 1].join(", "), parts[n - 1]),
    }
}

pub fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// A length of time in words: seconds to two minutes, then minutes (`MempoolText.span`).
pub fn span(seconds: f64) -> String {
    let seconds = if seconds.is_finite() { seconds.round().max(0.0) as i64 } else { 0 };
    if seconds < 120 {
        return format!("{seconds} s");
    }
    if seconds < 3600 {
        return format!("{} min", seconds / 60);
    }
    format!("{} h {} min", seconds / 3600, seconds % 3600 / 60)
}

/// The names that the page knows, and the setting that shows them, at one moment. The
/// words are made from it, so that they can be tested without a model.
#[derive(Clone, Debug, Default)]
pub struct Namer {
    pub names: HashMap<String, String>,
    pub shows_names: bool,
    pub chain_id: Option<String>,
    /// The connected chain's coin (`model.coin`).
    pub coin: String,
}

impl Namer {
    pub fn from_model(model: &AppModel) -> Self {
        let coin = model.coin();
        let state = model.state();
        Self {
            names: state.names.clone(),
            shows_names: crate::display::shows_names(),
            chain_id: state.status.as_ref().and_then(|s| s.chain_id.clone()),
            coin,
        }
    }

    /// A currency's name: the coin for the chain itself, the name without "@", or a short
    /// ID (`AppModel.currencyName`).
    pub fn currency_name(&self, id: &str) -> String {
        model::currency_name_in(&self.names, self.shows_names, self.chain_id.as_deref(), &self.coin, id)
    }

    /// An identity or currency ID as the "Show Names for IDs" setting says: the name, or a
    /// shortened ID (`AppModel.displayID`).
    pub fn display_id(&self, id: &str) -> String {
        model::display_id_in(&self.names, self.shows_names, id)
    }

    /// A currency's name with a shorter ID when no name is known (`currencyName` of
    /// `OutputRow`, `systemName` of `TransferTraceCard`, `name` of `RateLines`).
    pub fn short_name(&self, id: &str) -> String {
        match self.names.get(id) {
            Some(name) if self.shows_names => name.strip_suffix('@').unwrap_or(name).to_string(),
            _ => format::short(id, 6, 4),
        }
    }

    /// `Display.name(for:in:currency:)`: the name, or the whole ID.
    pub fn name_for(&self, id: &str, currency: bool) -> String {
        match self.names.get(id) {
            Some(name) if self.shows_names => {
                if currency { name.strip_suffix('@').unwrap_or(name).to_string() } else { name.clone() }
            }
            _ => id.to_string(),
        }
    }
}

/// The words of one statement.
pub struct StatementWords<'a> {
    pub statement: &'a TransactionStatement,
    /// The transaction's native coin.
    pub coin: &'a str,
    pub tags: &'a [String],
    pub coinbase: bool,
    pub fee_pool: bool,
    pub namer: &'a Namer,
}

impl StatementWords<'_> {
    pub fn currency(&self, id: Option<&str>) -> String {
        match id {
            Some(id) => self.namer.currency_name(id),
            None => self.coin.to_string(),
        }
    }

    pub fn amount(&self, amount: &StatementAmount) -> String {
        format!("{} {}", format::coins(amount.sats.saturating_abs()), self.currency(amount.currency_id.as_deref()))
    }

    /// A party in the middle of a sentence.
    pub fn name(&self, party: &StatementParty) -> String {
        match party.kind {
            // An output script can name more than one address. The lists below show each.
            PartyKind::Address => party.addresses.first().map(|a| self.namer.display_id(a)).unwrap_or_else(|| "an address".into()),
            PartyKind::ReserveDeposits => match &party.currency_id {
                Some(id) => format!("the reserve deposits of {}", self.namer.currency_name(id)),
                None => "the reserve deposits".into(),
            },
            PartyKind::WaitingTransfers => "the waiting transfers".into(),
            PartyKind::NewCoins => "new coins and fees".into(),
            PartyKind::ShieldedPool => "the shielded pool".into(),
            PartyKind::NoAddress => "an output with no address".into(),
        }
    }

    pub fn row_title(&self, party: &StatementParty) -> String {
        let name = self.name(party);
        capital(name.strip_prefix("the ").unwrap_or(&name))
    }

    pub fn sentence(&self) -> String {
        let statement = self.statement;
        let parties = &statement.parties;
        let gives = |p: &StatementParty| p.net.iter().any(|a| a.sats < 0);
        let givers: Vec<&StatementParty> = parties.iter().filter(|p| gives(p)).collect();
        let receivers: Vec<&StatementParty> = parties.iter().filter(|p| !p.net.is_empty() && !gives(p)).collect();
        let addresses: Vec<&StatementParty> =
            receivers.iter().copied().filter(|p| matches!(p.kind, PartyKind::Address | PartyKind::NoAddress)).collect();
        let pool = parties.iter().find(|p| p.kind == PartyKind::ShieldedPool);
        let written: Vec<_> = statement.marks.iter().filter(|m| !m.spent).collect();
        let one_column = statement.columns.len() == 1;
        let column: Option<&str> = statement.columns.first().and_then(|c| c.as_deref());
        let net = |p: &StatementParty| -> i64 {
            p.net.iter().find(|a| a.currency_id.as_deref() == column).map(|a| a.sats).unwrap_or(0)
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
            return format!("This transaction staked the block. {} stayed at {}. No value moved.", self.amount(stayed), self.name(owner));
        }

        // The kind, from Carbide's tags.
        if let Some(mark) = written.iter().find(|m| m.label == "Import") {
            parts.push(match &mark.currency_id {
                Some(id) => format!("An import into {}.", self.namer.currency_name(id)),
                None => "An import.".into(),
            });
        } else if let Some(mark) = written.iter().find(|m| m.label == "Export") {
            parts.push(match &mark.currency_id {
                Some(id) => format!("An export for {}.", self.namer.currency_name(id)),
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
                        let mut words = list(&route.amounts.iter().map(|a| self.amount(a)).collect::<Vec<_>>());
                        if let Some(to) = &route.to_currency {
                            words += &format!(" to {}", self.namer.currency_name(to));
                        }
                        if let Some(via) = &route.via {
                            words += &format!(" through {}", self.namer.currency_name(via));
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
                    list(&statement.columns.iter().map(|c| self.currency(c.as_deref())).collect::<Vec<_>>())
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
                parts.push(if net(pool) > 0 { "The recipient in the pool is hidden." } else { "The sender in the pool is hidden." }.into());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::api::{StatementMark, StatementRoute};

    pub(crate) fn amount(currency: Option<&str>, sats: i64) -> StatementAmount {
        StatementAmount { currency_id: currency.map(str::to_string), sats }
    }

    pub(crate) fn party(kind: PartyKind, address: Option<&str>, net: Vec<StatementAmount>) -> StatementParty {
        StatementParty {
            kind,
            addresses: address.map(|a| vec![a.to_string()]).unwrap_or_default(),
            currency_id: None,
            net,
            carried: vec![],
            inputs: vec![],
            outputs: vec![],
        }
    }

    fn statement(parties: Vec<StatementParty>, fee: Option<i64>) -> TransactionStatement {
        TransactionStatement {
            columns: vec![None],
            parties,
            closing: fee.map(|f| vec![amount(None, f)]).unwrap_or_default(),
            closing_kind: if fee.is_some() { ClosingKind::Fee } else { ClosingKind::None },
            marks: vec![],
            routes: vec![],
            complete: true,
        }
    }

    fn namer() -> Namer {
        let mut names = HashMap::new();
        names.insert("iLuck".to_string(), "LuckPool@".to_string());
        Namer { names, shows_names: true, chain_id: Some("iChain".into()), coin: "VRSC".into() }
    }

    #[test]
    fn brief_amounts() {
        assert_eq!(brief(5_270_050_000), "52.7005");
        assert_eq!(brief(10_967_170), "0.1096717");
        assert_eq!(brief(-412_000_000), "4.12");
        assert_eq!(brief(123_456_789_012_345), "1,234,567.89");
        // A half goes to the even neighbour.
        assert_eq!(brief(100_005_000), "1");
        assert_eq!(brief(100_015_000), "1.0002");
        assert_eq!(signed(-150_000_000), "−1.5");
        assert_eq!(signed(1), "+0.00000001");
    }

    #[test]
    fn lists_and_capitals() {
        assert_eq!(list(&[]), "");
        assert_eq!(list(&["a".into()]), "a");
        assert_eq!(list(&["a".into(), "b".into(), "c".into()]), "a, b and c");
        assert_eq!(capital("the shielded pool"), "The shielded pool");
        assert_eq!(span(59.6), "60 s");
        assert_eq!(span(7300.0), "2 h 1 min");
    }

    #[test]
    fn names_of_currencies() {
        let n = namer();
        assert_eq!(n.currency_name("iChain"), "VRSC");
        assert_eq!(n.currency_name("iLuck"), "LuckPool");
        assert_eq!(n.display_id("iLuck"), "LuckPool@");
        assert_eq!(n.currency_name("iUnknown0123456789abcdefghijklmnop"), "iUnknown…klmnop");
    }

    #[test]
    fn a_payment_sentence() {
        let n = namer();
        let mut parties = vec![party(PartyKind::Address, Some("iLuck"), vec![amount(None, -20_517_480_000)])];
        for i in 0..10 {
            parties.push(party(PartyKind::Address, Some(&format!("R{i}")), vec![amount(None, 1_000_000 * (i + 1))]));
        }
        parties.push(party(PartyKind::ShieldedPool, None, vec![amount(None, 175_508)]));
        let s = statement(parties, Some(4_120_000));
        let tags: Vec<String> = vec![];
        let words = StatementWords { statement: &s, coin: "VRSC", tags: &tags, coinbase: false, fee_pool: false, namer: &n };
        assert_eq!(
            words.sentence(),
            "LuckPool@ paid 0.55 VRSC to 10 addresses and 0.00175508 VRSC into the shielded pool. \
             The recipient in the pool is hidden. Half of the addresses received less than 0.06 VRSC. \
             The fee was 0.0412 VRSC."
        );
        assert_eq!(words.row_title(&s.parties[11]), "Shielded pool");
    }

    #[test]
    fn a_coinbase_sentence() {
        let n = namer();
        let s = statement(
            vec![
                party(PartyKind::NewCoins, None, vec![amount(None, -300_046_564)]),
                party(PartyKind::Address, Some("RXPzH3i4fZqqqqqqqqqqqqqqqqrFXErEGz"), vec![amount(None, 300_046_564)]),
            ],
            None,
        );
        let tags = vec!["Coinbase".to_string()];
        let words = StatementWords { statement: &s, coin: "VRSC", tags: &tags, coinbase: true, fee_pool: true, namer: &n };
        assert_eq!(words.sentence(), "The block reward of 3.00046564 VRSC went to RXPzH3i4…XErEGz. One output records the fee pool.");
    }

    #[test]
    fn routes_and_marks() {
        let n = namer();
        let mut s = statement(vec![party(PartyKind::Address, Some("iLuck"), vec![amount(None, -100_000_000)])], Some(10_000));
        s.routes = vec![StatementRoute {
            output: 0,
            amounts: vec![amount(None, 99_990_000)],
            via: Some("iBridge".into()),
            to_currency: Some("iChain".into()),
            conversion: true,
            destination: None,
        }];
        s.marks = vec![StatementMark { label: "Export".into(), currency_id: None, spent: false, index: 1 }];
        let tags: Vec<String> = vec![];
        let words = StatementWords { statement: &s, coin: "VRSC", tags: &tags, coinbase: false, fee_pool: false, namer: &n };
        assert_eq!(
            words.sentence(),
            "An export. LuckPool@ started 1 conversion: 0.9999 VRSC to VRSC through iBridge. The result is not in this transaction. The fee was 0.0001 VRSC."
        );
        let empty = statement(vec![], None);
        let tags = vec!["Notarization".to_string()];
        let words = StatementWords { statement: &empty, coin: "VRSC", tags: &tags, coinbase: false, fee_pool: false, namer: &n };
        assert_eq!(words.sentence(), "Notarization. No value moved.");
    }
}
