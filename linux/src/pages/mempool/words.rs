//! The sentence of a statement for the Mempool page: what a waiting transaction did, in
//! words. The words are the shared `StatementWords` of `shared/statement_words.rs`, as the
//! Swift app has one `StatementWords` for the Transaction page and the Mempool page.

use crate::bridge::api;
use crate::shared::statement_words::{Namer, StatementWords};

/// The words of a transaction that the Mempool page read. None when it has no statement.
pub fn words<'a>(detail: &'a api::WaitingTransaction, namer: &'a Namer) -> Option<StatementWords<'a>> {
    Some(StatementWords {
        statement: detail.statement.as_ref()?,
        coin: &detail.coin,
        tags: &detail.tags,
        coinbase: detail.coinbase,
        fee_pool: detail.fee_pool_checkpoint,
        namer,
    })
}

/// The sentence of a transaction that the page read, or None when it has none.
pub fn sentence(detail: Option<&api::WaitingTransaction>, namer: &Namer) -> Option<String> {
    Some(words(detail?, namer)?.sentence())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::bridge::api::{ClosingKind, PartyKind, StatementAmount, StatementParty, TransactionStatement};

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

    fn names() -> Namer {
        Namer { coin: "VRSC".into(), chain_id: None, shows_names: true, names: HashMap::new() }
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
        let words = StatementWords { statement: &s, coin: "VRSC", tags: &[], coinbase: false, fee_pool: false, namer: &names };
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
        let words = StatementWords { statement: &s, coin: "VRSC", tags: &tags, coinbase: false, fee_pool: false, namer: &names };
        assert_eq!(words.sentence(), "Identity. No value moved.");
        let s = statement(vec![party(PartyKind::Address, "RMiner", &[600_000_000])], None);
        let words = StatementWords { statement: &s, coin: "VRSC", tags: &[], coinbase: true, fee_pool: true, namer: &names };
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
