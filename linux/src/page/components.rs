//! Parts that many pages use (`Views/Components.swift`, `ShellLists.swift`,
//! `SourcePrompts.swift`): notes, the source line, links to objects, field rows, the copy
//! key, the page heading and tags.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::api::{self, Route};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::theme::{self, font, tone, Color};
use crate::widgets::display::{self, PlateFill};
use crate::widgets::icons::{self, Icon};
use crate::widgets::link;

/// A text in a colour of the display, for Pango markup.
pub fn span(text: &str, color: Color) -> String {
    format!(
        "<span foreground=\"#{:02x}{:02x}{:02x}\" alpha=\"{}%\">{}</span>",
        (color.r * 255.0).round() as u8,
        (color.g * 255.0).round() as u8,
        (color.b * 255.0).round() as u8,
        ((color.a * 100.0).round() as u32).max(1),
        glib::markup_escape_text(text)
    )
}

/// A plain label: body text, left aligned. `classes` are classes of `style.css`.
pub fn text(content: &str, classes: &[&str]) -> gtk::Label {
    let label = gtk::Label::new(Some(content));
    label.set_xalign(0.0);
    for class in classes {
        label.add_css_class(class);
    }
    label
}

/// A label that wraps at the width of its column.
pub fn paragraph(content: &str, classes: &[&str]) -> gtk::Label {
    let label = text(content, classes);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label
}

/// A separator between rows, as SwiftUI's `Divider`.
pub fn divider() -> gtk::Box {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    line.add_css_class("divider");
    line
}

/// A line of explanation. It carries a warning triangle only when it is a warning, so an
/// icon always means something (`Note`).
pub fn note(content: &str, warning: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let label = paragraph(content, &[]);
    label.set_hexpand(true);
    if warning {
        let mark = icons::icon(Icon::Warning, 14, tone::SYSTEM_ORANGE);
        mark.set_valign(gtk::Align::Start);
        mark.set_margin_top(1);
        row.append(&mark);
        label.add_css_class("warning");
    }
    row.append(&label);
    row
}

/// One line that says where a page's data came from. For a retained observation it says
/// when the source reported it and why no source answered now (`ProvenanceNote`).
pub fn provenance_note(provenance: &api::Provenance) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    if provenance.retained {
        column.append(&note(
            &format!(
                "Stored observation from {}, reported {}. No source answered now, so the page can be out of date.",
                provenance.source,
                format::date(provenance.observed_at)
            ),
            true,
        ));
        for extra in [&provenance.failure, &provenance.caution].into_iter().flatten() {
            column.append(&paragraph(extra, &["secondary", "caption-text"]));
        }
    } else {
        let line = text(&format!("From {}", provenance.source), &["secondary", "caption-text"]);
        line.set_tooltip_text(Some(&format!("Answered at {}", format::date(provenance.observed_at))));
        column.append(&line);
    }
    column
}

/// The source of a page's data in the status line: the source's name and whether the data
/// is live or a stored observation, as a shape, a colour and a word (`SourceLine`).
pub struct SourceLine {
    pub root: gtk::Box,
    mark: crate::widgets::canvas::Canvas,
    label: gtk::Label,
}

impl SourceLine {
    pub fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let mark = icons::icon(Icon::Dot, 8, tone::SYSTEM_GREEN);
        let label = gtk::Label::new(Some(" "));
        label.set_xalign(0.0);
        root.append(&mark);
        root.append(&label);
        mark.set_visible(false);
        Self { root, mark, label }
    }

    /// `short` is a short name to show in place of the full source, such as the chain. The
    /// full source is then in the tooltip. `labelled` shows the word "Source" before it.
    pub fn set(&self, provenance: Option<&api::Provenance>, short: Option<&str>, labelled: bool) {
        // Before the source answers, the line keeps its height, so that the page below it
        // does not move down when the answer comes.
        let Some(provenance) = provenance.filter(|p| !p.source.is_empty()) else {
            self.mark.set_visible(false);
            self.label.set_text(" ");
            self.root.set_tooltip_text(None);
            return;
        };
        let retained = provenance.retained;
        icons::set_icon(
            &self.mark,
            if retained { Icon::Stored } else { Icon::Dot },
            if retained { tone::SYSTEM_ORANGE } else { tone::SYSTEM_GREEN },
        );
        let size = if retained { 10 } else { 8 };
        self.mark.set_size(size, size);
        self.mark.set_visible(true);
        let mut markup = String::new();
        let bold = |t: &str| format!("<b>{}</b>", glib::markup_escape_text(t));
        match short {
            Some(short) => {
                if labelled {
                    markup.push_str(&span("Source", tone::SECONDARY));
                    markup.push(' ');
                }
                markup.push_str(&bold(short));
            }
            None => {
                markup.push_str(&span("Source", tone::SECONDARY));
                markup.push(' ');
                markup.push_str(&bold(&provenance.source));
            }
        }
        markup.push(' ');
        if retained {
            markup.push_str(&span(
                &format!("stored, reported {}", format::age(provenance.observed_at, format::now())),
                tone::SYSTEM_ORANGE,
            ));
        } else {
            markup.push_str(&span("live", tone::SECONDARY));
        }
        self.label.set_markup(&markup);
        let help = format!(
            "{}{}",
            if short.is_some() { format!("Source: {}. ", provenance.source) } else { String::new() },
            if retained {
                "No source answered now. This is the latest stored observation."
            } else {
                "The source answered this request now."
            }
        );
        self.root.set_tooltip_text(Some(&help));
    }
}

