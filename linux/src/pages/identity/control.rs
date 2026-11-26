//! Who holds each power of an identity and what follows when its key is lost or stolen
//! (`IdentityControlCard`), and every version of the identity, newest first
//! (`IdentityHistoryCard`). Port of `IdentityControl.swift`; the lanes are in `lanes.rs`.
//!
//! The control of an identity has four parts: the keys that spend and update, the identity
//! that can revoke, the identity that can recover, and the lock on the funds. They are a
//! table, because each identity has the same four. The two outcomes are in words and in
//! view together. An identity that is its own authority is called out, because that is the
//! difference between a recoverable identity and one that is only as safe as its keys.

use std::rc::Rc;

use gtk::prelude::*;

use crate::bridge::api::{self, IdentityState, Route, Timelock};
use crate::format;
use crate::page::{self, Page};
use crate::shared::identity_support::{self as support, abbreviated_date};
use crate::theme::{self, font, tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, PlateFill};
use crate::widgets::icons::{self, Icon};
use crate::widgets::link;

// MARK: - Words

/// A number of blocks as a length of time, at one block for each minute.
pub fn span(blocks: u64) -> String {
    match blocks {
        0..120 => format!("{blocks} minutes"),
        120..2880 => format!("{} hours", (blocks as f64 / 60.0).round() as i64),
        2880..86_400 => format!("{} days", (blocks as f64 / 1440.0).round() as i64),
        _ => format!("{} years", format::decimal(blocks as f64 / 525_600.0, 1)),
    }
}

/// The facts of a version that the words need.
pub struct Facts<'a> {
    pub name: &'a str,
    pub keys: usize,
    pub required: u64,
    pub self_revoke: bool,
    pub self_recover: bool,
    pub revocation: Option<&'a str>,
    pub recovery: Option<&'a str>,
    pub timelock: &'a Timelock,
}

impl<'a> Facts<'a> {
    pub fn of(identity: &'a IdentityState) -> Self {
        Self {
            name: &identity.name,
            keys: identity.primary_addresses.len(),
            required: identity.minimum_signatures.unwrap_or(1),
            self_revoke: identity.revocation_authority.is_none() || identity.revocation_is_self,
            self_recover: identity.recovery_authority.is_none() || identity.recovery_is_self,
            revocation: identity.revocation_authority.as_deref(),
            recovery: identity.recovery_authority.as_deref(),
            timelock: &identity.timelock,
        }
    }
}

/// An authority as words: "itself" when there is none, otherwise the name or the ID.
fn shown(id: Option<&str>, display: &dyn Fn(&str) -> String) -> String {
    match id {
        None => "itself".into(),
        Some(id) => display(id),
    }
}

/// The sentence at the top of the card.
pub fn sentence(facts: &Facts, display: &dyn Fn(&str) -> String) -> String {
    let name = facts.name;
    let mut parts = vec![if facts.keys <= 1 {
        format!("One key controls {name}.")
    } else {
        format!("{} of the {} keys must sign to control {name}.", facts.required, facts.keys)
    }];
    if facts.self_revoke && facts.self_recover {
        parts.push("It is its own authority. No other identity can revoke or recover it.".into());
    } else if !facts.self_revoke && !facts.self_recover && facts.revocation == facts.recovery {
        parts.push(format!("{} can revoke and recover it.", shown(facts.revocation, display)));
    } else {
        let revoke = if facts.self_revoke {
            "Only its own keys can revoke it".to_string()
        } else {
            format!("{} can revoke it", shown(facts.revocation, display))
        };
        let recover = if facts.self_recover {
            "no other identity can recover it".to_string()
        } else {
            format!("{} can recover it", shown(facts.recovery, display))
        };
        parts.push(format!("{revoke} and {recover}."));
    }
    match facts.timelock {
        Timelock::Unlocked => {}
        Timelock::Delay { blocks } => parts.push(format!(
            "The funds are locked. An unlock takes {} blocks, about {}.",
            format::integer(*blocks),
            span(*blocks)
        )),
        Timelock::UnlocksAt { height } => {
            parts.push(format!("The funds are locked until block {}.", format::integer(*height)))
        }
    }
    parts.join(" ")
}

