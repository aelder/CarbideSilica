//! The bar at the top of the window (`RootView`'s toolbar): the menu key, the search field
//! in the middle, the bookmark key and the chain status menu.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::bridge::api;
use crate::format;
use crate::model::{AppModel, Topic};
use crate::route;
use crate::theme::{self, font, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::icons::{self, Icon};
use crate::widgets::{lamp, loader};

pub fn build(model: &Rc<AppModel>, menu: &gio::Menu, open_settings: Rc<dyn Fn()>, bookmark: Rc<dyn Fn()>) -> gtk::HeaderBar {
    let bar = gtk::HeaderBar::new();
    bar.add_css_class("titlebar");

    // The menus of the app: View, Go, Investigate, Peers and Help. On macOS they are in
    // the menu bar of the system.
    let menu_key = gtk::MenuButton::new();
    menu_key.add_css_class("chain-badge");
    menu_key.set_child(Some(&icons::icon(Icon::Menu, 16, tone::LABEL)));
    menu_key.set_menu_model(Some(menu));
    menu_key.set_tooltip_text(Some("Menus"));
    lamp::key_sounds(&menu_key, true);
    bar.pack_start(&menu_key);

    bar.set_title_widget(Some(&search_field(model)));

    let badge = connection_badge(model, open_settings);
    bar.pack_end(&badge);

    let bookmark_key = gtk::Button::new();
    bookmark_key.add_css_class("chain-badge");
    bookmark_key.set_child(Some(&icons::icon(Icon::Bookmark, 15, tone::LABEL)));
    bookmark_key.set_tooltip_text(Some("Bookmark this page"));
    bookmark_key.connect_clicked(move |_| bookmark());
    lamp::key_sounds(&bookmark_key, true);
    let key = bookmark_key.clone();
    let update = move |model: &Rc<AppModel>| {
        key.set_visible(model.state().path.last().is_some_and(route::is_bookmarkable));
    };
    update(model);
    model.watch(&[Topic::Path], &bookmark_key, update);
    bar.pack_end(&bookmark_key);
    bar
}

/// The search box. Return resolves the text and opens the result. Up and Down go through
/// the earlier searches. Esc stops a search that runs, or clears the field and leaves it.
fn search_field(model: &Rc<AppModel>) -> gtk::Box {
    let field = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    field.add_css_class("search-field");
    field.set_size_request(420, -1);
    field.set_hexpand(false);
    field.set_valign(gtk::Align::Center);
    let mark_holder = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    mark_holder.set_size_request(16, 16);
    mark_holder.set_valign(gtk::Align::Center);
    mark_holder.append(&icons::icon(Icon::Search, 15, tone::LABEL));
    let text = gtk::Text::new();
    text.set_placeholder_text(Some("Height, hash, date, address, name@ or currency@@"));
    text.set_hexpand(true);
    field.append(&mark_holder);
    field.append(&text);

    let weak = Rc::downgrade(model);
    text.connect_activate(move |text| {
        if let Some(model) = weak.upgrade() {
            model.search(text.text().to_string());
        }
    });
    // The place in the earlier searches. None when the text is the person's own.
    let place: Rc<Cell<Option<usize>>> = Rc::default();
    let keys = gtk::EventControllerKey::new();
    let (weak, field_text, history_place) = (Rc::downgrade(model), text.clone(), place.clone());
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(model) = weak.upgrade() else { return glib::Propagation::Proceed };
        let recall = |step: i64| {
            let history = model.state().search_history.clone();
            if history.is_empty() {
                return glib::Propagation::Proceed;
            }
            let next = history_place.get().map(|p| p as i64).unwrap_or(-1) + step;
            if next < 0 {
                history_place.set(None);
                field_text.set_text("");
            } else {
                let index = (next as usize).min(history.len() - 1);
                history_place.set(Some(index));
                field_text.set_text(&history[index]);
                field_text.set_position(-1);
            }
            glib::Propagation::Stop
        };
        match key {
            gtk::gdk::Key::Up => recall(1),
            gtk::gdk::Key::Down => recall(-1),
            gtk::gdk::Key::Escape => {
                // Esc stops a search that runs. If no search runs, it cancels the input,
                // as in the terminal, and the keys of the page work again.
                if model.state().is_searching {
                    model.cancel_search();
                } else {
                    field_text.set_text("");
                    if let Some(root) = field_text.root() {
                        gtk::prelude::GtkWindowExt::set_focus(root.downcast_ref::<gtk::Window>().unwrap(), None::<&gtk::Widget>);
                    }
                }
                history_place.set(None);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    text.add_controller(keys);

    // The circuit mark while a search runs.
    let (holder, searching) = (mark_holder.clone(), Rc::new(Cell::new(false)));
    model.watch(&[Topic::Search], &field, move |model| {
        let now = model.state().is_searching;
        if now == searching.replace(now) {
            return;
        }
        crate::page::load::clear(&holder);
        if now {
            holder.append(&loader::small());
        } else {
            holder.append(&icons::icon(Icon::Search, 15, tone::LABEL));
        }
    });

    let (focus_text, show_text) = (text.clone(), text.clone());
    model.shell.borrow_mut().focus_search = Some(Box::new(move || {
        focus_text.grab_focus();
    }));
    model.shell.borrow_mut().show_search_text = Some(Box::new(move |value: &str| {
        show_text.set_text(value);
    }));
    field
}

/// Chain, height and where the data is coming from. The badge is also the chain menu:
/// every node configured on this computer (`ConnectionBadge`).
fn connection_badge(model: &Rc<AppModel>, open_settings: Rc<dyn Fn()>) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.add_css_class("chain-badge");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    let square_color = Rc::new(Cell::new(tone::LABEL));
    let sc = square_color.clone();
    let square = Canvas::new(7, 7, move |_, snapshot, w, h| {
        crate::theme::draw::fill_rect(snapshot, 0.0, 0.0, w, h, sc.get());
    });
    square.set_valign(gtk::Align::Center);
    let label = theme::label(font::SMALL, "Connecting…", "value");
    let down = icons::icon(Icon::Down, 9, tone::VALUE);
    row.append(&square);
    row.append(&label);
    row.append(&down);
    button.set_child(Some(&row));
    lamp::key_sounds(&button, true);

    let popover = gtk::Popover::new();
    let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
    list.set_margin_top(6);
    list.set_margin_bottom(6);
    popover.set_child(Some(&list));
    button.set_popover(Some(&popover));
    // The chains are read each time the menu opens.
    let (weak, list_ref, pop) = (Rc::downgrade(model), list.clone(), popover.clone());
    popover.connect_show(move |_| {
        let Some(model) = weak.upgrade() else { return };
        crate::page::load::clear(&list_ref);
        let heading = theme::label(font::SMALL, "Chains on this computer", "label");
        heading.add_css_class("menu-heading");
        heading.set_margin_start(10);
        heading.set_margin_bottom(4);
        list_ref.append(&heading);
        let current = model.current_chain();
        for chain in model.known_chain_list() {
            let title = chain_label(&chain);
            let item = gtk::Button::new();
            item.add_css_class("flat");
            item.add_css_class("menu-row");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let check = icons::icon(Icon::Check, 11, tone::VALUE);
            check.set_opacity(if chain.chain == current { 1.0 } else { 0.0 });
            line.append(&check);
            line.append(&gtk::Label::new(Some(&title)));
            item.set_child(Some(&line));
            let (weak, pop) = (Rc::downgrade(&model), pop.clone());
            item.connect_clicked(move |_| {
                pop.popdown();
                if let Some(model) = weak.upgrade() {
                    model.switch_chain(&chain);
                }
            });
            list_ref.append(&item);
        }
        list_ref.append(&crate::widgets::display::rule());
        let settings = gtk::Button::with_label("Connection Settings…");
        settings.add_css_class("flat");
        settings.add_css_class("menu-row");
        let (open, pop) = (open_settings.clone(), pop.clone());
        settings.connect_clicked(move |_| {
            pop.popdown();
            open();
        });
        list_ref.append(&settings);
    });

    let update = {
        let (label, square, button) = (label.clone(), square.clone(), button.clone());
        move |model: &Rc<AppModel>| {
            let state = model.state();
            let color = if state.status.is_some() && state.status_error.is_none() {
                tone::GOOD
            } else if state.status_error.is_none() {
                tone::LABEL
            } else {
                tone::ALERT
            };
            square_color.set(color);
            square.queue_draw();
            let text = match &state.status {
                None => if state.status_error.is_none() { "Connecting…".to_string() } else { "Not connected".to_string() },
                Some(status) => {
                    let source = if status.source == api::DataSource::PublicApi { "Public API" } else { "Local node" };
                    let network = if status.testnet { " testnet" } else { "" };
                    format!("{}{} · {} · {}", status.name, network, format::integer(status.blocks), source)
                }
            };
            theme::set_label(&label, font::SMALL, &text);
            let help = state.status_error.clone().unwrap_or_else(|| "Connection status. Click to select a chain.".into());
            button.set_tooltip_text(Some(&help));
        }
    };
    update(model);
    model.watch(&[Topic::Status, Topic::Connection], &button, update);
    button
}

/// PBaaS chains are stored under a hex folder name; show a short form of it.
fn chain_label(chain: &api::KnownChain) -> String {
    if chain.chain.chars().count() == 40 {
        format!("PBaaS chain {}…", &chain.chain[..8])
    } else {
        chain.chain.clone()
    }
}
