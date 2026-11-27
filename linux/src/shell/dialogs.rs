//! The dialogs of the window (`RootView` alerts and `SourcePrompts.swift`): start-up
//! arguments, search errors, bookmarks, the estimate entry and the permission to use a
//! public API. A dialog is a small window of the display. Return answers it with the
//! default key and Esc cancels it.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::api;
use crate::model::{AppModel, Topic};
use crate::page::{note, paragraph};
use crate::widgets::{display, lamp};

pub struct Dialog {
    pub window: gtk::Window,
    pub body: gtk::Box,
    pub keys: gtk::Box,
}

/// A dialog with a title and a message. Add keys with [`Dialog::key`].
pub fn dialog(parent: &impl IsA<gtk::Window>, title: &str, message: Option<&str>) -> Dialog {
    let window = gtk::Window::new();
    window.set_transient_for(Some(parent));
    window.set_modal(true);
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_default_size(420, -1);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.add_css_class("dialog-body");
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("dialog-title");
    heading.set_xalign(0.0);
    heading.set_wrap(true);
    body.append(&heading);
    if let Some(message) = message {
        let text = paragraph(message, &["value"]);
        text.set_max_width_chars(60);
        body.append(&text);
    }
    let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    keys.set_halign(gtk::Align::End);
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 14);
    outer.append(&body);
    let frame = display::panel(&outer, 0);
    outer.append(&keys);
    keys.set_margin_bottom(20);
    keys.set_margin_end(20);
    window.set_child(Some(&frame));
    let escape = gtk::EventControllerKey::new();
    let weak = window.downgrade();
    escape.connect_key_pressed(move |_, key, _, _| {
        if key == gtk::gdk::Key::Escape {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(escape);
    Dialog { window, body, keys }
}

impl Dialog {
    /// A key of the dialog. The action runs, then the dialog closes.
    pub fn key(&self, title: &str, default: bool, action: impl Fn() + 'static) -> gtk::Button {
        let weak = self.window.downgrade();
        let key = lamp::button(title, move || {
            action();
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        });
        if default {
            self.window.set_default_widget(Some(&key));
            lamp::set_chosen(&key, true);
        }
        self.keys.append(&key);
        key
    }

    pub fn show(&self) {
        self.window.present();
    }
}

/// An alert with one key, OK.
pub fn alert(parent: &impl IsA<gtk::Window>, title: &str, message: &str, done: impl Fn() + 'static) {
    let dialog = dialog(parent, title, Some(message));
    dialog.key("OK", true, done);
    dialog.show();
}

/// A dialog with a text field. `confirm` gets the text.
pub fn ask_text(
    parent: &impl IsA<gtk::Window>,
    title: &str,
    message: &str,
    placeholder: &str,
    confirm_title: &str,
    confirm: impl Fn(String) + 'static,
) {
    let dialog = dialog(parent, title, Some(message));
    let entry = gtk::Entry::new();
    entry.add_css_class("display-field");
    entry.set_placeholder_text(Some(placeholder));
    dialog.body.append(&entry);
    dialog.key("Cancel", false, || {});
    let field = entry.clone();
    let key = dialog.key(confirm_title, true, move || confirm(field.text().to_string()));
    entry.connect_activate(move |_| key.emit_clicked());
    dialog.show();
    entry.grab_focus();
}

/// A question before a change. `title` of the key does the change.
pub fn confirm(parent: &impl IsA<gtk::Window>, title: &str, message: &str, key: &str, action: impl Fn() + 'static) {
    let dialog = dialog(parent, title, Some(message));
    dialog.key("Cancel", true, || {});
    dialog.key(key, false, action);
    dialog.show();
}

/// Shows the dialog for a primary request that the local node cannot answer, when a
/// disabled public API could (`PermissionPrompt`). One dialog at a time, oldest first.
pub fn watch_permissions(model: &Rc<AppModel>, window: &gtk::ApplicationWindow) {
    let open: Rc<std::cell::Cell<Option<u64>>> = Rc::default();
    let weak_window = window.downgrade();
    model.watch(&[Topic::Permission], window, move |model| {
        let Some(window) = weak_window.upgrade() else { return };
        let Some(request) = model.state().permission_requests.front().cloned() else { return };
        if open.get() == Some(request.id) {
            return;
        }
        open.set(Some(request.id));
        permission_dialog(model, &window, request, open.clone());
    });
}

fn chain_name(chain: &str) -> String {
    if chain.chars().count() == 40 { format!("PBaaS chain {}…", &chain[..8]) } else { chain.to_string() }
}

/// The terminal's API permission modal: the local failure, the disabled APIs, and Retry,
/// Enable for this session, or Cancel.
fn permission_dialog(
    model: &Rc<AppModel>,
    window: &gtk::ApplicationWindow,
    request: api::PermissionRequest,
    open: Rc<std::cell::Cell<Option<u64>>>,
) {
    let dialog = dialog(window, "Use a public API for this request?", None);
    dialog.window.set_default_size(520, -1);
    dialog.body.append(&paragraph(
        &format!("The {} node cannot answer the request {}.", chain_name(&request.chain), request.method),
        &["value"],
    ));
    dialog.body.append(&note(&request.failure, true));
    let choice = Rc::new(std::cell::Cell::new(0u32));
    if request.endpoints.len() == 1 {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.append(&display::name_text("API"));
        row.append(&gtk::Label::new(Some(&request.endpoints[0])));
        dialog.body.append(&row);
    } else {
        let mut first: Option<gtk::CheckButton> = None;
        for (index, endpoint) in request.endpoints.iter().enumerate() {
            let radio = gtk::CheckButton::with_label(endpoint);
            if let Some(first) = &first {
                radio.set_group(Some(first));
            } else {
                radio.set_active(true);
                first = Some(radio.clone());
            }
            let choice = choice.clone();
            radio.connect_toggled(move |radio| {
                if radio.is_active() {
                    choice.set(index as u32);
                }
            });
            dialog.body.append(&radio);
        }
    }
    dialog.body.append(&paragraph(
        "If you enable the API, the app sends this request and later requests for this chain to it until the app quits. The operator of the API can see what you look up. Sources can save the setting.",
        &["secondary", "callout"],
    ));
    let answered = Rc::new(std::cell::Cell::new(false));
    let answer = {
        let (model, answered) = (Rc::downgrade(model), answered.clone());
        move |value: api::PermissionAnswer| {
            answered.set(true);
            if let Some(model) = model.upgrade() {
                model.answer_permission(value);
            }
        }
    };
    let a = answer.clone();
    dialog.key("Cancel", false, move || a(api::PermissionAnswer::Cancel));
    let a = answer.clone();
    dialog.key("Try the Node Again", false, move || a(api::PermissionAnswer::Retry));
    let a = answer.clone();
    let c = choice.clone();
    let enable = dialog.key("Enable API", true, move || a(api::PermissionAnswer::Enable { index: c.get() }));
    enable.set_sensitive(!request.endpoints.is_empty());
    // A dialog closed without a choice cancels the request.
    let a = answer.clone();
    let id = request.id;
    dialog.window.connect_close_request(move |_| {
        if open.get() == Some(id) {
            open.set(None);
        }
        if !answered.get() {
            a(api::PermissionAnswer::Cancel);
        }
        glib::Propagation::Proceed
    });
    dialog.show();
}