/// "If the key is lost".
pub fn lost(facts: &Facts, display: &dyn Fn(&str) -> String) -> String {
    if facts.self_recover {
        "No other identity can set new keys. The identity and its funds are lost.".into()
    } else {
        format!(
            "{} can set new keys after a revocation. {} keeps its name, its funds and its history.",
            shown(facts.recovery, display),
            facts.name
        )
    }
}

/// "If the key is stolen".
pub fn stolen(facts: &Facts, display: &dyn Fn(&str) -> String) -> String {
    let name = facts.name;
    let mut text;
    if facts.self_revoke && facts.self_recover {
        text = "No other identity can stop the thief.".to_string();
    } else {
        text = if facts.self_revoke {
            format!("Only a holder of the keys can revoke {name}")
        } else {
            format!(
                "{} can revoke {name}, and the stolen keys then cannot spend or sign",
                shown(facts.revocation, display)
            )
        };
        if facts.self_recover {
            text.push_str(". No other identity can recover it.");
        } else {
            text.push_str(&format!(". Then {} can set new keys.", shown(facts.recovery, display)));
        }
    }
    match facts.timelock {
        Timelock::Unlocked => {}
        Timelock::Delay { blocks } => text.push_str(&format!(
            " A thief must request an unlock and then wait about {} to move funds.",
            span(*blocks)
        )),
        Timelock::UnlocksAt { height } => {
            text.push_str(&format!(" A thief cannot move funds before block {}.", format::integer(*height)))
        }
    }
    text
}

pub fn lock_holder(timelock: &Timelock) -> String {
    match timelock {
        Timelock::Unlocked => "None".into(),
        Timelock::Delay { blocks } => format!("Delay of {} blocks", format::integer(*blocks)),
        Timelock::UnlocksAt { height } => format!("Until block {}", format::integer(*height)),
    }
}

pub fn lock_effect(timelock: &Timelock) -> String {
    match timelock {
        Timelock::Unlocked => "Funds can move at once.".into(),
        Timelock::Delay { blocks } => {
            format!("Funds cannot move until about {} after a request to unlock.", span(*blocks))
        }
        Timelock::UnlocksAt { .. } => "Funds cannot move before that block.".into(),
    }
}

/// "1 key" or "2 of 3 keys".
pub fn key_count(facts: &Facts) -> String {
    if facts.keys <= 1 { "1 key".into() } else { format!("{} of {} keys", facts.required, facts.keys) }
}

// MARK: - The card

