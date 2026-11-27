//! The Identity page: a VerusID at one of its versions, who controls it, how it was
//! registered, what it is associated with, and its transactions. Port of
//! `IdentityView.swift`, `IdentityControl.swift`, `IdentityEvidence.swift`,
//! `IdentityExtras.swift`, `IdentityOrigin.swift`, `IdentitySupport.swift` and
//! `RegistrationPayments.swift` in `macos/Sources/CarbideSilica/Views`.
//!
//! The route names only the identity. The version being viewed is state of this page: it
//! starts at the current version, and the version strip, the history list and the Go
//! menu's Previous Version and Next Version step through the others.

mod control;
mod evidence;
mod extras;
mod lanes;
mod origin;

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge;
use crate::bridge::api::{AddressOverview, IdentityOffers, IdentityState, IdentityTimeline, Route, Timelock};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, Adjacent, Card, Investigations, LoadView, Page};
use crate::shared::address_activity;
use crate::shared::identity_support::{self as support, height_and_time, small_progress, SectionLoader};
use crate::shared::json_tree::json_section_keeping;
use crate::theme::{self, font, tone, Color};
use crate::widgets::display::{self, PlateFill};
use crate::widgets::icons::{self, Icon};
use crate::widgets::{lamp, link};

use evidence::{EvidenceCard, EvidenceState};
use extras::TokenControlCard;
use lanes::VersionStrip;
use crate::shared::registration_payments::RegistrationPaymentsCard;

/// Pango markup for a text in a type role and a colour.
pub(crate) fn role_markup(role: font::Role, text: &str, color: Color) -> String {
    format!(
        "<span foreground=\"#{:02x}{:02x}{:02x}\" alpha=\"{}%\">{}</span>",
        (color.r * 255.0).round() as u8,
        (color.g * 255.0).round() as u8,
        (color.b * 255.0).round() as u8,
        ((color.a * 100.0).round() as u32).max(1),
        theme::markup(role, text)
    )
}

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>, name: &str) -> Rc<Page> {
    let page = Page::scrolling(model);
    // The page of the loaded timeline. A refresh loads the identity again and builds a new
    // one, as the Swift view does when its key changes.
    let current: Rc<RefCell<Option<Rc<IdentityView>>>> = Rc::default();
    let load_slot: Rc<RefCell<Weak<LoadView>>> = Rc::default();
    let refresh: Rc<dyn Fn()> = {
        let load_slot = load_slot.clone();
        Rc::new(move || {
            let load = load_slot.borrow().upgrade();
            if let Some(load) = load {
                load.reload();
            }
        })
    };
    let name = name.to_string();
    let load = {
        let (weak_page, current, refresh) = (Rc::downgrade(&page), current.clone(), refresh.clone());
        LoadView::new(
            model,
            move |explorer| {
                let name = name.clone();
                async move { explorer.identity_timeline(name).await }
            },
            move |holder, timeline: IdentityTimeline| {
                let Some(page) = weak_page.upgrade() else { return };
                {
                    let mut hooks = page.hooks();
                    hooks.adjacent = None;
                    hooks.investigations = Investigations::default();
                    hooks.toggle_json = None;
                }
                if timeline.versions.is_empty() {
                    current.replace(None);
                    holder.append(&no_versions());
                    return;
                }
                current.replace(Some(IdentityView::new(&page, holder, timeline, refresh.clone())));
            },
        )
    };
    *load_slot.borrow_mut() = Rc::downgrade(&load);
    page.append(&load.root);
    // The hooks keep the loader and the loaded page as long as the page.
    {
        let (load, current) = (load.clone(), current.clone());
        page.hooks().refresh = Some(Rc::new(move || {
            let _ = &current;
            load.reload();
        }));
    }
    page
}

