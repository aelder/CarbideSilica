//! Saved places, kept in Carbide's own bookmark file so the terminal app sees them too.
//! Port of BookmarksView.swift in `macos/Sources/CarbideSilica/Views`.
//!
//! Each row shows tags derived from the object in Carbide's cache and the source that
//! observed it. Cached data does not prove that the object is still on the active chain.

mod glyphs;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, api::Route};
use crate::display as settings_display;
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, Page};
use crate::shared::currency_stores::{known_name, observation_source};
use crate::shell::dialogs;
use crate::theme::{draw, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, PlateFill};
use crate::widgets::icons::{self, Icon};
use crate::widgets::{lamp, link, loader};
use glyphs::Glyph;

pub fn kind_word(kind: &str) -> String {
    match kind {
        "auto" => "Search target".into(),
        "notarizations" => "Notarizations".into(),
        other => {
            // `capitalized`: the first letter of each word in capitals, the rest small.
            other
                .split(' ')
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
    }
}

pub fn route_for(entry: &api::BookmarkEntry) -> Option<Route> {
    match entry.kind.as_str() {
        "block" => Some(Route::Block { id: entry.target.clone() }),
        "transaction" => Some(Route::Transaction { txid: entry.target.clone() }),
        "address" => Some(Route::Address { address: entry.target.clone() }),
        "identity" => Some(Route::Identity { name: entry.target.clone() }),
        "currency" => Some(Route::Currency { name: entry.target.clone() }),
        _ => None,
    }
}

fn glyph_for(entry: &api::BookmarkEntry) -> Glyph {
    match route_for(entry) {
        Some(Route::Block { .. }) => Glyph::Cube,
        Some(Route::Transaction { .. }) => Glyph::Arrows,
        Some(Route::Identity { .. }) => Glyph::Person,
        Some(Route::Currency { .. }) | Some(Route::Currencies) => Glyph::Hexagons,
        Some(Route::Address { .. }) => Glyph::Card,
        Some(Route::Estimate { .. }) => Glyph::Bookmark,
        None if entry.kind == "notarizations" => Glyph::Link,
        None => Glyph::Bookmark,
    }
}

thread_local! {
    /// Carbide's optional start-up population, once (`BookmarkStartup`).
    static STARTED: Cell<bool> = const { Cell::new(false) };
}

struct Bookmarks {
    model: std::rc::Weak<AppModel>,
    page: std::rc::Weak<Page>,
    file: RefCell<Option<api::BookmarkFile>>,
    failure: RefCell<Option<String>>,
    message: RefCell<Option<String>>,
    holder: gtk::Box,
    run: Cell<u64>,
}

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>) -> Rc<Page> {
    let page = Page::scrolling(model);
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 20);
    page.append(&holder);
    let bookmarks = Rc::new(Bookmarks {
        model: Rc::downgrade(model),
        page: Rc::downgrade(&page),
        file: RefCell::default(),
        failure: RefCell::default(),
        message: RefCell::default(),
        holder,
        run: Cell::new(0),
    });
    {
        let weak = Rc::downgrade(&bookmarks);
        page.hooks().refresh = Some(Rc::new(move || {
            if let Some(bookmarks) = weak.upgrade() {
                bookmarks.reload();
            }
        }));
    }
    bookmarks.draw();
    // Each visit reads the file again, then runs the start-up population if nothing ran
    // it yet, then reads the file again.
    let weak = Rc::downgrade(&bookmarks);
    page.root.connect_map(move |_| {
        if let Some(bookmarks) = weak.upgrade() {
            bookmarks.visit();
        }
    });
    let weak = Rc::downgrade(&bookmarks);
    model.watch(&[Topic::Connection], &page.root, move |_| {
        if let Some(bookmarks) = weak.upgrade() {
            bookmarks.visit();
        }
    });
    let weak = Rc::downgrade(&bookmarks);
    model.watch(&[Topic::Names, Topic::Display, Topic::Bookmarks, Topic::Status], &page.root, move |_| {
        if let Some(bookmarks) = weak.upgrade() {
            bookmarks.draw();
        }
    });
    let keep = bookmarks.clone();
    page.root.connect_destroy(move |_| {
        let _ = &keep;
    });
    page
}

