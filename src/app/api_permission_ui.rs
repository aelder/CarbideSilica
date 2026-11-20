use super::*;
use crate::sources::{ApiConsent, ApiDecision};

pub(super) struct Prompt {
    chain: String,
    method: String,
    failure: String,
    sources: Vec<String>,
    selected: usize,
    pub reply: tokio::sync::oneshot::Sender<ApiDecision>,
}

impl Prompt {
    pub fn new(request: ApiConsent) -> Self {
        Self {
            chain: request.chain,
            method: request.method,
            failure: request.local_failure,
            sources: request.sources,
            selected: 0,
            reply: request.reply,
        }
    }

    pub fn key(&mut self, key: KeyCode) -> Option<ApiDecision> {
        match key {
            KeyCode::Char('r') => Some(ApiDecision::Retry),
            KeyCode::Enter if !self.sources.is_empty() => Some(ApiDecision::Enable(self.selected)),
            KeyCode::Esc => Some(ApiDecision::Cancel),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.sources.len().saturating_sub(1));
                None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            _ => None,
        }
    }

    pub fn render(&self, frame: &mut Frame<'_>, theme: &Theme, footer: Rect) {
        let area = frame.area();
        frame
            .buffer_mut()
            .set_style(area, Style::default().add_modifier(Modifier::DIM));
        frame.render_widget(Clear, footer);
        let width = area.width.saturating_sub(2).min(86);
        let content_width = width.saturating_sub(2);
        let failure = Paragraph::new(vec![
            Line::styled(
                format!("{} failed on {}", self.method, self.chain),
                theme.focused(),
            ),
            Line::styled(
                self.failure
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                theme.warning(),
            ),
        ])
        .wrap(Wrap { trim: true })
        .style(theme.base());
        let header_height = failure.line_count(content_width.max(1)).min(5) as u16;
        let help = Paragraph::new(
            "r retry | j/k select source | Enter enable for session and retry | Esc cancel",
        )
        .wrap(Wrap { trim: true })
        .style(theme.base());
        let help_height = help.line_count(content_width.max(1)).min(3) as u16;
        let height = (header_height + help_height + 6).min(area.height.saturating_sub(2));
        let popup = Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        );
        let border = Block::default()
            .title(" API communication requires permission ")
            .borders(Borders::ALL)
            .border_style(theme.warning());
        let [reason, heading, list, controls] = Layout::vertical([
            Constraint::Length(header_height),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(help_height),
        ])
        .areas(border.inner(popup));
        frame.render_widget(Clear, popup);
        frame.render_widget(border.style(theme.base()), popup);
        frame.render_widget(failure, reason);
        frame.render_widget(
            Paragraph::new("Choose a configured source from F2 Sources").style(theme.muted()),
            heading,
        );
        let visible = usize::from(list.height);
        let first = self
            .selected
            .saturating_sub(visible.saturating_sub(1))
            .min(self.sources.len().saturating_sub(visible));
        let lines: Vec<_> = self
            .sources
            .iter()
            .enumerate()
            .skip(first)
            .take(visible)
            .map(|(index, source)| {
                selection_line(
                    theme,
                    index == self.selected,
                    format!("API {} · {source}", index + 1),
                )
            })
            .collect();
        frame.render_widget(
            Paragraph::new(lines).style(theme.base()),
            scroll_view::content_area(list, self.sources.len() > visible),
        );
        scroll_view::render(frame, theme, list, self.sources.len(), first, visible);
        frame.render_widget(help, controls);
    }
}
