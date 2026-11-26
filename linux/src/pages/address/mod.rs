//! The Address page. Port of AddressView.swift in `macos/Sources/CarbideSilica/Views`: a
//! transparent address, its balances, the identities that use it as a primary address, and
//! its transactions as tiles of inputs and outputs.


use std::rc::Rc;

use gtk::prelude::*;

use crate::bridge::{api, api::Route};
use crate::format;
use crate::model::AppModel;
use crate::page::{self, LoadView, Page};
use crate::shared::{address_activity, identity_support, json_tree};
use crate::widgets::link;

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>, address: &str) -> Rc<Page> {
    let page = Page::scrolling(model);
    let weak = Rc::downgrade(&page);
    let requested = address.to_string();
    let view: Rc<std::cell::RefCell<Option<Rc<LoadView>>>> = Rc::default();
    let slot = view.clone();
    let loaded = LoadView::new(
        model,
        move |explorer| {
            let address = requested.clone();
            async move { explorer.address_overview(address).await }
        },
        move |holder, overview| {
            let Some(page) = weak.upgrade() else { return };
            let reload: Rc<dyn Fn()> = {
                let slot = Rc::downgrade(&slot);
                Rc::new(move || {
                    if let Some(view) = slot.upgrade().and_then(|s| s.borrow().clone()) {
                        view.reload();
                    }
                })
            };
            show(&page, holder, overview, reload);
        },
    );
    page.append(&loaded.root);
    *view.borrow_mut() = Some(loaded.clone());
    // The refresh key holds the view, so it lives as long as the page.
    let keep = view.clone();
    page.hooks().refresh = Some(Rc::new(move || {
        if let Some(view) = keep.borrow().clone() {
            view.reload();
        }
    }));
    page
}

fn show(page: &Rc<Page>, holder: &gtk::Box, overview: api::AddressOverview, reload: Rc<dyn Fn()>) {
    let model = page.model.clone();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 20);
    holder.append(&column);
    column.append(&page::page_header(&overview.address, None, &[]));
    column.append(&page::provenance_note(&overview.provenance));

    let balances = page.card("Balances", None);
    if let Some(error) = &overview.balance_error {
        balances.append(&page::note(&format!("The balances did not load: {error}"), true));
    } else if overview.balances.is_empty() {
        balances.append(&page::text("No balances.", &["secondary"]));
    } else {
        balances.append(&identity_support::balance_list(&model, &overview.balances));
    }
    column.append(&balances.root);

    let activity = page.card("Activity", None);
    if let Some(error) = &overview.history_error {
        activity.append(&page::note(&format!("The transaction list did not load: {error}"), true));
    } else {
        activity.append(&identity_support::value_row("Transactions", &format::integer(overview.transaction_count), false));
        let date = |time: Option<u64>| time.map(format::date).unwrap_or_else(|| "unknown".into());
        activity.append(&identity_support::value_row("First", &date(overview.first_time), false));
        activity.append(&identity_support::value_row("Most recent", &date(overview.latest_time), false));
    }
    column.append(&activity.root);

    column.append(&primary_address_card(page, &overview.address).root);

    let transactions = page.card("Transactions", None);
    transactions.append(&address_activity::ActivityList::new(page, &overview.address, Some(reload)));
    column.append(&transactions.root);

    column.append(&json_tree::json_section(page, &overview.json, &format!("address-{}.json", overview.address)));

    model.resolve_names(overview.balances.iter().map(|b| b.currency_id.clone()));
}

/// The identities that have this address as a primary address now. The request needs the
/// identity index.
fn primary_address_card(page: &Rc<Page>, address: &str) -> page::Card {
    let card = page.card("Primary address for", None);
    let address = address.to_string();
    let loader = identity_support::SectionLoader::new(
        &page.model,
        "Requesting identities…",
        move |explorer| {
            let address = address.clone();
            async move { explorer.identities_with_primary_address(address).await }
        },
        {
            let model = Rc::downgrade(&page.model);
            move |holder: &gtk::Box, found: api::PrimaryAddressIdentities| {
                let Some(model) = model.upgrade() else { return };
                let count = found.identities.len();
                let source = if found.source == api::DataSource::PublicApi { "public API" } else { "local node" };
                holder.append(&page::text(
                    &format!("{count} identit{} via {source}", if count == 1 { "y" } else { "ies" }),
                    &["secondary"],
                ));
                if found.identities.is_empty() {
                    holder.append(&page::text("No associated identities found.", &["secondary"]));
                    return;
                }
                let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
                for (index, entry) in found.identities.iter().enumerate() {
                    if index > 0 {
                        list.append(&page::divider());
                    }
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    row.set_margin_top(5);
                    row.set_margin_bottom(5);
                    let (weak, id) = (Rc::downgrade(&model), entry.id.clone());
                    row.append(&link::link(&entry.name, false, move || {
                        if let Some(model) = weak.upgrade() {
                            model.open(Route::Identity { name: id.clone() });
                        }
                    }));
                    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    spacer.set_hexpand(true);
                    row.append(&spacer);
                    let id = page::text(&entry.id, &["mono", "label"]);
                    id.set_selectable(true);
                    row.append(&id);
                    list.append(&row);
                }
                holder.append(&list);
            }
        },
    );
    card.append(&loader.root);
    card
}