/// `ContentUnavailableView("No versions", …)`.
fn no_versions() -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
    column.set_halign(gtk::Align::Center);
    column.set_margin_top(60);
    let mark = icons::icon(Icon::Warning, 28, tone::LABEL);
    mark.set_halign(gtk::Align::Center);
    column.append(&mark);
    let title = gtk::Label::new(Some("No versions"));
    title.add_css_class("dialog-title");
    column.append(&title);
    let text = page::paragraph("The source returned this identity without a version.", &["secondary"]);
    text.set_xalign(0.5);
    text.set_justify(gtk::Justification::Center);
    column.append(&text);
    column
}

/// The identity at one of its versions.
struct IdentityView {
    page: Weak<Page>,
    model: Rc<AppModel>,
    timeline: Rc<IdentityTimeline>,
    viewed: Cell<usize>,
    evidence: Rc<RefCell<EvidenceState>>,
    offers: RefCell<Option<IdentityOffers>>,
    activity: RefCell<Option<AddressOverview>>,
    /// Set when a later tip shows that the current version's block left the active chain.
    reorganized: Cell<bool>,
    show_all_history: Cell<bool>,
    refresh: Rc<dyn Fn()>,
    /// A redraw of the texts is waiting for the main loop.
    pending: Cell<bool>,
    last_tip: Cell<Option<u64>>,

    header: gtk::Box,
    strip: RefCell<Option<Rc<VersionStrip>>>,
    past_note: gtk::Box,
    control: gtk::Box,
    properties: gtk::Box,
    balances: RefCell<Option<Card>>,
    token_slot: gtk::Box,
    token: RefCell<Option<Rc<TokenControlCard>>>,
    evidence_card: RefCell<Option<Rc<EvidenceCard>>>,
    origin: RefCell<Option<Rc<SectionLoader>>>,
    payments: RefCell<Option<Rc<RegistrationPaymentsCard>>>,
    history: gtk::Box,
    json: gtk::Box,
    /// Whether the raw JSON is open. It stays open when another version shows.
    json_open: Rc<Cell<bool>>,
}

fn slot() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 0)
}

impl IdentityView {
    fn new(page: &Rc<Page>, holder: &gtk::Box, timeline: IdentityTimeline, refresh: Rc<dyn Fn()>) -> Rc<Self> {
        let model = page.model.clone();
        let latest = timeline.versions.len() - 1;
        let view = Rc::new(Self {
            page: Rc::downgrade(page),
            model: model.clone(),
            timeline: Rc::new(timeline),
            viewed: Cell::new(latest),
            evidence: Rc::new(RefCell::new(EvidenceState::Loading)),
            offers: RefCell::new(None),
            activity: RefCell::new(None),
            reorganized: Cell::new(false),
            show_all_history: Cell::new(false),
            refresh,
            pending: Cell::new(false),
            last_tip: Cell::new(model.state().status.as_ref().map(|s| s.blocks)),
            header: gtk::Box::new(gtk::Orientation::Vertical, 8),
            strip: RefCell::new(None),
            past_note: slot(),
            control: slot(),
            properties: slot(),
            balances: RefCell::new(None),
            token_slot: slot(),
            token: RefCell::new(None),
            evidence_card: RefCell::new(None),
            origin: RefCell::new(None),
            payments: RefCell::new(None),
            history: slot(),
            json: slot(),
            json_open: Rc::new(Cell::new(false)),
        });
        let column = gtk::Box::new(gtk::Orientation::Vertical, 20);
        holder.append(&column);
        let timeline = view.timeline.clone();

        column.append(&view.header);
        column.append(&page::provenance_note(&timeline.provenance));
        let select: Rc<dyn Fn(usize)> = {
            let weak = Rc::downgrade(&view);
            Rc::new(move |index| {
                if let Some(view) = weak.upgrade() {
                    view.select(index);
                }
            })
        };
        let versions = Rc::new(timeline.versions.clone());
        let strip = VersionStrip::new(&model, versions, timeline.tip_height, latest, select);
        column.append(&strip.root);
        view.strip.replace(Some(strip));
        column.append(&view.past_note);
        column.append(&view.control);
        column.append(&view.properties);

        let balances = page.card("Balances and activity", Some("now"));
        column.append(&balances.root);
        view.balances.replace(Some(balances));
        column.append(&view.token_slot);

        let changed: Rc<dyn Fn()> = {
            let weak = Rc::downgrade(&view);
            Rc::new(move || {
                if let Some(view) = weak.upgrade() {
                    view.evidence_changed();
                }
            })
        };
        let evidence_card =
            EvidenceCard::new(page, &timeline.identity_address, view.evidence.clone(), timeline.tip_height, changed);
        column.append(&evidence_card.card.root);
        view.evidence_card.replace(Some(evidence_card));

        if let Some(first) = timeline.versions.first()
            && let Some(txid) = &first.txid
        {
            let (card, loader) = origin::origin_card(page, txid, first.height, first.time, Some(&timeline.identity_address));
            column.append(&card);
            view.origin.replace(Some(loader));
            let payments = RegistrationPaymentsCard::new(page, txid);
            column.append(&payments.card.root);
            view.payments.replace(Some(payments));
        }

        column.append(&view.history);
        let transactions = page.card("Transactions", None);
        transactions.append(&address_activity::ActivityList::new(page, &timeline.identity_address, None));
        column.append(&transactions.root);
        column.append(&view.json);

        // The menu command and the key v run the historical reverse request.
        {
            let weak = Rc::downgrade(&view);
            page.hooks().investigations = Investigations {
                shared_authorities: Some(Rc::new(move || {
                    if let Some(view) = weak.upgrade()
                        && let Some(card) = view.evidence_card.borrow().as_ref()
                    {
                        card.lookup.run();
                    }
                })),
                ..Default::default()
            };
        }

        view.render_version(true);
        view.render_balances();
        view.watch();
        view.resolve_all_names();
        view.load();
        view.watch_offers();
        view
    }

