//! Owner: the Estimate page (`pages/estimate`). The future block estimate from Carbide's
//! calibrated rate, with the calibration shown (`BlockEstimator.swift`). Used by the
//! Estimate page and the Charts page.
//!
//! Not ported yet: the part says so. Keep the signature.

use std::rc::Rc;

use crate::page::{self, Page};

/// The estimator. `query` is a frozen estimator target (a height, or `unix:` and
/// seconds), or None for an empty field.
pub fn block_estimator(page: &Rc<Page>, query: Option<&str>) -> gtk::Box {
    let _ = (page, query);
    page::note("This part is not ported yet. The macOS app shows it in BlockEstimator.swift.", false)
}
