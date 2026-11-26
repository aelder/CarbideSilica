//! The Currency page. Port of CurrencyView.swift, CurrencyOverTime.swift, CurrencyStateSeek.swift, CurrencyStores.swift, NotarizationLanes.swift and NotarizationPicture.swift in `macos/Sources/CarbideSilica/Views`.
//!
//! Not ported yet. This module owns its folder: add files here and declare them below.

use std::rc::Rc;

use crate::model::AppModel;
use crate::page::{self, Page};

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>, name: &str) -> Rc<Page> {
    let _ = name;
    page::not_ported(model, "Currency", "CurrencyView.swift, CurrencyOverTime.swift, CurrencyStateSeek.swift, CurrencyStores.swift, NotarizationLanes.swift and NotarizationPicture.swift")
}
