//! Owner: the Identity page (`pages/identity`). Small parts of the identity and address
//! pages (`IdentitySupport.swift`): `IDLink`, `ValueRow`, `SectionLoader` and
//! `heightAndTime`. Used by the Identity page and the Address page.

use std::rc::Rc;

use gtk::prelude::*;

use crate::bridge::api::Route;
use crate::format;
use crate::model::AppModel;
use crate::page::{self, Page};
use crate::widgets::display;

/// A link to an identity or currency ID, with its name when the name is known (`IDLink`).
pub fn id_link(page: &Page, id: &str, route: Route, shorten: bool) -> gtk::Button {
    page::route_link(page, id, route, shorten)
}

/// A label and a value, with a warning colour when `warning` (`ValueRow`).
pub fn value_row(label: &str, value: &str, warning: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let name = display::name_text(label);
    name.set_size_request(170, -1);
    row.append(&name);
    let shown = page::text(value, if warning { &["warning"] } else { &[] });
    shown.set_selectable(true);
    shown.set_wrap(true);
    row.append(&shown);
    row
}

/// A height and the time of its block: "4,339,576 (Nov 26, 2026 at …)" (`heightAndTime`).
pub fn height_and_time(height: u64, time: Option<u64>, model: &Rc<AppModel>) -> String {
    let text = crate::display::height_text(height, time, model);
    match time.filter(|t| *t > 0) {
        Some(time) if !crate::display::shows_dates() => format!("{text} ({})", format::date(time)),
        _ => text,
    }
}
