//! One transaction as a row (`MempoolRow`), and a column of rows that keeps each row's
//! widget while the row stays, so that a refresh changes only the text that changed.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;

use crate::format;
use crate::page::{self, load::clear};
use crate::widgets::display;

/// One transaction: its tags, what it does, and its facts.
pub struct MempoolRow {
    pub root: gtk::Box,
    tags: gtk::Box,
    words: gtk::Label,
    facts: gtk::Label,
    shown_tags: RefCell<Option<Vec<String>>>,
}

impl MempoolRow {
    pub fn new(txid: &str) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.set_margin_top(7);
        root.set_margin_bottom(7);
        root.set_hexpand(true);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 3);
        column.set_hexpand(true);
        column.set_valign(gtk::Align::Baseline);
        let tags = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tags.set_visible(false);
        column.append(&tags);
        let words = page::paragraph("", &[]);
        words.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        column.append(&words);
        let facts = page::text("", &["callout", "secondary", "tabular"]);
        facts.set_wrap(true);
        column.append(&facts);
        // The baseline of the row is the baseline of its words.
        column.set_baseline_child(1);
        root.append(&column);
        let id = page::text(&format::short(txid, 8, 6), &["mono", "secondary"]);
        id.set_valign(gtk::Align::Baseline);
        id.set_halign(gtk::Align::End);
        root.append(&id);
        Self { root, tags, words, facts, shown_tags: RefCell::new(None) }
    }

    /// `tags` is None until the app has the transaction. `words` is the sentence of its
    /// statement, None until the app has read it.
    pub fn set(&self, tags: Option<&[String]>, words: Option<&str>, facts: &str) {
        let wanted = tags.map(|t| t.to_vec());
        if *self.shown_tags.borrow() != wanted {
            clear(&self.tags);
            if let Some(tags) = tags {
                for tag in tags.iter().take(4) {
                    self.tags.append(&display::plate(tag, display::signal_tag_fill(tag)));
                }
            }
            self.tags.set_visible(tags.is_some_and(|t| !t.is_empty()));
            *self.shown_tags.borrow_mut() = wanted;
        }
        let text = words.filter(|w| !w.is_empty()).map(str::to_string).unwrap_or_else(|| {
            if tags.is_none() { "Classification pending.".into() } else { "The app reads this transaction.".into() }
        });
        if self.words.text() != text {
            self.words.set_text(&text);
        }
        let secondary = words.is_none();
        if self.words.has_css_class("secondary") != secondary {
            if secondary { self.words.add_css_class("secondary") } else { self.words.remove_css_class("secondary") }
        }
        if self.facts.text() != facts {
            self.facts.set_text(facts);
        }
        self.facts.set_visible(!facts.is_empty());
    }
}

/// A column of rows by key, with a divider between rows. A row that stays keeps its widget,
/// its controllers and its place as a cursor stop.
pub struct KeyedRows<T> {
    pub root: gtk::Box,
    rows: RefCell<HashMap<String, (gtk::Box, Rc<T>)>>,
    order: RefCell<Vec<String>>,
}

impl<T> KeyedRows<T> {
    pub fn new() -> Self {
        Self { root: gtk::Box::new(gtk::Orientation::Vertical, 0), rows: Default::default(), order: Default::default() }
    }

    /// Shows the rows of `keys` in this order. `make` builds the content of a new row; the
    /// caller then updates each row with what `set` returns.
    pub fn set(&self, keys: &[String], make: impl Fn(&str) -> (gtk::Widget, T)) -> Vec<Rc<T>> {
        let mut rows = self.rows.borrow_mut();
        rows.retain(|key, _| keys.contains(key));
        let mut result = Vec::with_capacity(keys.len());
        for key in keys {
            let entry = rows.entry(key.clone()).or_insert_with(|| {
                let (widget, value) = make(key);
                let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
                holder.append(&page::divider());
                holder.append(&widget);
                (holder, Rc::new(value))
            });
            result.push(entry.1.clone());
        }
        if *self.order.borrow() != keys {
            // The same widgets in the new order.
            while let Some(child) = self.root.first_child() {
                self.root.remove(&child);
            }
            for (position, key) in keys.iter().enumerate() {
                let holder = &rows[key].0;
                if let Some(divider) = holder.first_child() {
                    divider.set_visible(position > 0);
                }
                self.root.append(holder);
            }
            *self.order.borrow_mut() = keys.to_vec();
        }
        result
    }
}
