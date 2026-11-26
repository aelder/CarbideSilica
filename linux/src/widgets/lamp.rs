//! Illuminated keys, as on a console of the 1960s (`DisplayTheme.swift`): the sidebar key
//! (`LampKey`), the small key of a page (`LampButtonStyle`), a row of keys of which one is
//! on (`LampSegments`), a key that is on or off (`LampToggle`), a key that opens a list of
//! choices (`LampMenu`), a slider (`LampSlider`) and a part that opens (`DisplayDisclosure`).
//!
//! The light is in `style.css`: dark when off, a cool blue under the pointer, warm amber
//! for the chosen key or a press. Each key has the sounds of a key.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::canvas::Canvas;
use crate::theme::{self, draw, font, lamp, sound, tone};

/// Gives a widget the sounds of a key: the pointer on it, the press and the release. While
/// the pointer is down the widget has the class `pressed`, which lights a small key amber.
/// A key that is not in operation has no light and no sound.
pub fn key_sounds(widget: &impl IsA<gtk::Widget>, hover: bool) {
    let widget = widget.as_ref();
    if hover {
        let motion = gtk::EventControllerMotion::new();
        let target = widget.downgrade();
        motion.connect_enter(move |_, _, _| {
            if target.upgrade().is_some_and(|w| w.is_sensitive()) {
                sound::hover();
            }
        });
        widget.add_controller(motion);
    }
    let click = gtk::GestureClick::new();
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let target = widget.downgrade();
    click.connect_pressed(move |_, _, _, _| {
        if let Some(w) = target.upgrade().filter(|w| w.is_sensitive()) {
            w.add_css_class("pressed");
            sound::press();
        }
    });
    let target = widget.downgrade();
    click.connect_released(move |_, _, _, _| {
        if let Some(w) = target.upgrade() {
            if w.has_css_class("pressed") {
                sound::release();
            }
            w.remove_css_class("pressed");
        }
    });
    let target = widget.downgrade();
    click.connect_cancel(move |_, _| {
        if let Some(w) = target.upgrade() {
            w.remove_css_class("pressed");
        }
    });
    widget.add_controller(click);
}

/// A key with a lamp behind it, as on a console of the 1960s: the keys of the sidebar.
/// It is dark when it is off. The pointer on it gives it a cool blue light. The key of the
/// open place has a warm amber-white light.
pub struct LampKey {
    pub button: gtk::Button,
    title: gtk::Label,
    figure: gtk::Label,
}

impl LampKey {
    /// A key of `height` (30 for a place, 24 for a recent object). The legend is in the
    /// role of a place key, or in the text face for a recent object.
    pub fn new(title: &str, height: i32, recent: bool, action: impl Fn() + 'static) -> Self {
        let button = gtk::Button::new();
        button.add_css_class("lamp");
        if recent {
            button.add_css_class("recent");
        }
        button.set_size_request(-1, height);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let label = gtk::Label::new(None);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        if recent {
            label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        }
        let figure = theme::label(font::FIGURE_SMALL, "", "figure");
        figure.set_visible(false);
        row.append(&label);
        row.append(&figure);
        button.set_child(Some(&row));
        button.connect_clicked(move |_| action());
        key_sounds(&button, true);
        let key = Self { button, title: label, figure };
        key.set_title(title, recent);
        key
    }

    pub fn set_title(&self, title: &str, recent: bool) {
        if recent {
            self.title.set_markup(&format!(
                "<span font_size=\"{}\">{}</span>",
                12 * 1024,
                glib::markup_escape_text(title)
            ));
        } else {
            theme::set_label(&self.title, font::PLACE, title);
        }
    }

    /// The figure at the right of the key, such as the mempool count. None hides it.
    pub fn set_figure(&self, figure: Option<&str>) {
        match figure {
            Some(text) => {
                theme::set_label(&self.figure, font::FIGURE_SMALL, text);
                self.figure.set_visible(true);
            }
            None => self.figure.set_visible(false),
        }
    }

    /// True for the key of the open place.
    pub fn set_chosen(&self, chosen: bool) {
        if chosen {
            self.button.add_css_class("chosen");
        } else {
            self.button.remove_css_class("chosen");
        }
    }
}