/// The Control card at the viewed version.
pub fn control_card(page: &Rc<Page>, identity: &IdentityState) -> gtk::Box {
    let model = page.model.clone();
    let card = page.card("Control", Some("at the viewed version"));
    let facts = Facts::of(identity);
    let display = |id: &str| support::display_id(&model, id);

    // `.title3`: 15.
    let sentence_label = theme::text_label(font::TITLE3, &sentence(&facts, &display), &[]);
    sentence_label.set_wrap(true);
    sentence_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    sentence_label.set_selectable(true);
    sentence_label.add_css_class("value");
    card.append(&sentence_label);

    let grid = gtk::Grid::new();
    grid.set_column_spacing(18);
    grid.set_row_spacing(7);
    let mut row = 0;
    let cell = |widget: &gtk::Widget, column: i32, row: i32| {
        widget.set_hexpand(true);
        widget.set_halign(gtk::Align::Fill);
        widget.set_valign(gtk::Align::Baseline);
        grid.attach(widget, column, row, 1, 1);
    };
    for (column, title) in ["Power", "Held by", "What it does"].iter().enumerate() {
        let name = display::name_text(title);
        name.set_xalign(0.0);
        cell(name.upcast_ref(), column as i32, row);
    }
    let divider = |row: i32| {
        grid.attach(&page::divider(), 0, row, 3, 1);
    };
    let does = |text: &str| -> gtk::Widget {
        let label = page::paragraph(text, &["secondary"]);
        label.upcast()
    };
    row += 1;
    divider(row);

    // Spend and update.
    row += 1;
    let name = display::name_text("Spend and update");
    name.set_xalign(0.0);
    cell(name.upcast_ref(), 0, row);
    let keys = gtk::Box::new(gtk::Orientation::Vertical, 2);
    keys.set_baseline_child(0);
    keys.append(&page::text(&key_count(&facts), &[]));
    for key in &identity.primary_addresses {
        let link = page::route_link(page, key, crate::route::holder(key), true);
        keys.append(&link);
    }
    cell(keys.upcast_ref(), 1, row);
    cell(&does("Moves funds and changes the identity."), 2, row);

    // Revoke and recover.
    for (title, id, is_self, what) in [
        ("Revoke", &identity.revocation_authority, facts.self_revoke, "Stops all use of the identity at once."),
        ("Recover", &identity.recovery_authority, facts.self_recover, "Sets new keys for a revoked identity."),
    ] {
        row += 1;
        divider(row);
        row += 1;
        let name = display::name_text(title);
        name.set_xalign(0.0);
        cell(name.upcast_ref(), 0, row);
        let holder: gtk::Widget = match (is_self, id) {
            (false, Some(id)) => page::route_link(page, id, Route::Identity { name: id.clone() }, true).upcast(),
            _ => page::text("The identity itself", &[]).upcast(),
        };
        let holder_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        holder_box.append(&holder);
        cell(holder_box.upcast_ref(), 1, row);
        cell(&does(what), 2, row);
    }

    // Lock.
    row += 1;
    divider(row);
    row += 1;
    let name = display::name_text("Lock");
    name.set_xalign(0.0);
    cell(name.upcast_ref(), 0, row);
    cell(page::text(&lock_holder(&identity.timelock), &[]).upcast_ref(), 1, row);
    cell(&does(&lock_effect(&identity.timelock)), 2, row);
    card.append(&grid);

    if identity.revoked {
        card.append(&page::note(
            "This version is revoked. The keys cannot spend or sign until a recovery.",
            true,
        ));
    }
    let outcomes = gtk::Box::new(gtk::Orientation::Vertical, 4);
    outcomes.append(&outcome("If the key is lost", &lost(&facts, &display), facts.self_recover));
    outcomes.append(&outcome("If the key is stolen", &stolen(&facts, &display), facts.self_revoke));
    card.append(&outcomes);
    card.root
}

fn outcome(title: &str, text: &str, warning: bool) -> gtk::Label {
    let color = if warning { tone::MINED } else { tone::VALUE };
    let label = gtk::Label::new(None);
    label.set_markup(&format!(
        "<b>{}</b>{}",
        page::span(&format!("{title}: "), color),
        page::span(text, color)
    ));
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_selectable(true);
    label
}

// MARK: - History

/// Every version of an identity, newest first, with what changed each time. Select a row
/// to view that version. `show_all` is the state of "Show all"; `toggle` changes it.
pub fn history_card(
    page: &Rc<Page>,
    timeline: &api::IdentityTimeline,
    selection: usize,
    show_all: bool,
    select: Rc<dyn Fn(usize)>,
    toggle: Rc<dyn Fn()>,
) -> gtk::Box {
    const COLLAPSED: usize = 8;
    let count = timeline.versions.len();
    let card = page.card("History", Some(&format!("{count} version{}", if count == 1 { "" } else { "s" })));
    let newest_first: Vec<(usize, &IdentityState)> = timeline.versions.iter().enumerate().rev().collect();
    let shown: Vec<(usize, &IdentityState)> =
        if show_all { newest_first.clone() } else { newest_first.iter().take(COLLAPSED).cloned().collect() };
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for (position, (index, version)) in shown.iter().enumerate() {
        let select = select.clone();
        let index = *index;
        list.append(&history_row(
            page,
            index + 1,
            version,
            index == selection,
            position == 0,
            position == shown.len() - 1,
            Rc::new(move || select(index)),
        ));
    }
    card.append(&list);
    if newest_first.len() > COLLAPSED {
        let title = if show_all { "Show fewer".to_string() } else { format!("Show all {}", newest_first.len()) };
        card.append(&link::link(&title, false, move || toggle()));
    }
    if !timeline.history_complete {
        card.append(&page::note("The source did not return every earlier version.", true));
    }
    card.root
}

