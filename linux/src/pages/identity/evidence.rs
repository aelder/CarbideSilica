//! Carbide's registration and association evidence: the registration transaction and name
//! commitment, funding candidates, the referral chain, shared addresses and authorities,
//! and import provenance. Each link keeps its source outpoint or request. Port of
//! `IdentityEvidence.swift`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api};
use crate::bridge::api::{Confidence, EvidenceLinkRow, EvidenceTargetKind, IdentityEvidenceReport, Route};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, Card, Page};
use crate::shared::confidence::confidence_mark;
use crate::shared::identity_support::{self as support, small_progress};
use crate::theme::tone;
use crate::widgets::display;
use crate::widgets::{lamp, link, loader};

use super::marks::{self, Mark};

/// The evidence for the current version, shared by the header and the evidence section.
#[derive(Clone)]
pub enum EvidenceState {
    Loading,
    Failed(String),
    Loaded(IdentityEvidenceReport),
}

const EARLIER_AUTHORITIES: &str = "Authorities in earlier versions";

// MARK: - Pure parts

/// The groups of the links, in Carbide's order, by the index of each link. A link goes to
/// the first group that takes it; the rest are "Other".
pub fn groups(labels: &[&str]) -> Vec<(&'static str, Vec<usize>)> {
    type Matcher = fn(&str) -> bool;
    let order: [(&str, Matcher); 6] = [
        ("Registration", |label| {
            [
                "Registration transaction",
                "Name commitment",
                "Parent",
                "Name ID",
                "Reservation output destination",
                "Commitment control destination",
                "Identity import transaction",
            ]
            .contains(&label)
        }),
        ("Funding candidates", |label| label.ends_with("funding candidate")),
        ("Referral chain", |label| label == "Referral" || label == "Referral ancestor"),
        (EARLIER_AUTHORITIES, |label| label.starts_with("Historical recovery") || label.starts_with("Historical revocation")),
        ("Shared now", |label| label.starts_with("Shared ")),
        ("Shared in a past window", |label| label.starts_with("Historical shared")),
    ];
    let mut used = vec![false; labels.len()];
    let mut result = Vec::new();
    for (title, matches) in order {
        let mut links = Vec::new();
        for (index, label) in labels.iter().enumerate() {
            if !used[index] && matches(label) {
                used[index] = true;
                links.push(index);
            }
        }
        if !links.is_empty() {
            result.push((title, links));
        }
    }
    let rest: Vec<usize> = (0..labels.len()).filter(|i| !used[*i]).collect();
    if !rest.is_empty() {
        result.push(("Other", rest));
    }
    result
}

/// The referrer, then each ancestor in order of depth.
pub fn referral_chain(links: &[EvidenceLinkRow]) -> Vec<String> {
    let mut chain = Vec::new();
    if let Some(direct) = links.iter().find(|link| link.label == "Referral") {
        chain.push(direct.target.clone());
    }
    chain.extend(links.iter().filter(|link| link.label == "Referral ancestor").map(|link| link.target.clone()));
    chain
}

fn is_identity_id(word: &str) -> bool {
    word.starts_with('i') && word.chars().count() == 34
}

/// One word of an evidence source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Outpoint { txid: String, rest: String },
}

/// The words of a source, with outpoints and IDs made short. At most six, so the line
/// stays short. `display` names an identity ID as "Show Names for IDs" says.
pub fn pieces(source: &str, shows_names: bool, display: &dyn Fn(&str) -> String) -> Vec<Piece> {
    let words: Vec<&str> = source.split(' ').filter(|w| !w.is_empty()).collect();
    let mut result = Vec::new();
    for word in words.iter().take(6) {
        let trimmed = word.trim_matches(|c| c == ';' || c == ',');
        let mut parts = trimmed.splitn(2, ':');
        let first = parts.next().unwrap_or("");
        let rest = parts.next();
        if first.len() == 64 && first.chars().all(|c| c.is_ascii_hexdigit()) {
            result.push(Piece::Outpoint {
                txid: first.to_string(),
                rest: rest.map(|r| format!(":{r}")).unwrap_or_default(),
            });
        } else if is_identity_id(trimmed) {
            result.push(Piece::Text(if shows_names { display(trimmed) } else { format::short(trimmed, 8, 6) }));
        } else if let Some(equals) = trimmed.find('=')
            && is_identity_id(&trimmed[equals + 1..])
        {
            result.push(Piece::Text(format!("{}={}", &trimmed[..equals], display(&trimmed[equals + 1..]))));
        } else {
            result.push(Piece::Text(if trimmed.chars().count() > 28 {
                format::short(trimmed, 14, 6)
            } else {
                trimmed.to_string()
            }));
        }
    }
    if words.len() > 6 {
        result.push(Piece::Text("…".into()));
    }
    result
}

