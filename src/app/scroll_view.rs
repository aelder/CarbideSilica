use super::*;
use ratatui::widgets::{HighlightSpacing, List, ListItem, ListState, Padding};

const SCROLLBAR_COLUMNS: u16 = 2; // One blank gutter column and one rail column.

pub(super) fn content_area(area: Rect, overflowing: bool) -> Rect {
    let reserve = overflowing && area.width > SCROLLBAR_COLUMNS && area.height > 0;
    Rect {
        width: area.width - if reserve { SCROLLBAR_COLUMNS } else { 0 },
        ..area
    }
}

pub(super) fn render(
    frame: &mut Frame<'_>,
    theme: &Theme,
    area: Rect,
    total: usize,
    first: usize,
    visible: usize,
) {
    let content = content_area(area, total > visible);
    if content.width == area.width || visible == 0 {
        return;
    }
    let height = usize::from(area.height);
    let thumb = (visible.saturating_mul(height) / total).clamp(1, (height - 1).max(1));
    let max_first = total - visible;
    let start = first
        .min(max_first)
        .saturating_mul(height - thumb)
        .saturating_add(max_first / 2)
        / max_first;
    let gutter_x = content.right();
    let rail_x = gutter_x + 1;
    for (offset, y) in (area.y..area.bottom()).enumerate() {
        let active = (start..start + thumb).contains(&offset);
        if let Some(cell) = frame.buffer_mut().cell_mut((gutter_x, y)) {
            cell.reset();
            cell.set_style(theme.tile_gutter());
        }
        if let Some(cell) = frame.buffer_mut().cell_mut((rail_x, y)) {
            cell.reset();
            cell.set_symbol(if active { "█" } else { "│" })
                .set_style(if active {
                    theme.base().fg(theme.accent)
                } else {
                    theme.muted()
                });
        }
    }
}

pub(super) fn paragraph_area(paragraph: &Paragraph<'_>, area: Rect) -> Rect {
    content_area(
        area,
        paragraph.line_count(area.width.max(1)) > usize::from(area.height),
    )
}

pub(super) fn follow_selection(lines: &[Line<'_>], area: Rect, scroll: u16) -> u16 {
    let Some(index) = lines.iter().position(|line| {
        line.spans
            .first()
            .is_some_and(|span| span.content == SELECTION_MARKER)
    }) else {
        return scroll;
    };
    let paragraph = Paragraph::new(lines.to_vec()).wrap(Wrap { trim: false });
    let width = paragraph_area(&paragraph, area).width.max(1);
    let count = |lines: &[Line<'_>]| {
        Paragraph::new(lines.to_vec())
            .wrap(Wrap { trim: false })
            .line_count(width)
    };
    let start = count(&lines[..index]);
    let end = count(&lines[..=index]);
    let last_start = end.saturating_sub(usize::from(area.height)).min(start);
    usize::from(scroll)
        .clamp(last_start, start)
        .min(usize::from(u16::MAX)) as u16
}

pub(super) fn paragraph(
    frame: &mut Frame<'_>,
    theme: &Theme,
    paragraph: Paragraph<'_>,
    area: Rect,
    scroll: usize,
) -> u16 {
    let content = paragraph_area(&paragraph, area);
    let total = paragraph.line_count(content.width.max(1));
    let visible = usize::from(content.height);
    let first = scroll
        .min(total.saturating_sub(visible))
        .min(usize::from(u16::MAX));
    frame.render_widget(paragraph.scroll((first as u16, 0)), content);
    render(frame, theme, area, total, first, visible);
    first as u16
}

pub(super) fn list(
    frame: &mut Frame<'_>,
    theme: &Theme,
    rows: Vec<Vec<Line<'_>>>,
    area: Rect,
    selected: usize,
) {
    if area.is_empty() {
        return;
    }
    // List renders only whole items. Keep the first line and cursor visible
    // even when the terminal cannot fit a complete multi-line entry.
    let items: Vec<_> = rows
        .into_iter()
        .map(|mut lines| {
            lines.truncate(usize::from(area.height));
            ListItem::new(lines)
        })
        .collect();
    let heights: Vec<_> = items.iter().map(ListItem::height).collect();
    let total: usize = heights.iter().sum();
    let visible = usize::from(area.height);
    let mut selection = ListState::default().with_selected((!items.is_empty()).then_some(selected));
    let content = content_area(area, total > visible);
    let padding = u16::from(content.width > 3);
    frame.render_stateful_widget(
        List::new(items)
            .style(theme.base())
            .block(Block::default().padding(Padding::right(padding)))
            .highlight_symbol("  ")
            .highlight_spacing(HighlightSpacing::Always)
            .highlight_style(theme.tile_body(true)),
        content,
        &mut selection,
    );
    let first = heights.iter().take(selection.offset()).sum();
    if let Some(index) = selection.selected()
        && let Some(preceding) = heights.get(selection.offset()..index)
    {
        let offset = preceding.iter().sum::<usize>();
        if padding > 0 {
            let height = heights[index].min(visible.saturating_sub(offset));
            frame.render_widget(
                Block::default().style(theme.tile_body(true)),
                Rect::new(
                    content.right() - padding,
                    area.y + offset as u16,
                    padding,
                    height as u16,
                ),
            );
        }
        let height = heights[index]
            .saturating_sub(1)
            .max(1)
            .min(visible.saturating_sub(offset));
        if height > 0 {
            render_selection_gutter(
                frame,
                theme,
                Rect::new(area.x, area.y + offset as u16, 1, height as u16),
            );
        }
    }
    // Count complete entries. A trailing empty row can't fit a further entry.
    let mut used = 0;
    for height in heights.iter().skip(selection.offset()) {
        if used + height > visible {
            break;
        }
        used += height;
    }
    render(frame, theme, area, total, first, used);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    #[test]
    fn short_list_shows_selected_label_and_rail_on_separate_columns() {
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        let mut terminal = Terminal::new(TestBackend::new(12, 1)).unwrap();
        terminal
            .draw(|frame| {
                let items = ["First", "SecondEntry"]
                    .into_iter()
                    .map(|label| vec![Line::raw(label), Line::raw("details"), Line::default()])
                    .collect();
                list(frame, &theme, items, frame.area(), 1);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), SELECTION_MARKER);
        assert_eq!(buffer[(2, 0)].symbol(), "S");
        assert_eq!(buffer[(8, 0)].symbol(), "E");
        assert_eq!(buffer[(9, 0)].symbol(), " ");
        assert_eq!(buffer[(9, 0)].bg, theme.tile_body(true).bg.unwrap());
        assert_eq!(buffer[(10, 0)].symbol(), " ");
        assert_eq!(Some(buffer[(10, 0)].bg), theme.tile_gutter().bg);
        assert_eq!(buffer[(11, 0)].symbol(), "█");
        assert_ne!(buffer[(10, 0)].bg, buffer[(11, 0)].bg);
    }

    #[test]
    fn scrollbar_reserves_two_columns_only_when_there_is_room() {
        let area = Rect::new(4, 2, 5, 3);
        assert_eq!(content_area(area, true), Rect::new(4, 2, 3, 3));
        assert_eq!(content_area(area, false), area);
        for width in 0..=SCROLLBAR_COLUMNS {
            let narrow = Rect { width, ..area };
            assert_eq!(content_area(narrow, true), narrow);
        }
    }
}
