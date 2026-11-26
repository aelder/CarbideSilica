//! The keys that work in the main window when no text field is in use (`KeyLayer` in
//! `PageCursor.swift`). They are the keys of the terminal: j and k move the cursor, Return
//! opens, Esc goes back, one letter goes to a place, and the function keys start the
//! investigations.

use std::rc::Rc;

use gtk::gdk::{Key, ModifierType};
use gtk::glib;
use gtk::prelude::*;

use super::Shell;
use crate::model::Place;
use crate::settings::key;

/// True when the focus is in a field that the person types in.
fn typing(window: &gtk::Window) -> bool {
    let Some(focus) = GtkWindowExt::focus(window) else { return false };
    if let Some(text) = focus.downcast_ref::<gtk::Text>() {
        return text.is_editable();
    }
    if let Some(view) = focus.downcast_ref::<gtk::TextView>() {
        return view.is_editable();
    }
    focus.is::<gtk::Editable>() && focus.downcast_ref::<gtk::Editable>().is_some_and(|e| e.is_editable())
}

/// True when a label with selected text has the focus: Ctrl-C then copies that text.
fn has_text_selection(window: &gtk::Window) -> bool {
    GtkWindowExt::focus(window)
        .and_then(|f| f.downcast::<gtk::Label>().ok())
        .is_some_and(|label| label.selection_bounds().is_some())
}

pub fn install(shell: &Rc<Shell>) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(shell);
    keys.connect_key_pressed(move |_, keyval, _, modifiers| {
        let Some(shell) = weak.upgrade() else { return glib::Propagation::Proceed };
        if handle(&shell, keyval, modifiers) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
    });
    shell.window.add_controller(keys);
}