impl Bookmarks {
    fn visit(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Some(this) = weak.upgrade() else { return };
            this.load().await;
            if let Some(explorer) = model.explorer()
                && !STARTED.with(|s| s.replace(true))
            {
                let _ = bridge::run(async move { explorer.populate_bookmarks().await }).await;
            }
            this.load().await;
        });
    }

    fn reload(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            if let Some(this) = weak.upgrade() {
                this.load().await;
            }
        });
    }

    async fn load(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(explorer) = model.explorer() else { return };
        let run = self.run.get() + 1;
        self.run.set(run);
        let result = bridge::run(async move { explorer.bookmark_file().await }).await;
        if self.run.get() != run {
            return;
        }
        match result {
            Ok(found) => {
                let ids: Vec<String> = found.entries.iter().flat_map(|e| [e.target.clone(), e.system_id.clone()]).collect();
                *self.file.borrow_mut() = Some(found);
                *self.failure.borrow_mut() = None;
                model.resolve_names(ids);
            }
            Err(error) => *self.failure.borrow_mut() = Some(bridge::describe(&error)),
        }
        self.draw();
    }

    fn window(&self) -> Option<gtk::Window> {
        self.page.upgrade()?.root.root().and_downcast::<gtk::Window>()
    }

    fn draw(self: &Rc<Self>) {
        let (Some(page), Some(model)) = (self.page.upgrade(), self.model.upgrade()) else { return };
        clear(&self.holder);
        let file = self.file.borrow().clone();
        let card = page.card("Bookmarks", None);
        card.set_trailing(file.as_ref().map(|f| format!("{} saved", f.entries.len())).as_deref());
        let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let weak = Rc::downgrade(self);
        keys.append(&lamp::button("Add Bookmark…", move || {
            if let Some(bookmarks) = weak.upgrade() {
                bookmarks.add();
            }
        }));
        let weak = Rc::downgrade(self);
        keys.append(&lamp::button("Reload", move || {
            if let Some(bookmarks) = weak.upgrade() {
                bookmarks.reload();
            }
        }));
        card.append(&keys);
        let failure = self.failure.borrow().clone().or_else(|| model.state().bookmark_error.clone());
        if let Some(failure) = &failure {
            card.append(&page::note(failure, true));
        }
        if let Some(message) = self.message.borrow().as_ref() {
            card.append(&page::paragraph(message, &["secondary"]));
        }
        if let Some(file) = &file
            && !file.issues.is_empty()
        {
            let count = file.issues.len();
            card.append(&page::note(
                &format!(
                    "{count} line{} not a valid bookmark. Carbide keeps them unchanged. {}",
                    if count == 1 { " is" } else { "s are" },
                    file.issues[0]
                ),
                true,
            ));
        }
        match &file {
            Some(file) => {
                if file.entries.is_empty() {
                    let field = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    field.set_size_request(-1, 160);
                    let empty = gtk::Box::new(gtk::Orientation::Vertical, 6);
                    empty.set_valign(gtk::Align::Center);
                    empty.set_halign(gtk::Align::Center);
                    empty.set_vexpand(true);
                    field.append(&empty);
                    let title = gtk::Label::new(None);
                    title.set_markup(&format!("<span font_size=\"{}\">No bookmarks.</span>", 15 * 1024));
                    title.add_css_class("value");
                    empty.append(&title);
                    let text = page::paragraph(
                        "To add a bookmark, open a page and click the bookmark button in the toolbar, or click Add Bookmark.",
                        &["secondary"],
                    );
                    text.set_justify(gtk::Justification::Center);
                    text.set_xalign(0.5);
                    empty.append(&text);
                    card.append(&field);
                } else {
                    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    for entry in &file.entries {
                        if entry.index > 0 {
                            list.append(&page::divider());
                        }
                        list.append(&self.row(&page, &model, entry));
                    }
                    card.append(&list);
                }
                let path = page::paragraph(&format!("Bookmark file: {}", file.path), &["caption-text", "secondary"]);
                path.set_selectable(true);
                card.append(&path);
            }
            None if failure.is_none() => {
                let progress = loader::small();
                progress.set_halign(gtk::Align::Center);
                progress.set_size_request(-1, 120);
                card.append(&progress);
            }
            None => {}
        }
        self.holder.append(&card.root);
        if let Some(file) = &file {
            self.holder.append(&startup_card(&page, file));
        }
    }

    fn system(&self, model: &AppModel, entry: &api::BookmarkEntry) -> String {
        let name = known_name(&entry.system_id, model, entry.system_name.as_deref());
        if entry.this_chain { name } else { format!("{name}, another chain") }
    }

    fn target_text(&self, model: &AppModel, entry: &api::BookmarkEntry) -> String {
        if entry.kind == "notarizations" {
            return format!("Notarizations of {}", known_name(&entry.target, model, Some(&entry.target)));
        }
        let name = if settings_display::shows_names() { model.state().names.get(&entry.target).cloned() } else { None };
        name.unwrap_or_else(|| format::short(&entry.target, 16, 10))
    }

    fn row(self: &Rc<Self>, page: &Rc<Page>, model: &Rc<AppModel>, entry: &api::BookmarkEntry) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        let mark = glyphs::glyph(glyph_for(entry), 16, tone::LABEL);
        mark.set_size_request(20, 16);
        mark.set_margin_top(1);
        row.append(&mark);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 3);
        column.set_hexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let heading = if entry.description.is_empty() { kind_word(&entry.kind) } else { entry.description.clone() };
        let title = page::text(&heading, &["semibold", "value"]);
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        top.append(&title);
        let system = display::name_text(&self.system(model, entry));
        system.set_tooltip_text(Some(&entry.system_id));
        top.append(&system);
        column.append(&top);
        let second = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        second.append(&display::plate(&entry.kind, PlateFill::None));
        let target = self.target_text(model, entry);
        let open = {
            let (weak, entry) = (Rc::downgrade(self), entry.clone());
            move || {
                if let Some(bookmarks) = weak.upgrade() {
                    bookmarks.open(&entry);
                }
            }
        };
        let open_click = open.clone();
        let link = link::link(&target, false, move || open_click());
        if let Some(label) = link::label_of(&link) {
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        }
        link.set_tooltip_text(Some(&entry.target));
        page.stop(&link, Some(entry.index.to_string()), Some(target.clone()), open);
        second.append(&link);
        column.append(&second);
        column.append(&cache_line(entry));
        column.append(&page::text(&format!("Created {}", format::date(entry.created_at)), &["caption-text", "secondary"]));
        row.append(&column);
        let actions = gtk::Box::new(gtk::Orientation::Vertical, 4);
        actions.set_valign(gtk::Align::Start);
        let (weak, edited) = (Rc::downgrade(self), entry.clone());
        let edit = lamp::button("Edit", move || {
            if let Some(bookmarks) = weak.upgrade() {
                bookmarks.edit(&edited);
            }
        });
        edit.set_halign(gtk::Align::Fill);
        actions.append(&edit);
        let (weak, removed) = (Rc::downgrade(self), entry.clone());
        let remove = lamp::button("Remove", move || {
            if let Some(bookmarks) = weak.upgrade() {
                bookmarks.remove(&removed);
            }
        });
        remove.set_halign(gtk::Align::Fill);
        // A destructive key: its legend in the colour of an alert.
        if let Some(label) = remove.child().and_downcast::<gtk::Label>() {
            label.add_css_class("alert");
        }
        actions.append(&remove);
        row.append(&actions);
        row
    }

    fn edit(self: &Rc<Self>, entry: &api::BookmarkEntry) {
        let Some(window) = self.window() else { return };
        let dialog = dialogs::dialog(&window, "Edit description", None);
        let field = gtk::Entry::new();
        field.add_css_class("display-field");
        field.set_placeholder_text(Some("Description"));
        field.set_text(&entry.description);
        dialog.body.append(&field);
        dialog.key("Cancel", false, || {});
        let (weak, index, text) = (Rc::downgrade(self), entry.index, field.clone());
        let save = dialog.key("Save", true, move || {
            if let Some(bookmarks) = weak.upgrade()
                && let Some(model) = bookmarks.model.upgrade()
            {
                model.edit_bookmark(index, &text.text());
                bookmarks.reload();
            }
        });
        field.connect_activate(move |_| save.emit_clicked());
        dialog.show();
        field.grab_focus();
    }

    fn remove(self: &Rc<Self>, entry: &api::BookmarkEntry) {
        let Some(window) = self.window() else { return };
        let (weak, index) = (Rc::downgrade(self), entry.index);
        dialogs::confirm(
            &window,
            "Remove this bookmark?",
            &format!("The app removes {} from the Carbide bookmark file. The terminal app uses the same file.", entry.target),
            "Remove",
            move || {
                if let Some(bookmarks) = weak.upgrade()
                    && let Some(model) = bookmarks.model.upgrade()
                {
                    model.remove_bookmark(index);
                    bookmarks.reload();
                }
            },
        );
    }

    /// A notarization bookmark opens the currency page with the saved perspective. A
    /// bookmark without a kind goes through search, which works out what it names.
    fn open(self: &Rc<Self>, entry: &api::BookmarkEntry) {
        let Some(model) = self.model.upgrade() else { return };
        self.message.replace(None);
        let chain = model.state().status.as_ref().and_then(|s| s.chain_id.clone());
        let Some(chain) = chain else {
            self.message.replace(Some("The app is not connected to a node.".into()));
            self.draw();
            return;
        };
        if entry.kind == "notarizations" {
            if entry.system_id == chain {
                request_notarizations(&entry.target, &chain);
                model.open(Route::Currency { name: entry.target.clone() });
            } else if entry.target == chain {
                // The other chain's view of this chain: open the other chain's currency.
                request_notarizations(&entry.system_id, &entry.system_id);
                model.open(Route::Currency { name: entry.system_id.clone() });
            } else {
                self.message.replace(Some(format!(
                    "This bookmark is the {} view of {}. Connect to that chain to open it.",
                    known_name(&entry.system_id, &model, None),
                    entry.target
                )));
                self.draw();
            }
            return;
        }
        if !entry.this_chain {
            self.message.replace(Some(format!(
                "This bookmark belongs to {}. Connect to that chain to open it.",
                known_name(&entry.system_id, &model, entry.system_name.as_deref())
            )));
            self.draw();
            return;
        }
        match route_for(entry) {
            Some(route) => model.open(route),
            None => model.search(entry.target.clone()),
        }
    }

    fn add(self: &Rc<Self>) {
        let (Some(window), Some(model)) = (self.window(), self.model.upgrade()) else { return };
        let default_system = model.state().status.as_ref().and_then(|s| s.chain_id.clone()).unwrap_or_default();
        let weak = Rc::downgrade(self);
        add_sheet(&model, &window, &default_system, move |saved| {
            if saved
                && let Some(bookmarks) = weak.upgrade()
                && let Some(model) = bookmarks.model.upgrade()
            {
                model.reload_bookmarks();
                bookmarks.reload();
            }
        });
    }
}

