//! A read-only byte inspector with one cursor for both representations.
use super::*;

#[derive(Debug, Clone)]
pub(super) struct View {
    title: String,
    bytes: Arc<[u8]>,
    cursor: usize,
    top_row: usize,
}

impl View {
    pub(super) fn new(title: String, bytes: Arc<[u8]>) -> Self {
        Self {
            title,
            bytes,
            cursor: 0,
            top_row: 0,
        }
    }

    fn dimensions(area: Rect) -> (usize, usize) {
        let columns = usize::from(area.width.saturating_sub(13) / 4).clamp(1, 16);
        let rows = usize::from(area.height.saturating_sub(3)).max(1);
        (columns, rows)
    }

    fn first_visible_row(&self, columns: usize, rows: usize) -> usize {
        let last_page = self.bytes.len().div_ceil(columns).saturating_sub(rows);
        let cursor_row = self.cursor / columns;
        self.top_row
            .min(last_page)
            .min(cursor_row)
            .max(cursor_row.saturating_sub(rows - 1))
    }

    pub(super) fn key(&mut self, code: KeyCode, area: Rect) -> bool {
        let (columns, rows) = Self::dimensions(area);
        let last = self.bytes.len().saturating_sub(1);
        self.cursor = match code {
            KeyCode::Left | KeyCode::Char('h') => self.cursor.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => self.cursor.saturating_add(1).min(last),
            KeyCode::Up | KeyCode::Char('k') => self.cursor.saturating_sub(columns),
            KeyCode::Down | KeyCode::Char('j') => self.cursor.saturating_add(columns).min(last),
            KeyCode::PageUp => self.cursor.saturating_sub(columns * rows),
            KeyCode::PageDown => self.cursor.saturating_add(columns * rows).min(last),
            KeyCode::Home => 0,
            KeyCode::End => last,
            _ => return false,
        };
        self.top_row = self.first_visible_row(columns, rows);
        true
    }

    pub(super) fn render(&self, frame: &mut Frame<'_>, theme: &Theme, area: Rect) {
        let (columns, rows) = Self::dimensions(area);
        let offset = self.first_visible_row(columns, rows) * columns;
        let cursor = theme.selection_cursor();
        let mut lines = vec![
            Line::styled(self.title.clone(), theme.focused()),
            Line::from(self.bytes.get(self.cursor).map_or_else(
                || "Empty payload".into(),
                |byte| {
                    format!(
                        "{} bytes | Offset 0x{:08X} | 0x{byte:02X}",
                        self.bytes.len(),
                        self.cursor
                    )
                },
            )),
            Line::styled(
                format!(
                    "{:10}{:<hex_width$} ASCII",
                    "Offset",
                    "Hex",
                    hex_width = columns * 3
                ),
                theme.muted(),
            ),
        ];
        for (row, chunk) in self.bytes[offset..].chunks(columns).take(rows).enumerate() {
            let first = offset + row * columns;
            let mut spans = vec![Span::styled(format!("{first:08X}  "), theme.muted())];
            for (column, byte) in chunk.iter().enumerate() {
                let style = if first + column == self.cursor {
                    cursor
                } else {
                    theme.base()
                };
                spans.extend([Span::styled(format!("{byte:02X}"), style), Span::raw(" ")]);
            }
            spans.push(Span::raw(" ".repeat((columns - chunk.len()) * 3 + 1)));
            for (column, byte) in chunk.iter().enumerate() {
                let style = if first + column == self.cursor {
                    cursor
                } else {
                    theme.base()
                };
                let character = if byte.is_ascii_graphic() || *byte == b' ' {
                    char::from(*byte)
                } else {
                    '.'
                };
                spans.push(Span::styled(character.to_string(), style));
            }
            lines.push(Line::from(spans));
        }
        let total = self.bytes.len().div_ceil(columns);
        frame.render_widget(
            Paragraph::new(lines).style(theme.base()),
            scroll_view::content_area(area, total > rows),
        );
        let rail = Rect::new(
            area.x,
            area.y.saturating_add(3),
            area.width,
            area.height.saturating_sub(3),
        );
        scroll_view::render(frame, theme, rail, total, offset / columns, rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn both_columns_highlight_the_same_byte_across_rows_and_after_resize() {
        let data: Arc<[u8]> = (0x20..0x61).collect::<Vec<_>>().into();
        let mut view = View::new("OP_RETURN".into(), data.clone());
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        for width in [80, 43] {
            let mut terminal = Terminal::new(TestBackend::new(width, 8)).unwrap();
            let area = Rect::new(0, 0, width, 8);
            view.key(KeyCode::Home, area);
            view.key(KeyCode::Down, area);
            view.key(KeyCode::Right, area);
            let (columns, _) = View::dimensions(area);
            let cursor = columns + 1;
            terminal
                .draw(|frame| view.render(frame, &theme, area))
                .unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(view.cursor, cursor);
            let hex = format!("{:02X}", data[cursor]);
            assert_eq!(buffer[(13, 4)].symbol(), &hex[..1]);
            assert_eq!(buffer[(14, 4)].symbol(), &hex[1..]);
            let ascii_x = (12 + columns * 3) as u16;
            assert_eq!(
                buffer[(ascii_x, 4)].symbol(),
                char::from(data[cursor]).to_string()
            );
            for x in [13, 14, ascii_x] {
                assert_eq!(buffer[(x, 4)].bg, theme.accent);
            }
            assert_ne!(buffer[(ascii_x - 1, 4)].bg, theme.accent);
            view.key(KeyCode::End, area);
            assert_eq!(view.cursor, data.len() - 1);
            assert!(view.top_row > 0 || data.len().div_ceil(columns) <= 5);
            view.key(KeyCode::Home, area);
            assert_eq!((view.cursor, view.top_row), (0, 0));
        }
    }
}