    fn state(&self) -> &IdentityState {
        &self.timeline.versions[self.viewed.get()]
    }

    fn latest_index(&self) -> usize {
        self.timeline.versions.len() - 1
    }

    fn is_latest(&self) -> bool {
        self.viewed.get() == self.latest_index()
    }

    fn stale(&self) -> bool {
        self.timeline.stale || self.reorganized.get()
    }

    /// As the terminal: offer badges show only while the node's tip is healthy.
    fn tip_healthy(&self) -> bool {
        let state = self.model.state();
        state.status.is_some() && state.status_error.is_none()
    }

    /// Views another version. The parts are built again after the click that asked for it
    /// has ended, because the key or link that was clicked is one of the parts.
    fn select(self: &Rc<Self>, index: usize) {
        if index >= self.timeline.versions.len() {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(view) = weak.upgrade() else { return };
            let changed = index != view.viewed.get();
            view.viewed.set(index);
            view.render_kept(changed);
        });
    }

    /// Builds the parts of the viewed version again and keeps the keyboard cursor on the
    /// same link or row.
    fn render_kept(self: &Rc<Self>, version_changed: bool) {
        let Some(page) = self.page.upgrade() else { return };
        page.cursor.keeping_selection(|| {
            self.render_version(version_changed);
            self.render_balances();
        });
    }

    // MARK: Watching and loading

    fn watch(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &self.header, move |_| {
            if let Some(view) = weak.upgrade() {
                view.schedule_texts();
            }
        });
        // A new tip: the stale check, and the header (offers show only on a healthy tip).
        let weak = Rc::downgrade(self);
        self.model.watch(&[Topic::Status], &self.header, move |model| {
            let Some(view) = weak.upgrade() else { return };
            view.render_header();
            let tip = model.state().status.as_ref().map(|s| s.blocks);
            if tip != view.last_tip.get() {
                view.last_tip.set(tip);
                view.recheck_stale();
            }
        });
    }

    /// Redraws the texts that hold names, heights and dates once, after the current burst
    /// of changes.
    fn schedule_texts(self: &Rc<Self>) {
        if self.pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(view) = weak.upgrade() {
                view.pending.set(false);
                view.render_kept(false);
            }
        });
    }

    fn resolve_all_names(&self) {
        let mut ids = vec![self.timeline.identity_address.clone()];
        for version in &self.timeline.versions {
            for id in [&version.parent, &version.system_id, &version.revocation_authority, &version.recovery_authority]
                .into_iter()
                .flatten()
            {
                ids.push(id.clone());
            }
        }
        self.model.resolve_names(ids);
    }

    fn load(self: &Rc<Self>) {
        let Some(explorer) = self.model.explorer() else { return };
        let address = self.timeline.identity_address.clone();
        *self.evidence.borrow_mut() = EvidenceState::Loading;
        self.evidence_changed();
        let generation = self.model.generation();
        {
            let (explorer, address, weak) = (explorer.clone(), address.clone(), Rc::downgrade(self));
            glib::spawn_future_local(async move {
                let overview = bridge::run(async move { explorer.address_overview(address).await }).await.ok();
                let Some(view) = weak.upgrade() else { return };
                if view.model.generation() != generation {
                    return;
                }
                if let Some(overview) = &overview {
                    view.model.resolve_names(overview.balances.iter().map(|b| b.currency_id.clone()));
                }
                view.activity.replace(overview);
                view.render_balances();
            });
        }
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.identity_evidence(address).await }).await;
            let Some(view) = weak.upgrade() else { return };
            if view.model.generation() != generation {
                return;
            }
            *view.evidence.borrow_mut() = match result {
                Ok(report) => EvidenceState::Loaded(report),
                Err(error) => EvidenceState::Failed(bridge::describe(&error)),
            };
            view.evidence_changed();
        });
    }

    /// The terminal rechecks offers every 30 seconds while the identity is open.
    fn watch_offers(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            loop {
                let Some(view) = weak.upgrade() else { return };
                if let Some(explorer) = view.model.explorer() {
                    let address = view.timeline.identity_address.clone();
                    drop(view);
                    let offers = bridge::run(async move { explorer.identity_offers(address).await }).await.ok();
                    let Some(view) = weak.upgrade() else { return };
                    view.offers.replace(offers);
                    view.render_header();
                } else {
                    drop(view);
                }
                glib::timeout_future(Duration::from_secs(30)).await;
            }
        });
    }

    /// When the tip changes, checks that the current version's block is still on the
    /// active chain. If it is not, the page says so and loads the identity again.
    fn recheck_stale(self: &Rc<Self>) {
        if self.timeline.provenance.retained {
            return;
        }
        let Some(explorer) = self.model.explorer() else { return };
        let Some(hash) = self.timeline.versions.last().and_then(|v| v.block_hash.clone()) else { return };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let checked = bridge::run(async move { explorer.blocks_active(vec![hash]).await }).await;
            let Some(view) = weak.upgrade() else { return };
            if checked.first().and_then(|c| c.active) != Some(false) {
                return;
            }
            view.reorganized.set(true);
            view.render_header();
            view.render_properties();
            let refresh = view.refresh.clone();
            drop(view);
            glib::timeout_future(Duration::from_secs(2)).await;
            if weak.upgrade().is_some() {
                refresh();
            }
        });
    }

    fn evidence_changed(&self) {
        if let Some(card) = self.evidence_card.borrow().as_ref() {
            card.render();
        }
        self.render_properties();
    }

    // MARK: Rendering

    /// Everything that depends on the viewed version. `version_changed` also rebuilds the
    /// raw JSON, which keeps its state while only names or dates change.
    fn render_version(self: &Rc<Self>, version_changed: bool) {
        let Some(page) = self.page.upgrade() else { return };
        self.render_header();
        if let Some(strip) = self.strip.borrow().as_ref() {
            strip.set_selection(self.viewed.get());
        }
        clear(&self.past_note);
        if !self.is_latest() {
            let state = self.state();
            self.past_note.append(&page::note(
                &format!(
                    "This page shows the identity as it was after the update at {}. Balances, evidence and transactions are for the identity now.",
                    height_and_time(state.height, state.time, &self.model)
                ),
                true,
            ));
        }
        self.past_note.set_visible(!self.is_latest());
        clear(&self.control);
        self.control.append(&control::control_card(&page, self.state()));
        self.render_properties();
        self.render_token();
        self.render_history();
        if version_changed {
            clear(&self.json);
            self.json.append(&json_section_keeping(
                &page,
                &self.state().json,
                &format!("id-{}.json", self.timeline.identity_address),
                self.json_open.clone(),
            ));
        }
        // Previous and next version: n and p, Ctrl+Alt+[ and ], and the Go menu.
        let viewed = self.viewed.get();
        let step = |delta: isize| -> page::Action {
            let weak = Rc::downgrade(self);
            Rc::new(move || {
                if let Some(view) = weak.upgrade() {
                    let target = view.viewed.get() as isize + delta;
                    if target >= 0 {
                        view.select(target as usize);
                    }
                }
            })
        };
        page.hooks().adjacent = Some(Adjacent {
            noun: "Version".into(),
            previous: (viewed > 0).then(|| step(-1)),
            next: (!self.is_latest()).then(|| step(1)),
        });
    }

    fn render_header(self: &Rc<Self>) {
        clear(&self.header);
        let state = self.state().clone();
        self.header.append(&page::page_header(&self.timeline.name, Some(&self.timeline.identity_address), &[]));

        let plates = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        // The status: a shape, a colour and a word.
        let stale = self.stale() && self.is_latest();
        let word = if stale { "Stale after reorg".to_string() } else { capitalized(&state.status) };
        let good = !stale && state.status.to_lowercase() == "active";
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let mark = if good {
            icons::icon(Icon::CheckCircle, 12, tone::GOOD)
        } else {
            icons::icon(Icon::Warning, 12, tone::ALERT)
        };
        mark.set_valign(gtk::Align::Center);
        chip.append(&mark);
        chip.append(&display::plate(&word, if good { PlateFill::Good } else { PlateFill::Alert }));
        plates.append(&chip);
        if state.defines_currency {
            let weak = Rc::downgrade(&self.model);
            let address = self.timeline.identity_address.clone();
            let currency = link::link("Currency defined", false, move || {
                if let Some(model) = weak.upgrade() {
                    model.open(Route::Currency { name: address.clone() });
                }
            });
            if let Some(label) = link::label_of(&currency) {
                theme::set_role(&label, font::Role { weight: 500, ..font::TEXT_CAPTION });
                currency.set_child(None::<&gtk::Widget>);
                let content = gtk::Box::new(gtk::Orientation::Horizontal, 5);
                content.append(&icons::text_icon(Icon::HexagonGrid, 12));
                content.append(&label);
                currency.set_child(Some(&content));
            }
            currency.set_valign(gtk::Align::Center);
            currency.set_tooltip_text(Some("Open the currency that this identity defines"));
            plates.append(&currency);
        }
        if state.tokenized_control {
            plates.append(&display::plate("Tokenized control", PlateFill::None));
        } else if state.flags == 0 {
            plates.append(&theme::text_label(font::TEXT_CAPTION, "No flags set", &["secondary"]));
        }
        if let Some(offers) = self.offers.borrow().as_ref()
            && self.tip_healthy()
        {
            if offers.buy {
                plates.append(&display::plate("Buy offers", PlateFill::Staked));
            }
            if offers.sell {
                plates.append(&display::plate("Sell offers", PlateFill::Staked));
            }
        }
        self.header.append(&plates);
        self.header.append(&self.version_stepper());
    }

    fn version_stepper(self: &Rc<Self>) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let viewed = self.viewed.get();
        let count = self.timeline.versions.len();
        let key = |title: &str, mark: Icon, target: usize, help: &str| {
            let weak = Rc::downgrade(self);
            let button = lamp::button_with(title, Some(mark), move || {
                if let Some(view) = weak.upgrade() {
                    view.select(target);
                }
            });
            button.set_tooltip_text(Some(help));
            button
        };
        let previous = key("Previous version", Icon::ChevronLeft, viewed.saturating_sub(1), "Previous version (Ctrl+Alt+[)");
        previous.set_sensitive(viewed > 0);
        row.append(&previous);
        let version = page::text(&format!("Version {} of {count}", viewed + 1), &["semibold", "tabular", "value"]);
        version.set_valign(gtk::Align::Center);
        row.append(&version);
        let when = page::text(
            if self.is_latest() { "current" } else { "past" },
            &[if self.is_latest() { "good" } else { "mined" }],
        );
        when.set_valign(gtk::Align::Center);
        row.append(&when);
        let next = key("Next version", Icon::ChevronRight, (viewed + 1).min(self.latest_index()), "Next version (Ctrl+Alt+])");
        next.set_sensitive(!self.is_latest());
        row.append(&next);
        if !self.is_latest() {
            let weak = Rc::downgrade(self);
            let current = link::link("Show the current version", false, move || {
                if let Some(view) = weak.upgrade() {
                    let latest = view.latest_index();
                    view.select(latest);
                }
            });
            current.set_valign(gtk::Align::Center);
            row.append(&current);
        }
        row
    }

    fn render_properties(&self) {
        let Some(page) = self.page.upgrade() else { return };
        clear(&self.properties);
        let state = self.state();
        let model = &self.model;
        let card = page.card(&format!("Identity at version {}", self.viewed.get() + 1), None);
        card.append(&support::value_row("Name", &state.name, false));
        if let Some(fqn) = &self.timeline.fully_qualified_name {
            card.append(&support::value_row("Fully qualified", fqn, false));
        }
        card.append(&support::value_row(
            "Version",
            &state.format_version.map(|v| v.to_string()).unwrap_or_else(|| "unknown".into()),
            false,
        ));
        let stale = self.stale() && self.is_latest();
        let status = if stale {
            "This page is out of date after a reorganization. The page is refreshing.".to_string()
        } else {
            state.status.clone()
        };
        card.append(&support::value_row("Status", &status, stale || state.status.to_lowercase() != "active"));
        card.append(&support::value_row("Timelock", &timelock_text(model, &state.timelock), false));
        if let Some(parent) = &state.parent {
            card.append(&support::id_row(&page, "Parent", parent, Route::Identity { name: parent.clone() }, None));
        }
        if let Some(system) = &state.system_id {
            card.append(&support::id_row(&page, "System", system, Route::Currency { name: system.clone() }, None));
        }
        for (label, id, is_self) in [
            ("Revocation", &state.revocation_authority, state.revocation_is_self),
            ("Recovery", &state.recovery_authority, state.recovery_is_self),
        ] {
            if is_self {
                card.append(&support::value_row(label, "self", false));
            } else if let Some(id) = id {
                card.append(&support::id_row(&page, label, id, Route::Identity { name: id.clone() }, None));
            } else {
                card.append(&support::value_row(label, "unknown", false));
            }
        }
        if let Some(private) = &state.private_address {
            card.append(&support::value_row("Private address", private, false));
        }
        let imported = matches!(&*self.evidence.borrow(), EvidenceState::Loaded(report) if report.imported);
        card.append(&support::value_row(if imported { "Imported" } else { "Registered" }, &self.registered_text(), false));
        card.append(&support::value_row("Updated", &self.updated_text(), false));
        card.append(&support::value_row("Viewing update", &height_and_time(state.height, state.time, model), false));
        if state.on_active_chain == Some(false) {
            card.append(&page::note(
                "The block of this update is not on the active chain. A reorganization removed it.",
                true,
            ));
        }
        card.append(&support::value_row(
            "Versions",
            &format!(
                "{}{}",
                self.timeline.versions.len(),
                if self.timeline.history_complete { "" } else { " (the history is not complete)" }
            ),
            false,
        ));
        if let Some(note) = &self.timeline.history_note {
            card.append(&page::note(note, true));
        }
        if let Some(txid) = &state.txid {
            let note = state.output.map(|o| format!("output {o}"));
            card.append(&support::id_row(
                &page,
                "Update transaction",
                txid,
                Route::Transaction { txid: txid.clone() },
                note.as_deref(),
            ));
        }
        self.properties.append(&card.root);
    }

    fn registered_text(&self) -> String {
        let Some(first) = self.timeline.versions.first() else { return "unknown".into() };
        if !self.timeline.history_complete {
            return "History not complete".into();
        }
        height_and_time(first.height, first.time, &self.model)
    }

    fn updated_text(&self) -> String {
        let (Some(first), Some(last)) = (self.timeline.versions.first(), self.timeline.versions.last()) else {
            return "unknown".into();
        };
        if !self.timeline.history_complete {
            return "History not complete".into();
        }
        if last.height == first.height { "never".into() } else { height_and_time(last.height, last.time, &self.model) }
    }

    fn render_balances(&self) {
        let balances = self.balances.borrow();
        let Some(card) = balances.as_ref() else { return };
        clear(&card.content);
        let activity = self.activity.borrow();
        let Some(activity) = activity.as_ref() else {
            card.append(&small_progress("Loading balances…"));
            return;
        };
        if let Some(error) = &activity.balance_error {
            card.append(&page::note(&format!("The balances did not load: {error}"), true));
        } else if activity.balances.is_empty() {
            card.append(&page::text("No balances.", &["secondary"]));
        } else {
            card.append(&support::balance_list(&self.model, &activity.balances));
        }
        card.append(&page::paragraph(&activity_summary(activity), &["secondary"]));
    }

    fn render_token(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else { return };
        let tokenized = self.state().tokenized_control;
        if tokenized && self.token.borrow().is_none() {
            let card = TokenControlCard::new(&page, &self.timeline.identity_address, !self.is_latest());
            self.token_slot.append(&card.card.root);
            self.token.replace(Some(card));
        }
        if let Some(card) = self.token.borrow().as_ref() {
            card.set_viewing_past(!self.is_latest());
        }
        self.token_slot.set_visible(tokenized);
    }

    fn render_history(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else { return };
        clear(&self.history);
        let select: Rc<dyn Fn(usize)> = {
            let weak = Rc::downgrade(self);
            Rc::new(move |index| {
                if let Some(view) = weak.upgrade() {
                    view.select(index);
                }
            })
        };
        let toggle: Rc<dyn Fn()> = {
            let weak = Rc::downgrade(self);
            Rc::new(move || {
                // After the click: the link that was clicked is part of the card.
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    if let Some(view) = weak.upgrade() {
                        view.show_all_history.set(!view.show_all_history.get());
                        let Some(page) = view.page.upgrade() else { return };
                        page.cursor.keeping_selection(|| view.render_history());
                    }
                });
            })
        };
        self.history.append(&control::history_card(
            &page,
            &self.timeline,
            self.viewed.get(),
            self.show_all_history.get(),
            select,
            toggle,
        ));
    }
}