/// The sentence of a historical request's result.
pub fn summary(found: u32, window: &str) -> String {
    match found {
        0 => format!("{window}: no new links. Other windows can give different results."),
        1 => format!("{window}: 1 new link. It is in the list above."),
        n => format!("{window}: {n} new links. They are in the list above."),
    }
}

// MARK: - The card

pub struct EvidenceCard {
    pub card: Card,
    body: gtk::Box,
    page: std::rc::Weak<Page>,
    name: String,
    state: Rc<RefCell<EvidenceState>>,
    /// Called after the state changes: the page redraws this card and "Registered".
    changed: Rc<dyn Fn()>,
    pub lookup: Rc<HistoricalLookup>,
    /// Whether "Authorities in earlier versions" is open, kept when the card is redrawn.
    authorities_open: Rc<Cell<bool>>,
}

impl EvidenceCard {
    pub fn new(page: &Rc<Page>, name: &str, state: Rc<RefCell<EvidenceState>>, tip: Option<u64>, changed: Rc<dyn Fn()>) -> Rc<Self> {
        let card = page.card("Registration and associations", None);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        card.append(&body);
        card.append(&page::divider());
        let this = Rc::new_cyclic(|weak: &std::rc::Weak<EvidenceCard>| {
            let weak = weak.clone();
            let set: Rc<dyn Fn(EvidenceState)> = Rc::new(move |state| {
                if let Some(card) = weak.upgrade() {
                    card.set(state);
                }
            });
            let lookup = HistoricalLookup::new(page, name, tip, set);
            card.append(&lookup.root);
            Self {
                card,
                body,
                page: Rc::downgrade(page),
                name: name.to_string(),
                state,
                changed,
                lookup,
                authorities_open: Rc::new(Cell::new(false)),
            }
        });
        this.render();
        let weak = Rc::downgrade(&this);
        page.model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &this.body, move |_| {
            if let Some(card) = weak.upgrade() {
                card.render();
            }
        });
        this
    }

    /// Sets the state. `changed` redraws this card and the page's "Registered".
    pub fn set(&self, state: EvidenceState) {
        *self.state.borrow_mut() = state;
        (self.changed)();
    }

    pub fn render(&self) {
        let Some(page) = self.page.upgrade() else { return };
        clear(&self.body);
        let state = self.state.borrow().clone();
        match &state {
            EvidenceState::Loaded(report) => self.card.set_trailing(Some(&format!("{} evidence links", report.links.len()))),
            _ => self.card.set_trailing(None),
        }
        match state {
            EvidenceState::Loading => {
                self.body.append(&small_progress("Loading the registration and association evidence…"));
            }
            EvidenceState::Failed(message) => {
                self.body.append(&page::note(&format!("Evidence unavailable: {message}"), true));
                // Try Again loads the evidence again into this card.
                let (weak_page, name) = (Rc::downgrade(&page), self.name.clone());
                let (state, changed) = (self.state.clone(), self.changed.clone());
                self.body.append(&lamp::button("Try Again", move || {
                    let Some(page) = weak_page.upgrade() else { return };
                    reload(&page.model, &name, state.clone(), changed.clone());
                }));
            }
            EvidenceState::Loaded(report) => {
                let ids: Vec<String> =
                    report.links.iter().filter(|l| l.kind == EvidenceTargetKind::Identity).map(|l| l.target.clone()).collect();
                page.model.resolve_names(ids);
                self.body.append(&evidence_body(&page, &self.name, &report, &self.authorities_open));
            }
        }
    }
}

