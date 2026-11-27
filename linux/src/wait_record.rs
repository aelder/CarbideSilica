//! The chain's record of recent waits for a block (`WaitRecord.swift`). The chain pulse and
//! the Mempool page set the present wait against the same record, thus the app reads it once.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;

use crate::bridge::{self, api};
use crate::format;
use crate::model::AppModel;

/// The length of one counting window in the record.
pub const SPAN: f64 = 25.0 * 60.0;

#[derive(Default)]
pub struct WaitRecord {
    pub history: Option<api::PulseHistory>,
    pub problem: Option<String>,
    read_at: Option<f64>,
    reading_since: Option<f64>,
    generation: Option<u64>,
    /// True while a page that shows the record is live: the node is at the tip.
    live: bool,
    running: bool,
    /// Called after each read, for the pages that draw the record.
    listeners: Vec<(glib::WeakRef<glib::Object>, Rc<dyn Fn()>)>,
}

thread_local! {
    static RECORD: RefCell<WaitRecord> = RefCell::new(WaitRecord::default());
}

pub fn with<T>(read: impl FnOnce(&WaitRecord) -> T) -> T {
    RECORD.with(|cell| read(&cell.borrow()))
}

/// Calls `changed` after each new read of the record, while `owner` exists.
pub fn watch(owner: &impl gtk::prelude::IsA<glib::Object>, changed: impl Fn() + 'static) {
    use gtk::prelude::*;
    RECORD.with(|cell| {
        cell.borrow_mut().listeners.push((owner.upcast_ref::<glib::Object>().downgrade(), Rc::new(changed)))
    });
}

fn changed() {
    let listeners: Vec<Rc<dyn Fn()>> = RECORD.with(|cell| {
        let mut record = cell.borrow_mut();
        record.listeners.retain(|(owner, _)| owner.upgrade().is_some());
        record.listeners.iter().map(|(_, f)| f.clone()).collect()
    });
    for listener in listeners {
        listener();
    }
}

/// Keeps the record current while a page shows it live: it reads the record at the start
/// and each 10 minutes. The waits of a day change slowly.
pub fn keep(model: &Rc<AppModel>, live: bool) {
    let start = RECORD.with(|cell| {
        let mut record = cell.borrow_mut();
        record.live = live;
        !std::mem::replace(&mut record.running, true)
    });
    if !start {
        return;
    }
    let model = Rc::downgrade(model);
    glib::spawn_future_local(async move {
        loop {
            let Some(model) = model.upgrade() else { return };
            let tip = model.state().status.as_ref().map(|s| s.blocks);
            let public = model.state().status.as_ref().is_some_and(|s| s.source == api::DataSource::PublicApi);
            let live = RECORD.with(|cell| cell.borrow().live);
            let (Some(explorer), Some(tip), true) = (model.explorer(), tip, live) else {
                drop(model);
                glib::timeout_future(Duration::from_secs(2)).await;
                continue;
            };
            let generation = model.generation();
            let due = RECORD.with(|cell| {
                let mut record = cell.borrow_mut();
                if record.generation != Some(generation) {
                    record.generation = Some(generation);
                    record.history = None;
                    record.problem = None;
                    record.read_at = None;
                    record.reading_since = None;
                }
                let now = format::now();
                let free = record.reading_since.is_none_or(|since| now - since > 60.0);
                let due = free && record.read_at.is_none_or(|at| now - at >= 600.0);
                if due {
                    record.reading_since = Some(now);
                }
                due
            });
            drop(model);
            if due {
                // A public API answers slowly and sees each request. Ask it for less.
                let count: u32 = if public { 180 } else { 1440 };
                let found = bridge::run(async move { explorer.pulse_history(tip, count, SPAN as u32).await }).await;
                let current = RECORD.with(|cell| cell.borrow().generation);
                RECORD.with(|cell| {
                    let mut record = cell.borrow_mut();
                    if current == Some(generation) {
                        match found {
                            Ok(history) => {
                                record.history = Some(history);
                                record.problem = None;
                            }
                            Err(error) => record.problem = Some(bridge::describe(&error)),
                        }
                    }
                    record.read_at = Some(format::now());
                    record.reading_since = None;
                });
                changed();
            }
            glib::timeout_future(Duration::from_secs(10)).await;
        }
    });
}

impl WaitRecord {
    /// The share of recent waits that are shorter than `seconds`, from 0 to 1.
    pub fn share_shorter(&self, seconds: f64) -> Option<f64> {
        let waits = &self.history.as_ref()?.waits;
        if waits.is_empty() {
            return None;
        }
        let low = waits.partition_point(|wait| (*wait as f64) < seconds);
        Some(low as f64 / waits.len() as f64)
    }
}
