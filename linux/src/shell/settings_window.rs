//! Connection settings (`SettingsView.swift`). Changes to the connection apply when the
//! person presses Apply. Display settings apply at once.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use crate::bridge;
use crate::format;
use crate::model::AppModel;
use crate::page::{note, paragraph};
use crate::settings::{self, key};
use crate::widgets::{display, lamp};

fn section(title: &str) -> (gtk::Box, gtk::Box) {
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 10);
    outer.append(&display::rule());
    outer.append(&display::heading(title));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    outer.append(&body);
    (outer, body)
}

fn field_row(name: &str, entry: &gtk::Entry) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(name));
    label.set_xalign(0.0);
    label.set_size_request(190, -1);
    row.append(&label);
    entry.add_css_class("display-field");
    entry.set_hexpand(true);
    row.append(entry);
    row
}

fn text_entry(setting: &'static str, placeholder: &str, default: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_placeholder_text(Some(placeholder));
    entry.set_text(&settings::string(setting).unwrap_or_else(|| default.to_string()));
    entry.connect_changed(move |entry| settings::set_string(setting, &entry.text()));
    entry
}

fn check(model: &Rc<AppModel>, title: &str, setting: &'static str, default: bool, display_setting: bool) -> gtk::CheckButton {
    let weak = Rc::downgrade(model);
    crate::page::check_box(title, settings::bool(setting, default), move |on| {
        if let Some(model) = weak.upgrade() {
            if display_setting {
                model.set_display(setting, on);
            } else {
                settings::set_bool(setting, on);
            }
        }
    })
}

