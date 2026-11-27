//! The blocks that hold a transaction
//! across side branches, and the stored child blocks of a block (`InclusionCard`,
//! `BlockBranchesCard`, `ObservedBlockList`, `MembershipMark`, `Pager`, `NodeBranchCheck`
//! and `ReloadOnRevision` in `InclusionCard.swift`). Used by the Transaction page and the
//! Block page.
//!
//! The lists read the observations that the cache stored. Each source stays separate. They
//! load again when the background reconciliation commits a change, as the terminal's F7
//! panel does, and when the app connects again.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Route};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, Card, Page};
use crate::theme::tone;
use crate::widgets::icons::{self, Icon};
use crate::widgets::display;
use crate::widgets::{lamp, loader};

/// What a list of stored observations shows: the blocks of a transaction, or the children
/// of a block.
#[derive(Clone)]
enum Query {
    Inclusions(String),
    Children(String),
}

struct History {
    query: Query,
    model: Weak<AppModel>,
    page: Weak<Page>,
    body: gtk::Box,
    /// The page of 100 observations.
    number: Cell<u32>,
    found: RefCell<Option<api::ObservedBlockPage>>,
    failure: RefCell<Option<String>>,
    /// A count of loads, so that an old answer is not shown.
    attempt: Cell<u64>,
}

/// The blocks that hold the transaction (the terminal's F7): every block a transaction is
/// known to have been in, read from the observations that the cache stored.
pub fn inclusion_card(page: &Rc<Page>, txid: &str) -> Card {
    let card = page.card("Block history", None);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.append(&body);
    card.append(&page::paragraph(
        "The list shows only what sources reported to this app or to the terminal app. It does not search the chain. An empty list does not prove that no other block contained the transaction. \"Active\" and \"Orphaned\" are the last reports of each source. They are not a prediction of which branch stays.",
        &["caption-small", "secondary"],
    ));
    card.append(&node_branch_check(page, txid));
    start(page, &card, body, Query::Inclusions(txid.to_string()));
    card
}

/// The stored blocks that follow a block, on each branch. More than one child means that
/// sources reported more than one branch after this block.
pub fn block_branches_card(page: &Rc<Page>, hash: &str) -> Card {
    let card = page.card("Next blocks", None);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.append(&body);
    card.append(&page::paragraph(
        "The list shows only what sources reported. An empty list does not prove that no other branch exists.",
        &["caption-small", "secondary"],
    ));
    start(page, &card, body, Query::Children(hash.to_string()));
    card
}

fn start(page: &Rc<Page>, card: &Card, body: gtk::Box, query: Query) {
    let history = Rc::new(History {
        query,
        model: Rc::downgrade(&page.model),
        page: Rc::downgrade(page),
        body,
        number: Cell::new(0),
        found: RefCell::new(None),
        failure: RefCell::new(None),
        attempt: Cell::new(0),
    });
    history.load();
    // A new connection loads again, and starts a new wait for changes.
    let weak = Rc::downgrade(&history);
    page.model.watch(&[Topic::Connection], &history.body, move |_| {
        if let Some(history) = weak.upgrade() {
            history.load();
            history.watch_revisions();
        }
    });
    history.watch_revisions();
    // The card keeps its state while it is shown.
    let owned = history.clone();
    card.root.connect_destroy(move |_| {
        let _ = &owned;
    });
}

