//! One filter option: its name, its count for the current selection, and a bar for the
//! count. An option with no offers stays in place and cannot be selected (`FacetOption`).

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::format;
use crate::theme::tone;
use crate::widgets::canvas::Canvas;
use crate::widgets::lamp;

/// A key: lit when it is selected. The faint bar on a key with no light is the count.
pub fn facet_option(label: &str, count: usize, share: f64, on: bool, action: impl Fn() + 'static) -> gtk::Button {
    let usable = count > 0 || on;
    let button = gtk::Button::new();
    button.add_css_class("lamp");
    button.add_css_class("facet");
    lamp::set_chosen(&button, on);

    let hovering = Rc::new(Cell::new(false));
    let bar = {
        let hovering = hovering.clone();
        Canvas::new(0, 0, move |_, snapshot, w, h| {
            if !on && !(hovering.get() && usable) {
                crate::theme::draw::fill_rect(snapshot, 0.0, 0.0, w * share as f32, h, tone::RECORD.opacity(0.18));
            }
        })
    };
    bar.set_can_target(false);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.set_margin_start(6);
    row.set_margin_end(6);
    row.set_margin_top(3);
    row.set_margin_bottom(3);
    let name = gtk::Label::new(Some(label));
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_single_line_mode(true);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    name.add_css_class("callout");
    if on {
        name.add_css_class("semibold");
    }
    let figure = gtk::Label::new(Some(&format::integer(count as u64)));
    figure.add_css_class("callout");
    figure.add_css_class("tabular");
    figure.add_css_class("count");
    figure.set_margin_start(4);
    row.append(&name);
    row.append(&figure);
    // The bar is behind the text: both are in the one cell of the grid.
    let face = gtk::Grid::new();
    face.attach(&bar, 0, 0, 1, 1);
    face.attach(&row, 0, 0, 1, 1);
    button.set_child(Some(&face));

    let motion = gtk::EventControllerMotion::new();
    let (h, b) = (hovering.clone(), bar.downgrade());
    motion.connect_enter(move |_, _, _| {
        h.set(true);
        if let Some(bar) = b.upgrade() {
            bar.queue_draw();
        }
    });
    let (h, b) = (hovering, bar.downgrade());
    motion.connect_leave(move |_| {
        h.set(false);
        if let Some(bar) = b.upgrade() {
            bar.queue_draw();
        }
    });
    button.add_controller(motion);
    button.connect_clicked(move |_| action());
    lamp::key_sounds(&button, true);
    button.set_sensitive(usable);
    button.update_property(&[gtk::accessible::Property::Label(&format!("{label}, {count} offers"))]);
    button.update_state(&[gtk::accessible::State::Selected(Some(on))]);
    button
}
