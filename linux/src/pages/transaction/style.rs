//! The few styles of the Transaction page that `style.css` does not have: the mark of a
//! row that the statement marks, the frame of a leg and of an output's JSON, the boxes of
//! the receipt, and links in the tones of the statement. They use the named colours of
//! `theme/mod.rs` and are added once, for the display, by this page.

use std::cell::Cell;

use gtk::gdk;

const CSS: &str = r#"
.tx-marked { background-color: alpha(@present, 0.08); }
.tx-chosen-line {
    background-color: alpha(@present, 0.08);
    box-shadow: inset 2px 0 0 @present, inset 5px 0 3px -2px alpha(@present, 0.6);
}
.tx-frame { background-color: @screen; border: 1px solid @rule; padding: 12px; }
.tx-json { background-color: @screen; border: 1px solid @rule; padding: 10px; }
.tx-bytes { background-color: @screen; border: 1px solid @rule; border-radius: 4px; padding: 14px; }
.tx-bytes:focus-visible, .tx-bytes:focus { outline: none; }
.tx-box { border: 1px solid @axis; padding: 7px 10px; }
.tx-box.strong-good { border-color: @good; background-color: alpha(@good, 0.08); }
.tx-box.strong-present { border-color: @present; background-color: alpha(@present, 0.08); }
.tx-box.open { border-style: dashed; }
.display-link.tx-tone-value { color: @value; }
.display-link.tx-tone-label { color: @label; }
.display-link.tx-tone-value:hover, .display-link.tx-tone-label:hover { color: rgb(214, 242, 235); }
.tx-sentence { font-size: 15px; }
.tx-verdict { font-size: 15px; }
.tx-plate-key { background: none; border: none; box-shadow: none; padding: 0; min-height: 0; min-width: 0; }
.tx-plate-key:hover .plate { border-color: @value; color: @value; }
.tx-plate-key:hover .plate.fill-record, .tx-plate-key:hover .plate.fill-mined { color: @screen; }
.tx-icon-key { background: none; border: none; box-shadow: none; padding: 0 2px; min-height: 0; min-width: 16px; }
"#;

thread_local! {
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// Adds the styles once.
pub fn install() {
    if INSTALLED.with(|done| done.replace(true)) {
        return;
    }
    let Some(display) = gdk::Display::default() else { return };
    let provider = gtk::CssProvider::new();
    // The named colours are defined by the app's own style sheet. A sheet of its own
    // cannot see them, so it names them again from the same constants.
    let mut css = String::new();
    for line in crate::theme::stylesheet().lines().filter(|l| l.starts_with("@define-color")) {
        css.push_str(line);
        css.push('\n');
    }
    css.push_str(CSS);
    provider.load_from_string(&css);
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
}