pub fn window(model: &Rc<AppModel>, parent: &impl IsA<gtk::Window>) -> gtk::Window {
    let window = gtk::Window::new();
    window.set_title(Some("Settings"));
    window.set_transient_for(Some(parent));
    window.set_default_size(560, 720);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 20);
    column.set_margin_top(20);
    column.set_margin_bottom(20);
    column.set_margin_start(20);
    column.set_margin_end(20);

    // Local node.
    let (node, body) = section("Local node");
    body.append(&field_row("Chain", &text_entry(key::CHAIN, "VRSC", "VRSC")));
    body.append(&check(model, "Testnet", key::TESTNET, false, false));
    let conf = text_entry(key::CONFIG_FILE, "Standard location", "");
    let conf_row = field_row("Node configuration file", &conf);
    let choose = {
        let (conf, window) = (conf.clone(), window.downgrade());
        lamp::button("Choose…", move || {
            let dialog = gtk::FileDialog::new();
            dialog.set_title("Select the .conf file of the Verus node");
            let conf = conf.clone();
            dialog.open(window.upgrade().as_ref(), gio::Cancellable::NONE, move |result| {
                if let Ok(file) = result
                    && let Some(path) = file.path()
                {
                    conf.set_text(&path.to_string_lossy());
                }
            });
        })
    };
    conf_row.append(&choose);
    body.append(&conf_row);
    let summary = paragraph(&daemon_summary(model), &["label"]);
    body.append(&summary);
    column.append(&node);

    // Public API.
    let (api, body) = section("Public API");
    body.append(&field_row("API address", &text_entry(key::API_URL, key::DEFAULT_API, key::DEFAULT_API)));
    body.append(&check(model, "Allow this API without asking", key::API_ENABLED, false, false));
    body.append(&paragraph(
        "Sources lists this API. When the local node cannot answer a page that you open, the app asks before it sends the request to the API. Turn on the setting if this computer has no node. The app then uses the API without asking. The operator of the API can see the addresses and identities that you look up.",
        &["label"],
    ));
    column.append(&api);

    // Display.
    let (shown, body) = section("Display");
    body.append(&check(model, "Show names in place of identity and currency IDs", key::SHOWS_NAMES, true, true));
    body.append(&check(model, "Show dates for block heights and times", key::SHOWS_DATES, false, true));
    body.append(&check(model, "Show times in UTC", key::USE_UTC, false, true));
    body.append(&check(model, "Play a sound for keys and buttons", key::INTERFACE_SOUNDS, true, true));
    body.append(&paragraph("The settings apply at the latest to pages that you open after the change.", &["label"]));
    column.append(&shown);

    // Cache.
    let (cache_section, body) = section("Cache");
    let info_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    body.append(&info_box);
    let message = paragraph("", &["label"]);
    message.set_visible(false);
    let fill_info = {
        let (model, info_box) = (Rc::downgrade(model), info_box.clone());
        move || {
            crate::page::load::clear(&info_box);
            let Some(info) = model.upgrade().and_then(|m| m.explorer()).map(|e| e.cache_info()) else { return };
            for (name, value) in [("Size", format::bytes(info.bytes)), ("File", info.path.clone())] {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                let label = gtk::Label::new(Some(name));
                label.set_size_request(190, -1);
                label.set_xalign(0.0);
                row.append(&label);
                let shown = gtk::Label::new(Some(&value));
                shown.set_selectable(true);
                shown.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                shown.set_xalign(0.0);
                row.append(&shown);
                info_box.append(&row);
            }
            if let Some(problem) = &info.problem {
                info_box.append(&note(problem, true));
            }
        }
    };
    let fill_info = Rc::new(fill_info);
    fill_info();
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let choose_folder = {
        let (model, window, message) = (Rc::downgrade(model), window.downgrade(), message.clone());
        lamp::button("Choose Folder…", move || {
            let dialog = gtk::FileDialog::new();
            dialog.set_title("Select a folder for the cache. The app stores one file for each network in it.");
            let (model, message) = (model.clone(), message.clone());
            dialog.select_folder(window.upgrade().as_ref(), gio::Cancellable::NONE, move |result| {
                if let Ok(folder) = result
                    && let Some(path) = folder.path()
                {
                    settings::set_string(key::CACHE_DIR, &path.to_string_lossy());
                    message.set_visible(false);
                    if let Some(model) = model.upgrade() {
                        model.connect();
                    }
                }
            });
        })
    };
    keys.append(&choose_folder);
    let default_folder = {
        let model = Rc::downgrade(model);
        lamp::button("Use Default Folder", move || {
            settings::set_string(key::CACHE_DIR, "");
            if let Some(model) = model.upgrade() {
                model.connect();
            }
        })
    };
    default_folder.set_sensitive(!settings::string(key::CACHE_DIR).unwrap_or_default().is_empty());
    keys.append(&default_folder);
    let show_folder = {
        let model = Rc::downgrade(model);
        lamp::button("Show in Files", move || {
            if let Some(info) = model.upgrade().and_then(|m| m.explorer()).map(|e| e.cache_info())
                && let Some(folder) = std::path::Path::new(&info.path).parent()
            {
                let file = gio::File::for_path(folder);
                gtk::FileLauncher::new(Some(&file)).launch(gtk::Window::NONE, gio::Cancellable::NONE, |_| {});
            }
        })
    };
    keys.append(&show_folder);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    keys.append(&spacer);
    let clear = {
        let (model, window, message, fill_info) = (Rc::downgrade(model), window.downgrade(), message.clone(), fill_info.clone());
        lamp::button("Clear Cache…", move || {
            let Some(parent) = window.upgrade() else { return };
            let (model, message, fill_info) = (model.clone(), message.clone(), fill_info.clone());
            super::dialogs::confirm(
                &parent,
                "Clear the cache?",
                "The app deletes the cached data and downloads it again when necessary. Bookmarks and settings do not change.",
                "Clear Cache",
                move || {
                    let Some(explorer) = model.upgrade().and_then(|m| m.explorer()) else { return };
                    let text = match explorer.clear_cache() {
                        Ok(()) => "Cache cleared.".to_string(),
                        Err(error) => bridge::describe(&error),
                    };
                    message.set_text(&text);
                    message.set_visible(true);
                    fill_info();
                },
            );
        })
    };
    clear.set_sensitive(model.explorer().is_some());
    keys.append(&clear);
    body.append(&keys);
    body.append(&message);
    body.append(&paragraph(
        "The cache keeps every answer from your sources, with the source and the time. The app uses it to show stored data when no source answers, and for block history. It does not contain bookmarks or settings. The cache grows with use. Choose a folder on another disk to keep it off this computer's internal disk. A new folder starts a new cache. The previous cache stays where it is.",
        &["label"],
    ));
    column.append(&cache_section);

    let apply = {
        let model = Rc::downgrade(model);
        lamp::button("Apply", move || {
            if let Some(model) = model.upgrade() {
                model.connect();
            }
        })
    };
    lamp::set_chosen(&apply, true);
    apply.set_halign(gtk::Align::End);
    column.append(&apply);
    window.set_default_widget(Some(&apply));

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroller.set_child(Some(&column));
    window.set_child(Some(&scroller));
    let _ = glib::Propagation::Proceed;
    window
}

fn daemon_summary(model: &Rc<AppModel>) -> String {
    let Some(connection) = model.state().connection.clone() else { return "Not connected.".into() };
    let file = connection.config_file.clone().unwrap_or_else(|| "none".into());
    if connection.config_found {
        let credentials = if connection.has_credentials { "" } else { " The file has no RPC credentials." };
        format!("The app uses {file} and the node at {}.{credentials}", connection.daemon_url)
    } else {
        format!("The app found no configuration file at {file}. It tries {}.", connection.daemon_url)
    }
}
