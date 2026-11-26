//! Owner: the Transaction page (`pages/transaction`). The blocks that hold a transaction
//! across side branches, and the stored child blocks of a block (`InclusionCard`,
//! `BlockBranchesCard`, `MembershipMark` and `ReloadOnRevision` in `InclusionCard.swift`).
//! Used by the Transaction page and the Block page.
//!
//! Not ported yet: the cards say so. Keep the signatures.

use std::rc::Rc;

use crate::page::{self, Card, Page};

/// The blocks that hold the transaction (the terminal's F7).
pub fn inclusion_card(page: &Rc<Page>, txid: &str) -> Card {
    let _ = txid;
    let card = page.card("Block history", None);
    card.append(&page::note("This part is not ported yet. The macOS app shows it in InclusionCard.swift.", false));
    card
}

/// The stored blocks that follow a block, on each branch.
pub fn block_branches_card(page: &Rc<Page>, hash: &str) -> Card {
    let _ = hash;
    let card = page.card("Next blocks", None);
    card.append(&page::note("This part is not ported yet. The macOS app shows it in InclusionCard.swift.", false));
    card
}
