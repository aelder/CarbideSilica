//! The Mempool page. Port of MempoolView.swift in `macos/Sources/CarbideSilica/Views`.
//!
//! Not ported yet. This module owns its folder: add files here and declare them below.

use std::rc::Rc;

use crate::model::AppModel;
use crate::page::{self, Page};

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>) -> Rc<Page> {
    page::not_ported(model, "Mempool", "MempoolView.swift")
}

/// Work for the whole session: the Swift sidebar keeps the record of the mempool on every
/// page (`MempoolFeed.refresh` every `MempoolFeed.interval`), so that the Mempool page has
/// the last minutes when a person opens it. Not ported yet: it does nothing.
pub fn background(model: &Rc<AppModel>) {
    let _ = model;
}
