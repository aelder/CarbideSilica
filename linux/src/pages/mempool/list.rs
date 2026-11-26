//! The same data as the waiting list, as rows in the page (`MempoolList`): the terminal's
//! mempool view. A column heading sorts. Up and Down select, Return or a double-click opens
//! the transaction.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use gtk::prelude::*;

use crate::bridge::api::{self, Route};
use crate::format;
use crate::model::AppModel;
use crate::page::{self, load::clear, Page};
use crate::theme::tone;
use crate::widgets::canvas::Canvas;
use crate::widgets::display;
use crate::widgets::icons::{self, Icon};
use crate::widgets::link;

use super::feed::{self, Sort};
use super::logic;
use super::rows::KeyedRows;

/// The widths of the columns: transaction, arrival, fee, size, height and parents. The tags
/// take the rest.
const WIDTHS: [i32; 6] = [150, 70, 120, 70, 170, 80];
const TRAILING: [bool; 6] = [false, false, true, true, true, true];

/// One column layout for the list's heading and rows.
fn columns(cells: [gtk::Widget; 6], tags: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    for (index, cell) in cells.into_iter().enumerate() {
        let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        slot.set_size_request(WIDTHS[index], -1);
        slot.set_hexpand(false);
        slot.set_overflow(gtk::Overflow::Hidden);
        cell.set_halign(if TRAILING[index] { gtk::Align::End } else { gtk::Align::Start });
        cell.set_hexpand(true);
        cell.set_valign(gtk::Align::Baseline);
        if let Some(label) = cell.downcast_ref::<gtk::Label>() {
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_xalign(if TRAILING[index] { 1.0 } else { 0.0 });
        }
        slot.append(&cell);
        slot.set_valign(gtk::Align::Baseline);
        row.append(&slot);
    }
    tags.set_hexpand(true);
    tags.set_halign(gtk::Align::Start);
    tags.set_valign(gtk::Align::Baseline);
    row.append(tags);
    row
}

struct ListRow {
    arrival: gtk::Label,
    fee: gtk::Label,
    size: gtk::Label,
    height: gtk::Label,
    parents: gtk::Label,
    tags: gtk::Box,
    shown_tags: RefCell<Option<Option<Vec<String>>>>,
}

pub struct MempoolList {
    pub root: gtk::Box,
    page: Weak<Page>,
    model: Weak<AppModel>,
    heading: gtk::Box,
    rows: KeyedRows<ListRow>,
    shown_sort: RefCell<Option<(Sort, bool)>>,
}

impl MempoolList {
    pub fn new(page: &Rc<Page>, model: &Rc<AppModel>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        heading.set_margin_start(8);
        heading.set_margin_end(8);
        heading.set_margin_bottom(6);
        root.append(&heading);
        root.append(&page::divider());
        let rows = KeyedRows::new();
        root.append(&rows.root);
        root.append(&page::divider());
        let note = page::paragraph(
            "\"Unconfirmed parents\" is the number of mempool transactions that this transaction spends from. \"Classification pending\" means that the app has not loaded the transaction yet.",
            &["caption-text", "secondary"],
        );
        note.set_margin_top(12);
        root.append(&note);
        let list = Rc::new(Self {
            root,
            page: Rc::downgrade(page),
            model: Rc::downgrade(model),
            heading,
            rows,
            shown_sort: Default::default(),
        });
        // The list's selection is kept for the session.
        page.cursor.on_select(|id| {
            if let Some(id) = id {
                let known = feed::with(|feed| feed.entries.iter().any(|e| e.txid == id));
                if known {
                    feed::with_mut(|feed| feed.selection = Some(id.to_string()));
                }
            }
        });
        list
    }

