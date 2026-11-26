//! The transactions that the last blocks took, newest block first (`LeftList`). A person who
//! sent a transaction finds it here after it leaves the mempool.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::api::{self, Route};
use crate::format;
use crate::model::AppModel;
use crate::page::{self, load::clear};
use crate::widgets::link;

use super::feed::{self, Feed, LEFT_BLOCKS, LEFT_SHOWN};
use super::rows::MempoolRow;
use super::text::{count, span};
use super::words::{self, Names};

/// What one row shows, to see whether the list changed.
#[derive(Clone, Debug, PartialEq)]
enum Line {
    Gone { txid: String, tags: Vec<String>, words: String },
    Empty(String),
    Block { hash: String, height: u64, time: u64, divider: bool },
    Took { txid: String, tags: Option<Vec<String>>, words: Option<String>, facts: String },
    More(String),
}

pub struct LeftList {
    pub root: gtk::Box,
    model: Weak<AppModel>,
    shown: RefCell<Vec<Line>>,
    /// Ages that count: the label and the time.
    ages: RefCell<Vec<(glib::WeakRef<gtk::Label>, u64)>>,
}

impl LeftList {
    pub fn new(model: &Rc<AppModel>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let list = Rc::new(Self { root, model: Rc::downgrade(model), shown: Default::default(), ages: Default::default() });
        // The ages count every 5 seconds.
        let weak = Rc::downgrade(&list);
        page::every(&list.root, Duration::from_secs(5), move || {
            if let Some(list) = weak.upgrade() {
                let now = format::now();
                list.ages.borrow_mut().retain(|(label, time)| match label.upgrade() {
                    Some(label) => {
                        label.set_text(&format::age(*time, now));
                        true
                    }
                    None => false,
                });
            }
        });
        list
    }

    pub fn update(&self) {
        let Some(model) = self.model.upgrade() else { return };
        let names = Names::of(&model);
        let arrivals = model.state().arrivals.clone();
        let lines = feed::with(|feed| lines(feed, &names, &arrivals));
        if *self.shown.borrow() == lines {
            return;
        }
        clear(&self.root);
        self.ages.borrow_mut().clear();
        let now = format::now();
        for line in &lines {
            match line {
                Line::Gone { txid, tags, words } => {
                    let row = MempoolRow::new(txid);
                    row.set(
                        Some(tags),
                        Some(words),
                        "It can be expired, or another transaction can have replaced it.",
                    );
                    self.open_on_click(&row.root, Route::Transaction { txid: txid.clone() });
                    self.root.append(&row.root);
                    self.root.append(&page::divider());
                }
                Line::Empty(text) => self.root.append(&page::text(text, &["secondary"])),
                Line::Block { hash, height, time, divider } => {
                    if *divider {
                        self.root.append(&page::divider());
                    }
                    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                    header.add_css_class("callout");
                    header.set_margin_top(8);
                    let weak = self.model.clone();
                    let hash_value = hash.clone();
                    let block = link::link(&format!("Block {}", format::integer(*height)), false, move || {
                        if let Some(model) = weak.upgrade() {
                            model.open(Route::Block { id: hash_value.clone() });
                        }
                    });
                    block.add_css_class("callout");
                    header.append(&block);
                    let age = page::text(&format::age(*time, now), &["secondary", "callout"]);
                    age.set_valign(gtk::Align::Baseline);
                    self.ages.borrow_mut().push((age.downgrade(), *time));
                    header.append(&age);
                    self.root.append(&header);
                }
                Line::Took { txid, tags, words, facts } => {
                    let row = MempoolRow::new(txid);
                    row.set(tags.as_deref(), words.as_deref(), facts);
                    row.root.set_tooltip_text(Some("Open this transaction."));
                    self.open_on_click(&row.root, Route::Transaction { txid: txid.clone() });
                    self.root.append(&row.root);
                }
                Line::More(text) => {
                    self.root.append(&page::divider());
                    let more = page::text(text, &["callout", "secondary"]);
                    more.set_margin_top(6);
                    self.root.append(&more);
                }
            }
        }
        *self.shown.borrow_mut() = lines;
    }

    fn open_on_click(&self, row: &gtk::Box, route: Route) {
        let weak = self.model.clone();
        let click = gtk::GestureClick::new();
        click.connect_released(move |gesture, presses, _, _| {
            if presses == 1 {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                if let Some(model) = weak.upgrade() {
                    model.open(route.clone());
                }
            }
        });
        row.add_controller(click);
    }
}

/// The lines of the list, from the feed.
fn lines(feed: &Feed, names: &Names, arrivals: &std::collections::HashMap<String, f64>) -> Vec<Line> {
    let rows: Vec<&api::RecentTransaction> = feed.took.iter().take(LEFT_SHOWN).collect();
    let mut gone: Vec<&feed::Sighting> = feed.sightings.values().filter(|s| s.gone).collect();
    gone.sort_by(|a, b| b.arrival.total_cmp(&a.arrival));
    let words_of = |txid: &str| words::sentence(feed.details.get(txid), names).filter(|s| !s.is_empty());
    let mut lines = Vec::new();
    for sighting in gone {
        lines.push(Line::Gone {
            txid: sighting.txid.clone(),
            tags: feed.details.get(&sighting.txid).map(|d| d.tags.clone()).unwrap_or_default(),
            words: words_of(&sighting.txid)
                .unwrap_or_else(|| "This transaction left the mempool. The node does not have it in a block.".into()),
        });
    }
    if rows.is_empty() {
        lines.push(Line::Empty(if feed.took_blocks.is_empty() {
            "The app reads the last blocks.".into()
        } else {
            format!("The last {} blocks took no transaction from the mempool.", feed.took_blocks.len())
        }));
    }
    for (position, row) in rows.iter().enumerate() {
        if position == 0 || rows[position - 1].block_hash != row.block_hash {
            lines.push(Line::Block {
                hash: row.block_hash.clone(),
                height: row.block_height,
                time: row.block_time,
                divider: position > 0,
            });
        }
        let fallback = row.loaded.then(|| {
            format!(
                "{}, {}.",
                count(row.input_count as usize, "input", "inputs"),
                count(row.output_count as usize, "output", "outputs")
            )
        });
        lines.push(Line::Took {
            txid: row.txid.clone(),
            tags: row.loaded.then(|| row.tags.clone()),
            words: words_of(&row.txid).or(fallback),
            facts: facts(feed, row, arrivals),
        });
    }
    if feed.took.len() > rows.len() {
        lines.push(Line::More(format!("{} more in the last {LEFT_BLOCKS} blocks.", feed.took.len() - rows.len())));
    }
    lines
}

/// The wait of a transaction that the app saw: from its arrival to the arrival of its
/// block, or to the refresh that first did not have it.
fn facts(feed: &Feed, row: &api::RecentTransaction, arrivals: &std::collections::HashMap<String, f64>) -> String {
    let Some(sighting) = feed.sightings.get(&row.txid) else { return String::new() };
    let Some(left) = sighting.left else { return String::new() };
    let end = arrivals.get(&row.block_hash).map(|a| a.min(left)).unwrap_or(left);
    format!("Waited {}", span(end - sighting.arrival))
}