/// `String.capitalized`: the first letter of each word in capitals, the rest small.
fn capitalized(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn timelock_text(model: &Rc<AppModel>, timelock: &Timelock) -> String {
    match timelock {
        Timelock::Unlocked => "Unlocked".into(),
        Timelock::Delay { blocks } => format!(
            "Locked. An unlock takes effect {} block{} after it is requested.",
            format::integer(*blocks),
            if *blocks == 1 { "" } else { "s" }
        ),
        Timelock::UnlocksAt { height } => format!("Locked until {}", model.height_text(*height, None)),
    }
}

fn activity_summary(overview: &AddressOverview) -> String {
    if let Some(error) = &overview.history_error {
        return format!("The activity did not load: {error}");
    }
    let mut text = format!(
        "{} transaction{}",
        format::integer(overview.transaction_count),
        if overview.transaction_count == 1 { "" } else { "s" }
    );
    if let Some(first) = overview.first_time {
        text.push_str(&format!(", first {}", format::date(first)));
    }
    if let Some(latest) = overview.latest_time {
        text.push_str(&format!(", latest {}", format::date(latest)));
    }
    text + "."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capitalized_words() {
        assert_eq!(capitalized("active"), "Active");
        assert_eq!(capitalized("REVOKED"), "Revoked");
        assert_eq!(capitalized("two words"), "Two Words");
    }
}