/// Asks the currency page to open the notarizations of `currency` as `observer` sees them
/// (`NotarizationRequests.shared.request`). The store is the Currency page's part of
/// `shared/currency_stores.rs`; until it is there, the currency page opens with its own
/// default perspective.
fn request_notarizations(_currency: &str, _observer: &str) {}

/// "Cached observation" with the tags and the source, "Uncached", or the failure.
fn cache_line(entry: &api::BookmarkEntry) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    match &entry.cache {
        api::BookmarkCache::Cached { source, tags } => {
            row.append(&glyphs::glyph(Glyph::TrayFull, 13, tone::GOOD));
            let label = page::text("Cached observation", &["caption-text", "semibold", "good"]);
            row.append(&label);
            for tag in tags {
                row.append(&display::plate(tag, PlateFill::None));
            }
            let from = page::text(&format!("from {}", observation_source(source)), &["caption-text", "label"]);
            from.set_tooltip_text(Some(source));
            row.append(&from);
        }
        api::BookmarkCache::Uncached => {
            row.append(&glyphs::glyph(Glyph::Tray, 13, tone::LABEL));
            row.append(&page::text("Uncached", &["caption-text", "semibold", "label"]));
        }
        api::BookmarkCache::Failed { message } => {
            let mark = icons::icon(Icon::Warning, 12, tone::MINED);
            mark.set_valign(gtk::Align::Start);
            mark.set_margin_top(2);
            row.append(&mark);
            let text = page::paragraph(&format!("Unavailable: {message}"), &["caption-text", "mined"]);
            text.set_lines(2);
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&text);
        }
    }
    row
}

