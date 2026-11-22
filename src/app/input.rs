//! Normalize navigation events once, before dispatch to any screen or overlay.
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, MouseEventKind};

pub(super) fn quit_pressed(event: &Event) -> bool {
    matches!(event, Event::Key(key) if key.code == KeyCode::Char('q') && key.kind == KeyEventKind::Press)
}

pub(super) fn navigation_key(event: Event) -> Option<KeyEvent> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => Some(key),
        Event::Mouse(mouse) => match mouse.kind {
            MouseEventKind::ScrollUp => Some(KeyEvent::new(KeyCode::Up, mouse.modifiers)),
            MouseEventKind::ScrollDown => Some(KeyEvent::new(KeyCode::Down, mouse.modifiers)),
            _ => None,
        },
        _ => None,
    }
}

/// Release all button reporting so the terminal owns selection and PRIMARY paste.
pub(super) fn set_mouse_capture(
    writer: &mut impl std::io::Write,
    enabled: bool,
) -> std::io::Result<()> {
    if enabled {
        crossterm::execute!(writer, crossterm::event::EnableMouseCapture)
    } else {
        crossterm::execute!(writer, crossterm::event::DisableMouseCapture)
    }
}

pub(super) fn paste_text(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_control())
        .take(4096)
        .collect()
}

pub(super) fn paste_height(input: &mut String, text: &str) {
    let text = text.trim();
    if text.chars().all(|ch| ch.is_ascii_digit()) {
        input.extend(text.chars().take(12usize.saturating_sub(input.len())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    #[test]
    fn selection_mode_releases_button_motion_and_sgr_mouse_reporting() {
        let mut bytes = Vec::new();
        set_mouse_capture(&mut bytes, false).unwrap();
        for sequence in [
            b"\x1b[?1000l",
            b"\x1b[?1002l",
            b"\x1b[?1003l",
            b"\x1b[?1006l",
        ] {
            assert!(bytes.windows(sequence.len()).any(|part| part == sequence));
        }
        assert!(!bytes.contains(&b'h'));
    }

    #[test]
    fn paste_strips_control_characters_and_preserves_unicode() {
        assert_eq!(paste_text("VRSC@\r\n\t\x1b\0🌱"), "VRSC@🌱");
        let mut height = "12".to_string();
        paste_height(&mut height, "34");
        assert_eq!(height, "1234");
        paste_height(&mut height, "foo56");
        assert_eq!(height, "1234");
    }

    fn mouse(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 12,
            row: 8,
            modifiers: KeyModifiers::NONE,
        })
    }
    #[test]
    fn each_reported_wheel_event_becomes_exactly_one_arrow_step() {
        for (kind, code) in [
            (MouseEventKind::ScrollUp, KeyCode::Up),
            (MouseEventKind::ScrollDown, KeyCode::Down),
        ] {
            let key = navigation_key(mouse(kind)).expect("wheel must reach shared navigation");
            assert_eq!(key.code, code);
            assert_eq!(key.kind, KeyEventKind::Press);
            assert_eq!(
                (0..3).filter_map(|_| navigation_key(mouse(kind))).count(),
                3
            );
        }
    }
    #[test]
    fn pointer_motion_buttons_and_key_release_never_trigger_navigation() {
        for kind in [
            MouseEventKind::Moved,
            MouseEventKind::Down(MouseButton::Left),
            MouseEventKind::ScrollLeft,
            MouseEventKind::ScrollRight,
        ] {
            assert!(navigation_key(mouse(kind)).is_none());
        }
        let mut key = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(navigation_key(Event::Key(key)), Some(key));
        key.kind = KeyEventKind::Release;
        assert!(navigation_key(Event::Key(key)).is_none());
    }
}