/// The style of a button of the display: a small square key with a lamp, its legend in
/// spaced capitals.
pub fn button(title: &str, action: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("lamp");
    button.add_css_class("small");
    let label = theme::label(font::KEY, title, "");
    label.set_xalign(0.5);
    button.set_child(Some(&label));
    button.set_halign(gtk::Align::Start);
    button.set_valign(gtk::Align::Center);
    button.connect_clicked(move |_| action());
    key_sounds(&button, true);
    button
}

/// Sets the legend of a key made by [`button`].
pub fn set_button_title(button: &gtk::Button, title: &str) {
    if let Some(label) = button.child().and_downcast::<gtk::Label>() {
        theme::set_label(&label, font::KEY, title);
    }
}

/// A row of keys of which one is on: the display's form of a segmented control.
pub struct Segments {
    pub root: gtk::Box,
    keys: Vec<gtk::Button>,
    selected: Rc<Cell<usize>>,
}

impl Segments {
    pub fn new(titles: &[&str], selected: usize, changed: impl Fn(usize) + 'static) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let selected = Rc::new(Cell::new(selected));
        let changed = Rc::new(changed);
        let keys: Rc<RefCell<Vec<gtk::Button>>> = Rc::default();
        for (index, title) in titles.iter().enumerate() {
            let key = gtk::Button::new();
            key.add_css_class("lamp");
            key.add_css_class("segment");
            key.set_child(Some(&theme::label(font::KEY, title, "")));
            key.set_tooltip_text(None);
            key_sounds(&key, true);
            let (chosen, changed, keys_ref) = (selected.clone(), changed.clone(), keys.clone());
            key.connect_clicked(move |_| {
                chosen.set(index);
                for (other, key) in keys_ref.borrow().iter().enumerate() {
                    set_chosen(key, other == index);
                }
                changed(index);
            });
            set_chosen(&key, index == selected.get());
            root.append(&key);
            keys.borrow_mut().push(key);
        }
        let keys = keys.borrow().clone();
        Self { root, keys, selected }
    }

    pub fn selected(&self) -> usize {
        self.selected.get()
    }

    /// Selects a key without calling the action.
    pub fn set_selected(&self, index: usize) {
        self.selected.set(index);
        for (other, key) in self.keys.iter().enumerate() {
            set_chosen(key, other == index);
        }
    }
}

pub fn set_chosen(widget: &impl IsA<gtk::Widget>, chosen: bool) {
    if chosen {
        widget.add_css_class("chosen");
    } else {
        widget.remove_css_class("chosen");
    }
}

/// A key that is on or off: lit amber when on, blue under the pointer. With `struck`, the
/// legend has a line through it, for a key that is on to remove something.
pub fn toggle(title: &str, lit: bool, struck: bool, action: impl Fn() + 'static) -> gtk::Button {
    let key = gtk::Button::new();
    key.add_css_class("lamp");
    key.add_css_class("small");
    let label = gtk::Label::new(None);
    set_toggle_title(&label, title, struck);
    key.set_child(Some(&label));
    if struck {
        key.add_css_class("struck");
    }
    set_chosen(&key, lit);
    key.connect_clicked(move |_| action());
    key_sounds(&key, true);
    key
}

fn set_toggle_title(label: &gtk::Label, title: &str, struck: bool) {
    let markup = theme::markup(font::KEY, title);
    label.set_markup(&if struck { format!("<s>{markup}</s>") } else { markup });
}

/// A key that opens a list of choices and shows the chosen one.
pub struct Menu {
    pub button: gtk::MenuButton,
    label: gtk::Label,
    titles: Vec<String>,
    selected: Rc<Cell<usize>>,
}