/// Loads the evidence again after Try Again, and redraws the card through the page's
/// watcher of the state.
fn reload(model: &Rc<AppModel>, name: &str, state: Rc<RefCell<EvidenceState>>, changed: Rc<dyn Fn()>) {
    *state.borrow_mut() = EvidenceState::Loading;
    changed();
    let explorer = match model.require_explorer() {
        Ok(explorer) => explorer,
        Err(message) => {
            *state.borrow_mut() = EvidenceState::Failed(message);
            changed();
            return;
        }
    };
    let name = name.to_string();
    glib::spawn_future_local(async move {
        let result = bridge::run(async move { explorer.identity_evidence(name).await }).await;
        *state.borrow_mut() = match result {
            Ok(report) => EvidenceState::Loaded(report),
            Err(error) => EvidenceState::Failed(bridge::describe(&error)),
        };
        changed();
    });
}

fn evidence_body(page: &Rc<Page>, name: &str, report: &IdentityEvidenceReport, authorities_open: &Rc<Cell<bool>>) -> gtk::Box {
    let model = page.model.clone();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 14);
    if let Some(registration) = &report.registration {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = display::name_text(if report.imported { "Imported" } else { "Registration" });
        title.set_valign(gtk::Align::Baseline);
        row.append(&title);
        let text = page::text(registration, &["mono"]);
        text.set_selectable(true);
        text.set_valign(gtk::Align::Baseline);
        row.append(&text);
        if let Some(link) = report
            .links
            .iter()
            .find(|l| l.label == "Registration transaction" || l.label == "Identity import transaction")
        {
            row.append(&page::route_link(page, &link.target, Route::Transaction { txid: link.target.clone() }, true));
        }
        column.append(&row);
    }
    let chain = referral_chain(&report.links);
    if !chain.is_empty() {
        column.append(&referral_view(page, name, &chain));
    }
    if report.links.is_empty() {
        column.append(&page::text("No additional associations to display", &["secondary"]));
    }
    let labels: Vec<&str> = report.links.iter().map(|l| l.label.as_str()).collect();
    for (title, indices) in groups(&labels) {
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        for (position, index) in indices.iter().enumerate() {
            if position > 0 {
                rows.append(&page::divider());
            }
            rows.append(&evidence_row(page, &report.links[*index]));
        }
        if title == EARLIER_AUTHORITIES {
            // The lanes at the top of the page show who held each power at each version.
            // The rows stay as evidence, closed at the start.
            let open = authorities_open.clone();
            let disclosure =
                lamp::Disclosure::new(&format!("{title} ({} rows)", indices.len()), open.get(), move |now| open.set(now));
            let inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let note = page::text(
                "The lanes at the top of the page show the same authorities for each version.",
                &["callout", "secondary"],
            );
            note.set_margin_top(4);
            note.set_margin_bottom(4);
            inner.append(&note);
            inner.append(&rows);
            disclosure.content.append(&inner);
            column.append(&disclosure.root);
        } else {
            let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
            let heading = display::name_text(title);
            heading.set_xalign(0.0);
            heading.set_margin_bottom(4);
            group.append(&heading);
            group.append(&rows);
            column.append(&group);
        }
    }
    // The notes.
    let notes = gtk::Box::new(gtk::Orientation::Vertical, 4);
    for note in &report.notes {
        notes.append(&page::paragraph(note, &["callout", "secondary"]));
    }
    notes.append(&page::paragraph(
        "An association is evidence, not ownership. Private funding can be invisible.",
        &["callout", "secondary"],
    ));
    if !report.history_complete {
        notes.append(&page::note("The registration evidence needs the complete identity history.", true));
    }
    column.append(&notes);
    // Where the links are stored.
    if report.retained_only {
        column.append(&page::note("The live refresh failed. These links come from stored evidence.", true));
    }
    if report.saved {
        let mut text = format!("Stored with the version at {}", support::height_text(&model, report.version_height, None));
        if let Some(block) = &report.block_hash {
            text.push_str(&format!(", anchored to block {}", format::short(block, 10, 8)));
        }
        if let Some(source) = &report.source_id {
            text.push_str(&format!(" from source {}", format::short(source, 16, 6)));
        }
        text.push_str(". A reorganization that removes that block makes the stored links invalid.");
        let label = page::paragraph(&text, &["secondary"]);
        super::set_px(&label, 10.0, 400);
        label.set_selectable(true);
        column.append(&label);
    }
    column
}

