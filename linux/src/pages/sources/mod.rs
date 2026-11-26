//! Every node and public API that the app knows, with its state: the terminal's Sources
//! (F2). Port of SourcesView.swift in `macos/Sources/CarbideSilica/Views`.
//!
//! States apply to this session at once. Save writes them to Carbide's settings file, which
//! the terminal app also reads.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api};
use crate::format;
use crate::model::{AppModel, Place, Topic};
use crate::page::{self, load::clear, Card, Page};
use crate::settings::{self, key};
use crate::shell::dialogs;
use crate::theme::draw;
use crate::theme::{tone, Color};
use crate::widgets::icons::{self, Icon};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, PlateFill};
use crate::widgets::{lamp, loader};

/// A source's chain by name: the bridge's name, a resolved name, or the short form.
pub fn source_name(row: &api::SourceRow, model: &AppModel) -> String {
    if let Some(name) = &row.name {
        return name.clone();
    }
    if let Some(name) = model.state().names.get(&row.system_id) {
        return name.strip_suffix('@').unwrap_or(name).to_string();
    }
    chain_name(&row.chain)
}

/// PBaaS chains are stored under their hex ID. Show a short form until a name is known.
pub fn chain_name(chain: &str) -> String {
    if chain.chars().count() == 40 { format!("PBaaS chain {}…", chain.chars().take(8).collect::<String>()) } else { chain.to_string() }
}

/// Carbide's short ages: <1s, 42s, 5m, 3h, 2d.
pub fn short_age(seconds: u64) -> String {
    match seconds {
        0 => "<1s".into(),
        1..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m", seconds / 60),
        3600..=86_399 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

fn state_meaning(row: &api::SourceRow) -> &'static str {
    match (row.kind, row.state) {
        (_, api::SourceState::Disabled) => "Disabled. The app sends no request to this source.",
        (api::SourceKind::PublicApi, _) => "Enabled. The app uses this API when the local node cannot answer.",
        (_, api::SourceState::Enabled) => {
            if row.current {
                "Enabled. The app sends requests for what you open, and keeps the chain's block history current."
            } else {
                "Enabled. The app sends requests for what you open on this chain."
            }
        }
        (_, api::SourceState::Monitored) => {
            "Monitored. The source is enabled, and the app also follows the tip and the mempool in the background."
        }
    }
}

fn traffic_word(traffic: api::SourceTraffic) -> &'static str {
    match traffic {
        api::SourceTraffic::Off => "Off",
        api::SourceTraffic::NotUsed => "Not used",
        api::SourceTraffic::Working => "Working",
        api::SourceTraffic::Responding => "Responding",
        api::SourceTraffic::Idle => "Idle",
        api::SourceTraffic::Unavailable => "Unavailable",
    }
}

fn traffic_color(traffic: api::SourceTraffic) -> Color {
    match traffic {
        api::SourceTraffic::Responding => tone::GOOD,
        api::SourceTraffic::Working => tone::RECORD,
        api::SourceTraffic::Unavailable => tone::ALERT,
        _ => tone::SECONDARY,
    }
}

/// Observed traffic, as shape, colour and word (`TrafficMark`). The shapes are the SF
/// Symbols of the Swift app, drawn: minus.circle, circle, arrow.triangle.2.circlepath,
/// checkmark.circle.fill, circle.fill and xmark.octagon.fill.
fn traffic_mark(traffic: api::SourceTraffic) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    let color = traffic_color(traffic);
    let kind = match traffic {
        api::SourceTraffic::Off => Icon::MinusCircle,
        api::SourceTraffic::NotUsed => Icon::Ring,
        api::SourceTraffic::Working => Icon::Circlepath,
        api::SourceTraffic::Responding => Icon::CheckCircleFill,
        api::SourceTraffic::Idle => Icon::Dot,
        api::SourceTraffic::Unavailable => Icon::CrossOctagonFill,
    };
    let mark = icons::icon(kind, 13, color);
    mark.set_valign(gtk::Align::Center);
    row.append(&mark);
    let word = page::text(traffic_word(traffic), &[]);
    word.set_markup(&page::span(traffic_word(traffic), color));
    row.append(&word);
    row
}

