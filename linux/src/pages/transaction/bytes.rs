//! Raw output data as hex and text side by side, sixteen bytes to a row (`BytesView` in
//! `TransactionExtras.swift`). One cursor marks the same byte in both columns. Move it with
//! the arrow keys, Page Up, Page Down, Home and End, or click a byte.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::gdk::Key;
use gtk::glib;
use gtk::prelude::*;

use crate::page::{self, Page};
use crate::theme::{draw, font, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;

const COLUMNS: usize = 16;
const PAGE_ROWS: usize = 16;
const LIMIT: usize = 8192;
/// The width of the hex column, as the Swift view's frame of 16 × 22.
const HEX_WIDTH: f32 = (COLUMNS * 22) as f32;
const SPACING: f32 = 14.0;

pub fn parse(hex: &str) -> Vec<u8> {
    let bytes = hex.as_bytes();
    bytes
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .filter_map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

pub fn printable(byte: u8) -> char {
    if (32..=126).contains(&byte) { byte as char } else { '.' }
}

/// Where a key moves the cursor, in a view of `count` bytes.
pub fn moved(cursor: usize, key: Key, count: usize) -> usize {
    if count == 0 {
        return cursor;
    }
    let last = count - 1;
    match key {
        Key::Left => cursor.saturating_sub(1),
        Key::Right => (cursor + 1).min(last),
        Key::Up => cursor.saturating_sub(COLUMNS),
        Key::Down => (cursor + COLUMNS).min(last),
        Key::Page_Up => cursor.saturating_sub(COLUMNS * PAGE_ROWS),
        Key::Page_Down => (cursor + COLUMNS * PAGE_ROWS).min(last),
        Key::Home => 0,
        Key::End => last,
        _ => cursor,
    }
}

fn status_text(data: &[u8], shown: usize, cursor: usize) -> String {
    let byte = if cursor < shown { data[cursor] } else { 0 };
    format!(
        "Cursor at offset {cursor} (0x{cursor:04x}): 0x{byte:02x}, decimal {byte}, text {}",
        printable(byte)
    )
}

/// The view. `script_body` is true when the pushes did not decode and the bytes are the
/// script after `OP_RETURN`.
pub fn bytes_view(page: &Rc<Page>, hex: &str, script_body: bool) -> gtk::Overlay {
    let data: Rc<Vec<u8>> = Rc::new(parse(hex));
    let shown = data.len().min(LIMIT);
    let cursor = Rc::new(Cell::new(0usize));

    let frame = gtk::Box::new(gtk::Orientation::Vertical, 6);
    frame.add_css_class("tx-bytes");
    frame.set_focusable(true);
    frame.set_hexpand(true);
    frame.append(&page::text(
        if script_body {
            "The pushes did not decode. These are the script bytes after OP_RETURN."
        } else {
            "The payload bytes of every push, joined in order."
        },
        &["caption-text", "label"],
    ));
    let copy = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    copy.append(&page::text("Copy all the bytes as hexadecimal", &["caption-text", "label"]));
    copy.append(&page::copy_button(hex));
    frame.append(&copy);

    // The rows: one layout for each part of a row, made once.
    type Row = (gtk::pango::Layout, gtk::pango::Layout, gtk::pango::Layout);
    let layouts: Rc<RefCell<Vec<Row>>> = Rc::default();
    let metrics: Rc<Cell<(f32, f32)>> = Rc::new(Cell::new((7.2, 16.0)));
    let rows = shown.div_ceil(COLUMNS);
    let canvas = {
        let (data, cursor, layouts, metrics) = (data.clone(), cursor.clone(), layouts.clone(), metrics.clone());
        Canvas::new(0, 0, move |canvas, snapshot, _, _| {
            let (cw, line) = metrics.get();
            if layouts.borrow().is_empty() {
                let mut made = layouts.borrow_mut();
                for row in 0..rows {
                    let offset = row * COLUMNS;
                    let end = (offset + COLUMNS).min(shown);
                    let hex: Vec<String> = data[offset..end].iter().map(|b| format!("{b:02x}")).collect();
                    let text: String = data[offset..end].iter().map(|b| printable(*b)).collect();
                    made.push((
                        draw::layout(canvas, font::MONO_CALLOUT, &format!("{offset:04x}")),
                        draw::layout(canvas, font::MONO_CALLOUT, &hex.join(" ")),
                        draw::layout(canvas, font::MONO_CALLOUT, &text),
                    ));
                }
            }
            let hex_left = 4.0 * cw + SPACING;
            let text_left = hex_left + HEX_WIDTH + SPACING;
            let at = cursor.get();
            for (row, (offset, hex, text)) in layouts.borrow().iter().enumerate() {
                let y = row as f32 * (line + 1.0);
                draw::draw_layout(snapshot, offset, 0.0, y, draw::Anchor::TopLeading, tone::AXIS);
                draw::draw_layout(snapshot, hex, hex_left + 2.0, y, draw::Anchor::TopLeading, tone::VALUE);
                draw::draw_layout(snapshot, text, text_left, y, draw::Anchor::TopLeading, tone::LABEL);
                if at / COLUMNS == row && at < shown {
                    let column = (at % COLUMNS) as f32;
                    let byte = data[at];
                    let x = hex_left + column * 3.0 * cw;
                    draw::fill_rect(snapshot, x, y, 2.0 * cw + 4.0, line, tone::PRESENT);
                    draw::text(snapshot, canvas, font::MONO_CALLOUT, &format!("{byte:02x}"), x + 2.0, y, draw::Anchor::TopLeading, tone::SCREEN);
                    let tx = text_left + column * cw;
                    draw::fill_rect(snapshot, tx, y, cw, line, tone::PRESENT);
                    draw::text(snapshot, canvas, font::MONO_CALLOUT, &printable(byte).to_string(), tx, y, draw::Anchor::TopLeading, tone::SCREEN);
                }
            }
        })
    };
    // The size of a character of the monospaced face.
    let (cw, line) = {
        let probe = draw::layout(&canvas, font::MONO_CALLOUT, "0000000000");
        let (_, logical) = probe.pixel_extents();
        (logical.width() as f32 / 10.0, logical.height() as f32)
    };
    metrics.set((cw, line));
    let width = 4.0 * cw + SPACING + HEX_WIDTH + SPACING + COLUMNS as f32 * cw;
    canvas.set_size(width.ceil() as i32, (rows as f32 * (line + 1.0)).ceil() as i32);
    canvas.set_halign(gtk::Align::Start);
    frame.append(&canvas);

    let status = gtk::Box::new(gtk::Orientation::Vertical, 2);
    status.set_margin_top(4);
    let cursor_line = page::text(&status_text(&data, shown, 0), &["caption-small", "present"]);
    cursor_line.set_selectable(true);
    cursor_line.set_visible(shown > 0);
    status.append(&cursor_line);
    status.append(&page::text(
        &if data.len() > shown {
            format!("{} bytes. The view shows the first {shown}.", data.len())
        } else {
            format!("{} bytes. Dots stand for bytes that are not printable.", data.len())
        },
        &["caption-small", "label"],
    ));
    frame.append(&status);

    let set = {
        let (cursor, canvas, line_label, data) = (cursor.clone(), canvas.downgrade(), cursor_line.downgrade(), data.clone());
        Rc::new(move |index: usize| {
            cursor.set(index);
            if let Some(canvas) = canvas.upgrade() {
                canvas.queue_draw();
                canvas.update_property(&[gtk::accessible::Property::Label(&format!("Bytes. The cursor is at byte {index}."))]);
            }
            if let Some(label) = line_label.upgrade() {
                label.set_text(&status_text(&data, shown, index));
            }
        })
    };

    // A click on a byte, in either column, moves the cursor there.
    let click = gtk::GestureClick::new();
    {
        let (set, metrics, frame) = (set.clone(), metrics.clone(), frame.downgrade());
        click.connect_pressed(move |_, _, x, y| {
            if let Some(frame) = frame.upgrade() {
                frame.grab_focus();
            }
            let (cw, line) = metrics.get();
            let (x, y) = (x as f32, y as f32);
            let row = (y / (line + 1.0)).floor().max(0.0) as usize;
            let hex_left = 4.0 * cw + SPACING;
            let text_left = hex_left + HEX_WIDTH + SPACING;
            let column = if x >= text_left {
                ((x - text_left) / cw).floor() as usize
            } else if x >= hex_left {
                ((x - hex_left) / (3.0 * cw)).floor() as usize
            } else {
                return;
            };
            if column >= COLUMNS {
                return;
            }
            let index = row * COLUMNS + column;
            if index < shown {
                set(index);
            }
        });
    }
    canvas.add_controller(click);

    let keys = gtk::EventControllerKey::new();
    {
        let (set, cursor) = (set.clone(), cursor.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            if matches!(key, Key::Left | Key::Right | Key::Up | Key::Down | Key::Page_Up | Key::Page_Down | Key::Home | Key::End) {
                set(moved(cursor.get(), key, shown));
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    frame.add_controller(keys);

    // While the bytes have the keyboard, the arrow keys move the byte cursor.
    let focus = gtk::EventControllerFocus::new();
    let page_cursor = page.cursor.clone();
    focus.connect_enter(move |_| page_cursor.suspended.set(true));
    let page_cursor = page.cursor.clone();
    focus.connect_leave(move |_| page_cursor.suspended.set(false));
    frame.add_controller(focus);
    let page_cursor = Rc::downgrade(&page.cursor);
    frame.connect_destroy(move |_| {
        if let Some(cursor) = page_cursor.upgrade() {
            cursor.suspended.set(false);
        }
    });

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&frame));
    overlay.add_overlay(&display::corner_marks());
    overlay
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_and_moves() {
        assert_eq!(parse("00ff4142zz7"), vec![0, 255, 0x41, 0x42]);
        assert_eq!(printable(0x41), 'A');
        assert_eq!(printable(0x7f), '.');
        assert_eq!(moved(0, Key::Left, 40), 0);
        assert_eq!(moved(5, Key::Down, 40), 21);
        assert_eq!(moved(30, Key::Down, 40), 39);
        assert_eq!(moved(39, Key::Page_Up, 40), 0);
        assert_eq!(moved(3, Key::End, 40), 39);
        assert_eq!(status_text(&[0x41], 1, 0), "Cursor at offset 0 (0x0000): 0x41, decimal 65, text A");
    }
}
