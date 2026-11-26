//! The Estimate page (`EstimateView.swift`): a future block height or moment, opened from
//! search (`estimate:`, or a height or date beyond the tip). The page holds the same
//! estimator as the Charts page.

use std::rc::Rc;

use gtk::prelude::*;

use crate::model::AppModel;
use crate::page::load::clear;
use crate::page::Page;
use crate::shared::estimator::block_estimator;

/// The page. Its signature is fixed by `pages/mod.rs`. `query` is an estimator target: a
/// height, or `unix:` and seconds.
pub fn build(model: &Rc<AppModel>, query: &str) -> Rc<Page> {
    let page = Page::scrolling(model);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    page.append(&holder);
    holder.append(&block_estimator(&page, Some(query)));
    // Refresh builds the estimator again, which runs the query again.
    let (weak, query, holder_ref) = (Rc::downgrade(&page), query.to_string(), holder.downgrade());
    page.hooks().refresh = Some(Rc::new(move || {
        if let (Some(page), Some(holder)) = (weak.upgrade(), holder_ref.upgrade()) {
            clear(&holder);
            holder.append(&block_estimator(&page, Some(&query)));
        }
    }));
    page
}
