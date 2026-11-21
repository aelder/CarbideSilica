use super::*;
use crate::cache::SourceEvent;

impl SourceEvent {
    fn block_scope(&self) -> Option<(&str, &str, &str)> {
        let (system, rest) = self.key.split_once(':')?;
        let (height, rest) = rest.split_once(':')?;
        height.parse::<u64>().ok()?;
        // The two trailing components are block hashes. Endpoint IDs themselves
        // contain a colon, so keep the remaining pair intact for matching.
        let (sources, _) = rest.rsplit_once(':')?.0.rsplit_once(':')?;
        Some((system, height, sources))
    }

    fn reorg_scope(&self) -> Option<(&str, &str)> {
        let (system, rest) = self.key.strip_prefix("reorg:")?.split_once(':')?;
        Some((system, rest.rsplit_once(':')?.0))
    }

    fn system(&self) -> Option<&str> {
        self.reorg_scope()
            .map(|(system, _)| system)
            .or_else(|| self.block_scope().map(|(system, _, _)| system))
    }

    fn involves(&self, system: &str, source: &str) -> bool {
        if let Some(scope) = self.reorg_scope() {
            return scope == (system, source);
        }
        self.block_scope().is_some_and(|(chain, _, sources)| {
            chain == system
                && (sources
                    .strip_prefix(source)
                    .is_some_and(|s| s.starts_with(':'))
                    || sources
                        .strip_suffix(source)
                        .is_some_and(|s| s.ends_with(':')))
        })
    }

    fn status(&self) -> &'static str {
        if self.reorg_scope().is_some() {
            "Reconciled"
        } else if self.message.contains(": sources disagree at height ") {
            "Not rechecked"
        } else {
            "Recorded"
        }
    }

    fn summary(&self) -> String {
        if let Some((_, height, _)) = self.block_scope() {
            let label = if self.status() == "Not rechecked" {
                "Sources differed"
            } else {
                "Block changed"
            };
            return format!("{label} at height {height}");
        }
        if self.reorg_scope().is_some()
            && let Some((_, message)) = self
                .message
                .lines()
                .next()
                .and_then(|line| line.split_once(": "))
        {
            let summary = message
                .rsplit_once(" (")
                .map_or(message, |(summary, _)| summary);
            return format!("Reorg · {summary}");
        }
        self.message.clone()
    }
}

fn selected_source(config: &Config, api: Option<usize>) -> (String, String) {
    let source = if let Some(api) = api {
        crate::rpc::source_id("public-api", &config.daemon.api_pool.endpoints[api].url)
    } else {
        config.daemon_cache_key()
    };
    (config.chain_cache_key(), source)
}

#[derive(Default)]
pub(in crate::app) struct Activity {
    events: Vec<SourceEvent>,
    load: Option<tokio::sync::oneshot::Receiver<Result<Vec<SourceEvent>>>>,
    last_load: Option<Instant>,
    error: bool,
    expanded: bool,
    all: bool,
    selection: usize,
    detail: Option<SourceEvent>,
    scroll: u16,
}

