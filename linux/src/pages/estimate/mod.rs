//! The Estimate page. Port of EstimateView.swift and BlockEstimator.swift in `macos/Sources/CarbideSilica/Views`.
//!
//! Not ported yet. This module owns its folder: add files here and declare them below.

use std::rc::Rc;

use crate::model::AppModel;
use crate::page::{self, Page};

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>, query: &str) -> Rc<Page> {
    let _ = query;
    page::not_ported(model, "Estimate", "EstimateView.swift and BlockEstimator.swift")
}
