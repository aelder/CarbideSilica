//! Every keyboard shortcut in the app, with the terminal key it replaces, and the cache
//! location and size, as the terminal's Help screen shows them (`KeyboardReference.swift`).
//!
//! The shortcuts are set in `commands.rs`. Keep this list in step with it. The keys are the
//! Linux keys: Ctrl for Command, Alt for Option.

use std::rc::Rc;

use gtk::prelude::*;

use crate::format;
use crate::model::AppModel;
use crate::page::paragraph;
use crate::widgets::display;

pub struct Entry {
    pub keys: &'static str,
    pub action: &'static str,
    pub terminal: &'static str,
}

const fn e(keys: &'static str, action: &'static str, terminal: &'static str) -> Entry {
    Entry { keys, action, terminal }
}

pub const SECTIONS: &[(&str, &[Entry])] = &[
    ("Places", &[
        e("Ctrl+1", "Dashboard", "d"),
        e("Ctrl+2", "Mempool", "m"),
        e("Ctrl+3", "Charts", "F9"),
        e("Ctrl+4", "Currencies", "c"),
        e("Ctrl+5", "Marketplace", "o"),
        e("Ctrl+6", "Bookmarks", "F6"),
        e("Ctrl+7", "Node and peers", "`"),
        e("Ctrl+8", "Sources", "F2"),
    ]),
    ("Search and history", &[
        e("Ctrl+K", "The command list has each command and each card of the open page. Type some letters, then press Return.", "none"),
        e("Ctrl+F", "Search. In the field, Up and Down go through the earlier searches, and Esc stops a search that runs.", "/"),
        e("Ctrl+[", "Back. At a place in the sidebar, go to the dashboard.", "h, Left, Backspace, Esc"),
        e("Ctrl+]", "Forward", "none"),
        e("Ctrl+Alt+[", "Previous block or identity version", "[ or p"),
        e("Ctrl+Alt+]", "Next block or identity version", "] or n"),
        e("Ctrl+Shift+E", "Estimate a future block or date", "F10"),
        e("Ctrl+R", "Refresh the visible page", "F5, r"),
        e("Ctrl+D", "Bookmark this page", "Shift-B"),
    ]),
    ("Investigations", &[
        e("Ctrl+Shift+T", "Follow a transfer", "F8"),
        e("Ctrl+Shift+W", "Witnesses", "F4"),
        e("Ctrl+Shift+L", "Protocol links", "F3"),
        e("Ctrl+Shift+B", "Block history across side branches", "F7"),
        e("Ctrl+Shift+A", "Shared keys and authorities", "v"),
    ]),
    ("Display", &[
        e("Ctrl+Shift+I", "Show names or raw IDs for identities and currencies", "Shift-I"),
        e("Ctrl+Shift+D", "Show dates for heights, confirmations and timestamps", "Shift-D"),
        e("Ctrl+Alt+U", "Show times in UTC or local time", "t in Help"),
        e("Ctrl+Alt+S", "Dashboard: stream blocks", "Home in the block list"),
        e("Ctrl+Alt+L", "Mempool: tiles or list", "none"),
        e("Ctrl+Alt+G", "Peers: world map", "w"),
    ]),
    ("Peers", &[
        e("Ctrl+Alt+N", "Add a node", "a"),
        e("Ctrl+Alt+P", "Ping all peers", "p"),
        e("Peers menu", "Clear the ban list", "c"),
        e("Return", "Open the selected peer. Disconnect and Ban are in its page.", "Enter, x, b"),
    ]),
    ("The cursor of a page", &[
        e("j k, ↑ ↓", "Move the cursor through the rows and the links of the page. No click is necessary first.", "j, k"),
        e("Return, l, →", "Open the row or the link under the cursor", "Enter, l, Right"),
        e("Esc", "Go back one page. At a place in the sidebar, go to the dashboard.", "Esc"),
        e("h, ←, Backspace", "Go back one page", "h, Left, Backspace"),
        e("Tab", "Show the raw JSON of the page and go to it. Tab again hides it. In the JSON, the cursor moves through its lines, Return follows an ID or opens a node, and s saves the file.", "Tab, s"),
        e("Tab, Shift+Tab", "On a page with no JSON, go to the next card or to the card before", "Tab on the dashboard"),
        e("Page Up, Page Down", "Move the cursor 10 places", "PgUp, PgDn"),
        e("Home, End", "Go to the first or the last row or link of the page", "Home, End"),
        e("y, Ctrl+C", "Copy the full value under the cursor. Ctrl+C copies selected text when text is selected.", "y in JSON"),
    ]),
    ("The keys of the terminal", &[
        e("d m o c `", "Dashboard, mempool, marketplace, currencies, node and peers", "d m o c `"),
        e("F2 F6 F9", "Sources, bookmarks, charts", "F2 F6 F9"),
        e("/", "Search", "/"),
        e("n or ], p or [", "Next or previous block, identity version, or transaction of the same block, in place", "n ] p ["),
        e("r, F5", "Read the page again", "r, F5"),
        e("b", "On a transaction, open its block", "b"),
        e("a", "On a block of a side branch, open the active block at that height", "a"),
        e("g", "On a currency, go to the state at a chosen block and type there", "g"),
        e("n", "On a currency, go to the notarizations", "n"),
        e("v", "On an identity, show the shared keys and authorities", "v"),
        e("F8, F4", "On a transaction, follow a transfer or find the signers for the output under the cursor", "F8, F4"),
        e("F3, F7", "On a transaction, show the protocol links or the block history", "F3, F7"),
        e("F10", "Estimate a future block or date", "F10"),
        e("Shift+B, Shift+I, Shift+D", "Bookmark this page, names or IDs, dates or heights", "Shift-B, Shift-I, Shift-D"),
        e("?, F1", "This window", "?, F1"),
    ]),
    ("Where the keys do not apply", &[
        e("In a text field", "The keys type text. Esc leaves the field.", "the same"),
        e("In a dialog", "Return and Esc answer the dialog", "the same"),
        e("Charts", "The charts have their own keys. The line below the charts lists them.", "the same keys"),
        e("Ctrl+Q", "Quit", "q"),
    ]),
];

