//! The chart page's state and its loading (`ChartState` in ChartsView.swift). Loading runs in
//! steps so that a new request, or leaving the page, stops it after the current step.
//!
//! The Swift view observes the state and redraws what changed. Here each part of the page
//! registers a listener, and [`Charts::changed`] calls them after each change. A part keeps
//! a signature of what it shows and builds its widgets again only when that changes.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::time::Instant;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api};
use crate::format;
use crate::model::AppModel;

use super::model::{FeePoolState, Span};

pub type Listener = Rc<dyn Fn(&Rc<Charts>)>;

pub struct State {
    pub span: Span,
    pub page: Option<Rc<api::ChartPage>>,
    pub loading: bool,
    pub headers_read: u32,
    pub failure: Option<String>,
    pub stopped: Option<String>,
    pub selected: Option<String>,
    pub seek_note: Option<String>,
    pub via_api: bool,
    pub fee: FeePoolState,

    pub request_id: u64,
    request: Option<api::ChartRequest>,
    loaded_context: u32,
    prepared: bool,
    /// The request whose loop runs now, if any.
    running: Option<u64>,
    /// True when the loop stopped because the page went off screen. It goes on when the
    /// page shows again, as the Swift view's task starts again when the view appears.
    paused: bool,
    fee_key: String,
    fee_task: u64,
    fee_running: Option<u64>,
}

pub struct Charts {
    pub model: Weak<AppModel>,
    pub state: RefCell<State>,
    listeners: RefCell<Vec<Listener>>,
    /// The page's widget: the loop stops while it is off screen.
    pub root: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    /// Moves the focus to the seek field (the key g).
    pub focus_seek: RefCell<Option<Rc<dyn Fn()>>>,
    /// Gives the panels the focus.
    pub focus_plot: RefCell<Option<Rc<dyn Fn()>>>,
    notifying: Cell<bool>,
    again: Cell<bool>,
}

impl Charts {
    pub fn new(model: &Rc<AppModel>) -> Rc<Self> {
        Rc::new(Self {
            model: Rc::downgrade(model),
            state: RefCell::new(State {
                span: Span::Day,
                page: None,
                loading: false,
                headers_read: 0,
                failure: None,
                stopped: None,
                selected: None,
                seek_note: None,
                via_api: false,
                fee: FeePoolState::default(),
                request_id: 0,
                request: None,
                loaded_context: 0,
                prepared: false,
                running: None,
                paused: false,
                fee_key: String::new(),
                fee_task: 0,
                fee_running: None,
            }),
            listeners: RefCell::new(Vec::new()),
            root: RefCell::new(None),
            focus_seek: RefCell::new(None),
            focus_plot: RefCell::new(None),
            notifying: Cell::new(false),
            again: Cell::new(false),
        })
    }

