//! Small parts of the chart page: keys with a mark (SwiftUI's `Label` with a system image
//! in `LampButtonStyle`), a holder that builds its contents again only when they change, the
//! tables of samples and checkpoints (SwiftUI's `Table`), and the chart legend
//! (`ChartLegend` in ChartPlot.swift).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gio, glib, gsk};

use crate::page::load::clear;
use crate::theme::{draw, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;
use crate::widgets::lamp;

/// The marks of the keys of this page.
#[derive(Clone, Copy)]
pub enum Mark {
    Left,
    Right,
    Refresh,
}

/// A mark in the colour of the key's legend, so that it follows the light of the key.
fn mark(kind: Mark) -> Canvas {
    let canvas = Canvas::new(10, 10, move |canvas, snapshot, w, h| {
        let rgba = canvas.color();
        let color = Color { r: rgba.red(), g: rgba.green(), b: rgba.blue(), a: rgba.alpha() };
        let builder = gsk::PathBuilder::new();
        match kind {
            Mark::Left => {
                builder.move_to(w * 0.62, h * 0.18);
                builder.line_to(w * 0.34, h * 0.5);
                builder.line_to(w * 0.62, h * 0.82);
            }
            Mark::Right => {
                builder.move_to(w * 0.38, h * 0.18);
                builder.line_to(w * 0.66, h * 0.5);
                builder.line_to(w * 0.38, h * 0.82);
            }
            Mark::Refresh => {
                // A circle open at the upper right, with an arrow head at the end of its
                // clockwise turn: arrow.clockwise.
                let (cx, cy, r) = (w / 2.0, h / 2.0 + 0.3, w * 0.4);
                let start = -30f32.to_radians();
                let sweep = 300f32.to_radians();
                for i in 0..=24 {
                    let a = start + (i as f32 / 24.0) * sweep;
                    let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
                    if i == 0 { builder.move_to(x, y) } else { builder.line_to(x, y) }
                }
                let end = start + sweep;
                let (ax, ay) = (cx + r * end.cos(), cy + r * end.sin());
                // Back along the turn, and to each side of it.
                let (bx, by) = (end.sin(), -end.cos());
                for turn in [0.65f32, -0.65] {
                    let (wx, wy) = (bx * turn.cos() - by * turn.sin(), bx * turn.sin() + by * turn.cos());
                    builder.move_to(ax, ay);
                    builder.line_to(ax + wx * w * 0.32, ay + wy * h * 0.32);
                }
            }
        }
        let stroke = gsk::Stroke::new(1.3);
        stroke.set_line_cap(gsk::LineCap::Round);
        stroke.set_line_join(gsk::LineJoin::Round);
        snapshot.append_stroke(&builder.to_path(), &stroke, &color.rgba());
    });
    canvas.set_valign(gtk::Align::Center);
    canvas
}

/// A small key of the page with a mark before its legend.
pub fn key(title: &str, kind: Mark, action: impl Fn() + 'static) -> gtk::Button {
    let button = lamp::button(title, action);
    let label = button.child().and_downcast::<gtk::Label>();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    row.append(&mark(kind));
    if let Some(label) = label {
        button.set_child(None::<&gtk::Widget>);
        row.append(&label);
    }
    button.set_child(Some(&row));
    button
}

/// A part that builds its contents again only when its signature changes.
pub struct Holder {
    pub root: gtk::Box,
    signature: RefCell<Option<String>>,
}

impl Holder {
    pub fn new(spacing: i32) -> Rc<Self> {
        Rc::new(Self { root: gtk::Box::new(gtk::Orientation::Vertical, spacing), signature: RefCell::new(None) })
    }

    /// Calls `build` with the emptied box when `signature` differs from the last one.
    pub fn set(&self, signature: String, build: impl FnOnce(&gtk::Box)) {
        if self.signature.borrow().as_deref() == Some(signature.as_str()) {
            return;
        }
        *self.signature.borrow_mut() = Some(signature);
        clear(&self.root);
        build(&self.root);
        self.root.set_visible(self.root.first_child().is_some());
    }
}

/// A caption in the secondary tone, as `.font(.caption).foregroundStyle(.secondary)`.
pub fn caption(text: &str) -> gtk::Label {
    let label = crate::page::paragraph(text, &["caption-text", "secondary"]);
    label.set_xalign(0.0);
    label
}

/// A note in the caption type, in the secondary tone.
pub fn caption_note(text: &str, warning: bool) -> gtk::Box {
    let note = crate::page::note(text, warning);
    note.add_css_class("caption-text");
    if !warning {
        note.add_css_class("secondary");
    }
    note
}

/// One row of a table: its cells as text.
pub type Row = Vec<String>;

/// A table of text with a heading of names in spaced capitals, `height` high, that scrolls.
/// `columns` are the names and the least width of each column.
pub fn table(columns: &[(&str, i32)], rows: Vec<Row>, height: i32) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    header.set_margin_start(6);
    header.set_margin_bottom(4);
    for (name, width) in columns {
        let label = display::name_text(name);
        label.set_size_request(*width, -1);
        label.set_xalign(0.0);
        header.append(&label);
    }
    column.append(&header);
    column.append(&display::rule());
    let store = gio::ListStore::new::<glib::BoxedAnyObject>();
    for row in rows {
        store.append(&glib::BoxedAnyObject::new(row));
    }
    let widths: Vec<i32> = columns.iter().map(|(_, w)| *w).collect();
    let factory = gtk::SignalListItemFactory::new();
    let setup_widths = widths;
    factory.connect_setup(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.set_margin_start(6);
        row.set_margin_top(2);
        row.set_margin_bottom(2);
        for width in &setup_widths {
            let cell = gtk::Label::new(None);
            cell.set_xalign(0.0);
            cell.set_size_request(*width, -1);
            cell.add_css_class("tabular");
            cell.set_ellipsize(gtk::pango::EllipsizeMode::End);
            row.append(&cell);
        }
        item.set_child(Some(&row));
    });
    factory.connect_bind(move |_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else { return };
        let (Some(object), Some(row)) = (item.item().and_downcast::<glib::BoxedAnyObject>(), item.child()) else { return };
        let cells: std::cell::Ref<Row> = object.borrow();
        let mut child = row.first_child();
        for text in cells.iter() {
            if let Some(label) = child.as_ref().and_then(|c| c.downcast_ref::<gtk::Label>()) {
                label.set_text(text);
            }
            child = child.and_then(|c| c.next_sibling());
        }
    });
    let list = gtk::ListView::new(Some(gtk::NoSelection::new(Some(store))), Some(factory));
    list.add_css_class("chart-table");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_child(Some(&list));
    scroller.set_size_request(-1, height);
    scroller.set_hscrollbar_policy(gtk::PolicyType::Automatic);
    column.append(&scroller);
    column
}

/// The shape of a legend item.
#[derive(Clone, Copy)]
pub enum Shape {
    Line,
}

/// A legend row: a small mark and the words for it. Short items lay out in one row, and in a
/// column when the row does not fit.
pub fn legend(items: &[(Shape, Color, String)]) -> gtk::FlowBox {
    let flow = gtk::FlowBox::new();
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_column_spacing(16);
    flow.set_row_spacing(4);
    flow.set_max_children_per_line(items.len().max(1) as u32);
    flow.set_homogeneous(false);
    flow.set_halign(gtk::Align::Start);
    for (shape, color, text) in items {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let (shape, color) = (*shape, *color);
        let swatch = Canvas::new(16, 10, move |_, snapshot, w, h| match shape {
            Shape::Line => draw::rule(snapshot, 0.0, h / 2.0, w, h / 2.0, color, 1.0, &[]),
        });
        swatch.set_valign(gtk::Align::Center);
        row.append(&swatch);
        row.append(&crate::page::text(text, &["caption-text", "label"]));
        flow.append(&row);
    }
    flow
}