/// The start-up population: on or off, the lines that turn it on, and its report.
fn startup_card(page: &Rc<Page>, file: &api::BookmarkFile) -> gtk::Box {
    let card = page.card("Start-up population", None);
    if file.startup_enabled {
        card.append(&page::paragraph(
            &format!(
                "On. When the app starts, it requests up to {} uncached bookmarks, one at a time, with a 70-second limit for each. It uses only configured sources and does not ask for permission. Failures stay visible in the list.",
                file.startup_limit
            ),
            &["value"],
        ));
    } else {
        card.append(&page::paragraph(
            "Off. This is the default. To turn it on, add these lines to Carbide's config.toml. The terminal app reads the same setting.",
            &["value"],
        ));
        let lines = page::text("[bookmarks]\nfetch_missing_on_startup = true\nstartup_limit = 20", &["mono", "callout", "value"]);
        lines.set_selectable(true);
        lines.set_margin_top(8);
        lines.set_margin_bottom(8);
        lines.set_margin_start(8);
        lines.set_margin_end(8);
        let frame = gtk::Overlay::new();
        let field = gtk::Box::new(gtk::Orientation::Vertical, 0);
        field.append(&lines);
        frame.set_child(Some(&field));
        let border = Canvas::new(0, 0, |_, snapshot, w, h| {
            draw::stroke(snapshot, &draw::rect_path(0.5, 0.5, w - 1.0, h - 1.0), tone::RULE, 1.0);
        });
        border.set_can_target(false);
        frame.add_overlay(&border);
        frame.set_halign(gtk::Align::Start);
        card.append(&frame);
    }
    if !file.settings_path.is_empty() {
        let path = page::paragraph(&format!("Settings file: {}", file.settings_path), &["caption-text", "secondary"]);
        path.set_selectable(true);
        card.append(&path);
    }
    if let Some(issue) = &file.settings_issue {
        card.append(&page::note(issue, true));
    }
    if let Some(report) = &file.startup_report {
        card.append(&page::paragraph(report, &["secondary"]));
    }
    card.root
}