    pub fn listen(&self, listener: impl Fn(&Rc<Charts>) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(listener));
    }

    /// Tells each part that the state changed, then starts the fee pool's load when its key
    /// changed. A change made by a listener runs the listeners once more after this round.
    pub fn changed(self: &Rc<Self>) {
        if self.notifying.get() {
            self.again.set(true);
            return;
        }
        self.notifying.set(true);
        loop {
            self.again.set(false);
            let listeners: Vec<Listener> = self.listeners.borrow().clone();
            for listener in listeners {
                listener(self);
            }
            if !self.again.get() {
                break;
            }
        }
        self.notifying.set(false);
        self.sync_fee_task();
    }

    fn mapped(&self) -> bool {
        self.root.borrow().as_ref().and_then(|w| w.upgrade()).is_some_and(|w| w.is_mapped())
    }

    pub fn via_api_now(&self) -> bool {
        self.model
            .upgrade()
            .and_then(|m| m.state().status.as_ref().map(|s| s.source == api::DataSource::PublicApi))
            .unwrap_or(false)
    }

    // MARK: Requests

    pub fn prepare(self: &Rc<Self>, anchor: Option<u64>, hash: Option<String>, via_api: bool) {
        {
            let mut state = self.state.borrow_mut();
            if state.prepared {
                return;
            }
            state.prepared = true;
            state.via_api = via_api;
            if via_api {
                state.span = Span::TwoHours;
            }
            // A local node can read each block's checkpoint. A public API gets the 100 samples.
            state.fee.detail = !via_api;
        }
        self.begin(anchor, hash, false, None, None);
    }

    fn begin(self: &Rc<Self>, end: Option<u64>, anchor: Option<String>, newer: bool, focus: Option<api::ChartBlock>, context: Option<u32>) {
        {
            let mut state = self.state.borrow_mut();
            let via_api = state.via_api;
            state.request = Some(api::ChartRequest {
                span_seconds: state.span.seconds(),
                end_height: end,
                anchor_hash: anchor,
                newer,
                // The 1,000-block share needs 1,000 earlier blocks. A public API gets the short context.
                context: context.unwrap_or(if via_api { 100 } else { 1_000 }),
                focus_height: focus.as_ref().map(|f| f.height),
                focus_hash: focus.map(|f| f.hash),
                chunk: if via_api { 25 } else { 500 },
            });
            state.failure = None;
            state.stopped = None;
            state.loading = true;
            state.headers_read = 0;
            state.request_id += 1;
            state.paused = false;
        }
        self.changed();
        self.start_run();
    }

    /// Starts the loop of the current request, unless it runs already.
    fn start_run(self: &Rc<Self>) {
        let id = {
            let mut state = self.state.borrow_mut();
            if !state.loading || state.running == Some(state.request_id) {
                return;
            }
            state.running = Some(state.request_id);
            state.request_id
        };
        let charts = self.clone();
        glib::spawn_future_local(async move {
            charts.run(id).await;
            let mut state = charts.state.borrow_mut();
            if state.running == Some(id) {
                state.running = None;
            }
        });
    }

    async fn run(self: &Rc<Self>, id: u64) {
        let Some(request) = self.state.borrow().request.clone() else { return };
        let Some(model) = self.model.upgrade() else { return };
        let Ok(explorer) = model.require_explorer() else { return };
        drop(model);
        let load = explorer.clone().chart_load(request.clone());
        let mut published: Option<Instant> = None;
        let mut failure = None;
        loop {
            if self.state.borrow().request_id != id {
                return;
            }
            if !self.mapped() {
                self.state.borrow_mut().paused = true;
                return;
            }
            let step = load.clone();
            let progress = bridge::run(async move { step.step().await }).await;
            if self.state.borrow().request_id != id {
                return;
            }
            let progress = match progress {
                Ok(progress) => progress,
                Err(error) => {
                    failure = Some(bridge::describe(&error));
                    break;
                }
            };
            self.state.borrow_mut().headers_read = progress.headers;
            if progress.done || published.is_none_or(|at| at.elapsed().as_secs_f64() > 0.4) {
                let reader = load.clone();
                let next = bridge::run(async move { reader.page().await }).await;
                let mut state = self.state.borrow_mut();
                if let Some(next) = next
                    && state.request_id == id
                {
                    state.page = Some(Rc::new(next));
                    if progress.done {
                        state.loaded_context = request.context;
                    }
                }
                published = Some(Instant::now());
            }
            if progress.done {
                self.state.borrow_mut().stopped = progress.stopped;
                break;
            }
            self.changed();
        }
        {
            let mut state = self.state.borrow_mut();
            if state.request_id != id {
                return;
            }
            if failure.is_some() {
                state.failure = failure;
            }
            state.loading = false;
        }
        self.changed();
    }

    /// The page came on screen: a load that stopped when it went off goes on.
    pub fn shown(self: &Rc<Self>) {
        let resume = {
            let mut state = self.state.borrow_mut();
            let resume = state.paused && state.loading;
            state.paused = false;
            resume
        };
        if resume {
            self.start_run();
        }
        self.sync_fee_task();
    }

    /// A new connection can be another chain or source. Load the latest range again.
    pub fn reconnected(self: &Rc<Self>, via_api: bool) {
        {
            let mut state = self.state.borrow_mut();
            state.via_api = via_api;
            state.fee.reset(false);
            state.page = None;
            // The fee pool's task starts again for the new connection.
            state.fee_key.clear();
        }
        self.latest();
    }

    pub fn stop(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            state.request_id += 1;
            state.loading = false;
            state.stopped = Some("Loading stopped. The chart shows what loaded before.".into());
        }
        self.changed();
    }

    pub fn set_span(self: &Rc<Self>, value: Span) {
        let (end, anchor) = {
            let mut state = self.state.borrow_mut();
            if value == state.span {
                return;
            }
            state.span = value;
            state.seek_note = None;
            (state.page.as_ref().map(|p| p.last), state.page.as_ref().and_then(|p| p.samples.last().map(|s| s.hash.clone())))
        };
        self.begin(end, anchor, false, None, None);
    }

    pub fn can_page_older(&self) -> bool {
        let state = self.state.borrow();
        !state.loading && state.page.as_ref().map(|p| p.first).unwrap_or(0) > 0
    }

    pub fn can_page_newer(&self) -> bool {
        let state = self.state.borrow();
        !state.loading && state.page.as_ref().is_some_and(|p| p.last < p.tip)
    }

    pub fn older(self: &Rc<Self>) {
        if !self.can_page_older() {
            return;
        }
        let first = {
            let mut state = self.state.borrow_mut();
            state.seek_note = None;
            state.page.as_ref().map(|p| p.first).unwrap_or(0)
        };
        self.begin(Some(first - 1), None, false, None, None);
    }

    pub fn newer(self: &Rc<Self>) {
        if !self.can_page_newer() {
            return;
        }
        let (last, anchor) = {
            let mut state = self.state.borrow_mut();
            state.seek_note = None;
            let page = state.page.as_ref().unwrap();
            (page.last, page.samples.last().map(|s| s.hash.clone()))
        };
        self.begin(Some(last), anchor, true, None, None);
    }

    pub fn refresh(self: &Rc<Self>) {
        let (end, anchor) = {
            let state = self.state.borrow();
            (state.page.as_ref().map(|p| p.last), state.page.as_ref().and_then(|p| p.samples.last().map(|s| s.hash.clone())))
        };
        self.state.borrow_mut().fee.reset(false);
        self.begin(end, anchor, false, None, None);
    }

    pub fn latest(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            state.seek_note = None;
            state.selected = None;
        }
        self.begin(None, None, false, None, None);
    }

    /// Loads a range centred on a sought block, and selects that block.
    pub fn show(self: &Rc<Self>, outcome: api::SeekOutcome) {
        {
            let mut state = self.state.borrow_mut();
            state.selected = Some(outcome.block.hash.clone());
            state.seek_note = Some(outcome.note.clone());
        }
        self.begin(Some(outcome.end.height), Some(outcome.end.hash.clone()), false, Some(outcome.block), None);
    }

    // MARK: Selection

    /// The selected block. Each block can be selected: the panels show all measures at once.
    pub fn selected_sample(&self) -> Option<api::ChartSample> {
        let state = self.state.borrow();
        let samples = &state.page.as_ref()?.samples;
        if let Some(selected) = &state.selected {
            if let Some(found) = samples.iter().find(|s| &s.hash == selected) {
                return Some(found.clone());
            }
            if state.loading {
                return None;
            }
        }
        samples.last().cloned()
    }

    pub fn select(self: &Rc<Self>, hash: &str) {
        {
            let mut state = self.state.borrow_mut();
            state.selected = Some(hash.to_string());
            state.seek_note = None;
        }
        self.changed();
    }

    /// Moves the selection to an older (negative) or newer (positive) sample.
    pub fn move_by(self: &Rc<Self>, delta: i64) {
        let current = self.selected_sample().map(|s| s.hash);
        {
            let mut state = self.state.borrow_mut();
            let Some(page) = state.page.clone() else { return };
            let list = &page.samples;
            if list.is_empty() {
                return;
            }
            let index = current.and_then(|hash| list.iter().position(|s| s.hash == hash)).unwrap_or(list.len() - 1);
            let next = (index as i64 + delta).clamp(0, list.len() as i64 - 1) as usize;
            state.selected = Some(list[next].hash.clone());
            state.seek_note = None;
        }
        self.changed();
    }

    pub fn move_to_end(self: &Rc<Self>, newest: bool) {
        {
            let mut state = self.state.borrow_mut();
            let list = state.page.as_ref().map(|p| p.samples.clone()).unwrap_or_default();
            state.selected = if newest { list.last() } else { list.first() }.map(|s| s.hash.clone());
            state.seek_note = None;
        }
        self.changed();
    }

    pub fn step_span(self: &Rc<Self>, delta: i32) {
        let span = self.state.borrow().span.stepped(delta);
        self.set_span(span);
    }

    pub fn open(&self, hash: &str) {
        if let Some(model) = self.model.upgrade() {
            model.open(api::Route::Block { id: hash.to_string() });
        }
    }

    // MARK: Text

    pub fn range_label(&self) -> Option<String> {
        let state = self.state.borrow();
        let page = state.page.as_ref()?;
        Some(format!("blocks {} to {}", format::integer(page.first), format::integer(page.last)))
    }

    /// What the current load reads, in words.
    pub fn loading_text(&self) -> String {
        let state = self.state.borrow();
        if state.headers_read > 0 {
            return format!("Read {} headers", format::integer(state.headers_read as u64));
        }
        if let (Some(page), Some(request)) = (&state.page, &state.request)
            && request.context > state.loaded_context
            && request.end_height == Some(page.last)
        {
            return format!("Reading {} earlier blocks", format::integer(request.context as u64 - 1));
        }
        "Reading the range's block headers".into()
    }

    fn limit_part(&self, page: &api::ChartPage, loading: bool) -> Option<&'static str> {
        if page.limited {
            Some("partial, because the request reached the 20,000-header limit")
        } else if !page.complete && loading {
            Some("loading")
        } else if page.first == 0 && !page.complete {
            Some("starts at genesis")
        } else {
            None
        }
    }

    /// What a reader needs: how many blocks, and anything that limits the range.
    pub fn coverage_short(&self) -> String {
        let state = self.state.borrow();
        let Some(page) = &state.page else { return String::new() };
        let mut parts = vec![format!("{} blocks", format::integer(page.samples.len() as u64))];
        if let Some(part) = self.limit_part(page, state.loading) {
            parts.push(part.into());
        }
        if !page.status.is_empty() && page.status != "Loading" {
            parts.push(page.status.clone());
        }
        parts.join(" · ") + "."
    }

    pub fn coverage_sentence(&self) -> String {
        let state = self.state.borrow();
        let Some(page) = &state.page else { return String::new() };
        let mut parts = vec![
            format!("{} blocks shown", format::integer(page.samples.len() as u64)),
            format!(
                "{} headers read, with {} earlier blocks for averages and intervals",
                format::integer(page.headers as u64),
                format::integer(page.context as u64)
            ),
            format!("span {}", state.span.title()),
        ];
        if let Some(part) = self.limit_part(page, state.loading) {
            parts.push(part.into());
        }
        if page.stored > 0 {
            parts.push(format!("{} headers from Carbide's cache", format::integer(page.stored as u64)));
        }
        if !page.status.is_empty() && page.status != "Loading" {
            parts.push(page.status.clone());
        }
        parts.join(" · ") + "."
    }

    // MARK: Fee pool

    /// Changes when the fee pool checkpoints must load again.
    fn fee_pool_key(&self) -> String {
        let state = self.state.borrow();
        let generation = self.model.upgrade().map(|m| m.generation()).unwrap_or(0);
        match &state.page {
            Some(page) if !state.loading => {
                format!("{}|{}|{}|{}|{}", page.first, page.last, state.fee.detail, state.fee.generation, generation)
            }
            _ => format!("idle|{generation}"),
        }
    }

    /// Starts the load of the checkpoints when their key changed, as the Swift view's
    /// `.task(id: feePoolKey)`: a new key stops the load before it.
    fn sync_fee_task(self: &Rc<Self>) {
        let key = self.fee_pool_key();
        let task = {
            let mut state = self.state.borrow_mut();
            if state.fee_key == key {
                return;
            }
            state.fee_key = key;
            state.fee_task += 1;
            state.fee_task
        };
        let charts = self.clone();
        glib::spawn_future_local(async move {
            charts.state.borrow_mut().fee_running = Some(task);
            charts.load_fee_pool(task).await;
            let changed = {
                let mut state = charts.state.borrow_mut();
                if state.fee_running == Some(task) {
                    state.fee_running = None;
                    state.fee.loading = false;
                    true
                } else {
                    false
                }
            };
            if changed {
                charts.changed();
            }
        });
    }

    fn fee_cancelled(&self, task: u64) -> bool {
        self.state.borrow().fee_task != task
    }

    async fn load_fee_pool(self: &Rc<Self>, task: u64) {
        let Some(page) = self.state.borrow().page.clone() else { return };
        if page.samples.is_empty() {
            return;
        }
        let Some(model) = self.model.upgrade() else { return };
        let Ok(explorer) = model.require_explorer() else { return };
        drop(model);
        let samples = &page.samples;
        let needs_current = {
            let mut state = self.state.borrow_mut();
            state.fee.loading = true;
            state.fee.error = None;
            state.fee.current.is_none()
        };
        self.changed();
        if needs_current {
            let ex = explorer.clone();
            let current = bridge::run(async move { ex.fee_pool_current().await }).await;
            if self.fee_cancelled(task) {
                return;
            }
            {
                let mut state = self.state.borrow_mut();
                match current {
                    Ok(current) => state.fee.current = Some(current),
                    Err(error) => state.fee.current_error = Some(bridge::describe(&error)),
                }
            }
            self.changed();
        }
        let (missing, via_api) = {
            let mut state = self.state.borrow_mut();
            let selected_index = state.selected.as_ref().and_then(|hash| samples.iter().position(|s| &s.hash == hash));
            let order: Vec<usize> =
                api::fee_pool_order(samples.len() as u32, selected_index.map(|i| i as u32), state.fee.detail)
                    .into_iter()
                    .map(|i| i as usize)
                    .filter(|i| *i < samples.len())
                    .collect();
            state.fee.scheduled = order.iter().map(|i| samples[*i].hash.clone()).collect::<HashSet<_>>();
            state.fee.requested = order.len();
            let missing: Vec<usize> = order.iter().copied().filter(|i| !state.fee.checkpoints.contains_key(&samples[*i].hash)).collect();
            (missing, state.via_api)
        };
        self.changed();
        // The endpoints and the selected block first, one at a time, then larger groups.
        let mut start = 0;
        while start < missing.len() {
            if self.fee_cancelled(task) {
                return;
            }
            let size = if start < 4 {
                if start < 2 { 2 } else { 1 }
            } else if via_api {
                4
            } else {
                16
            };
            let batch = &missing[start..(start + size).min(missing.len())];
            start += size;
            let blocks: Vec<api::ChartBlock> = batch
                .iter()
                .map(|i| api::ChartBlock { height: samples[*i].height, hash: samples[*i].hash.clone(), time: samples[*i].time })
                .collect();
            let ex = explorer.clone();
            let found = bridge::run(async move { ex.fee_pool_checkpoints(blocks).await }).await;
            if self.fee_cancelled(task) {
                return;
            }
            match found {
                Ok(found) => {
                    let mut state = self.state.borrow_mut();
                    for checkpoint in found {
                        state.fee.checkpoints.insert(checkpoint.hash.clone(), checkpoint);
                    }
                }
                Err(error) => {
                    self.state.borrow_mut().fee.error = Some(bridge::describe(&error));
                    self.changed();
                    return;
                }
            }
            self.changed();
        }
    }
}
