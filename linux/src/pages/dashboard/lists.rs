//! The dashboard's two lists: recent blocks and recent confirmed transactions, side by side
//! when the page is wide enough, one above the other when it is not (`RecentBlocks`,
//! `RecentTransactions`, `BlockLine`, `TransactionLine`).

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::api::{self, Route};
use crate::format;
use crate::model::AppModel;
use crate::page::load::clear;
use crate::page::{self, Card, Page};
use crate::theme::tone;
use crate::widgets::display::{self, PlateFill};
use crate::widgets::{lamp, link, loader};

use super::feed::Feed;

pub struct Lists {
    pub root: gtk::FlowBox,
    page: Weak<Page>,
    blocks_card: Card,
    stream: gtk::CheckButton,
    block_rows: gtk::Box,
    older: gtk::Box,
    transactions_card: Card,
    transaction_rows: gtk::Box,
    /// The hashes of the rows on screen.
    shown_blocks: RefCell<Vec<String>>,
    shown_transactions: RefCell<Vec<String>>,
    /// Ages that count: the label and the time.
    ages: RefCell<Vec<(glib::WeakRef<gtk::Label>, u64)>>,
    on_stream: RefCell<Option<Rc<dyn Fn(bool)>>>,
    feed: Weak<Feed>,
}

impl Lists {
    pub fn new(page: &Rc<Page>, feed: &Rc<Feed>) -> Rc<Self> {
        // Two columns of at least 430 when they fit, as the Swift app's `ViewThatFits`.
        let root = gtk::FlowBox::new();
        root.set_selection_mode(gtk::SelectionMode::None);
        root.set_homogeneous(true);
        root.set_min_children_per_line(1);
        root.set_max_children_per_line(2);
        root.set_column_spacing(24);
        root.set_row_spacing(20);
        root.set_valign(gtk::Align::Start);
        root.set_activate_on_single_click(false);

        let blocks_card = page.card("Recent blocks", None);
        let stream = page::check_box("Stream blocks", super::streaming(), |_| {});
        stream.set_tooltip_text(Some("When this is on, the app selects each new block."));
        stream.set_halign(gtk::Align::Start);
        blocks_card.append(&stream);
        let block_rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        blocks_card.append(&block_rows);
        let older = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        blocks_card.append(&older);
        blocks_card.root.set_size_request(430, -1);
        blocks_card.root.set_valign(gtk::Align::Start);

        let transactions_card = page.card("Recent transactions", None);
        transactions_card.append(&page::text(
            "The newest transactions in the recent blocks, newest block first.",
            &["callout", "secondary"],
        ));
        let transaction_rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        transactions_card.append(&transaction_rows);
        transactions_card.root.set_size_request(430, -1);
        transactions_card.root.set_valign(gtk::Align::Start);

        for card in [&blocks_card.root, &transactions_card.root] {
            let child = gtk::FlowBoxChild::new();
            child.set_child(Some(card));
            child.set_focusable(false);
            child.set_valign(gtk::Align::Start);
            root.append(&child);
        }

        let lists = Rc::new(Self {
            root,
            page: Rc::downgrade(page),
            blocks_card,
            stream,
            block_rows,
            older,
            transactions_card,
            transaction_rows,
            shown_blocks: Default::default(),
            shown_transactions: Default::default(),
            ages: Default::default(),
            on_stream: Default::default(),
            feed: Rc::downgrade(feed),
        });
        let weak = Rc::downgrade(&lists);
        lists.stream.connect_toggled(move |check| {
            if let Some(lists) = weak.upgrade() {
                let action = lists.on_stream.borrow().clone();
                if let Some(action) = action
                    && check.is_active() != super::streaming()
                {
                    action(check.is_active());
                }
            }
        });
        // The ages count every 5 seconds.
        let weak = Rc::downgrade(&lists);
        page::every(&lists.root, Duration::from_secs(5), move || {
            if let Some(lists) = weak.upgrade() {
                let now = format::now();
                lists.ages.borrow_mut().retain(|(label, time)| match label.upgrade() {
                    Some(label) => {
                        label.set_text(&format::age(*time, now));
                        true
                    }
                    None => false,
                });
            }
        });
        lists
    }

    pub fn on_stream(&self, action: Rc<dyn Fn(bool)>) {
        *self.on_stream.borrow_mut() = Some(action);
    }

    pub fn set_streaming(&self, on: bool) {
        if self.stream.is_active() != on {
            self.stream.set_active(on);
        }
    }

