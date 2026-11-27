//! Small parts of the identity and address
//! pages (`IdentitySupport.swift`): `IDLink`, `IDRow`, `ValueRow`, `SectionLoader`,
//! `heightAndTime`, `BalanceList` (`IdentityView.swift`) and the `AppModel` extension
//! (`heightText`, `amountText`). Used by the Identity page and the Address page.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Route, Explorer};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, Page};
use crate::theme::tone;
use crate::widgets::icons::{self, Icon};
use crate::widgets::{display, lamp, link, loader};

/// A link to an identity, address, currency or transaction, with its name when the name
/// is known and "Show Names for IDs" is on (`IDLink`). It is a stop of the page's cursor.
pub fn id_link(page: &Page, id: &str, route: Route, shorten: bool) -> gtk::Button {
    page::route_link(page, id, route, shorten)
}

/// A label and a linked ID, with a copy key (`IDRow`). `note` follows the link in the
/// secondary colour, such as "output 2".
pub fn id_row(page: &Page, label: &str, id: &str, route: Route, note: Option<&str>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let name = display::name_text(label);
    name.set_size_request(170, -1);
    name.set_xalign(0.0);
    name.set_valign(gtk::Align::Baseline);
    row.append(&name);
    let link = id_link(page, id, route, false);
    if let Some(child) = crate::widgets::link::label_of(&link) {
        child.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    }
    row.append(&link);
    if let Some(note) = note {
        let text = page::text(note, &["callout", "secondary"]);
        text.set_valign(gtk::Align::Baseline);
        row.append(&text);
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    row.append(&spacer);
    row.append(&page::copy_button(id));
    row
}

/// A label and a plain value (`ValueRow`). A warning shows the value with a triangle in
/// the mined colour.
pub fn value_row(label: &str, value: &str, warning: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let name = display::name_text(label);
    name.set_size_request(170, -1);
    name.set_xalign(0.0);
    name.set_valign(gtk::Align::Baseline);
    row.append(&name);
    if warning {
        let mark = icons::icon(Icon::Warning, 13, tone::MINED);
        mark.set_valign(gtk::Align::Center);
        row.append(&mark);
        let shown = page::paragraph(value, &["mined"]);
        shown.set_valign(gtk::Align::Baseline);
        shown.set_hexpand(true);
        row.append(&shown);
    } else {
        let shown = page::paragraph(value, &[]);
        shown.set_selectable(true);
        shown.set_valign(gtk::Align::Baseline);
        shown.set_hexpand(true);
        row.append(&shown);
    }
    row
}

/// A row of the small progress mark and a sentence: SwiftUI's small `ProgressView` with a
/// text after it.
pub fn small_progress(text: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let mark = loader::small();
    mark.set_valign(gtk::Align::Center);
    row.append(&mark);
    row.append(&page::text(text, &["secondary"]));
    row
}

/// Loads data for one section and shows progress or an error in the section, not over
/// the whole page (`SectionLoader`). It loads again after a new connection and after
/// Try Again. The section keeps working while its widget lives, also when the page does
/// not keep this value.
pub struct SectionLoader {
    pub root: gtk::Box,
    reload: Rc<dyn Fn()>,
}

impl SectionLoader {
    /// `load` runs on the bridge; `content` fills the holder, which it gets empty.
    pub fn new<T, Work, Fut, Show>(model: &Rc<AppModel>, loading_text: &str, load: Work, content: Show) -> Rc<Self>
    where
        T: Send + 'static,
        Work: Fn(Explorer) -> Fut + 'static,
        Fut: Future<Output = Result<T, api::CarbideError>> + Send + 'static,
        Show: Fn(&gtk::Box, T) + 'static,
    {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_hexpand(true);
        let attempt = Rc::new(Cell::new(0u64));
        let (load, content, loading_text) = (Rc::new(load), Rc::new(content), loading_text.to_string());
        // The load holds itself weakly, for its Try Again key. The watcher of the
        // connection, which lives with the widget, holds it strongly.
        let this: Rc<RefCell<Weak<dyn Fn()>>> = Rc::new(RefCell::new(Weak::<fn()>::new()));
        let reload: Rc<dyn Fn()> = Rc::new({
            let (holder, model, this) = (root.downgrade(), Rc::downgrade(model), this.clone());
            move || {
                let (Some(holder), Some(model)) = (holder.upgrade(), model.upgrade()) else { return };
                let again = this.borrow().upgrade();
                let current = attempt.get() + 1;
                attempt.set(current);
                page::load::clear(&holder);
                holder.append(&small_progress(&loading_text));
                let explorer = match model.require_explorer() {
                    Ok(explorer) => explorer,
                    Err(message) => return show_failure(&holder, &message, again),
                };
                let (load, content, attempt) = (load.clone(), content.clone(), attempt.clone());
                let generation = model.generation();
                let (holder_weak, model_weak) = (holder.downgrade(), Rc::downgrade(&model));
                glib::spawn_future_local(async move {
                    let result = bridge::run(load(explorer)).await;
                    let (Some(holder), Some(model)) = (holder_weak.upgrade(), model_weak.upgrade()) else { return };
                    if attempt.get() != current || model.generation() != generation {
                        return;
                    }
                    page::load::clear(&holder);
                    match result {
                        Ok(value) => content(&holder, value),
                        Err(error) => show_failure(&holder, &bridge::describe(&error), again),
                    }
                });
            }
        });
        *this.borrow_mut() = Rc::downgrade(&reload);
        let again = reload.clone();
        model.watch(&[Topic::Connection], &root, move |_| again());
        reload();
        Rc::new(Self { root, reload })
    }

    pub fn reload(&self) {
        (self.reload)();
    }
}

/// The failure of a section: the message as a warning and Try Again.
fn show_failure(holder: &gtk::Box, message: &str, again: Option<Rc<dyn Fn()>>) {
    page::load::clear(holder);
    holder.append(&page::note(message, true));
    holder.append(&lamp::button("Try Again", move || {
        if let Some(again) = &again {
            again();
        }
    }));
}

/// Balances by currency, the chain's own coin first (`BalanceList`). The names follow
/// "Show Names for IDs" and arrive after the list.
pub fn balance_list(model: &Rc<AppModel>, balances: &[api::AddressBalance]) -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let fill = {
        let (list, balances, model) = (list.downgrade(), balances.to_vec(), Rc::downgrade(model));
        move || {
            let (Some(model), Some(list)) = (model.upgrade(), list.upgrade()) else { return };
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
                if let Some(label) = link::label_of(&key) {
                    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    label.set_single_line_mode(true);
                }
                // The name has a column of 170, and a long name is cut in it.
                let cell = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                cell.set_size_request(170, -1);
                key.set_hexpand(false);
                cell.append(&key);
                row.append(&cell);
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

/// The name of a balance's currency: the chain's coin, the resolved name without "@", or
/// the name that the source gave.
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

/// A height and the date of its block, as Carbide's `format_identity_height_time`: the date
/// alone when dates are shown for heights, otherwise "block 123, date" (`heightAndTime`).
pub fn height_and_time(height: u64, time: Option<u64>, model: &Rc<AppModel>) -> String {
    if crate::display::shows_dates() {
        return crate::display::height_text(height, time, model);
    }
    match time {
        None => format!("block {}, time not available", format::integer(height)),
        Some(time) => format!("block {}, {}", format::integer(height), format::date(time)),
    }
}

/// A date with no time, as `Date.formatted(date: .abbreviated, time: .omitted)`:
/// "Dec 4, 2020", in the local time zone as the Swift form is.
pub fn abbreviated_date(unix: u64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(unix as i64, 0)
        .single()
        .map(|time| time.format("%b %-d, %Y").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviated_date_drops_the_time() {
        let text = abbreviated_date(1_607_100_000);
        assert!(!text.contains(" at "));
        assert!(text.contains("2020"));
    }
}
