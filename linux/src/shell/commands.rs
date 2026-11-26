//! The app's menu commands and shortcuts (`ShellCommands.swift`). Each function of the
//! terminal has a menu item. The Command key of macOS is Ctrl on Linux; the full mapping is
//! in [`SHORTCUTS`] and in `linux/PORT.md`. `keyboard_reference.rs` lists the same keys.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib};

use super::Shell;
use crate::model::{Place, Topic};
use crate::pages::PLACES;
use crate::route;
use crate::settings::key;

/// Each shortcut: the action, the GTK accelerator, and the macOS key that it replaces.
pub const SHORTCUTS: &[(&str, &str, &str)] = &[
    ("app.find", "<Control>f", "⌘F"),
    ("app.names", "<Control><Shift>i", "⇧⌘I"),
    ("app.dates", "<Control><Shift>d", "⇧⌘D"),
    ("app.utc", "<Control><Alt>u", "⌥⌘U"),
    ("app.stream-blocks", "<Control><Alt>s", "⌥⌘S"),
    ("app.mempool-list", "<Control><Alt>l", "⌥⌘L"),
    ("app.peer-map", "<Control><Alt>g", "⌥⌘G"),
    ("app.place-1", "<Control>1", "⌘1"),
    ("app.place-2", "<Control>2", "⌘2"),
    ("app.place-3", "<Control>3", "⌘3"),
    ("app.place-4", "<Control>4", "⌘4"),
    ("app.place-5", "<Control>5", "⌘5"),
    ("app.place-6", "<Control>6", "⌘6"),
    ("app.place-7", "<Control>7", "⌘7"),
    ("app.place-8", "<Control>8", "⌘8"),
    ("app.back", "<Control>bracketleft", "⌘["),
    ("app.forward", "<Control>bracketright", "⌘]"),
    ("app.previous", "<Control><Alt>bracketleft", "⌥⌘["),
    ("app.next", "<Control><Alt>bracketright", "⌥⌘]"),
    ("app.palette", "<Control>k", "⌘K"),
    ("app.estimate", "<Control><Shift>e", "⇧⌘E"),
    ("app.refresh", "<Control>r", "⌘R"),
    ("app.follow-transfer", "<Control><Shift>t", "⌃⌘T"),
    ("app.witnesses", "<Control><Shift>w", "⌃⌘W"),
    ("app.protocol-links", "<Control><Shift>l", "⌃⌘L"),
    ("app.block-history", "<Control><Shift>b", "⌃⌘B"),
    ("app.shared-authorities", "<Control><Shift>a", "⌃⌘A"),
    ("app.bookmark", "<Control>d", "⌘D"),
    ("app.add-node", "<Control><Alt>n", "⌥⌘N"),
    ("app.ping-all", "<Control><Alt>p", "⌥⌘P"),
    ("app.keyboard", "<Control>slash", "⌘/"),
    ("app.settings", "<Control>comma", "⌘,"),
    ("app.quit", "<Control>q", "⌘Q"),
];

/// The Linux keys of an action, in words: "Ctrl+Shift+I".
pub fn keys_of(action: &str) -> String {
    SHORTCUTS
        .iter()
        .find(|(name, _, _)| *name == action)
        .map(|(_, accel, _)| words(accel))
        .unwrap_or_default()
}

/// An accelerator in words: "<Control><Alt>bracketleft" -> "Ctrl+Alt+[".
pub fn words(accel: &str) -> String {
    let mut parts = Vec::new();
    let mut rest = accel;
    while let Some(start) = rest.find('<') {
        let end = rest.find('>').unwrap_or(rest.len());
        parts.push(match &rest[start + 1..end] {
            "Control" | "Primary" => "Ctrl".to_string(),
            other => other.to_string(),
        });
        rest = &rest[end + 1..];
    }
    let key = match rest {
        "bracketleft" => "[".to_string(),
        "bracketright" => "]".to_string(),
        "slash" => "/".to_string(),
        "comma" => ",".to_string(),
        other => other.to_uppercase(),
    };
    parts.push(key);
    parts.join("+")
}