    pub fn has_block(&self, hash: &str) -> bool {
        self.shown_blocks.borrow().iter().any(|h| h == hash)
    }

    pub fn select(&self, hash: Option<&str>) {
        if let Some(page) = self.page.upgrade() {
            page.cursor.select(hash, false);
        }
    }

    /// The model's blocks, then the older pages that the person loaded.
    pub fn set_blocks(self: &Rc<Self>, model: &Rc<AppModel>, blocks: &[api::BlockRow], feed: &Feed) {
        let Some(page) = self.page.upgrade() else { return };
        let all: Vec<api::BlockRow> = blocks.iter().cloned().chain(feed.older.borrow().iter().cloned()).collect();
        self.blocks_card.set_trailing((!all.is_empty()).then(|| format!("{} blocks", all.len())).as_deref());
        let hashes: Vec<String> = all.iter().map(|b| b.hash.clone()).collect();
        if *self.shown_blocks.borrow() != hashes {
            clear(&self.block_rows);
            if all.is_empty() {
                let waiting = page::text("Waiting for data from the node.", &["secondary"]);
                waiting.set_size_request(-1, 120);
                waiting.set_xalign(0.5);
                self.block_rows.append(&waiting);
            }
            for block in &all {
                let row = self.block_line(model, block);
                let (weak, hash) = (Rc::downgrade(model), block.hash.clone());
                page.selectable_row(&block.hash, &row, Some(block.hash.clone()), move || {
                    if let Some(model) = weak.upgrade() {
                        model.open(Route::Block { id: hash.clone() });
                    }
                });
                self.block_rows.append(&row);
                self.block_rows.append(&page::divider());
            }
            *self.shown_blocks.borrow_mut() = hashes;
        }
        // Older blocks.
        clear(&self.older);
        self.older.set_visible(!all.is_empty());
        if feed.loading_older.get() {
            self.older.append(&loader::small());
            self.older.append(&page::text("Loading older blocks…", &["secondary"]));
        } else if all.last().is_some_and(|b| b.height == 0) {
            self.older.append(&page::text("The first block is reached.", &["secondary"]));
        } else {
            let (weak, model_weak) = (Rc::downgrade(self), Rc::downgrade(model));
            let feed_weak = self.feed.clone();
            self.older.append(&lamp::button("Load Older Blocks", move || {
                let (Some(lists), Some(model), Some(feed)) = (weak.upgrade(), model_weak.upgrade(), feed_weak.upgrade()) else { return };
                glib::spawn_future_local(async move { feed.load_older(&model, &lists).await });
            }));
        }
        if let Some(error) = feed.older_error.borrow().as_ref() {
            let text = page::paragraph(error, &["secondary"]);
            text.set_lines(2);
            self.older.append(&text);
        }
    }

