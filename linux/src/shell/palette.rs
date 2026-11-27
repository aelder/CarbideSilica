//! The command list (Ctrl-K; Command-K on macOS) (`CommandPalette.swift`). It has each
//! command of the app and each card of the open page. Type some letters, select with the
//! arrow keys, press Return. Each line shows its key, so the list also teaches the keys.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::commands::keys_of;
use super::Shell;
use crate::page::Action;
use crate::page::load::clear;
use crate::pages::PLACES;
use crate::route;
use crate::settings::key;
use crate::theme::{self, font};
use crate::widgets::display;

struct Item {
    title: String,
    /// The group of the line, shown in small text: "This page", "Go", "Display".
    group: String,
    /// The key that does the same, or an empty text.
    key: String,
    run: Action,
}

pub struct Palette {
    pub root: gtk::Box,
    entry: gtk::Entry,
    list: gtk::Box,
    scroller: gtk::ScrolledWindow,
    items: RefCell<Vec<Item>>,
    shown: RefCell<Vec<usize>>,
    selection: Cell<usize>,
    shell: std::rc::Weak<Shell>,
}

impl Palette {
    pub fn new(shell: &Rc<Shell>) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("palette-shade");
        root.set_hexpand(true);
        root.set_vexpand(true);
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.add_css_class("palette");
        let entry = gtk::Entry::new();
        entry.set_placeholder_text(Some("Type a command, a place or a card of this page"));
        panel.append(&entry);
        panel.append(&display::rule());
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.set_margin_top(4);
        list.set_margin_bottom(4);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_propagate_natural_height(true);
        scroller.set_max_content_height(380);
        scroller.set_child(Some(&list));
        panel.append(&scroller);
        panel.set_size_request(560, -1);
        let framed = display::panel(&panel, 0);
        framed.set_halign(gtk::Align::Center);
        framed.set_valign(gtk::Align::Start);
        framed.set_margin_top(70);
        root.append(&framed);
        let palette = Rc::new(Self {
            root,
            entry,
            list,
            scroller,
            items: Default::default(),
            shown: Default::default(),
            selection: Cell::new(0),
            shell: Rc::downgrade(shell),
        });

