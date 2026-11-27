//! The light of a row of a list under the pointer (`RowLight`): a faint field, a lit line
//! at its left edge, and a quiet sound when the pointer comes to it. With an action, a
//! click on the row does the action, with the quiet sound of a link.

use gtk::prelude::*;

use crate::theme::sound;

pub fn row_light(row: &impl IsA<gtk::Widget>, action: Option<Box<dyn Fn()>>) {
    let row = row.as_ref();
    row.add_css_class("row-light");
    let motion = gtk::EventControllerMotion::new();
    let target = row.downgrade();
    motion.connect_enter(move |_, _, _| {
        if let Some(row) = target.upgrade() {
            row.add_css_class("hover");
            sound::row();
        }
    });
    let target = row.downgrade();
    motion.connect_leave(move |_| {
        if let Some(row) = target.upgrade() {
            row.remove_css_class("hover");
        }
    });
    row.add_controller(motion);
    if let Some(action) = action {
        let click = gtk::GestureClick::new();
        click.connect_released(move |gesture, presses, _, _| {
            if presses == 1 {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                sound::link();
                action();
            }
        });
        row.add_controller(click);
    }
}