const KINDS: [&str; 7] = ["auto", "block", "transaction", "address", "identity", "currency", "notarizations"];

fn prompt(kind: &str) -> &'static str {
    match kind {
        "notarizations" => "A currency name or ID, such as vETH",
        "block" => "A block height or hash",
        "transaction" => "A transaction ID",
        "address" => "An address",
        "identity" => "An identity, such as alice@",
        "currency" => "A currency name or ID",
        _ => "Anything that search accepts",
    }
}

fn help(kind: &str) -> &'static str {
    if kind == "notarizations" {
        "A notarization bookmark opens the notarization browser for the currency, as seen by the chain with this system ID. A target such as notarizations:vETH with the kind Search target also works."
    } else {
        "The system ID is the chain that the object belongs to. The terminal app shares this bookmark file."
    }
}

/// Manual entry of a bookmark: a system ID, a kind and a target, as in Carbide's `n`
/// (`AddBookmarkSheet`).
fn add_sheet(model: &Rc<AppModel>, parent: &gtk::Window, default_system: &str, done: impl Fn(bool) + 'static) {
    let done = Rc::new(done);
    let window = gtk::Window::new();
    window.set_transient_for(Some(parent));
    window.set_modal(true);
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_default_size(520, -1);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 14);
    column.set_margin_top(20);
    column.set_margin_bottom(20);
    column.set_margin_start(20);
    column.set_margin_end(20);
    column.set_size_request(480, -1);
    let title = gtk::Label::new(None);
    title.set_markup(&format!("<span font_size=\"{}\" weight=\"600\">Add a bookmark</span>", 17 * 1024));
    title.set_xalign(0.0);
    title.add_css_class("value");
    column.append(&title);
    let form = gtk::Grid::new();
    form.set_column_spacing(10);
    form.set_row_spacing(8);
    let field = |row: i32, label: &str, placeholder: &str| -> gtk::Entry {
        let name = page::text(&format!("{label}:"), &["value"]);
        name.set_xalign(1.0);
        form.attach(&name, 0, row, 1, 1);
        let entry = gtk::Entry::new();
        entry.add_css_class("display-field");
        entry.set_placeholder_text(Some(placeholder));
        entry.set_hexpand(true);
        form.attach(&entry, 1, row, 1, 1);
        entry
    };
    let system = field(0, "System ID", "The chain that observes the object");
    system.set_text(default_system);
    let kind_name = page::text("Kind:", &["value"]);
    kind_name.set_xalign(1.0);
    form.attach(&kind_name, 0, 1, 1, 1);
    let kind = Rc::new(Cell::new(0usize));
    let target = field(2, "Target", prompt("auto"));
    let description = field(3, "Description", "Optional");
    column.append(&form);
    let help_text = page::paragraph(help("auto"), &["callout", "secondary"]);
    let failure = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let words: Vec<String> = KINDS.iter().map(|k| kind_word(k)).collect();
    let titles: Vec<&str> = words.iter().map(String::as_str).collect();
    let menu = {
        let (kind, target, help_text) = (kind.clone(), target.clone(), help_text.clone());
        lamp::Menu::new(&titles, 0, move |index| {
            kind.set(index);
            target.set_placeholder_text(Some(prompt(KINDS[index])));
            help_text.set_text(help(KINDS[index]));
        })
    };
    menu.button.set_halign(gtk::Align::Start);
    form.attach(&menu.button, 1, 1, 1, 1);
    column.append(&help_text);
    column.append(&failure);
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    keys.set_halign(gtk::Align::End);
    let (w, d) = (window.downgrade(), done.clone());
    let cancel = lamp::button("Cancel", move || {
        d(false);
        if let Some(w) = w.upgrade() {
            w.close();
        }
    });
    keys.append(&cancel);
    let (w, d, m) = (window.downgrade(), done.clone(), Rc::downgrade(model));
    let (sys, tgt, desc, k, fail) = (system.clone(), target.clone(), description.clone(), kind.clone(), failure.clone());
    let save = lamp::button("Save", move || {
        let Some(model) = m.upgrade() else { return };
        let result = model
            .require_explorer()
            .and_then(|explorer| {
                explorer
                    .save_bookmark(Some(sys.text().to_string()), KINDS[k.get()].to_string(), tgt.text().to_string(), desc.text().to_string())
                    .map_err(|e| bridge::describe(&e))
            });
        match result {
            Ok(()) => {
                d(true);
                if let Some(w) = w.upgrade() {
                    w.close();
                }
            }
            Err(message) => {
                clear(&fail);
                fail.append(&page::note(&message, true));
            }
        }
    });
    lamp::set_chosen(&save, true);
    save.set_sensitive(false);
    keys.append(&save);
    column.append(&keys);
    let s = save.clone();
    target.connect_changed(move |entry| s.set_sensitive(!entry.text().trim().is_empty()));
    for entry in [&system, &target, &description] {
        let s = save.clone();
        entry.connect_activate(move |_| {
            if s.is_sensitive() {
                s.emit_clicked();
            }
        });
    }
    let frame = display::panel(&column, 0);
    window.set_child(Some(&frame));
    window.set_default_widget(Some(&save));
    let escape = gtk::EventControllerKey::new();
    let c = cancel.clone();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            c.emit_clicked();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(escape);
    // The menu lives as long as the window.
    let keep = menu.clone();
    window.connect_destroy(move |_| {
        let _ = &keep;
    });
    window.present();
    target.grab_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_in_words() {
        assert_eq!(kind_word("auto"), "Search target");
        assert_eq!(kind_word("notarizations"), "Notarizations");
        assert_eq!(kind_word("block"), "Block");
        assert_eq!(kind_word("transaction"), "Transaction");
        assert_eq!(prompt("identity"), "An identity, such as alice@");
        assert_eq!(prompt("auto"), "Anything that search accepts");
    }
}
