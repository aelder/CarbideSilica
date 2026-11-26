//! Owner: the Block page (`pages/block`). The node's JSON for the current object, collapsed
//! by default (`JSONSection` in `Components.swift`). Expanded, the Swift app shows it as a
//! tree with links, names and dates, a cursor, copy and save (`JSONTree.swift`). Used by
//! the Block, Transaction, Identity, Currency and Address pages.
//!
//! Partly ported: the card, Tab, copy and the JSON as text are here. The tree, its links,
//! names, dates, cursor and Save (s) are not.

use std::rc::Rc;

use gtk::prelude::*;

use crate::page::{self, Page};
use crate::widgets::lamp::Disclosure;

/// The "Raw JSON" card. `filename` is the file name that Save offers, with the object in it.
pub fn json_section(page: &Rc<Page>, json: &str, filename: &str) -> gtk::Box {
    let _ = filename;
    let card = page.card("Raw JSON", None);
    let pretty = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| json.to_string());
    let text = gtk::Label::new(Some(&pretty));
    text.add_css_class("mono");
    text.set_xalign(0.0);
    text.set_selectable(true);
    text.set_wrap(true);
    text.set_wrap_mode(gtk::pango::WrapMode::Char);
    let hint = page::text("Show the data from the node", &["secondary"]);
    let disclosure = Disclosure::new("", false, {
        let hint = hint.clone();
        move |open| hint.set_text(if open { "Hide" } else { "Show the data from the node" })
    });
    disclosure.header.append(&hint);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.append(&disclosure.root);
    row.append(&spacer);
    row.append(&page::copy_button(json));
    disclosure.content.append(&text);
    card.append(&row);
    // Tab shows the JSON and goes to it. Tab again hides it and goes to the top.
    let (weak, cursor) = (Rc::downgrade(&disclosure), page.cursor.clone());
    let hint_toggle = hint.clone();
    page.hooks().toggle_json = Some(Rc::new(move || {
        if let Some(disclosure) = weak.upgrade() {
            let open = !disclosure.expanded();
            disclosure.set_expanded(open);
            hint_toggle.set_text(if open { "Hide" } else { "Show the data from the node" });
            if open { cursor.scroll_to_card("Raw JSON") } else { cursor.scroll_to_top() }
        }
    }));
    card.root
}