impl History {
    fn load(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let attempt = self.attempt.get() + 1;
        self.attempt.set(attempt);
        let explorer = match model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                *self.failure.borrow_mut() = Some(message);
                self.fill();
                return;
            }
        };
        if self.found.borrow().is_none() && self.failure.borrow().is_none() {
            self.fill();
        }
        let (query, number, generation) = (self.query.clone(), self.number.get(), model.generation());
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move {
                match query {
                    Query::Inclusions(txid) => explorer.inclusion_history(txid, number).await,
                    Query::Children(hash) => explorer.block_children(hash, number).await,
                }
            })
            .await;
            let Some(history) = weak.upgrade() else { return };
            let Some(model) = history.model.upgrade() else { return };
            if history.attempt.get() != attempt || model.generation() != generation {
                return;
            }
            match result {
                Ok(found) => {
                    *history.found.borrow_mut() = Some(found);
                    *history.failure.borrow_mut() = None;
                }
                Err(error) => *history.failure.borrow_mut() = Some(bridge::describe(&error)),
            }
            history.fill();
        });
    }

    /// Reloads stored history when the background reconciliation commits a change. It
    /// waits on the bridge and sends no request to a source.
    fn watch_revisions(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(explorer) = model.explorer() else { return };
        let generation = model.generation();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let mut seen = explorer.observation_revision();
            loop {
                let ex = explorer.clone();
                let revision = bridge::run(async move { ex.wait_for_revision(seen).await }).await;
                let Some(history) = weak.upgrade() else { return };
                let Some(model) = history.model.upgrade() else { return };
                if model.generation() != generation {
                    return;
                }
                if revision != seen {
                    seen = revision;
                    history.load();
                }
            }
        });
    }

    fn fill(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else { return };
        clear(&self.body);
        let found = self.found.borrow().clone();
        if let Some(found) = found {
            let blocks: HashSet<&str> = found.rows.iter().map(|r| r.block_hash.as_str()).collect();
            match self.query {
                Query::Inclusions(_) => {
                    let orphaned: HashSet<&str> = found
                        .rows
                        .iter()
                        .filter(|r| r.membership == api::Membership::Orphaned)
                        .map(|r| r.block_hash.as_str())
                        .collect();
                    if found.rows.is_empty() {
                        self.body.append(&page::paragraph("No source has reported a block for this transaction yet.", &["secondary"]));
                    } else if blocks.len() == 1 {
                        self.body.append(&page::paragraph("Sources reported this transaction in one block.", &[]));
                    } else {
                        self.body.append(&page::note(
                            &format!(
                                "Sources reported this transaction in {} different blocks. {} of them were last reported off the active chain.",
                                blocks.len(),
                                orphaned.len()
                            ),
                            true,
                        ));
                    }
                }
                Query::Children(_) => {
                    if found.rows.is_empty() {
                        self.body.append(&page::paragraph(
                            "The cache holds no stored block that follows this one. The page header gives the node's next block.",
                            &["secondary"],
                        ));
                    } else if blocks.len() > 1 {
                        self.body.append(&page::note(
                            &format!("Sources reported {} different blocks after this one. Each is a branch.", blocks.len()),
                            true,
                        ));
                    }
                }
            }
            if let Some(list) = observed_block_list(&page, &found.rows) {
                self.body.append(&list);
            }
            self.body.append(&self.pager(found.more));
        } else if let Some(failure) = self.failure.borrow().as_ref() {
            self.body.append(&page::note(failure, true));
        } else {
            let progress = loader::small();
            progress.set_halign(gtk::Align::Start);
            self.body.append(&progress);
        }
    }

    /// Pages of 100 observations, and a reload of the stored data.
    fn pager(self: &Rc<Self>, more: bool) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let number = self.number.get();
        if number > 0 || more {
            let weak = Rc::downgrade(self);
            let previous = lamp::button("Previous", move || {
                if let Some(history) = weak.upgrade() {
                    history.number.set(history.number.get().saturating_sub(1));
                    history.load();
                }
            });
            previous.set_sensitive(number > 0);
            row.append(&previous);
            let label = page::text(&format!("Page {}", number + 1), &["tabular", "secondary"]);
            label.set_valign(gtk::Align::Center);
            row.append(&label);
            let weak = Rc::downgrade(self);
            let next = lamp::button("Next", move || {
                if let Some(history) = weak.upgrade() {
                    history.number.set(history.number.get() + 1);
                    history.load();
                }
            });
            next.set_sensitive(more);
            row.append(&next);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        row.append(&spacer);
        let weak = Rc::downgrade(self);
        let reload = lamp::button("Reload", move || {
            if let Some(history) = weak.upgrade() {
                history.load();
            }
        });
        reload.set_tooltip_text(Some("Read the stored observations again"));
        row.append(&reload);
        row
    }
}

/// Last observed membership as shape, colour and word (`MembershipMark`).
pub fn membership_mark(membership: api::Membership) -> gtk::Box {
    let (word, color, class) = match membership {
        api::Membership::Active => ("Active", tone::GOOD, "good"),
        api::Membership::Orphaned => ("Orphaned", tone::MINED, "mined"),
        api::Membership::Unverified => ("Unverified", tone::LABEL, "label"),
    };
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    row.append(&icons::icon(
        match membership {
            api::Membership::Active => Icon::CheckCircleFill,
            api::Membership::Orphaned => Icon::Branch,
            api::Membership::Unverified => Icon::QuestionCircle,
        },
        13,
        color,
    ));
    row.append(&page::text(word, &[class]));
    if membership == api::Membership::Unverified {
        row.set_tooltip_text(Some("No stored report says whether this block is on the active chain."));
    }
    row
}