struct Sources {
    model: std::rc::Weak<AppModel>,
    page: std::rc::Weak<Page>,
    overview: RefCell<Option<api::SourcesOverview>>,
    /// The overview as drawn, to draw again only when it changed.
    signature: RefCell<String>,
    selected_key: RefCell<Option<String>>,
    message: RefCell<Option<String>>,
    holder: gtk::Box,
    /// The key of the row whose activity shows, and its loop.
    activity: RefCell<Option<Rc<Activity>>>,
}

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>) -> Rc<Page> {
    let page = Page::scrolling(model);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 20);
    page.append(&holder);
    let sources = Rc::new(Sources {
        model: Rc::downgrade(model),
        page: Rc::downgrade(&page),
        overview: RefCell::default(),
        signature: RefCell::default(),
        selected_key: RefCell::default(),
        message: RefCell::default(),
        holder,
        activity: RefCell::default(),
    });
    {
        let weak = Rc::downgrade(&sources);
        page.hooks().refresh = Some(Rc::new(move || {
            if let Some(sources) = weak.upgrade() {
                sources.reload(true);
            }
        }));
    }
    sources.draw();
    // The rows hold the session's state, read in memory. Refresh them while visible.
    {
        let weak = Rc::downgrade(&sources);
        let root = page.root.downgrade();
        let last = Rc::new(Cell::new(0.0f64));
        glib::timeout_add_local(Duration::from_millis(250), move || {
            let (Some(sources), Some(root)) = (weak.upgrade(), root.upgrade()) else { return glib::ControlFlow::Break };
            let Some(model) = sources.model.upgrade() else { return glib::ControlFlow::Break };
            let shown = root.is_mapped() && model.place() == Place::Sources && model.state().path.is_empty();
            if shown && format::now() - last.get() >= 3.0 {
                last.set(format::now());
                sources.reload(false);
            } else if !shown {
                last.set(0.0);
            }
            glib::ControlFlow::Continue
        });
    }
    let weak = Rc::downgrade(&sources);
    model.watch(&[Topic::Connection, Topic::Names, Topic::Display], &page.root, move |_| {
        if let Some(sources) = weak.upgrade() {
            sources.reload(true);
        }
    });
    let keep = sources.clone();
    page.root.connect_destroy(move |_| {
        let _ = &keep;
    });
    page
}