fn add(app: &gtk::Application, name: &str, run: impl Fn() + 'static) -> gio::SimpleAction {
    let action = gio::SimpleAction::new(name, None);
    action.connect_activate(move |_, _| run());
    app.add_action(&action);
    action
}

fn add_toggle(app: &gtk::Application, name: &str, on: bool, set: impl Fn(bool) + 'static) -> gio::SimpleAction {
    let action = gio::SimpleAction::new_stateful(name, None, &on.to_variant());
    action.connect_change_state(move |action, value| {
        if let Some(on) = value.and_then(|v| v.get::<bool>()) {
            action.set_state(&on.to_variant());
            set(on);
        }
    });
    app.add_action(&action);
    action
}

/// Adds the actions and the shortcuts, and returns the menus.
pub fn install(app: &gtk::Application, shell: &Rc<Shell>) -> gio::Menu {
    let weak = Rc::downgrade(shell);
    let with = move |run: fn(&Rc<Shell>)| {
        let weak = weak.clone();
        move || {
            if let Some(shell) = weak.upgrade() {
                run(&shell);
            }
        }
    };

    add(app, "find", with(|s| s.model.focus_search()));
    let names = add_toggle(app, "names", crate::display::shows_names(), {
        let model = Rc::downgrade(&shell.model);
        move |on| if let Some(m) = model.upgrade() { m.set_display(key::SHOWS_NAMES, on) }
    });
    let dates = add_toggle(app, "dates", crate::display::shows_dates(), {
        let model = Rc::downgrade(&shell.model);
        move |on| if let Some(m) = model.upgrade() { m.set_display(key::SHOWS_DATES, on) }
    });
    let utc = add_toggle(app, "utc", crate::display::uses_utc(), {
        let model = Rc::downgrade(&shell.model);
        move |on| if let Some(m) = model.upgrade() { m.set_display(key::USE_UTC, on) }
    });
    // The toggles of the visible place.
    let page_toggle = |name: &str, pick: fn(&crate::page::PlaceActions) -> Option<crate::page::Toggle>| {
        let weak = Rc::downgrade(shell);
        add_toggle(app, name, false, move |on| {
            if let Some(shell) = weak.upgrade()
                && let Some(page) = shell.navigator.current()
                && let Some(toggle) = pick(&page.hooks.borrow().place)
            {
                (toggle.set)(on);
            }
        })
    };
    let stream = page_toggle("stream-blocks", |p| p.stream_blocks.clone());
    let mempool_list = page_toggle("mempool-list", |p| p.mempool_as_list.clone());
    let peer_map = page_toggle("peer-map", |p| p.show_map.clone());

    for spec in PLACES.iter() {
        let place = spec.place;
        let weak = Rc::downgrade(shell);
        add(app, &format!("place-{}", spec.digit), move || {
            if let Some(shell) = weak.upgrade() {
                shell.model.show(place);
            }
        });
    }
    let back = add(app, "back", with(|s| {
        s.navigator.go_back();
    }));
    let forward = add(app, "forward", with(|s| s.navigator.go_forward()));
    let previous = add(app, "previous", with(|s| s.step(false)));
    let next = add(app, "next", with(|s| s.step(true)));
    add(app, "palette", with(|s| s.open_palette()));
    add(app, "estimate", with(|s| s.estimate()));
    add(app, "refresh", with(|s| s.refresh()));
    let investigate = |name: &str, pick: fn(&crate::page::Investigations) -> Option<crate::page::Action>| {
        let weak = Rc::downgrade(shell);
        add(app, name, move || {
            if let Some(shell) = weak.upgrade()
                && let Some(page) = shell.navigator.current()
            {
                let run = pick(&page.hooks.borrow().investigations);
                if let Some(run) = run {
                    run();
                }
            }
        })
    };
    let follow = investigate("follow-transfer", |i| i.follow_transfer.clone());
    let witnesses = investigate("witnesses", |i| i.witnesses.clone());
    let links = investigate("protocol-links", |i| i.protocol_links.clone());
    let history = investigate("block-history", |i| i.inclusions.clone());
    let shared = investigate("shared-authorities", |i| i.shared_authorities.clone());
    let bookmark = add(app, "bookmark", with(|s| s.bookmark_current()));
    let place_action = |name: &str, pick: fn(&crate::page::PlaceActions) -> Option<crate::page::Action>| {
        let weak = Rc::downgrade(shell);
        add(app, name, move || {
            if let Some(shell) = weak.upgrade()
                && let Some(page) = shell.navigator.current()
            {
                let run = pick(&page.hooks.borrow().place);
                if let Some(run) = run {
                    run();
                }
            }
        })
    };
    let add_node = place_action("add-node", |p| p.add_node.clone());
    let ping_all = place_action("ping-all", |p| p.ping_all.clone());
    let clear_bans = place_action("clear-bans", |p| p.clear_bans.clone());
    add(app, "keyboard", with(|s| s.show_keys()));
    add(app, "settings", with(|s| s.show_settings()));
    let app_weak = app.downgrade();
    add(app, "quit", move || {
        if let Some(app) = app_weak.upgrade() {
            app.quit();
        }
    });

    for (name, accel, _) in SHORTCUTS {
        app.set_accels_for_action(name, &[accel]);
    }

    // The enabled state follows the open page, as the menus of the Swift app.
    let update = {
        let weak = Rc::downgrade(shell);
        move |_: &Rc<crate::model::AppModel>| {
            let Some(shell) = weak.upgrade() else { return };
            let model = &shell.model;
            let page = shell.navigator.current();
            let hooks = page.as_ref().map(|p| p.hooks.borrow());
            let path_empty = model.state().path.is_empty();
            back.set_enabled(!(path_empty && model.place() == Place::Dashboard));
            forward.set_enabled(shell.navigator.can_go_forward());
            let block_open = matches!(model.state().path.last(), Some(crate::bridge::api::Route::Block { .. }));
            let adjacent = hooks.as_ref().and_then(|h| h.adjacent.clone());
            previous.set_enabled(adjacent.as_ref().map(|a| a.previous.is_some()).unwrap_or(block_open));
            next.set_enabled(adjacent.as_ref().map(|a| a.next.is_some()).unwrap_or(block_open));
            let inv = hooks.as_ref().map(|h| h.investigations.clone()).unwrap_or_default();
            follow.set_enabled(inv.follow_transfer.is_some());
            witnesses.set_enabled(inv.witnesses.is_some());
            links.set_enabled(inv.protocol_links.is_some());
            history.set_enabled(inv.inclusions.is_some());
            shared.set_enabled(inv.shared_authorities.is_some());
            bookmark.set_enabled(model.state().path.last().is_some_and(route::is_bookmarkable));
            let place = hooks.as_ref().map(|h| h.place.clone()).unwrap_or_default();
            add_node.set_enabled(place.add_node.is_some());
            ping_all.set_enabled(place.ping_all.is_some());
            clear_bans.set_enabled(place.clear_bans.is_some());
            for (action, toggle) in [(&stream, &place.stream_blocks), (&mempool_list, &place.mempool_as_list), (&peer_map, &place.show_map)] {
                action.set_enabled(toggle.is_some());
                if let Some(toggle) = toggle {
                    action.set_state(&(toggle.get)().to_variant());
                }
            }
            names.set_state(&crate::display::shows_names().to_variant());
            dates.set_state(&crate::display::shows_dates().to_variant());
            utc.set_state(&crate::display::uses_utc().to_variant());
        }
    };
    let owner = shell.window.clone();
    update(&shell.model);
    shell.model.watch(&[Topic::Path, Topic::Display, Topic::Status, Topic::Node], &owner, update);

    menus()
}