/// Returns true when the key is one of the app's keys here and it was used.
fn handle(shell: &Rc<Shell>, keyval: Key, modifiers: ModifierType) -> bool {
    let window: &gtk::Window = shell.window.upcast_ref();
    if shell.palette_shown() {
        return false;
    }
    let page = shell.navigator.current();
    let cursor = page.as_ref().map(|p| p.cursor.clone()).filter(|c| c.enabled.get());
    let control = modifiers.contains(ModifierType::CONTROL_MASK);
    let alt = modifiers.contains(ModifierType::ALT_MASK);
    let shift = modifiers.contains(ModifierType::SHIFT_MASK);

    // Ctrl-C with no text selected copies the value under the cursor.
    if control && !alt && !shift && (keyval == Key::c || keyval == Key::C) && !typing(window) && !has_text_selection(window) {
        if let Some(text) = cursor.as_ref().and_then(|c| c.selected_copy()) {
            window.clipboard().set_text(&text);
            return true;
        }
        return false;
    }
    // A field that the person types in keeps its keys. Esc in it is handled by the field.
    if control || alt || typing(window) {
        return false;
    }
    let model = &shell.model;
    let suspended = cursor.as_ref().is_some_and(|c| c.suspended.get());
    let has_stops = cursor.as_ref().is_some_and(|c| c.has_stops());
    let hooks = page.as_ref().map(|p| p.hooks.borrow());
    let run = |action: Option<crate::page::Action>| -> bool {
        match action {
            Some(action) => {
                action();
                true
            }
            None => false,
        }
    };
    let investigations = hooks.as_ref().map(|h| h.investigations.clone()).unwrap_or_default();
    let adjacent = hooks.as_ref().and_then(|h| h.adjacent.clone());
    let toggle_json = hooks.as_ref().and_then(|h| h.toggle_json.clone());
    let save_json = hooks.as_ref().and_then(|h| h.save_json.clone());
    let page_key = keyval.to_unicode().and_then(|ch| hooks.as_ref().and_then(|h| h.keys.get(&ch.to_string()).map(|k| k.1.clone())));
    drop(hooks);

    // Keys with no character.
    match keyval {
        Key::Escape => return shell.navigator.go_back(),
        // A part that takes the arrow keys keeps Left, as it keeps Right.
        Key::Left if suspended => return false,
        Key::BackSpace | Key::Left => {
            if model.state().path.is_empty() {
                return false;
            }
            shell.navigator.go_back();
            return true;
        }
        Key::Return | Key::KP_Enter | Key::Right => {
            if suspended {
                return false;
            }
            return cursor.is_some_and(|c| c.open_selected());
        }
        Key::Down | Key::Up | Key::Page_Down | Key::Page_Up | Key::Home | Key::End if suspended => return false,
        Key::Down | Key::Up => {
            if !has_stops {
                return false;
            }
            cursor.unwrap().move_by(if keyval == Key::Down { 1 } else { -1 });
            return true;
        }
        Key::Page_Down | Key::Page_Up => {
            if !has_stops {
                return false;
            }
            cursor.unwrap().move_by(if keyval == Key::Page_Down { 10 } else { -10 });
            return true;
        }
        Key::Home | Key::End => {
            if !has_stops {
                return false;
            }
            cursor.unwrap().go_to_end(keyval == Key::End);
            return true;
        }
        // Tab: the raw JSON of the page. On a page with no JSON, the next card.
        Key::Tab | Key::ISO_Left_Tab => {
            if let Some(toggle) = toggle_json {
                toggle();
                return true;
            }
            if !has_stops {
                return false;
            }
            cursor.unwrap().move_group(!(shift || keyval == Key::ISO_Left_Tab));
            return true;
        }
        Key::F1 => {
            shell.show_keys();
            return true;
        }
        Key::F2 => {
            model.show(Place::Sources);
            return true;
        }
        Key::F3 => return run(investigations.protocol_links),
        Key::F4 => return run(investigations.witnesses),
        Key::F5 => {
            shell.refresh();
            return true;
        }
        Key::F6 => {
            model.show(Place::Bookmarks);
            return true;
        }
        Key::F7 => return run(investigations.inclusions),
        Key::F8 => return run(investigations.follow_transfer),
        Key::F9 => {
            model.show(Place::Charts);
            return true;
        }
        Key::F10 => {
            shell.estimate();
            return true;
        }
        _ => {}
    }

    // The keys of the open page come first.
    if let Some(action) = page_key {
        action();
        return true;
    }

    let Some(typed) = keyval.to_unicode() else { return false };
    match typed {
        'j' | 'k' => {
            if !has_stops {
                return false;
            }
            cursor.unwrap().move_by(if typed == 'j' { 1 } else { -1 });
            true
        }
        'l' => cursor.is_some_and(|c| c.open_selected()),
        'h' => {
            if model.state().path.is_empty() {
                return false;
            }
            shell.navigator.go_back();
            true
        }
        'y' => match cursor.and_then(|c| c.selected_copy()) {
            Some(text) => {
                window.clipboard().set_text(&text);
                true
            }
            None => false,
        },
        's' => run(save_json),
        'd' => {
            model.show(Place::Dashboard);
            true
        }
        'm' => {
            model.show(Place::Mempool);
            true
        }
        'o' => {
            model.show(Place::Marketplace);
            true
        }
        'c' => {
            model.show(Place::Currencies);
            true
        }
        '`' => {
            model.show(Place::Node);
            true
        }
        '/' => {
            model.focus_search();
            true
        }
        '?' => {
            shell.show_keys();
            true
        }
        'n' | ']' => match adjacent.and_then(|a| a.next) {
            Some(next) => {
                next();
                true
            }
            None => false,
        },
        'p' | '[' => match adjacent.and_then(|a| a.previous) {
            Some(previous) => {
                previous();
                true
            }
            None => false,
        },
        'r' => {
            shell.refresh();
            true
        }
        'v' => run(investigations.shared_authorities),
        'B' => {
            if !model.state().path.last().is_some_and(crate::route::is_bookmarkable) {
                return false;
            }
            shell.bookmark_current();
            true
        }
        'I' => {
            model.toggle_display(key::SHOWS_NAMES, true);
            true
        }
        'D' => {
            model.toggle_display(key::SHOWS_DATES, false);
            true
        }
        _ => false,
    }
}