fn history_row(
    page: &Rc<Page>,
    number: usize,
    version: &IdentityState,
    selected: bool,
    is_first: bool,
    is_last: bool,
    select: Rc<dyn Fn()>,
) -> gtk::Box {
    let model = page.model.clone();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    // The line and its dot.
    let dot_color: Color = if version.revoked {
        tone::ALERT
    } else if selected {
        tone::PRESENT
    } else {
        tone::RECORD
    };
    let line = Canvas::new(13, 0, move |_, snapshot, w, h| {
        let mid = w / 2.0;
        if !is_first {
            theme::draw::fill_rect(snapshot, mid - 0.5, 0.0, 1.0, 8.0, tone::AXIS);
        }
        let size = if selected { 11.0 } else { 8.0 };
        let top = 8.0;
        if selected {
            theme::draw::push_shadow(snapshot, tone::PRESENT, 0.7, 3.0);
            theme::draw::fill_rect(snapshot, mid - size / 2.0, top, size, size, dot_color);
            snapshot.pop();
        } else {
            theme::draw::fill_rect(snapshot, mid - size / 2.0, top, size, size, dot_color);
        }
        if !is_last {
            theme::draw::fill_rect(snapshot, mid - 0.5, top + size, 1.0, (h - top - size).max(0.0), tone::AXIS);
        }
    });
    line.set_vexpand(true);
    row.append(&line);

    let column = gtk::Box::new(gtk::Orientation::Vertical, 3);
    column.set_margin_bottom(14);
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let version_link = link::link(&format!("Version {number}"), false, move || select());
    if let Some(label) = link::label_of(&version_link) {
        label.add_css_class("semibold");
    }
    heading.append(&version_link);
    if selected {
        heading.append(&display::plate("Viewed", PlateFill::None));
    }
    let when = match version.time {
        Some(time) => abbreviated_date(time),
        None => "Date not available".into(),
    };
    let when_label = page::text(&when, &["secondary"]);
    when_label.set_valign(gtk::Align::Baseline);
    heading.append(&when_label);
    let block_id = version.block_hash.clone().unwrap_or_else(|| version.height.to_string());
    let height = support::height_text(&model, version.height, version.time);
    heading.append(&height_link(page, &height, Route::Block { id: block_id }));
    if version.on_active_chain == Some(false) {
        let warning = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        warning.append(&icons::icon(Icon::Warning, 11, tone::MINED));
        warning.append(&theme::text_label(font::TEXT_CAPTION, "Not on the active chain", &["mined"]));
        heading.append(&warning);
    }
    column.append(&heading);
    for change in &version.changes {
        column.append(&page::paragraph(change, &["secondary"]));
    }
    column.set_hexpand(true);
    row.append(&column);
    if let Some(txid) = &version.txid {
        let link = page::route_link(page, txid, Route::Transaction { txid: txid.clone() }, true);
        link.set_valign(gtk::Align::Start);
        link.set_margin_top(4);
        link.set_margin_start(8);
        row.append(&link);
    }
    row
}