/// The menus of the app, as the menu bar of the Swift app: View, Go, Investigate, Peers,
/// File and Help.
fn menus() -> gio::Menu {
    let root = gio::Menu::new();

    let view = gio::Menu::new();
    view.append(Some("Show Names for IDs"), Some("app.names"));
    view.append(Some("Show Dates for Heights"), Some("app.dates"));
    view.append(Some("Show Times in UTC"), Some("app.utc"));
    let place_toggles = gio::Menu::new();
    place_toggles.append(Some("Stream Blocks"), Some("app.stream-blocks"));
    place_toggles.append(Some("Show Mempool as List"), Some("app.mempool-list"));
    place_toggles.append(Some("Show Peer Map"), Some("app.peer-map"));
    view.append_section(None, &place_toggles);
    root.append_submenu(Some("View"), &view);

    let go = gio::Menu::new();
    let places = gio::Menu::new();
    for spec in PLACES.iter() {
        places.append(Some(spec.title), Some(&format!("app.place-{}", spec.digit)));
    }
    go.append_section(None, &places);
    let history = gio::Menu::new();
    history.append(Some("Back"), Some("app.back"));
    history.append(Some("Forward"), Some("app.forward"));
    go.append_section(None, &history);
    let steps = gio::Menu::new();
    steps.append(Some("Previous Block or Version"), Some("app.previous"));
    steps.append(Some("Next Block or Version"), Some("app.next"));
    go.append_section(None, &steps);
    let other = gio::Menu::new();
    other.append(Some("Go To or Do…"), Some("app.palette"));
    other.append(Some("Estimate a Future Block…"), Some("app.estimate"));
    other.append(Some("Find…"), Some("app.find"));
    go.append_section(None, &other);
    let refresh = gio::Menu::new();
    refresh.append(Some("Refresh"), Some("app.refresh"));
    go.append_section(None, &refresh);
    root.append_submenu(Some("Go"), &go);

    let investigate = gio::Menu::new();
    investigate.append(Some("Follow Transfer"), Some("app.follow-transfer"));
    investigate.append(Some("Witnesses"), Some("app.witnesses"));
    investigate.append(Some("Protocol Links"), Some("app.protocol-links"));
    investigate.append(Some("Block History"), Some("app.block-history"));
    investigate.append(Some("Shared Keys and Authorities"), Some("app.shared-authorities"));
    root.append_submenu(Some("Investigate"), &investigate);

    let peers = gio::Menu::new();
    peers.append(Some("Add Node…"), Some("app.add-node"));
    peers.append(Some("Ping All Peers"), Some("app.ping-all"));
    peers.append(Some("Clear Ban List…"), Some("app.clear-bans"));
    root.append_submenu(Some("Peers"), &peers);

    let file = gio::Menu::new();
    file.append(Some("Bookmark This Page…"), Some("app.bookmark"));
    root.append_section(None, &file);

    let app = gio::Menu::new();
    app.append(Some("Keyboard Shortcuts"), Some("app.keyboard"));
    app.append(Some("Settings…"), Some("app.settings"));
    app.append(Some("Quit"), Some("app.quit"));
    root.append_section(None, &app);
    let _ = glib::Variant::from(true);
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerators_in_words() {
        assert_eq!(words("<Control><Alt>bracketleft"), "Ctrl+Alt+[");
        assert_eq!(words("<Control>k"), "Ctrl+K");
        assert_eq!(keys_of("app.palette"), "Ctrl+K");
    }

    #[test]
    fn each_shortcut_is_valid_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for (action, accel, _) in SHORTCUTS {
            let modifiers = accel.matches('<').count();
            assert!(accel.matches('>').count() == modifiers && modifiers >= 1, "{accel}");
            assert!(accel.split('>').filter(|p| p.starts_with('<')).all(|p| matches!(&p[1..], "Control" | "Shift" | "Alt")), "{accel}");
            assert!(seen.insert(*accel), "{accel} is used twice");
            assert!(action.starts_with("app."));
        }
    }
}