    fn show_heading(self: &Rc<Self>, sort: Sort, descending: bool) {
        if *self.shown_sort.borrow() == Some((sort, descending)) {
            return;
        }
        clear(&self.heading);
        let header = |column: Sort| -> gtk::Widget {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 3);
            let name = display::name_text(column.title());
            name.set_valign(gtk::Align::Center);
            content.append(&name);
            if sort == column {
                let down = descending;
                let chevron = Canvas::new(8, 8, move |_, snapshot, w, h| {
                    if !down {
                        // chevron.up: the mark of chevron.down, turned.
                        snapshot.translate(&gtk::graphene::Point::new(w, h));
                        snapshot.rotate(180.0);
                    }
                    icons::paint(snapshot, Icon::Down, w, h, tone::PRESENT);
                });
                chevron.set_valign(gtk::Align::Center);
                chevron.update_property(&[gtk::accessible::Property::Label(if down { "descending" } else { "ascending" })]);
                content.append(&chevron);
            }
            let button = gtk::Button::new();
            button.add_css_class("mempool-plain");
            button.set_child(Some(&content));
            button.set_tooltip_text(Some(&format!("Sort by {}", column.title().to_lowercase())));
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                feed::with_mut(|feed| {
                    if feed.sort == column {
                        feed.descending = !feed.descending;
                    } else {
                        feed.sort = column;
                        feed.descending = true;
                    }
                });
                if let Some(list) = weak.upgrade() {
                    list.update();
                }
            });
            button.upcast()
        };
        let row = columns(
            [
                display::name_text("Transaction").upcast(),
                header(Sort::Arrival),
                header(Sort::Fee),
                header(Sort::Size),
                header(Sort::Height),
                header(Sort::Parents),
            ],
            &display::name_text("Tags"),
        );
        self.heading.append(&row);
        row.set_hexpand(true);
        *self.shown_sort.borrow_mut() = Some((sort, descending));
    }

    /// Redraws the rows. The ages count once a second.
    pub fn update(self: &Rc<Self>) {
        let (Some(model), Some(page)) = (self.model.upgrade(), self.page.upgrade()) else { return };
        let (entries, sort, descending) = feed::with(|feed| (feed.entries.clone(), feed.sort, feed.descending));
        self.show_heading(sort, descending);
        let rows = logic::sorted(&entries, sort, descending);
        let keys: Vec<String> = rows.iter().map(|e| e.txid.clone()).collect();
        let now = format::now();
        let coin = model.coin();
        let widgets = self.rows.set(&keys, |txid| {
            let open = {
                let (model, txid) = (Rc::downgrade(&model), txid.to_string());
                move || {
                    if let Some(model) = model.upgrade() {
                        model.open(Route::Transaction { txid: txid.clone() });
                    }
                }
            };
            let transaction = link::link(&format::short(txid, 10, 6), true, open.clone());
            transaction.add_css_class("callout");
            transaction.set_tooltip_text(Some(txid));
            let label = || page::text("", &["tabular"]);
            let row = ListRow {
                arrival: label(),
                fee: label(),
                size: label(),
                height: label(),
                parents: label(),
                tags: gtk::Box::new(gtk::Orientation::Horizontal, 4),
                shown_tags: RefCell::new(None),
            };
            let line = columns(
                [
                    transaction.upcast(),
                    row.arrival.clone().upcast(),
                    row.fee.clone().upcast(),
                    row.size.clone().upcast(),
                    row.height.clone().upcast(),
                    row.parents.clone().upcast(),
                ],
                &row.tags,
            );
            line.set_margin_top(5);
            line.set_margin_bottom(5);
            line.set_margin_start(8);
            line.set_margin_end(8);
            // The row of the cursor is the line with its padding.
            let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
            holder.append(&line);
            page.selectable_row(txid, &holder, Some(txid.to_string()), open);
            (holder.upcast(), row)
        });
        for (entry, row) in rows.iter().zip(widgets) {
            set_text(&row.arrival, &entry.time.map(|t| format::age(t, now)).unwrap_or_else(|| "unknown".into()));
            set_text(&row.fee, &entry.fee_sats.map(|f| format!("{} {coin}", format::coins(f))).unwrap_or_else(|| "unknown".into()));
            set_text(&row.size, &format::bytes(entry.size));
            set_text(
                &row.height,
                &entry.height.map(|h| crate::display::height_text(h, None, &model)).unwrap_or_else(|| "unknown".into()),
            );
            set_text(&row.parents, &if entry.parents.is_empty() { "none".to_string() } else { entry.parents.len().to_string() });
            set_tags(&row, entry);
        }
    }
}

fn set_text(label: &gtk::Label, text: &str) {
    if label.text() != text {
        label.set_text(text);
    }
}

fn set_tags(row: &ListRow, entry: &api::MempoolEntry) {
    let wanted = Some(entry.classified.then(|| entry.tags.clone()));
    if *row.shown_tags.borrow() == wanted {
        return;
    }
    clear(&row.tags);
    if !entry.classified {
        row.tags.append(&page::text("classification pending", &["secondary"]));
    } else if entry.tags.is_empty() {
        row.tags.append(&page::text("no tags", &["secondary"]));
    } else {
        for tag in &entry.tags {
            row.tags.append(&display::plate(tag, display::signal_tag_fill(tag)));
        }
    }
    *row.shown_tags.borrow_mut() = wanted;
}
