//! Recent confirmed transactions and older pages of blocks for the dashboard
//! (`DashboardFeed`). It keeps what it loaded, so a refresh requests only the blocks that
//! are new.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use crate::bridge::{self, api};
use crate::model::AppModel;

use super::lists::Lists;

const BLOCK_WINDOW: usize = 30;
const TRANSACTION_LIMIT: usize = 100;

#[derive(Default)]
pub struct Feed {
    /// Newest block first; within a block, last position first, as in the terminal.
    pub transactions: RefCell<Vec<api::RecentTransaction>>,
    pub loading: Cell<bool>,
    pub failure: RefCell<Option<String>>,
    /// Blocks older than the ones the app model keeps, loaded on request.
    pub older: RefCell<Vec<api::BlockRow>>,
    pub loading_older: Cell<bool>,
    pub older_error: RefCell<Option<String>>,
    /// The selections and the length of the transaction list, kept for the session.
    pub selected_block: RefCell<Option<String>>,
    pub selected_transaction: RefCell<Option<String>>,
    pub transactions_shown: Cell<usize>,
    loaded_blocks: RefCell<HashSet<String>>,
    generation: Cell<Option<u64>>,
}

impl Feed {
    pub fn new() -> Rc<Self> {
        let feed = Rc::new(Self::default());
        feed.transactions_shown.set(30);
        feed
    }

    /// Selects a block in the list (the stream, or a click).
    pub fn select_block(&self, hash: Option<String>, lists: &Lists) {
        self.selected_block.replace(hash.clone());
        lists.select(hash.as_deref());
    }

    pub async fn update(&self, model: &Rc<AppModel>, lists: &Rc<Lists>) {
        let Some(explorer) = model.explorer() else { return };
        if self.generation.get() != Some(model.generation()) {
            self.generation.set(Some(model.generation()));
            self.transactions.borrow_mut().clear();
            self.loaded_blocks.borrow_mut().clear();
            self.older.borrow_mut().clear();
            self.older_error.replace(None);
        }
        let window: Vec<api::BlockRow> = model.state().blocks.iter().take(BLOCK_WINDOW).cloned().collect();
        let current: HashSet<String> = window.iter().map(|b| b.hash.clone()).collect();
        let new: Vec<String> = window
            .iter()
            .filter(|b| !self.loaded_blocks.borrow().contains(&b.hash))
            .map(|b| b.hash.clone())
            .collect();
        if new.is_empty() || self.loading.get() {
            return;
        }
        self.loading.set(true);
        lists.show_transactions(model, self);
        let generation = model.generation();
        let asked = new.clone();
        let fetched = bridge::run(async move { explorer.recent_transactions(asked, TRANSACTION_LIMIT as u32).await }).await;
        self.loading.set(false);
        if generation != model.generation() {
            return;
        }
        match fetched {
            Ok(fetched) => {
                let mut seen = HashSet::new();
                let mut rows: Vec<api::RecentTransaction> = fetched
                    .into_iter()
                    .chain(self.transactions.borrow().iter().cloned())
                    // Rows of blocks that left the active chain are dropped with them.
                    .filter(|row| current.contains(&row.block_hash))
                    .collect();
                rows.sort_by(|a, b| (b.block_height, b.position).cmp(&(a.block_height, a.position)));
                rows.retain(|row| seen.insert(row.txid.clone()));
                rows.truncate(TRANSACTION_LIMIT);
                *self.transactions.borrow_mut() = rows;
                let mut loaded = self.loaded_blocks.borrow_mut();
                loaded.extend(new);
                loaded.retain(|hash| current.contains(hash));
                self.failure.replace(None);
            }
            Err(error) => {
                self.failure.replace(Some(bridge::describe(&error)));
            }
        }
        lists.show_transactions(model, self);
    }

    /// The next 30 blocks below the oldest one shown, as the terminal loads older pages.
    pub async fn load_older(&self, model: &Rc<AppModel>, lists: &Rc<Lists>) {
        let Some(explorer) = model.explorer() else { return };
        if self.loading_older.get() {
            return;
        }
        let oldest = self.older.borrow().last().map(|b| b.height).or_else(|| model.state().blocks.last().map(|b| b.height));
        let Some(oldest) = oldest.filter(|h| *h > 0) else { return };
        self.loading_older.set(true);
        lists.set_blocks(model, &model.state().blocks.clone(), self);
        let page = bridge::run(async move { explorer.recent_blocks(Some(oldest - 1), 30).await }).await;
        self.loading_older.set(false);
        match page {
            Ok(page) => {
                let known: HashSet<String> = model
                    .state()
                    .blocks
                    .iter()
                    .chain(self.older.borrow().iter())
                    .map(|b| b.hash.clone())
                    .collect();
                self.older.borrow_mut().extend(page.into_iter().filter(|b| !known.contains(&b.hash)));
                self.older_error.replace(None);
            }
            Err(error) => {
                self.older_error.replace(Some(bridge::describe(&error)));
            }
        }
        lists.set_blocks(model, &model.state().blocks.clone(), self);
    }
}
