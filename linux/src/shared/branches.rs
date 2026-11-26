//! Owner: the Node page (`pages/node`). Side branches: the ends of the chains that the node
//! knows, drawn against the active chain (`BranchesCard` in `NodeDiagrams.swift`). Used by
//! the Node page and the Block page.
//!
//! Not ported yet: the card says so. Keep the signature.

use std::rc::Rc;

use crate::page::{self, Card, Page};

/// The card of side branches, for a page.
pub fn branches_card(page: &Rc<Page>) -> Card {
    let card = page.card("Branches", None);
    card.append(&page::note("This part is not ported yet. The macOS app shows it in NodeDiagrams.swift.", false));
    card
}