/// The identity and who referred it, back as far as Carbide follows: four ancestors.
fn referral_view(page: &Rc<Page>, identity: &str, chain: &[String]) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_top(2);
    row.set_margin_bottom(2);
    let chip = |id: &str| {
        let link = support::id_link(page, id, Route::Identity { name: id.to_string() }, true);
        let holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        holder.append(&link);
        let chip = marks::outlined(&holder, tone::AXIS, 10, 5);
        chip.set_valign(gtk::Align::Center);
        chip
    };
    row.append(&chip(identity));
    for (depth, referrer) in chain.iter().enumerate() {
        let arrow = gtk::Box::new(gtk::Orientation::Vertical, 0);
        arrow.set_valign(gtk::Align::Center);
        let mark = marks::mark(Mark::ArrowLeft, 13, tone::AXIS);
        mark.set_halign(gtk::Align::Center);
        arrow.append(&mark);
        arrow.append(&display::name_text(if depth == 0 { "referred by" } else { "who was referred by" }));
        row.append(&arrow);
        row.append(&chip(referrer));
    }
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::External, gtk::PolicyType::Never);
    scroller.set_child(Some(&row));
    scroller.set_propagate_natural_height(true);
    column.append(&scroller);
    let caption = page::paragraph(
        "Each box is an identity. An arrow points from an identity to the identity named as its referrer in its registration.",
        &["secondary"],
    );
    super::set_px(&caption, 10.0, 400);
    column.append(&caption);
    column
}

fn evidence_row(page: &Rc<Page>, link: &EvidenceLinkRow) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.set_margin_top(6);
    row.set_margin_bottom(6);
    let mark = confidence_mark(link.confidence, Some(if link.confidence == Confidence::Candidate { "Candidate" } else { "Observed" }));
    mark.set_size_request(110, -1);
    mark.set_valign(gtk::Align::Start);
    mark.set_margin_top(1);
    row.append(&mark);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
    column.set_hexpand(true);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = page::text(&link.label, &[]);
    label.set_valign(gtk::Align::Baseline);
    heading.append(&label);
    let route = match link.kind {
        EvidenceTargetKind::Identity => Route::Identity { name: link.target.clone() },
        EvidenceTargetKind::Address => Route::Address { address: link.target.clone() },
        EvidenceTargetKind::Transaction => Route::Transaction { txid: link.target.clone() },
    };
    heading.append(&support::id_link(page, &link.target, route, true));
    column.append(&heading);
    if !link.detail.is_empty() {
        let detail = page::paragraph(&link.detail, &["callout", "secondary"]);
        detail.set_selectable(true);
        column.append(&detail);
    }
    column.append(&evidence_source(page, &link.source));
    row.append(&column);
    row
}

/// An evidence source, such as "i5w5…ZcV e8e77a…:4": the system by name, a transaction
/// outpoint as a short link, and long IDs shortened. The full text is in the tooltip and
/// the copy key.
fn evidence_source(page: &Rc<Page>, source: &str) -> gtk::Box {
    let model = page.model.clone();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let name = display::name_text("Source");
    name.set_valign(gtk::Align::Baseline);
    row.append(&name);
    let display = |id: &str| support::display_id(&model, id);
    for piece in pieces(source, crate::display::shows_names(), &display) {
        match piece {
            Piece::Text(text) => {
                let label = super::px_label(&text, 10.0, 400, &["secondary"]);
                label.set_single_line_mode(true);
                label.set_valign(gtk::Align::Baseline);
                row.append(&label);
            }
            Piece::Outpoint { txid, rest } => {
                let weak = Rc::downgrade(&model);
                let open = txid.clone();
                let button = link::link(&format!("{}{rest}", format::short(&txid, 6, 6)), false, move || {
                    if let Some(model) = weak.upgrade() {
                        model.open(Route::Transaction { txid: open.clone() });
                    }
                });
                if let Some(label) = link::label_of(&button) {
                    super::set_px(&label, 10.0, 400);
                }
                button.set_tooltip_text(Some(&txid));
                row.append(&button);
            }
        }
    }
    row.append(&page::copy_button(source));
    row.set_tooltip_text(Some(source));
    model.resolve_names(source.split(' ').filter(|w| is_identity_id(w)).map(str::to_string));
    row
}