/// Rows of stored block observations: membership, height, block, source and whether the
/// cache holds the block's body.
fn observed_block_list(page: &Page, rows: &[api::ObservedBlock]) -> Option<gtk::Box> {
    if rows.is_empty() {
        return None;
    }
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let grid = gtk::Grid::new();
    grid.set_column_spacing(14);
    grid.set_row_spacing(6);
    for (index, title) in ["Last report", "Height", "Block", "Source", "Reported", "Stored body"].iter().enumerate() {
        grid.attach(&display::name_text(title), index as i32, 0, 1, 1);
    }
    for (index, row) in rows.iter().enumerate() {
        let line = index as i32 + 1;
        // Rows of one block are adjacent. Name the block once, then each source.
        let repeated = index > 0 && rows[index - 1].block_hash == row.block_hash;
        grid.attach(&membership_mark(row.membership), 0, line, 1, 1);
        if repeated {
            grid.attach(&page::text("", &[]), 1, line, 1, 1);
            grid.attach(&page::text("Same block, another source", &["secondary"]), 2, line, 1, 1);
        } else {
            let height = page::text(&row.height.map(format::integer).unwrap_or_else(|| "Unknown".into()), &["tabular"]);
            grid.attach(&height, 1, line, 1, 1);
            grid.attach(&page::route_link(page, &row.block_hash, Route::Block { id: row.block_hash.clone() }, true), 2, line, 1, 1);
        }
        let source = page::text(&row.source, &[]);
        source.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        source.set_max_width_chars(30);
        source.set_tooltip_text(Some(&row.source_id));
        grid.attach(&source, 3, line, 1, 1);
        let reported = page::text(&row.observed_at.map(|t| format::age(t, format::now())).unwrap_or_else(|| "Unknown".into()), &["secondary"]);
        if let Some(time) = row.observed_at {
            reported.set_tooltip_text(Some(&format::date(time)));
        }
        grid.attach(&reported, 4, line, 1, 1);
        let body = page::text(if row.cached_body { "Yes" } else { "No" }, &["secondary"]);
        body.set_tooltip_text(Some(if row.cached_body {
            "The block opens without a source."
        } else {
            "Opening the block needs an enabled source."
        }));
        grid.attach(&body, 5, line, 1, 1);
    }
    column.append(&grid);
    let blocks: HashSet<&str> = rows.iter().map(|r| r.block_hash.as_str()).collect();
    if blocks.len() < rows.len() {
        column.append(&page::paragraph(
            "Each source that reported a block has its own line. Lines under one block are one block reported by more than one source, not more than one inclusion.",
            &["callout", "secondary"],
        ));
    }
    Some(column)
}

struct BranchCheck {
    txid: String,
    page: Weak<Page>,
    body: gtk::Box,
    found: RefCell<Option<Vec<api::Inclusion>>>,
    failure: RefCell<Option<String>>,
    running: Cell<bool>,
}

/// The previous check: side branches that the node still holds near the transaction's
/// height. It asks the node for its chain tips, which can take several seconds.
fn node_branch_check(page: &Rc<Page>, txid: &str) -> gtk::Box {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let check = Rc::new(BranchCheck {
        txid: txid.to_string(),
        page: Rc::downgrade(page),
        body: body.clone(),
        found: RefCell::new(None),
        failure: RefCell::new(None),
        running: Cell::new(false),
    });
    check.fill();
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    holder.append(&body);
    let owned = check.clone();
    holder.connect_destroy(move |_| {
        let _ = &owned;
    });
    holder
}

impl BranchCheck {
    fn fill(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else { return };
        clear(&self.body);
        if let Some(found) = self.found.borrow().as_ref() {
            let orphaned: Vec<&api::Inclusion> = found.iter().filter(|i| !i.active).collect();
            let count = orphaned.len();
            self.body.append(&page::note(
                &if orphaned.is_empty() {
                    "No side branch on the node contains the transaction.".to_string()
                } else {
                    format!(
                        "The node holds {count} block{} on a side branch that contain the transaction.",
                        if count == 1 { "" } else { "s" }
                    )
                },
                !orphaned.is_empty(),
            ));
            for inclusion in orphaned {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                if let Some(height) = inclusion.height {
                    row.append(&page::text(&format!("Block {}", format::integer(height)), &["tabular"]));
                }
                row.append(&page::route_link(&page, &inclusion.block_hash, Route::Block { id: inclusion.block_hash.clone() }, true));
                self.body.append(&row);
            }
        } else if self.running.get() {
            let progress = loader::small();
            progress.set_halign(gtk::Align::Start);
            self.body.append(&progress);
        } else {
            if let Some(failure) = self.failure.borrow().as_ref() {
                self.body.append(&page::note(failure, true));
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            let text = page::paragraph(
                "Also ask the node for side branches near this height. The request can take several seconds.",
                &["secondary"],
            );
            text.set_hexpand(true);
            row.append(&text);
            let weak = Rc::downgrade(self);
            row.append(&lamp::button("Check Node", move || {
                if let Some(check) = weak.upgrade() {
                    check.run();
                }
            }));
            self.body.append(&row);
        }
    }

    fn run(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else { return };
        let explorer = match page.model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                *self.failure.borrow_mut() = Some(message);
                self.fill();
                return;
            }
        };
        self.running.set(true);
        *self.failure.borrow_mut() = None;
        self.fill();
        let txid = self.txid.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.inclusions(txid).await }).await;
            let Some(check) = weak.upgrade() else { return };
            check.running.set(false);
            match result {
                Ok(found) => *check.found.borrow_mut() = Some(found),
                Err(error) => *check.failure.borrow_mut() = Some(bridge::describe(&error)),
            }
            check.fill();
        });
    }
}