/// A native-coin amount with its ticker, as markup: "3.00044031 VRSC" (`CoinAmount`).
pub fn coin_markup(sats: i64, coin: &str, figure: Color) -> String {
    format!("{} {}", span(&format::coins(sats), figure), span(coin, figure.opacity(0.6)))
}

/// The copy key: copies `value` and shows a check mark for a moment (`CopyButton`).
pub fn copy_button(value: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("copy-key");
    let mark = icons::icon(Icon::Copy, 13, tone::TERTIARY);
    button.set_child(Some(&mark));
    button.set_tooltip_text(Some("Copy"));
    let value = value.to_string();
    button.connect_clicked(move |button| {
        button.clipboard().set_text(&value);
        icons::set_icon(&mark, Icon::Check, tone::GOOD);
        let mark = mark.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
            if let Some(mark) = mark.upgrade() {
                icons::set_icon(&mark, Icon::Copy, tone::TERTIARY);
            }
        });
    });
    button
}

/// True for a currency route: a currency's name is its identity's name without "@".
fn is_currency(route: &Route) -> bool {
    matches!(route, Route::Currency { .. })
}

/// The text of a link to an object: the name when it is known and names are on, otherwise
/// the ID, shortened.
fn link_text(model: &AppModel, id: &str, route: &Route, shorten: bool) -> (String, bool) {
    if crate::display::shows_names()
        && let Some(name) = model.state().names.get(id)
    {
        let name = if is_currency(route) { name.strip_suffix('@').unwrap_or(name).to_string() } else { name.clone() };
        return (name, false);
    }
    (if shorten { format::short(id, 10, 8) } else { id.to_string() }, true)
}

/// A link that opens an object, with its name when the name is known (`RouteLink`). It is
/// a stop of the page's cursor. It changes when a name arrives or the setting changes.
pub fn route_link(page: &super::Page, id: &str, route: Route, shorten: bool) -> gtk::Button {
    let model = page.model.clone();
    let (label, mono) = link_text(&model, id, &route, shorten);
    let open_route = route.clone();
    let weak_model = Rc::downgrade(&model);
    let button = link::link(&label, mono, move || {
        if let Some(model) = weak_model.upgrade() {
            model.open(open_route.clone());
        }
    });
    button.set_tooltip_text(Some(id));
    let (id_owned, route_owned) = (id.to_string(), route.clone());
    let weak = button.downgrade();
    model.watch(&[Topic::Names, Topic::Display], &button, move |model| {
        if let Some(button) = weak.upgrade() {
            let (label, mono) = link_text(model, &id_owned, &route_owned, shorten);
            link::set_text(&button, &label);
            if let Some(child) = link::label_of(&button) {
                if mono { child.add_css_class("mono") } else { child.remove_css_class("mono") }
            }
        }
    });
    model.resolve_names([id.to_string()]);
    let open_model = Rc::downgrade(&model);
    page.stop(&button, None, Some(id.to_string()), move || {
        if let Some(model) = open_model.upgrade() {
            model.open(route.clone());
        }
    });
    button
}

/// A label and value line. The value can be selected, copied, and optionally followed
/// (`FieldRow`).
pub fn field_row(page: &super::Page, label: &str, value: &str, route: Option<Route>, monospaced: bool) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let name = display::name_text(label);
    name.set_size_request(170, -1);
    name.set_valign(gtk::Align::Baseline);
    row.append(&name);
    match route {
        Some(route) => {
            let link = route_link(page, value, route, false);
            if monospaced && let Some(child) = link::label_of(&link) {
                child.add_css_class("mono");
            }
            if let Some(child) = link::label_of(&link) {
                child.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            }
            row.append(&link);
        }
        None => {
            let shown = text(value, if monospaced { &["mono"] } else { &[] });
            shown.set_selectable(true);
            shown.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            shown.set_valign(gtk::Align::Baseline);
            row.append(&shown);
        }
    }
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    row.append(&spacer);
    row.append(&copy_button(value));
    row
}

/// The heading of a detail page: the title, an optional ID and tags (`PageHeader`).
pub fn page_header(title: &str, subtitle: Option<&str>, tags: &[(String, PlateFill)]) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
    let heading = theme::label(font::TITLE, title, "value");
    heading.set_selectable(true);
    heading.set_wrap(true);
    heading.set_lines(2);
    column.append(&heading);
    if let Some(subtitle) = subtitle {
        let line = text(subtitle, &["mono", "secondary"]);
        line.set_selectable(true);
        line.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        column.append(&line);
    }
    if !tags.is_empty() {
        column.append(&tag_row(tags));
    }
    column
}

/// Tags as plates in a row (`TagRow`).
pub fn tag_row(tags: &[(String, PlateFill)]) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    for (tag, fill) in tags {
        row.append(&display::plate(tag, *fill));
    }
    row
}

/// A check box of the display, as the Swift app's `.toggleStyle(.checkbox)`.
pub fn check_box(title: &str, on: bool, changed: impl Fn(bool) + 'static) -> gtk::CheckButton {
    let check = gtk::CheckButton::with_label(title);
    check.set_active(on);
    check.connect_toggled(move |check| changed(check.is_active()));
    crate::widgets::lamp::key_sounds(&check, false);
    check
}