// MARK: - Historical lookup

/// Carbide's `v`: a reverse request over the 1,000 blocks that end at a chosen height.
pub struct HistoricalLookup {
    pub root: gtk::Box,
    model: Rc<AppModel>,
    name: String,
    tip: Option<u64>,
    entry: gtk::Entry,
    request: gtk::Button,
    progress: gtk::Widget,
    failure: gtk::Box,
    result: gtk::Box,
    running: Cell<bool>,
    last: RefCell<Option<api::HistoricalEvidence>>,
    set_evidence: Rc<dyn Fn(EvidenceState)>,
}

impl HistoricalLookup {
    fn new(page: &Rc<Page>, name: &str, tip: Option<u64>, set_evidence: Rc<dyn Fn(EvidenceState)>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let title = display::name_text("Historical reverse request");
        title.set_xalign(0.0);
        root.append(&title);
        root.append(&page::paragraph(
            "Find identities that used a known authority or primary address of this identity in one window of 1,000 blocks. The request needs a source with the identity index. The app stores the links with this version and its block.",
            &["callout", "secondary"],
        ));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = display::name_text("Window ends at block");
        label.set_valign(gtk::Align::Center);
        row.append(&label);
        let entry = gtk::Entry::new();
        entry.add_css_class("display-field");
        entry.set_placeholder_text(Some(&tip.map(|t| t.to_string()).unwrap_or_else(|| "height".into())));
        entry.set_width_chars(1);
        entry.set_size_request(130, -1);
        entry.set_tooltip_text(Some("Block"));
        row.append(&entry);
        let progress: gtk::Widget = loader::small().upcast();
        let this = Rc::new_cyclic(|weak: &std::rc::Weak<HistoricalLookup>| {
            let weak = weak.clone();
            let request = lamp::button("Request", move || {
                if let Some(this) = weak.upgrade() {
                    this.run();
                }
            });
            Self {
                root,
                model: page.model.clone(),
                name: name.to_string(),
                tip,
                entry,
                request,
                progress,
                failure: gtk::Box::new(gtk::Orientation::Vertical, 0),
                result: gtk::Box::new(gtk::Orientation::Vertical, 8),
                running: Cell::new(false),
                last: RefCell::new(None),
                set_evidence,
            }
        });
        row.append(&this.request);
        this.progress.set_visible(false);
        this.progress.set_valign(gtk::Align::Center);
        row.append(&this.progress);
        this.root.append(&row);
        this.root.append(&this.failure);
        this.root.append(&this.result);
        let weak = Rc::downgrade(&this);
        this.entry.connect_activate(move |_| {
            if let Some(this) = weak.upgrade() {
                this.run();
            }
        });
        // The text field takes the keys while it has the focus.
        let weak = Rc::downgrade(&this);
        page.model.watch(&[Topic::Display, Topic::HeightDates], &this.root, move |_| {
            if let Some(this) = weak.upgrade() {
                this.show_result();
            }
        });
        this.failure.set_visible(false);
        this.result.set_visible(false);
        this
    }