/// A link whose text is a height or a date, not an ID (`RouteLink(text:route:shorten:
/// false)` with a text that is not an ID). It is a stop of the cursor.
pub fn height_link(page: &Rc<Page>, text: &str, route: Route) -> gtk::Button {
    let model = Rc::downgrade(&page.model);
    let open = route.clone();
    let button = link::link(text, false, move || {
        if let Some(model) = model.upgrade() {
            model.open(open.clone());
        }
    });
    let model = Rc::downgrade(&page.model);
    let copy = match &route {
        Route::Block { id } => Some(id.clone()),
        _ => None,
    };
    page.stop(&button, None, copy, move || {
        if let Some(model) = model.upgrade() {
            model.open(route.clone());
        }
    });
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts<'a>(keys: usize, required: u64, revoke: Option<&'a str>, recover: Option<&'a str>, lock: &'a Timelock) -> Facts<'a> {
        Facts {
            name: "vETH@",
            keys,
            required,
            self_revoke: revoke.is_none(),
            self_recover: recover.is_none(),
            revocation: revoke,
            recovery: recover,
            timelock: lock,
        }
    }

    fn named(id: &str) -> String {
        if id == "iFoundation" { "Verus Coin Foundation@".into() } else { id.into() }
    }

    #[test]
    fn spans() {
        assert_eq!(span(30), "30 minutes");
        assert_eq!(span(119), "119 minutes");
        assert_eq!(span(120), "2 hours");
        assert_eq!(span(1440), "24 hours");
        assert_eq!(span(2880), "2 days");
        assert_eq!(span(86_399), "60 days");
        assert_eq!(span(525_600), "1 years");
        assert_eq!(span(788_400), "1.5 years");
    }

    #[test]
    fn sentence_with_one_authority_for_both() {
        let lock = Timelock::Unlocked;
        let f = facts(2, 2, Some("iFoundation"), Some("iFoundation"), &lock);
        assert_eq!(
            sentence(&f, &named),
            "2 of the 2 keys must sign to control vETH@. Verus Coin Foundation@ can revoke and recover it."
        );
        assert_eq!(
            lost(&f, &named),
            "Verus Coin Foundation@ can set new keys after a revocation. vETH@ keeps its name, its funds and its history."
        );
        assert_eq!(
            stolen(&f, &named),
            "Verus Coin Foundation@ can revoke vETH@, and the stolen keys then cannot spend or sign. Then Verus Coin Foundation@ can set new keys."
        );
    }

    #[test]
    fn sentence_for_its_own_authority_with_a_delay() {
        let lock = Timelock::Delay { blocks: 1440 };
        let f = facts(1, 1, None, None, &lock);
        assert_eq!(
            sentence(&f, &named),
            "One key controls vETH@. It is its own authority. No other identity can revoke or recover it. The funds are locked. An unlock takes 1,440 blocks, about 24 hours."
        );
        assert_eq!(lost(&f, &named), "No other identity can set new keys. The identity and its funds are lost.");
        assert_eq!(
            stolen(&f, &named),
            "No other identity can stop the thief. A thief must request an unlock and then wait about 24 hours to move funds."
        );
        assert_eq!(lock_holder(&lock), "Delay of 1,440 blocks");
        assert_eq!(lock_effect(&lock), "Funds cannot move until about 24 hours after a request to unlock.");
    }

    #[test]
    fn sentence_with_mixed_authorities_and_a_set_block() {
        let lock = Timelock::UnlocksAt { height: 5_000_000 };
        let f = facts(3, 2, Some("iFoundation"), None, &lock);
        assert_eq!(
            sentence(&f, &named),
            "2 of the 3 keys must sign to control vETH@. Verus Coin Foundation@ can revoke it and no other identity can recover it. The funds are locked until block 5,000,000."
        );
        assert_eq!(
            stolen(&f, &named),
            "Verus Coin Foundation@ can revoke vETH@, and the stolen keys then cannot spend or sign. No other identity can recover it. A thief cannot move funds before block 5,000,000."
        );
        let g = facts(1, 1, None, Some("iFoundation"), &lock);
        assert_eq!(
            sentence(&g, &named),
            "One key controls vETH@. Only its own keys can revoke it and Verus Coin Foundation@ can recover it. The funds are locked until block 5,000,000."
        );
        assert_eq!(key_count(&f), "2 of 3 keys");
        assert_eq!(key_count(&g), "1 key");
    }
}
