//! The Address page. Port of AddressView.swift in `macos/Sources/CarbideSilica/Views`: a
//! transparent address, its balances, the identities that use it as a primary address, and
//! its transactions as tiles of inputs and outputs.

mod activity;

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Route, Explorer};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, LoadView, Page};
use crate::shared::{identity_support, json_tree};
use crate::widgets::{lamp, link, loader};

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
        balances.append(&balance_list(&model, &overview.balances));
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
    transactions.append(&activity::ActivityList::new(page, &overview.address, Some(reload)));
    column.append(&transactions.root);

    column.append(&json_tree::json_section(page, &overview.json, &format!("address-{}.json", overview.address)));

    model.resolve_names(overview.balances.iter().map(|b| b.currency_id.clone()));
}

/// Balances by currency, the chain's own coin first (`BalanceList`). The names follow the
/// setting and arrive after the page.
fn balance_list(model: &Rc<AppModel>, balances: &[api::AddressBalance]) -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let fill = {
        let (list, balances, model) = (list.clone(), balances.to_vec(), Rc::downgrade(model));
        move || {
            let Some(model) = model.upgrade() else { return };
            page::load::clear(&list);
            for balance in &balances {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                let name = balance_label(&model, balance);
                let route = Route::Currency {
                    name: if balance.native { model.coin() } else { balance.currency_id.clone() },
                };
                let weak = Rc::downgrade(&model);
                let key = link::link(&name, false, move || {
                    if let Some(model) = weak.upgrade() {
                        model.open(route.clone());
                    }
                });
                key.set_tooltip_text(Some(&balance.currency_id));
                key.set_size_request(170, -1);
                if let Some(label) = link::label_of(&key) {
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label.set_max_width_chars(20);
                }
                row.append(&key);
                let amount = page::text(&format::coins(balance.amount_sats), &["tabular"]);
                amount.set_valign(gtk::Align::Baseline);
                row.append(&amount);
                let ticker = page::text(&name, &["secondary"]);
                ticker.set_valign(gtk::Align::Baseline);
                row.append(&ticker);
                list.append(&row);
            }
        }
    };
    fill();
    model.watch(&[Topic::Names, Topic::Display], &list, move |_| fill());
    list
}

fn balance_label(model: &AppModel, balance: &api::AddressBalance) -> String {
    if balance.native {
        return model.coin();
    }
    if crate::display::shows_names()
        && let Some(name) = model.state().names.get(&balance.currency_id)
    {
        return name.strip_suffix('@').unwrap_or(name).to_string();
    }
    balance.name.clone()
}

/// The identities that have this address as a primary address now. The request needs the
/// identity index.
fn primary_address_card(page: &Rc<Page>, address: &str) -> page::Card {
    let card = page.card("Primary address for", None);
    let address = address.to_string();
    let loader = section_loader(
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
    card.append(&loader);
    card
}

/// Loads data for one section and shows progress or an error in the section, not over the
/// whole page (`SectionLoader` in `IdentitySupport.swift`). It loads again on a new
/// connection and on Try Again.
fn section_loader<T, Work, Fut, Show>(model: &Rc<AppModel>, loading_text: &str, work: Work, show: Show) -> gtk::Box
where
    T: Send + 'static,
    Work: Fn(Explorer) -> Fut + 'static,
    Fut: Future<Output = Result<T, api::CarbideError>> + Send + 'static,
    Show: Fn(&gtk::Box, T) + 'static,
{
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 8);
    holder.set_hexpand(true);
    let attempt = Rc::new(Cell::new(0u64));
    let (work, show, loading_text) = (Rc::new(work), Rc::new(show), loading_text.to_string());
    // The load holds itself weakly, for its Try Again key. The watcher below holds it
    // while the section exists.
    let this: Rc<RefCell<Weak<dyn Fn()>>> = Rc::new(RefCell::new(Weak::<fn()>::new()));
    let load: Rc<dyn Fn()> = Rc::new({
        let (holder, model, this) = (holder.downgrade(), Rc::downgrade(model), this.clone());
        move || {
            let (Some(holder), Some(model)) = (holder.upgrade(), model.upgrade()) else { return };
            let again = this.borrow().upgrade();
            let current = attempt.get() + 1;
            attempt.set(current);
            page::load::clear(&holder);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.append(&loader::small());
            row.append(&page::text(&loading_text, &["secondary"]));
            holder.append(&row);
            let explorer = match model.require_explorer() {
                Ok(explorer) => explorer,
                Err(message) => return failed(&holder, &message, again),
            };
            let (work, show, attempt) = (work.clone(), show.clone(), attempt.clone());
            let generation = model.generation();
            let (holder_weak, model_weak) = (holder.downgrade(), Rc::downgrade(&model));
            glib::spawn_future_local(async move {
                let result = bridge::run(work(explorer)).await;
                let (Some(holder), Some(model)) = (holder_weak.upgrade(), model_weak.upgrade()) else { return };
                if attempt.get() != current || model.generation() != generation {
                    return;
                }
                page::load::clear(&holder);
                match result {
                    Ok(value) => show(&holder, value),
                    Err(error) => failed(&holder, &bridge::describe(&error), again),
                }
            });
        }
    });
    *this.borrow_mut() = Rc::downgrade(&load);
    let again = load.clone();
    model.watch(&[Topic::Connection], &holder, move |_| again());
    load();
    holder
}

fn failed(holder: &gtk::Box, message: &str, again: Option<Rc<dyn Fn()>>) {
    page::load::clear(holder);
    holder.append(&page::note(message, true));
    holder.append(&lamp::button("Try Again", move || {
        if let Some(again) = &again {
            again();
        }
    }));
}