impl Activity {
    pub(in crate::app) fn poll(&mut self, path: &std::path::Path) -> bool {
        if (!self.expanded || self.events.is_empty())
            && let Some(load) = &mut self.load
        {
            match load.try_recv() {
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return false,
                Ok(Ok(events)) => {
                    self.events = events;
                    self.error = false;
                }
                _ => self.error = true,
            }
            self.load = None;
            return true;
        }
        if (!self.expanded || self.last_load.is_none())
            && self.load.is_none()
            && self
                .last_load
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(5))
        {
            self.last_load = Some(Instant::now());
            let path = path.to_owned();
            let (send, receive) = tokio::sync::oneshot::channel();
            self.load = Some(receive);
            tokio::task::spawn_blocking(move || {
                let _ = send.send(Cache::open(&path).and_then(|cache| cache.events()));
            });
        }
        false
    }

    fn matching(&self, config: &Config, api: Option<usize>, all: bool) -> Vec<&SourceEvent> {
        let (system, source) = selected_source(config, api);
        self.events
            .iter()
            .filter(|event| all || event.involves(&system, &source))
            .collect()
    }

    pub(in crate::app) fn key(
        &mut self,
        key: KeyCode,
        config: &Config,
        api: Option<usize>,
    ) -> bool {
        if !self.expanded {
            if key != KeyCode::Char('a') {
                return false;
            }
            self.expanded = true;
            self.all = false;
            self.selection = 0;
        } else if self.detail.is_some() {
            match key {
                KeyCode::Esc => self.detail = None,
                KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_add(1),
                KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
                _ => {}
            }
        } else {
            match key {
                KeyCode::Esc | KeyCode::Char('a') => self.expanded = false,
                KeyCode::Char('f') => {
                    self.all = !self.all;
                    self.selection = 0;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.selection = self.selection.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let count = self.matching(config, api, self.all).len();
                    self.selection = (self.selection + 1).min(count.saturating_sub(1));
                }
                KeyCode::Enter => {
                    self.detail = self
                        .matching(config, api, self.all)
                        .get(self.selection)
                        .map(|event| (*event).clone());
                    self.scroll = 0;
                }
                _ => {}
            }
        }
        true
    }

    pub(super) fn preview(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        theme: &Theme,
        config: &Config,
        api: Option<usize>,
    ) {
        if area.height == 0 {
            return;
        }
        let events = self.matching(config, api, false);
        let elsewhere = self.events.len() - events.len();
        let mut heading = String::from("Recent activity   a history");
        if elsewhere > 0 {
            heading.push_str(&format!("   {elsewhere} other records"));
        }
        frame.render_widget(
            Paragraph::new(heading).style(theme.focused()),
            Rect { height: 1, ..area },
        );
        let body = Rect {
            y: area.y.saturating_add(1),
            height: area.height - 1,
            ..area
        };
        self.table(
            frame,
            body,
            theme,
            &events[..events.len().min(3)],
            None,
            |_| String::new(),
        );
    }

    pub(super) fn render_history(
        &mut self,
        frame: &mut Frame<'_>,
        theme: &Theme,
        config: &Config,
        api: Option<usize>,
        chain_name: impl Fn(&str) -> String,
    ) {
        if !self.expanded {
            return;
        }
        let controls = if self.detail.is_some() {
            "j/k scroll | Esc history | q quit"
        } else {
            "j/k select | f change scope | Enter details | Esc sources | q quit"
        };
        let inner = render_workspace(frame, theme, "Source activity", controls);
        let (help, body) = if let Some(event) = &self.detail {
            let chain = event.system().map(&chain_name).unwrap_or_default();
            let text = vec![
                Line::from(Span::styled(
                    format!("{}   {chain}", event.status()),
                    theme.focused(),
                )),
                Line::from(format!("Observed {}", theme.format_time(event.observed_at))),
                Line::default(),
                Line::from(event.message.as_str()),
            ];
            (
                Paragraph::new("Event details"),
                Some(Paragraph::new(text).wrap(Wrap { trim: false })),
            )
        } else {
            let scope = if self.all {
                "All sources"
            } else {
                "Selected source"
            };
            (Paragraph::new(scope), None)
        };
        let regions = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(inner);
        frame.render_widget(help.style(theme.muted()), regions[0]);
        if let Some(body) = body {
            self.scroll = scroll_view::paragraph(
                frame,
                theme,
                body.style(theme.base()),
                regions[1],
                usize::from(self.scroll),
            );
        } else {
            self.table(
                frame,
                regions[1],
                theme,
                &self.matching(config, api, self.all),
                Some(self.selection),
                |event| {
                    if self.all {
                        event
                            .system()
                            .map(&chain_name)
                            .unwrap_or_else(|| "Unscoped".into())
                    } else {
                        String::new()
                    }
                },
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn table(
        &self,
        frame: &mut Frame<'_>,
        area: Rect,
        theme: &Theme,
        events: &[&SourceEvent],
        selected: Option<usize>,
        chain: impl Fn(&SourceEvent) -> String,
    ) {
        if events.is_empty() || self.error {
            let text = if self.error {
                "Activity history unavailable"
            } else if self.load.is_some() && self.events.is_empty() {
                "Loading activity…"
            } else {
                "No recorded activity for this selection"
            };
            frame.render_widget(Paragraph::new(text).style(theme.muted()), area);
            return;
        }
        let now = current_unix_timestamp().max(0) as u64;
        let rows = events.iter().map(|event| {
            let chain = chain(event);
            let summary = if chain.is_empty() {
                event.summary()
            } else {
                format!("{chain} · {}", event.summary())
            };
            Row::new([
                Cell::from(format!(
                    "{} ago",
                    short_age(now.saturating_sub(event.observed_at))
                )),
                Cell::from(event.status()),
                Cell::from(summary),
            ])
        });
        let table = Table::new(
            rows,
            [
                Constraint::Length(8),
                Constraint::Length(13),
                Constraint::Min(1),
            ],
        )
        .header(Row::new(["Age", "Status", "Event"]).style(theme.focused()))
        .style(theme.muted())
        .column_spacing(2);
        render_source_table(frame, theme, table, area, selected, 1, events.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_disagreement_matches_both_sources_without_crossing_chains() {
        let local = crate::rpc::source_id("local-daemon", "http://127.0.0.1:21000");
        let api = crate::rpc::source_id("public-api", "https://example.invalid/rpc");
        let event = SourceEvent {
            key: format!("iTestSystem:42:{local}:{api}:new:old"),
            message: "iTestSystem: sources disagree at height 42: endpoints and hashes".into(),
            observed_at: 1_700_000_000,
        };
        assert!(event.involves("iTestSystem", &local));
        assert!(event.involves("iTestSystem", &api));
        assert!(!event.involves("iOtherSystem", &local));
        assert!(!event.involves("iTestSystem", &api[..api.len() - 1]));
        assert_eq!(event.status(), "Not rechecked");
        assert_eq!(event.summary(), "Sources differed at height 42");
    }
}