impl Menu {
    pub fn new(titles: &[&str], selected: usize, changed: impl Fn(usize) + 'static) -> Rc<Self> {
        let button = gtk::MenuButton::new();
        button.add_css_class("lamp");
        button.add_css_class("menu-key");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(None);
        let mark = Canvas::new(7, 6, |_, snapshot, w, h| {
            let builder = gtk::gsk::PathBuilder::new();
            builder.move_to(0.0, 0.0);
            builder.line_to(w, 0.0);
            builder.line_to(w / 2.0, h);
            builder.close();
            draw::fill(snapshot, &builder.to_path(), tone::VALUE.opacity(0.9));
        });
        mark.set_valign(gtk::Align::Center);
        row.append(&label);
        row.append(&mark);
        button.set_child(Some(&row));
        key_sounds(&button, true);
        let popover = gtk::Popover::new();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        popover.set_child(Some(&list));
        button.set_popover(Some(&popover));
        let menu = Rc::new(Self {
            button,
            label,
            titles: titles.iter().map(|t| t.to_string()).collect(),
            selected: Rc::new(Cell::new(selected)),
        });
        let changed = Rc::new(changed);
        for (index, title) in titles.iter().enumerate() {
            let item = gtk::Button::with_label(title);
            item.add_css_class("flat");
            item.add_css_class("menu-row");
            let (weak, popover, changed) = (Rc::downgrade(&menu), popover.clone(), changed.clone());
            item.connect_clicked(move |_| {
                popover.popdown();
                if let Some(menu) = weak.upgrade() {
                    menu.set_selected(index);
                }
                changed(index);
            });
            list.append(&item);
        }
        menu.set_selected(selected);
        menu
    }

    pub fn set_selected(&self, index: usize) {
        self.selected.set(index);
        let title = self.titles.get(index).map(String::as_str).unwrap_or("–");
        theme::set_label(&self.label, font::KEY, title);
    }

    pub fn selected(&self) -> usize {
        self.selected.get()
    }
}

/// A slider of the display: a thin line with a lit mark. The value goes in steps of one.
pub struct Slider {
    pub canvas: Canvas,
    value: Rc<Cell<f64>>,
}

impl Slider {
    pub fn new(value: f64, low: f64, high: f64, changed: impl Fn(f64) + 'static) -> Self {
        let value = Rc::new(Cell::new(value));
        let hovering = Rc::new(Cell::new(false));
        let dragging = Rc::new(Cell::new(false));
        let span = (high - low).max(1.0);
        let (v, h, d) = (value.clone(), hovering.clone(), dragging.clone());
        let canvas = Canvas::new(60, 22, move |_, snapshot, w, height| {
            let width = (w - 12.0).max(1.0);
            let x = 6.0 + ((v.get() - low) / span) as f32 * width;
            let mid = height / 2.0;
            draw::fill_rect(snapshot, 0.0, mid - 0.5, w, 1.0, tone::AXIS);
            draw::fill_rect(snapshot, 0.0, mid - 1.0, x, 2.0, tone::RECORD);
            let lit = if d.get() { Some((lamp::AMBER_CENTRE, lamp::AMBER_EDGE)) } else if h.get() { Some((lamp::BLUE_CENTRE, lamp::BLUE_EDGE)) } else { None };
            let (kx, ky) = (x - 5.0, mid - 9.0);
            match lit {
                Some((centre, edge)) => {
                    draw::push_shadow(snapshot, lamp::HALATION, if d.get() { 0.7 } else { 0.25 }, 4.0);
                    draw::fill_rect(snapshot, kx, ky, 10.0, 18.0, edge);
                    draw::fill_rect(snapshot, kx + 2.0, ky + 3.0, 6.0, 12.0, centre);
                    snapshot.pop();
                    draw::fill_rect(snapshot, x - 1.0, mid - 5.0, 2.0, 10.0, lamp::LEGEND);
                }
                None => {
                    draw::fill_rect(snapshot, kx, ky, 10.0, 18.0, lamp::FACE);
                    draw::stroke(snapshot, &draw::rect_path(kx + 0.5, ky + 0.5, 9.0, 17.0), tone::AXIS, 1.0);
                    draw::fill_rect(snapshot, x - 1.0, mid - 5.0, 2.0, 10.0, tone::PRESENT);
                }
            }
        });
        canvas.set_hexpand(true);
        let changed = Rc::new(changed);
        let motion = gtk::EventControllerMotion::new();
        let (h, c) = (hovering.clone(), canvas.downgrade());
        motion.connect_enter(move |_, _, _| {
            h.set(true);
            if let Some(c) = c.upgrade() {
                c.queue_draw();
            }
        });
        let (h, c) = (hovering.clone(), canvas.downgrade());
        motion.connect_leave(move |_| {
            h.set(false);
            if let Some(c) = c.upgrade() {
                c.queue_draw();
            }
        });
        canvas.add_controller(motion);
        let drag = gtk::GestureDrag::new();
        let set_from = {
            let (v, c, changed) = (value.clone(), canvas.downgrade(), changed.clone());
            move |x: f64| {
                let Some(c) = c.upgrade() else { return };
                let width = (c.width() as f64 - 12.0).max(1.0);
                let part = ((x - 6.0) / width).clamp(0.0, 1.0);
                let next = (low + part * span).round();
                if next != v.get() {
                    v.set(next);
                    changed(next);
                }
                c.queue_draw();
            }
        };
        let set_from = Rc::new(set_from);
        let (d, s) = (dragging.clone(), set_from.clone());
        drag.connect_drag_begin(move |_, x, _| {
            d.set(true);
            sound::press();
            s(x);
        });
        let s = set_from.clone();
        drag.connect_drag_update(move |gesture, dx, _| {
            if let Some((x, _)) = gesture.start_point() {
                s(x + dx);
            }
        });
        let (d, c) = (dragging.clone(), canvas.downgrade());
        drag.connect_drag_end(move |_, _, _| {
            d.set(false);
            sound::release();
            if let Some(c) = c.upgrade() {
                c.queue_draw();
            }
        });
        canvas.add_controller(drag);
        Self { canvas, value }
    }

