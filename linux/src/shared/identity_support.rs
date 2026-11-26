//! Owner: the Identity page (`pages/identity`). Small parts of the identity and address
//! pages (`IdentitySupport.swift`): `IDLink`, `IDRow`, `ValueRow`, `SectionLoader`,
//! `heightAndTime` and the `AppModel` extension (`currencyName`, `displayID`, `heightText`,
//! `amountText`). Used by the Identity page and the Address page.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Route, Explorer};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, Page};
use crate::theme::tone;
use crate::widgets::icons::{self, Icon};
use crate::widgets::{display, lamp, loader};

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
/// Try Again.
pub struct SectionLoader {
    pub root: gtk::Box,
    attempt: Cell<u64>,
    reload: RefCell<Option<Rc<dyn Fn()>>>,
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
        let loader = Rc::new(Self { root, attempt: Cell::new(0), reload: RefCell::default() });
        let (load, content) = (Rc::new(load), Rc::new(content));
        let loading_text = loading_text.to_string();
        let (weak, weak_model) = (Rc::downgrade(&loader), Rc::downgrade(model));
        let reload: Rc<dyn Fn()> = Rc::new(move || {
            let (Some(this), Some(model)) = (weak.upgrade(), weak_model.upgrade()) else { return };
            let attempt = this.attempt.get() + 1;
            this.attempt.set(attempt);
            page::load::clear(&this.root);
            this.root.append(&small_progress(&loading_text));
            let explorer = match model.require_explorer() {
                Ok(explorer) => explorer,
                Err(message) => {
                    this.show_failure(&message);
                    return;
                }
            };
            let (load, content) = (load.clone(), content.clone());
            let generation = model.generation();
            let (weak, weak_model) = (Rc::downgrade(&this), Rc::downgrade(&model));
            glib::spawn_future_local(async move {
                let result = bridge::run(load(explorer)).await;
                let (Some(this), Some(model)) = (weak.upgrade(), weak_model.upgrade()) else { return };
                if this.attempt.get() != attempt || model.generation() != generation {
                    return;
                }
                page::load::clear(&this.root);
                match result {
                    Ok(value) => content(&this.root, value),
                    Err(error) => this.show_failure(&bridge::describe(&error)),
                }
            });
        });
        *loader.reload.borrow_mut() = Some(reload.clone());
        let again = reload.clone();
        model.watch(&[Topic::Connection], &loader.root, move |_| again());
        reload();
        loader
    }

    pub fn reload(&self) {
        let reload = self.reload.borrow().clone();
        if let Some(reload) = reload {
            reload();
        }
    }

    fn show_failure(self: &Rc<Self>, message: &str) {
        page::load::clear(&self.root);
        self.root.append(&page::note(message, true));
        let weak = Rc::downgrade(self);
        self.root.append(&lamp::button("Try Again", move || {
            if let Some(this) = weak.upgrade() {
                this.reload();
            }
        }));
    }
}

// MARK: - The AppModel extension of IdentitySupport.swift

/// A currency's display name: the chain's own coin, a resolved name without "@", or a
/// shortened ID (`currencyName`).
pub fn currency_name(model: &AppModel, id: &str) -> String {
    let coin = model.coin();
    let chain_id = model.state().status.as_ref().and_then(|s| s.chain_id.clone());
    if chain_id.as_deref() == Some(id) || id == coin {
        return coin;
    }
    if !crate::display::shows_names() {
        return format::short(id, 8, 6);
    }
    match model.state().names.get(id) {
        Some(name) => name.strip_suffix('@').unwrap_or(name).to_string(),
        None => format::short(id, 8, 6),
    }
}

/// An identity or currency ID as "Show Names for IDs" says: the name, or a shortened ID
/// (`displayID`).
pub fn display_id(model: &AppModel, id: &str) -> String {
    if crate::display::shows_names()
        && let Some(name) = model.state().names.get(id)
    {
        return name.clone();
    }
    format::short(id, 8, 6)
}

/// A block height as "Show Dates for Heights" says (`heightText`). `time` is the block's
/// time when it is known.
pub fn height_text(model: &Rc<AppModel>, height: u64, time: Option<u64>) -> String {
    if crate::display::shows_dates() {
        crate::display::height_text(height, time, model)
    } else {
        format!("block {}", format::integer(height))
    }
}

/// An amount in satoshis with its ticker (`amountText`).
pub fn amount_text(model: &AppModel, sats: i64, currency: &str) -> String {
    format!("{} {}", format::coins(sats), currency_name(model, currency))
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
