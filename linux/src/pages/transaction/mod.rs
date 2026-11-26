//! The Transaction page. Port of TransactionView.swift, TransactionStatement.swift, TransactionExtras.swift, TransferTrace.swift, TraceRoute.swift, PoolBatch.swift and InclusionCard.swift in `macos/Sources/CarbideSilica/Views`.
//!
//! Not ported yet. This module owns its folder: add files here and declare them below.

use std::rc::Rc;

use crate::model::AppModel;
use crate::page::{self, Page};

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>, txid: &str) -> Rc<Page> {
    let _ = txid;
    page::not_ported(model, "Transaction", "TransactionView.swift, TransactionStatement.swift, TransactionExtras.swift, TransferTrace.swift, TraceRoute.swift, PoolBatch.swift and InclusionCard.swift")
}