    fn block_line(&self, model: &Rc<AppModel>, block: &api::BlockRow) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.set_margin_top(5);
        row.set_margin_bottom(5);
        row.set_margin_start(8);
        row.set_margin_end(8);
        let height = page::text(&format::integer(block.height), &["medium", "tabular"]);
        height.set_size_request(84, -1);
        row.append(&height);
        let plate_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        plate_box.set_size_request(64, -1);
        plate_box.append(&display::plate(
            if block.staked { "Staked" } else { "Mined" },
            if block.staked { PlateFill::Staked } else { PlateFill::Mined },
        ));
        row.append(&plate_box);
        let age = page::text(&format::age(block.time, format::now()), &["secondary", "tabular"]);
        age.set_size_request(76, -1);
        self.ages.borrow_mut().push((age.downgrade(), block.time));
        row.append(&age);
        let count = page::text(&format!("{} tx", block.transaction_count), &["tabular"]);
        count.set_xalign(1.0);
        count.set_size_request(44, -1);
        row.append(&count);
        let (weak, hash) = (Rc::downgrade(model), block.hash.clone());
        let link = link::link(&format::short(&block.hash, 8, 6), true, move || {
            if let Some(model) = weak.upgrade() {
                model.open(Route::Block { id: hash.clone() });
            }
        });
        link.set_tooltip_text(Some(&block.hash));
        link.set_valign(gtk::Align::Center);
        row.append(&link);
        row
    }

    /// Recent confirmed transactions with their tags, from the blocks in the other list.
    /// The list shows 30 rows at first and grows with the page.
    pub fn show_transactions(self: &Rc<Self>, model: &Rc<AppModel>, feed: &Feed) {
        let Some(page) = self.page.upgrade() else { return };
        let rows = feed.transactions.borrow().clone();
        self.transactions_card.set_trailing((!rows.is_empty()).then(|| format!("{} confirmed", rows.len())).as_deref());
        let shown_count = feed.transactions_shown.get();
        let visible: Vec<&api::RecentTransaction> = rows.iter().take(shown_count).collect();
        let ids: Vec<String> = visible.iter().map(|r| r.txid.clone()).chain([format!("{}|{}|{:?}", rows.len(), feed.loading.get(), feed.failure.borrow())]).collect();
        if *self.shown_transactions.borrow() == ids {
            return;
        }
        *self.shown_transactions.borrow_mut() = ids;
        clear(&self.transaction_rows);
        if rows.is_empty() {
            if let Some(failure) = feed.failure.borrow().as_ref() {
                self.transaction_rows.append(&page::note(failure, true));
            } else {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                line.set_halign(gtk::Align::Center);
                line.set_size_request(-1, 120);
                if feed.loading.get() {
                    line.append(&loader::small());
                }
                let text = if feed.loading.get() { "Loading transactions…" } else { "No transactions in recent blocks." };
                let label = page::text(text, &["secondary"]);
                label.set_valign(gtk::Align::Center);
                line.append(&label);
                self.transaction_rows.append(&line);
            }
            return;
        }
        let coin = model.coin();
        for row in visible {
            let line = self.transaction_line(model, row, &coin);
            let (weak, txid) = (Rc::downgrade(model), row.txid.clone());
            page.selectable_row(&row.txid, &line, Some(row.txid.clone()), move || {
                if let Some(model) = weak.upgrade() {
                    model.open(Route::Transaction { txid: txid.clone() });
                }
            });
            self.transaction_rows.append(&line);
            self.transaction_rows.append(&page::divider());
        }
        if rows.len() > shown_count {
            let more = (rows.len() - shown_count).min(30);
            let (weak, model_weak, feed_weak) = (Rc::downgrade(self), Rc::downgrade(model), self.feed.clone());
            let key = lamp::button(&format!("Show {more} More"), move || {
                let (Some(lists), Some(model), Some(feed)) = (weak.upgrade(), model_weak.upgrade(), feed_weak.upgrade()) else { return };
                feed.transactions_shown.set(feed.transactions_shown.get() + 30);
                lists.show_transactions(&model, &feed);
            });
            key.set_margin_top(12);
            self.transaction_rows.append(&key);
        }
    }

    fn transaction_line(&self, model: &Rc<AppModel>, row: &api::RecentTransaction, coin: &str) -> gtk::Box {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 3);
        column.set_margin_top(3);
        column.set_margin_bottom(3);
        column.set_margin_start(8);
        column.set_margin_end(8);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let (weak, txid) = (Rc::downgrade(model), row.txid.clone());
        let id = link::link(&format::short(&row.txid, 10, 6), true, move || {
            if let Some(model) = weak.upgrade() {
                model.open(Route::Transaction { txid: txid.clone() });
            }
        });
        id.set_tooltip_text(Some(&row.txid));
        top.append(&id);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        top.append(&spacer);
        let (weak, hash) = (Rc::downgrade(model), row.block_hash.clone());
        let block = link::link(&format!("Block {}", format::integer(row.block_height)), false, move || {
            if let Some(model) = weak.upgrade() {
                model.open(Route::Block { id: hash.clone() });
            }
        });
        top.append(&block);
        let age = page::text(&format::age(row.block_time, format::now()), &["secondary", "tabular"]);
        self.ages.borrow_mut().push((age.downgrade(), row.block_time));
        top.append(&age);
        column.append(&top);

        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        bottom.add_css_class("callout");
        if row.loaded {
            for tag in row.tags.iter().take(4) {
                bottom.append(&display::plate(tag, display::signal_tag_fill(tag)));
            }
            bottom.append(&page::text(&format!("{} in, {} out", row.input_count, row.output_count), &["secondary"]));
            if let Some(sats) = row.output_sats.filter(|s| *s > 0) {
                let amount = gtk::Label::new(None);
                amount.set_markup(&page::coin_markup(sats, coin, tone::SECONDARY));
                bottom.append(&amount);
            }
        } else {
            bottom.append(&page::text("The details did not load.", &["secondary"]));
        }
        column.append(&bottom);
        column
    }
}