    pub fn value(&self) -> f64 {
        self.value.get()
    }

    pub fn set_value(&self, value: f64) {
        self.value.set(value);
        self.canvas.queue_draw();
    }
}

/// A part of a page that opens and closes: a small mark and the name, then the contents.
pub struct Disclosure {
    pub root: gtk::Box,
    pub header: gtk::Box,
    pub content: gtk::Box,
    expanded: Rc<Cell<bool>>,
    revealer: gtk::Revealer,
    mark: Canvas,
}

impl Disclosure {
    pub fn new(title: &str, expanded: bool, changed: impl Fn(bool) + 'static) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let button = gtk::Button::new();
        button.add_css_class("disclosure");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        let state = Rc::new(Cell::new(expanded));
        let s = state.clone();
        let mark = Canvas::new(7, 7, move |_, snapshot, w, h| {
            let builder = gtk::gsk::PathBuilder::new();
            if s.get() {
                builder.move_to(0.0, 1.0);
                builder.line_to(w, 1.0);
                builder.line_to(w / 2.0, h);
            } else {
                builder.move_to(1.0, 0.0);
                builder.line_to(w, h / 2.0);
                builder.line_to(1.0, h);
            }
            builder.close();
            draw::fill(snapshot, &builder.to_path(), tone::PRESENT);
        });
        mark.set_valign(gtk::Align::Center);
        header.append(&mark);
        if !title.is_empty() {
            header.append(&theme::label(font::SMALL, title, ""));
        }
        button.set_child(Some(&header));
        key_sounds(&button, false);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::None);
        revealer.set_child(Some(&content));
        revealer.set_reveal_child(expanded);
        root.append(&button);
        root.append(&revealer);
        let disclosure = Rc::new(Self { root, header, content, expanded: state, revealer, mark });
        let weak = Rc::downgrade(&disclosure);
        button.connect_clicked(move |_| {
            if let Some(d) = weak.upgrade() {
                let now = !d.expanded.get();
                d.set_expanded(now);
                changed(now);
            }
        });
        disclosure
    }

    pub fn expanded(&self) -> bool {
        self.expanded.get()
    }

    pub fn set_expanded(&self, expanded: bool) {
        self.expanded.set(expanded);
        self.revealer.set_reveal_child(expanded);
        self.mark.queue_draw();
    }
}

/// Keeps `glib` in the public interface of this module for the page code that uses it.
pub type WeakButton = glib::WeakRef<gtk::Button>;
