//! Map displayed values to the cells Ratatui actually rendered, including wrapped
//! lines and clipped fields. The temporary style IDs are removed before drawing.
use std::fmt;

use ratatui::{
    buffer::Buffer,
    layout::Position,
    style::{Color, Style},
    text::{Line, Span},
};

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Text { label: String, value: String },
    Json { row: usize, pointer: String },
}

#[derive(Clone)]
struct Mark {
    target: Target,
    underline_color: Color,
}

#[derive(Clone, Default)]
pub(crate) struct Values {
    marks: Vec<Mark>,
    hits: std::collections::HashMap<Position, usize>,
    selected: Option<(Position, Target)>,
    recording: bool,
}

impl fmt::Debug for Values {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Values").finish_non_exhaustive()
    }
}

impl Values {
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    pub fn begin(&mut self, enabled: bool) {
        self.recording = enabled;
        self.marks.clear();
        self.hits.clear();
        if !enabled {
            self.selected = None;
        }
    }

    pub fn span(&mut self, mut span: Span<'static>, target: Target) -> Span<'static> {
        if self.recording && !span.content.trim().is_empty() && self.marks.len() < 0xff_ffff {
            let id = self.marks.len() + 1;
            self.marks.push(Mark {
                target,
                underline_color: span.style.underline_color.unwrap_or(Color::Reset),
            });
            span.style =
                span.style
                    .underline_color(Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8));
        }
        span
    }

    pub fn finish(&mut self, buffer: &mut Buffer, highlight: Style) {
        self.recording = false;
        if self.marks.is_empty() {
            return;
        }
        let area = buffer.area;
        for (index, cell) in buffer.content.iter_mut().enumerate() {
            // No theme style uses an RGB underline. Underline modifiers, such
            // as those on links, are independent of this temporary color.
            let Color::Rgb(r, g, b) = cell.underline_color else {
                continue;
            };
            let id = (usize::from(r) << 16) | (usize::from(g) << 8) | usize::from(b);
            let Some(mark) = id.checked_sub(1).and_then(|i| self.marks.get(i)) else {
                continue;
            };
            cell.underline_color = mark.underline_color;
            let columns = usize::from(area.width);
            let position = Position::new(
                area.x + (index % columns) as u16,
                area.y + (index / columns) as u16,
            );
            let width = if cell.symbol().len() == 1 {
                1
            } else {
                Line::from(cell.symbol()).width() as u16
            };
            for x in position.x..position.x.saturating_add(width).min(area.right()) {
                self.hits.insert(Position::new(x, position.y), id - 1);
            }
        }
        if let Some((position, target)) = &self.selected
            && let Some(&id) = self.hits.get(position)
            && self.marks[id].target == *target
        {
            for (position, &hit) in &self.hits {
                if hit == id
                    && let Some(cell) = buffer.cell_mut(*position)
                {
                    cell.set_style(highlight);
                }
            }
        }
    }

    pub fn select(&mut self, position: Position) -> Option<Target> {
        let target = self.marks.get(*self.hits.get(&position)?)?.target.clone();
        self.selected = Some((position, target.clone()));
        Some(target)
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        layout::Rect,
        widgets::{Paragraph, Widget, Wrap},
    };

    #[test]
    fn wrapped_values_and_wide_glyphs_share_one_copy_target() {
        let mut values = Values::default();
        let area = Rect::new(0, 0, 8, 3);
        let mut buffer = Buffer::empty(area);
        let target = Target::Text {
            label: "field".into(),
            value: "🌱abc defgh".into(),
        };
        values.begin(true);
        let field = values.span(Span::raw("🌱abc defgh"), target.clone());
        Paragraph::new(vec![Line::raw("Label:"), Line::from(field)])
            .wrap(Wrap { trim: true })
            .render(area, &mut buffer);
        values.finish(&mut buffer, Style::default());

        assert!(values.select(Position::new(0, 0)).is_none());
        assert!(values.select(Position::new(1, 1)) == Some(target.clone()));
        assert!(values.select(Position::new(0, 2)) == Some(target));
        assert!(
            buffer
                .content
                .iter()
                .all(|cell| cell.underline_color == Color::Reset)
        );
    }
}
