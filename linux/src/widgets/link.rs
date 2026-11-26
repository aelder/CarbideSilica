//! A link in the display: pale blue text with no underline (`DisplayLinkStyle`). Under the
//! pointer a link gets light and a line below it, and a press has a quiet sound of a key.

use gtk::prelude::*;

use crate::theme::{self, font, sound};

/// A link with `text` in the body face, or in the monospaced face for an ID.
pub fn link(text: &str, mono: bool, action: impl Fn() + 'static) -> gtk::Button {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    if mono {
        label.add_css_class("mono");
    }
    wrap(label, action)
}

/// A link whose text is in a type role.
pub fn link_in(role: font::Role, text: &str, action: impl Fn() + 'static) -> gtk::Button {
    wrap(theme::label(role, text, ""), action)
}

fn wrap(label: gtk::Label, action: impl Fn() + 'static) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("display-link");
    button.set_child(Some(&label));
    button.set_halign(gtk::Align::Start);
    button.set_valign(gtk::Align::Baseline);
    button.set_cursor_from_name(Some("pointer"));
    button.connect_clicked(move |_| action());
    let click = gtk::GestureClick::new();
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let target = button.downgrade();
    click.connect_pressed(move |_, _, _, _| {
        if let Some(button) = target.upgrade().filter(|b| b.is_sensitive()) {
            button.add_css_class("pressed");
            sound::link();
        }
    });
    let target = button.downgrade();
    click.connect_released(move |_, _, _, _| {
        if let Some(button) = target.upgrade() {
            button.remove_css_class("pressed");
        }
    });
    let target = button.downgrade();
    click.connect_cancel(move |_, _| {
        if let Some(button) = target.upgrade() {
            button.remove_css_class("pressed");
        }
    });
    button.add_controller(click);
    button
}

/// The label of a link made here.
pub fn label_of(button: &gtk::Button) -> Option<gtk::Label> {
    button.child().and_downcast::<gtk::Label>()
}

/// Changes the text of a link made by [`link`].
pub fn set_text(button: &gtk::Button, text: &str) {
    if let Some(label) = label_of(button) {
        label.set_text(text);
    }
}