        // A click on the shade closes the list.
        let shade_click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&palette);
        let panel_weak = framed.downgrade();
        shade_click.connect_pressed(move |gesture, _, x, y| {
            let (Some(palette), Some(panel)) = (weak.upgrade(), panel_weak.upgrade()) else { return };
            let inside = panel
                .compute_bounds(&palette.root)
                .is_some_and(|b| b.contains_point(&gtk::graphene::Point::new(x as f32, y as f32)));
            if !inside {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                palette.close();
            }
        });
        palette.root.add_controller(shade_click);

        let weak = Rc::downgrade(&palette);
        palette.entry.connect_changed(move |_| {
            if let Some(p) = weak.upgrade() {
                p.selection.set(0);
                p.render();
            }
        });
        let weak = Rc::downgrade(&palette);
        palette.entry.connect_activate(move |_| {
            if let Some(p) = weak.upgrade() {
                p.run();
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&palette);
        keys.connect_key_pressed(move |_, keyval, _, _| {
            let Some(p) = weak.upgrade() else { return glib::Propagation::Proceed };
            match keyval {
                gtk::gdk::Key::Escape => {
                    p.close();
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Down | gtk::gdk::Key::Up => {
                    let count = p.shown.borrow().len();
                    if count > 0 {
                        let step: i64 = if keyval == gtk::gdk::Key::Down { 1 } else { -1 };
                        let next = (p.selection.get() as i64 + step).clamp(0, count as i64 - 1) as usize;
                        p.selection.set(next);
                        p.render();
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        palette.root.add_controller(keys);
        palette
    }

    pub fn open(&self) {
        *self.items.borrow_mut() = self.build();
        self.entry.set_text("");
        self.selection.set(0);
        self.render();
        self.root.set_visible(true);
        self.entry.grab_focus();
    }

    pub fn close(&self) {
        self.root.set_visible(false);
        if let Some(shell) = self.shell.upgrade() {
            shell.palette_closed();
        }
    }

    fn render(&self) {
        let words: Vec<String> = self.entry.text().to_lowercase().split_whitespace().map(str::to_string).collect();
        let items = self.items.borrow();
        let shown: Vec<usize> = items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                let text = format!("{} {}", item.title, item.group).to_lowercase();
                words.iter().all(|w| text.contains(w.as_str()))
            })
            .map(|(i, _)| i)
            .collect();
        clear(&self.list);
        if shown.is_empty() {
            let none = gtk::Label::new(Some("No command has these letters."));
            none.add_css_class("label");
            none.set_xalign(0.0);
            none.set_margin_start(14);
            none.set_margin_top(14);
            none.set_margin_bottom(14);
            self.list.append(&none);
        }
        let selection = self.selection.get();
        for (position, index) in shown.iter().enumerate() {
            let item = &items[*index];
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("palette-row");
            if position == selection {
                row.add_css_class("selected");
            }
            let title = gtk::Label::new(Some(&item.title));
            title.add_css_class("value");
            row.append(&title);
            let group = display::name_text(&item.group);
            group.set_hexpand(true);
            row.append(&group);
            if !item.key.is_empty() {
                let key = gtk::Label::new(Some(&item.key));
                key.add_css_class("key-cap");
                row.append(&key);
            }
            let click = gtk::GestureClick::new();
            let weak = self.shell.clone();
            click.connect_released(move |_, _, _, _| {
                if let Some(shell) = weak.upgrade()
                    && let Some(palette) = shell.palette()
                {
                    palette.selection.set(position);
                    palette.run();
                }
            });
            row.add_controller(click);
            self.list.append(&row);
            if position == selection {
                let row = row.clone();
                let scroller = self.scroller.clone();
                glib::idle_add_local_once(move || {
                    if let Some(bounds) = row.compute_bounds(&scroller.child().unwrap()) {
                        let adj = scroller.vadjustment();
                        let (top, bottom) = (bounds.y() as f64, (bounds.y() + bounds.height()) as f64);
                        if top < adj.value() {
                            adj.set_value(top);
                        } else if bottom > adj.value() + adj.page_size() {
                            adj.set_value(bottom - adj.page_size());
                        }
                    }
                });
            }
        }
        drop(items);
        *self.shown.borrow_mut() = shown;
    }

    fn run(&self) {
        let action = {
            let shown = self.shown.borrow();
            let Some(index) = shown.get(self.selection.get()) else { return };
            self.items.borrow()[*index].run.clone()
        };
        self.close();
        action();
    }

    fn build(&self) -> Vec<Item> {
        let Some(shell) = self.shell.upgrade() else { return vec![] };
        let model = shell.model.clone();
        let mut lines: Vec<Item> = Vec::new();
        let mut add = |title: String, group: &str, key: &str, run: Action| {
            lines.push(Item { title, group: group.to_string(), key: key.to_string(), run });
        };
        let page = shell.navigator.current();
        let this = "This page";

        if let Some(page) = &page {
            // The cards of the open page.
            for card in page.cursor.card_titles() {
                let cursor = page.cursor.clone();
                let title = card.clone();
                add(format!("Go to {card}"), this, "", Rc::new(move || cursor.scroll_to_card(&title)));
            }
            let hooks = page.hooks.borrow();
            if let Some(toggle) = &hooks.toggle_json {
                add("Show or hide the raw JSON".into(), this, "Tab", toggle.clone());
            }
            if let Some(save) = &hooks.save_json {
                add("Save the raw JSON".into(), this, "s", save.clone());
            }
            // The functions of the open page.
            if let Some(adjacent) = &hooks.adjacent {
                if let Some(next) = &adjacent.next {
                    add(format!("Next {}", adjacent.noun), this, "n", next.clone());
                }
                if let Some(previous) = &adjacent.previous {
                    add(format!("Previous {}", adjacent.noun), this, "p", previous.clone());
                }
            }
            let found = &hooks.investigations;
            for (title, key, run) in [
                ("Follow Transfer", "F8", &found.follow_transfer),
                ("Witnesses", "F4", &found.witnesses),
                ("Protocol Links", "F3", &found.protocol_links),
                ("Block History", "F7", &found.inclusions),
                ("Shared Keys and Authorities", "v", &found.shared_authorities),
            ] {
                if let Some(run) = run {
                    add(title.into(), this, key, run.clone());
                }
            }
            for (key, (title, run)) in &hooks.keys {
                add(title.clone(), this, key, run.clone());
            }
            for command in &hooks.commands {
                add(command.title.clone(), this, &command.key, command.run.clone());
            }
            let place = &hooks.place;
            if let Some(run) = &place.add_node {
                add("Add Node".into(), this, &keys_of("app.add-node"), run.clone());
            }
            if let Some(run) = &place.ping_all {
                add("Ping All Peers".into(), this, &keys_of("app.ping-all"), run.clone());
            }
            if let Some(run) = &place.clear_bans {
                add("Clear Ban List".into(), this, "", run.clone());
            }
        }
        let weak = Rc::downgrade(&shell);
        add("Read the page again".into(), this, "r", Rc::new(move || {
            if let Some(shell) = weak.upgrade() {
                shell.refresh();
            }
        }));
        if model.state().path.last().is_some_and(route::is_bookmarkable) {
            let weak = Rc::downgrade(&shell);
            add("Bookmark This Page".into(), this, "Shift+B", Rc::new(move || {
                if let Some(shell) = weak.upgrade() {
                    shell.bookmark_current();
                }
            }));
        }

        // Places and history.
        for spec in PLACES.iter() {
            let (weak, place) = (Rc::downgrade(&model), spec.place);
            add(spec.title.into(), "Go", spec.terminal_key, Rc::new(move || {
                if let Some(model) = weak.upgrade() {
                    model.show(place);
                }
            }));
        }
        let weak = Rc::downgrade(&shell);
        add("Back".into(), "Go", "Esc", Rc::new(move || {
            if let Some(shell) = weak.upgrade() {
                shell.navigator.go_back();
            }
        }));
        if shell.navigator.can_go_forward() {
            let weak = Rc::downgrade(&shell);
            add("Forward".into(), "Go", &keys_of("app.forward"), Rc::new(move || {
                if let Some(shell) = weak.upgrade() {
                    shell.navigator.go_forward();
                }
            }));
        }
        let weak = Rc::downgrade(&model);
        add("Search".into(), "Go", "/", Rc::new(move || {
            if let Some(model) = weak.upgrade() {
                model.focus_search();
            }
        }));
        let weak = Rc::downgrade(&shell);
        add("Estimate a Future Block".into(), "Go", "F10", Rc::new(move || {
            if let Some(shell) = weak.upgrade() {
                shell.estimate();
            }
        }));
        let recent: Vec<_> = model.state().recent.iter().take(8).cloned().collect();
        for target in recent {
            let weak = Rc::downgrade(&model);
            let title = format!("Open {}", route::title(&target));
            add(title, "Recent", "", Rc::new(move || {
                if let Some(model) = weak.upgrade() {
                    model.open(target.clone());
                }
            }));
        }

        // Display.
        for (title, setting, default, keys) in [
            ("Show Names or IDs", key::SHOWS_NAMES, true, "Shift+I".to_string()),
            ("Show Dates or Heights", key::SHOWS_DATES, false, "Shift+D".to_string()),
            ("Show Times in UTC or Local Time", key::USE_UTC, false, keys_of("app.utc")),
        ] {
            let weak = Rc::downgrade(&model);
            add(title.into(), "Display", &keys, Rc::new(move || {
                if let Some(model) = weak.upgrade() {
                    model.toggle_display(setting, default);
                }
            }));
        }

        // Chains and the app.
        let current = model.current_chain();
        for chain in model.known_chain_list().into_iter().filter(|c| c.chain != current) {
            let weak = Rc::downgrade(&model);
            let title = format!("Change the Chain to {}", chain.chain);
            add(title, "Chain", "", Rc::new(move || {
                if let Some(model) = weak.upgrade() {
                    model.switch_chain(&chain);
                }
            }));
        }
        let weak = Rc::downgrade(&shell);
        add("Keyboard Shortcuts".into(), "App", "?", Rc::new(move || {
            if let Some(shell) = weak.upgrade() {
                shell.show_keys();
            }
        }));
        let weak = Rc::downgrade(&shell);
        add("Settings".into(), "App", &keys_of("app.settings"), Rc::new(move || {
            if let Some(shell) = weak.upgrade() {
                shell.show_settings();
            }
        }));
        let _ = theme::markup(font::SMALL, "");
        lines
    }
}