    /// Runs the request for the typed height, or the tip.
    pub fn run(self: &Rc<Self>) {
        if self.running.get() {
            return;
        }
        let typed = self.entry.text().trim().to_string();
        let height = if typed.is_empty() { self.tip } else { typed.parse::<u64>().ok() };
        let Some(height) = height else {
            self.set_failure(Some("Enter a block height."));
            return;
        };
        let explorer = match self.model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                self.set_failure(Some(&message));
                return;
            }
        };
        self.set_running(true);
        self.set_failure(None);
        let name = self.name.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.identity_historical_evidence(name, height).await }).await;
            let Some(this) = weak.upgrade() else { return };
            this.set_running(false);
            match result {
                Ok(found) => {
                    let report = found.report.clone();
                    *this.last.borrow_mut() = Some(found);
                    this.show_result();
                    (this.set_evidence)(EvidenceState::Loaded(report));
                }
                Err(error) => this.set_failure(Some(&bridge::describe(&error))),
            }
        });
    }

    fn set_running(&self, running: bool) {
        self.running.set(running);
        self.request.set_sensitive(!running);
        self.progress.set_visible(running);
    }

    fn set_failure(&self, message: Option<&str>) {
        clear(&self.failure);
        if let Some(message) = message {
            self.failure.append(&page::note(message, true));
        }
        self.failure.set_visible(message.is_some());
    }

    fn show_result(&self) {
        clear(&self.result);
        let last = self.last.borrow();
        let Some(result) = last.as_ref() else {
            self.result.set_visible(false);
            return;
        };
        let window = format!(
            "From {} to {}",
            support::height_text(&self.model, result.from_height, None),
            support::height_text(&self.model, result.to_height, None)
        );
        self.result.append(&page::paragraph(&summary(result.found, &window), &[]));
        if result.index_missing {
            self.result.append(&page::note(
                "The source has no identity index. It cannot answer historical reverse requests. Start the node with -idindex=1, or enable a public API in Settings.",
                true,
            ));
        } else if result.unavailable > 0 {
            self.result.append(&page::note(
                &format!("{} of {} requests did not get an answer.", result.unavailable, result.requests),
                true,
            ));
        }
        self.result.set_visible(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_follow_carbides_order() {
        let labels = [
            "Shared primary address",
            "Registration transaction",
            "Registration funding candidate",
            "Referral",
            "Historical recovery authority",
            "Historical shared authority",
            "Something else",
            "Referral ancestor",
            "Name commitment",
        ];
        let found = groups(&labels);
        let titles: Vec<&str> = found.iter().map(|g| g.0).collect();
        assert_eq!(
            titles,
            vec![
                "Registration",
                "Funding candidates",
                "Referral chain",
                EARLIER_AUTHORITIES,
                "Shared now",
                "Shared in a past window",
                "Other"
            ]
        );
        assert_eq!(found[0].1, vec![1, 8]);
        assert_eq!(found[2].1, vec![3, 7]);
        assert_eq!(found[6].1, vec![6]);
    }

    #[test]
    fn source_pieces_shorten_and_link() {
        let txid = "e8e77a".to_string() + &"0".repeat(52) + "abcdef";
        let id = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        let source = format!("{id} {txid}:4; system={id}");
        let named = |_: &str| "VRSC".to_string();
        let found = pieces(&source, true, &named);
        assert_eq!(
            found,
            vec![
                Piece::Text("VRSC".into()),
                Piece::Outpoint { txid: txid.clone(), rest: ":4".into() },
                Piece::Text("system=VRSC".into()),
            ]
        );
        let unnamed = pieces(&source, false, &named);
        assert_eq!(unnamed[0], Piece::Text("i5w5MuNi…bB6MGV".into()));
        let long = pieces("a b c d e f g", false, &named);
        assert_eq!(long.len(), 7);
        assert_eq!(long[6], Piece::Text("…".into()));
        let words = pieces("averyveryverylongwordthatgoesonandon", false, &named);
        assert_eq!(words, vec![Piece::Text("averyveryveryl…nandon".into())]);
    }

    #[test]
    fn summaries() {
        assert_eq!(summary(0, "W"), "W: no new links. Other windows can give different results.");
        assert_eq!(summary(1, "W"), "W: 1 new link. It is in the list above.");
        assert_eq!(summary(3, "W"), "W: 3 new links. They are in the list above.");
    }
}