/// A key, in an outline. The key keeps its case: "j" and "J" are not the same key.
pub fn key_cap(keys: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(keys));
    label.add_css_class("key-cap");
    label.set_halign(gtk::Align::Start);
    label.set_valign(gtk::Align::Baseline);
    label
}

pub fn window(model: &Rc<AppModel>, parent: &impl IsA<gtk::Window>) -> gtk::Window {
    let window = gtk::Window::new();
    window.set_title(Some("Keyboard Shortcuts"));
    window.set_transient_for(Some(parent));
    window.set_default_size(720, 720);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
    column.set_margin_top(24);
    column.set_margin_bottom(24);
    column.set_margin_start(24);
    column.set_margin_end(24);
    // One grid for all sections, so the columns line up down the window.
    let grid = gtk::Grid::new();
    grid.set_column_spacing(16);
    grid.set_row_spacing(4);
    let mut row = 0;
    for (index, (title, entries)) in SECTIONS.iter().enumerate() {
        let head = gtk::Box::new(gtk::Orientation::Vertical, 8);
        head.set_margin_top(if index == 0 { 0 } else { 14 });
        head.append(&display::rule());
        head.append(&display::heading(title));
        grid.attach(&head, 0, row, 3, 1);
        row += 1;
        for (column_index, name) in ["Keys", "Action", "Terminal key"].iter().enumerate() {
            grid.attach(&display::name_text(name), column_index as i32, row, 1, 1);
        }
        row += 1;
        for entry in entries.iter() {
            grid.attach(&key_cap(entry.keys), 0, row, 1, 1);
            let action = paragraph(entry.action, &["value"]);
            action.set_hexpand(true);
            action.set_max_width_chars(48);
            grid.attach(&action, 1, row, 1, 1);
            let terminal = gtk::Label::new(Some(entry.terminal));
            terminal.add_css_class("mono");
            terminal.add_css_class("label");
            terminal.set_xalign(0.0);
            terminal.set_valign(gtk::Align::Start);
            grid.attach(&terminal, 2, row, 1, 1);
            row += 1;
        }
    }
    column.append(&grid);
    column.append(&display::rule());
    let search = gtk::Box::new(gtk::Orientation::Vertical, 6);
    search.append(&display::heading("Search"));
    for text in [
        "Enter a block height, a block hash, a transaction ID, an address, an identity such as name@ or an i-address, or a currency such as name@@. Enter currencies to open the currency browser.",
        "Enter a date such as 2026-11-07, time:Nov 7 2026 2:30 pm UTC, or Unix seconds such as 1700000000. A date without a time means midnight UTC. The app opens the nearest block that it finds.",
        "A number up to the tip is a block height. A larger number at or after the first block's time is Unix seconds. A number between the two is a future height.",
        "A future height or date opens an estimate. Enter estimate:5000000, estimate:in 2 hours or estimate:unix:1857254400 to open the estimator directly.",
    ] {
        search.append(&paragraph(text, &["label"]));
    }
    column.append(&search);
    column.append(&display::rule());
    let cache = gtk::Box::new(gtk::Orientation::Vertical, 6);
    cache.append(&display::heading("Cache"));
    match model.explorer().map(|e| e.cache_info()) {
        Some(info) => {
            for (name, value, mono) in [("Location", info.path.clone(), true), ("Size", format::bytes(info.bytes), false)] {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                let label = display::name_text(name);
                label.set_size_request(90, -1);
                line.append(&label);
                let value = gtk::Label::new(Some(&value));
                value.set_selectable(true);
                value.add_css_class("value");
                if mono {
                    value.add_css_class("mono");
                }
                line.append(&value);
                cache.append(&line);
            }
        }
        None => cache.append(&paragraph("Not connected. The cache location is not known.", &["secondary"])),
    }
    column.append(&cache);
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroller.set_child(Some(&column));
    window.set_child(Some(&scroller));
    window
}