impl Sources {
    fn reload(self: &Rc<Self>, force: bool) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(explorer) = model.explorer() else { return };
        let fresh = explorer.sources_overview();
        // A PBaaS chain's folder is its hex ID. Learn its name from its identity.
        let unnamed: Vec<String> = fresh.rows.iter().filter(|r| r.name.is_none()).map(|r| r.system_id.clone()).collect();
        let signature = format!("{fresh:?}");
        *self.overview.borrow_mut() = Some(fresh);
        if force || *self.signature.borrow() != signature {
            *self.signature.borrow_mut() = signature;
            self.draw();
        }
        if !unnamed.is_empty() {
            model.resolve_names(unnamed);
        }
    }

    fn selected(&self, overview: &api::SourcesOverview) -> Option<api::SourceRow> {
        let key = self.selected_key.borrow().clone();
        overview.rows.iter().find(|r| Some(&r.key) == key.as_ref()).or_else(|| overview.rows.iter().find(|r| r.current)).cloned()
    }

    fn draw(self: &Rc<Self>) {
        let (Some(page), Some(model)) = (self.page.upgrade(), self.model.upgrade()) else { return };
        clear(&self.holder);
        let overview = self.overview.borrow().clone();
        let Some(overview) = overview else {
            let card = page.card("Nodes and APIs", None);
            let progress = loader::small();
            progress.set_halign(gtk::Align::Start);
            card.append(&progress);
            self.holder.append(&card.root);
            return;
        };
        let card = page.card("Nodes and APIs", overview.unsaved.then_some("Not saved"));
        card.append(&page::paragraph(
            "A disabled source receives no request. An enabled source answers what you open. A monitored node is also followed in the background. Changes apply at once. Save writes them to the settings file that the terminal app also reads.",
            &["secondary"],
        ));
        if let Some(problem) = &overview.settings_problem {
            card.append(&page::note(problem, true));
        }
        if let Some(problem) = &overview.cache_problem {
            card.append(&page::note(problem, true));
        }
        if let Some(file) = &overview.settings_file {
            card.append(&page::field_row(&page, "Settings file", file, None, true));
        }
        card.append(&self.table(&page, &model, &overview));
        let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if let Some(message) = self.message.borrow().as_ref() {
            let note = page::note(message, true);
            note.set_hexpand(true);
            bottom.append(&note);
        } else {
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_hexpand(true);
            bottom.append(&spacer);
        }
        let weak = Rc::downgrade(self);
        let save = lamp::button("Save", move || {
            if let Some(sources) = weak.upgrade() {
                sources.save();
            }
        });
        save.set_sensitive(overview.unsaved && overview.settings_problem.is_none());
        save.set_tooltip_text(Some("Write the states to Carbide's settings file"));
        save.set_valign(gtk::Align::Start);
        bottom.append(&save);
        card.append(&bottom);
        self.holder.append(&card.root);
        if let Some(row) = self.selected(&overview) {
            self.holder.append(&self.detail(&page, &model, &row).root);
            let activity = {
                let current = self.activity.borrow().clone();
                match current {
                    Some(activity) if activity.key == row.key => activity,
                    _ => Activity::new(&model, &row.key),
                }
            };
            activity.attach(&page);
            self.holder.append(&activity.card_root());
            *self.activity.borrow_mut() = Some(activity);
        } else {
            *self.activity.borrow_mut() = None;
        }
    }

    /// The sources as rows: chain, kind, endpoint, state, traffic and age of the last answer.
    fn table(self: &Rc<Self>, page: &Rc<Page>, model: &Rc<AppModel>, overview: &api::SourcesOverview) -> gtk::Grid {
        let grid = gtk::Grid::new();
        grid.set_column_spacing(14);
        grid.set_row_spacing(6);
        for (column, title) in ["Chain", "Source", "Endpoint", "State", "Traffic", "Last answer"].iter().enumerate() {
            let name = display::name_text(title);
            if column == 5 {
                name.set_xalign(1.0);
                name.set_halign(gtk::Align::End);
            }
            grid.attach(&name, column as i32, 0, 1, 1);
        }
        grid.attach(&page::divider(), 0, 1, 6, 1);
        let selected = self.selected_key.borrow().clone();
        let mut backgrounds: Vec<(Canvas, String)> = Vec::new();
        for (index, row) in overview.rows.iter().enumerate() {
            let y = index as i32 + 2;
            let chosen = selected.as_ref() == Some(&row.key);
            // The field of the selected row, under its cells.
            let field = Canvas::new(0, 0, move |_, snapshot, w, h| {
                if chosen {
                    draw::fill_rect(snapshot, 0.0, 0.0, w, h, tone::PRESENT.opacity(0.1));
                }
            });
            field.set_hexpand(true);
            grid.attach(&field, 0, y, 6, 1);
            let name_cell = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            name_cell.set_margin_top(2);
            name_cell.set_margin_bottom(2);
            let name = page::text(&source_name(row, model), if row.current { &["semibold"] } else { &[] });
            name.set_valign(gtk::Align::Center);
            name_cell.append(&name);
            if row.current {
                name_cell.append(&display::plate("Current", PlateFill::None));
            }
            grid.attach(&name_cell, 0, y, 1, 1);
            let kind = page::text(if row.kind == api::SourceKind::LocalNode { "Node" } else { "API" }, &[]);
            grid.attach(&kind, 1, y, 1, 1);
            let endpoint = page::text(&row.endpoint, &["mono", "callout"]);
            endpoint.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            endpoint.set_max_width_chars(34);
            endpoint.set_width_chars(1);
            endpoint.set_size_request(-1, -1);
            grid.attach(&endpoint, 2, y, 1, 1);
            grid.attach(&self.state_picker(row), 3, y, 1, 1);
            let traffic = traffic_mark(row.traffic);
            grid.attach(&traffic, 4, y, 1, 1);
            let age = page::text(&row.last_success_age.map(|a| format!("{} ago", short_age(a))).unwrap_or_else(|| "None".into()), &["tabular", "secondary"]);
            age.set_xalign(1.0);
            age.set_halign(gtk::Align::End);
            grid.attach(&age, 5, y, 1, 1);
            for cell in [name_cell.upcast_ref::<gtk::Widget>(), kind.upcast_ref(), endpoint.upcast_ref(), traffic.upcast_ref(), age.upcast_ref()] {
                cell.set_valign(gtk::Align::Center);
            }
            let (weak, key) = (Rc::downgrade(self), row.key.clone());
            page.stop(&field, Some(format!("source-{}", row.key)), None, move || {
                if let Some(sources) = weak.upgrade() {
                    sources.select(&key);
                }
            });
            backgrounds.push((field, row.key.clone()));
        }
        // A click on a row selects it.
        let click = gtk::GestureClick::new();
        let (weak, grid_ref) = (Rc::downgrade(self), grid.downgrade());
        click.connect_released(move |_, _, _, y| {
            let (Some(sources), Some(grid)) = (weak.upgrade(), grid_ref.upgrade()) else { return };
            for (field, key) in &backgrounds {
                if let Some(bounds) = field.compute_bounds(&grid)
                    && (y as f32) >= bounds.y()
                    && (y as f32) < bounds.y() + bounds.height()
                {
                    sources.select(key);
                    return;
                }
            }
        });
        grid.add_controller(click);
        grid
    }

    fn select(self: &Rc<Self>, key: &str) {
        if self.selected_key.borrow().as_deref() == Some(key) {
            return;
        }
        *self.selected_key.borrow_mut() = Some(key.to_string());
        self.draw();
    }

    /// Disabled, Enabled and, for a node, Monitored.
    fn state_picker(self: &Rc<Self>, row: &api::SourceRow) -> gtk::Box {
        let mut states = vec![(api::SourceState::Disabled, "Disabled"), (api::SourceState::Enabled, "Enabled")];
        if row.kind == api::SourceKind::LocalNode {
            states.push((api::SourceState::Monitored, "Monitored"));
        }
        let titles: Vec<&str> = states.iter().map(|s| s.1).collect();
        let selected = states.iter().position(|s| s.0 == row.state).unwrap_or(0);
        let (weak, row_copy) = (Rc::downgrade(self), row.clone());
        let segments = lamp::Segments::new(&titles, selected, move |index| {
            if let Some(sources) = weak.upgrade() {
                sources.change(&row_copy, states[index].0);
            }
        });
        segments.root.set_sensitive(!(row.kind == api::SourceKind::LocalNode && !row.has_credentials));
        segments.root.set_halign(gtk::Align::Start);
        segments.root.set_valign(gtk::Align::Center);
        segments.root
    }

    fn change(self: &Rc<Self>, row: &api::SourceRow, state: api::SourceState) {
        if row.state == state {
            return;
        }
        if row.kind == api::SourceKind::PublicApi && row.state == api::SourceState::Disabled && state != api::SourceState::Disabled {
            let Some(window) = self.page.upgrade().and_then(|p| p.root.root()).and_downcast::<gtk::Window>() else { return };
            let dialog = dialogs::dialog(
                &window,
                "Enable this public API?",
                Some(&format!(
                    "When the local node cannot answer, the app sends the request to {}. The operator of the API can see what you look up.",
                    row.endpoint
                )),
            );
            // A dialog closed without Enable leaves the state as it was.
            let weak = Rc::downgrade(self);
            dialog.window.connect_close_request(move |_| {
                if let Some(sources) = weak.upgrade() {
                    sources.reload(true);
                }
                glib::Propagation::Proceed
            });
            dialog.key("Cancel", true, || {});
            let (weak, row) = (Rc::downgrade(self), row.clone());
            dialog.key("Enable", false, move || {
                if let Some(sources) = weak.upgrade() {
                    sources.apply(&row, state);
                }
            });
            dialog.show();
        } else {
            self.apply(row, state);
        }
    }

    fn apply(self: &Rc<Self>, row: &api::SourceRow, state: api::SourceState) {
        let Some(model) = self.model.upgrade() else { return };
        *self.message.borrow_mut() = match model.require_explorer() {
            Ok(explorer) => explorer.set_source_state(row.key.clone(), state).err().map(|e| bridge::describe(&e)),
            Err(message) => Some(message),
        };
        self.reload(true);
    }

    fn save(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        *self.message.borrow_mut() = match model.require_explorer() {
            Ok(explorer) => explorer.save_sources().err().map(|e| bridge::describe(&e)),
            Err(message) => Some(message),
        };
        self.reload(true);
    }

    /// What one source is, what its state permits, and its last chain check.
    fn detail(&self, page: &Rc<Page>, model: &Rc<AppModel>, row: &api::SourceRow) -> Card {
        let card = page.card(&format!("{} {}", source_name(row, model), if row.kind == api::SourceKind::LocalNode { "node" } else { "API" }), None);
        card.append(&page::field_row(page, "System ID", &row.system_id, None, true));
        card.append(&page::field_row(page, "Endpoint", &row.endpoint, None, true));
        if let Some(file) = &row.config_file {
            card.append(&page::field_row(page, "Configuration file", file, None, true));
        }
        let line = |label: &str, value: &gtk::Widget| {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let name = display::name_text(label);
            name.set_size_request(170, -1);
            name.set_valign(gtk::Align::Baseline);
            row.append(&name);
            value.set_valign(gtk::Align::Baseline);
            value.set_hexpand(true);
            row.append(value);
            row
        };
        card.append(&line("State", page::paragraph(state_meaning(row), &[]).upcast_ref()));
        if let Some(check) = &row.chain_check {
            card.append(&line("Chain check", page::note(check, row.chain_check_problem).upcast_ref()));
        }
        if row.kind == api::SourceKind::LocalNode && !row.has_credentials {
            card.append(&page::note("The configuration file has no RPC user and password. The app cannot use this node.", true));
        }
        if row.kind == api::SourceKind::LocalNode && !row.current && row.config_file.is_some() {
            let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let text = page::paragraph("The current chain sets the dashboard, mempool, peers and search.", &["secondary"]);
            text.set_hexpand(true);
            bottom.append(&text);
            let (weak, source) = (Rc::downgrade(model), row.clone());
            bottom.append(&lamp::button("Make Current Chain", move || {
                if let Some(model) = weak.upgrade() {
                    model.switch_chain_to_source(&source);
                }
            }));
            card.append(&bottom);
        }
        card
    }
}

