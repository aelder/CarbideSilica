//! The Block page. Port of BlockView.swift in `macos/Sources/CarbideSilica/Views`.
//!
//! Not ported yet. This module owns its folder: add files here and declare them below.

use std::rc::Rc;

use crate::model::AppModel;
use crate::page::{self, Page};

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>, id: &str) -> Rc<Page> {
    let _ = id;
    page::not_ported(model, "Block", "BlockView.swift")
}