/// Significant events that the cache recorded: reorganizations, changed blocks and
/// disagreements between sources (`SourceActivity`).
struct Activity {
    key: String,
    model: std::rc::Weak<AppModel>,
    root: gtk::Box,
    content: gtk::Box,
    trailing: RefCell<Option<Card>>,
    events: RefCell<Option<Vec<api::ActivityRow>>>,
    failure: RefCell<Option<String>>,
    signature: RefCell<Option<String>>,
    expanded: RefCell<HashSet<usize>>,
    /// The parts that open, kept while they show.
    lines: RefCell<Vec<Rc<lamp::Disclosure>>>,
    /// Bumped when the selection or the setting changes: an older answer is dropped.
    run: Cell<u64>,
}

fn all_sources() -> bool {
    settings::bool(key::ALL_SOURCE_ACTIVITY, false)
}

impl Activity {
    fn new(model: &Rc<AppModel>, key: &str) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let activity = Rc::new(Self {
            key: key.to_string(),
            model: Rc::downgrade(model),
            root,
            content,
            trailing: RefCell::default(),
            events: RefCell::default(),
            failure: RefCell::default(),
            signature: RefCell::default(),
            expanded: RefCell::default(),
            lines: RefCell::default(),
            run: Cell::new(0),
        });
        // Every five seconds while the card is on screen.
        let weak = Rc::downgrade(&activity);
        page::every(&activity.root, Duration::from_secs(5), move || {
            if let Some(activity) = weak.upgrade() {
                activity.load();
            }
        });
        activity
    }

    fn card_root(&self) -> gtk::Box {
        self.root.clone()
    }

    /// Puts the card in the page again: the page builds its cards again on each change.
    fn attach(self: &Rc<Self>, page: &Rc<Page>) {
        if let Some(parent) = self.root.parent().and_downcast::<gtk::Box>() {
            parent.remove(&self.root);
        }
        clear(&self.root);
        let card = page.card("Recent activity", Some(if all_sources() { "All sources" } else { "This source" }));
        let weak = Rc::downgrade(self);
        card.append(&page::check_box("Show the activity of all sources", all_sources(), move |on| {
            settings::set_bool(key::ALL_SOURCE_ACTIVITY, on);
            if let Some(activity) = weak.upgrade() {
                if let Some(card) = activity.trailing.borrow().as_ref() {
                    card.set_trailing(Some(if on { "All sources" } else { "This source" }));
                }
                activity.events.replace(None);
                activity.signature.replace(None);
                activity.show();
                activity.load();
            }
        }));
        if let Some(parent) = self.content.parent().and_downcast::<gtk::Box>() {
            parent.remove(&self.content);
        }
        card.append(&self.content);
        self.root.append(&card.root);
        *self.trailing.borrow_mut() = Some(card);
        if self.signature.borrow().is_none() {
            self.show();
        }
    }

    fn load(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let run = self.run.get() + 1;
        self.run.set(run);
        let (weak, key, all) = (Rc::downgrade(self), self.key.clone(), all_sources());
        glib::spawn_future_local(async move {
            let result = match model.require_explorer() {
                Ok(explorer) => bridge::run(async move { explorer.source_activity(key, all).await }).await.map_err(|e| bridge::describe(&e)),
                Err(message) => Err(message),
            };
            let Some(activity) = weak.upgrade() else { return };
            if activity.run.get() != run {
                return;
            }
            match result {
                Ok(events) => {
                    *activity.events.borrow_mut() = Some(events);
                    *activity.failure.borrow_mut() = None;
                }
                Err(message) => *activity.failure.borrow_mut() = Some(message),
            }
            let signature = format!("{:?}{:?}", activity.events.borrow(), activity.failure.borrow());
            if activity.signature.borrow().as_deref() != Some(signature.as_str()) {
                *activity.signature.borrow_mut() = Some(signature);
                activity.show();
            }
        });
    }

    fn show(self: &Rc<Self>) {
        clear(&self.content);
        self.lines.borrow_mut().clear();
        if let Some(failure) = self.failure.borrow().as_ref() {
            self.content.append(&page::note(failure, true));
            return;
        }
        let events = self.events.borrow();
        let Some(events) = events.as_ref() else {
            let progress = loader::small();
            progress.set_halign(gtk::Align::Start);
            self.content.append(&progress);
            return;
        };
        if events.is_empty() {
            self.content.append(&page::paragraph("The cache has no recorded activity for this selection.", &["secondary"]));
        } else {
            let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
            for (index, event) in events.iter().enumerate() {
                list.append(&self.line(index, event));
            }
            self.content.append(&list);
        }
        self.content.append(&page::paragraph(
            "The cache keeps the newest 200 events. A missing event does not prove that nothing happened.",
            &["caption-text", "secondary"],
        ));
    }

    fn line(self: &Rc<Self>, index: usize, event: &api::ActivityRow) -> gtk::Box {
        let weak = Rc::downgrade(self);
        let disclosure = lamp::Disclosure::new("", self.expanded.borrow().contains(&index), move |open| {
            if let Some(activity) = weak.upgrade() {
                let mut expanded = activity.expanded.borrow_mut();
                if open {
                    expanded.insert(index);
                } else {
                    expanded.remove(&index);
                }
            }
        });
        let age = page::text(&format::age(event.observed_at, format::now()), &["tabular", "secondary"]);
        age.set_size_request(90, -1);
        let status = page::text(&event.status, &["value"]);
        status.set_size_request(110, -1);
        let summary = page::text(&event.summary, &["value"]);
        summary.set_ellipsize(gtk::pango::EllipsizeMode::End);
        summary.set_hexpand(true);
        let label = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        label.append(&age);
        label.append(&status);
        label.append(&summary);
        label.set_hexpand(true);
        disclosure.header.append(&label);
        let message = page::text(&event.message, &["mono", "callout"]);
        message.set_selectable(true);
        message.set_wrap(true);
        message.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        disclosure.content.append(&message);
        let root = disclosure.root.clone();
        self.lines.borrow_mut().push(disclosure);
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_and_names() {
        assert_eq!(short_age(0), "<1s");
        assert_eq!(short_age(42), "42s");
        assert_eq!(short_age(300), "5m");
        assert_eq!(short_age(3 * 3600 + 5), "3h");
        assert_eq!(short_age(2 * 86_400), "2d");
        assert_eq!(chain_name("VRSC"), "VRSC");
        assert_eq!(chain_name("0123456789abcdef0123456789abcdef01234567"), "PBaaS chain 01234567…");
    }
}
