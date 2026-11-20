//! Source-attributed, cache-only inclusion history and child-branch selection.
use super::*;
use crate::cache::investigation::{Investigation, InvestigationPage, InvestigationRow};

enum Update {
    Page(Result<InvestigationPage, String>),
    Open(InvestigationRow, Result<Option<(String, Value)>, String>),
}

pub(super) struct Panel {
    subject: Investigation,
    system: String,
    path: PathBuf,
    page: usize,
    rows: Vec<InvestigationRow>,
    more: bool,
    selected: usize,
    message: String,
    loaded_at: Option<i64>,
    receiver: Option<tokio::sync::oneshot::Receiver<Update>>,
    task: Option<tokio::task::JoinHandle<()>>,
    opening: bool,
    revision: Option<u64>,
    mode: ViewMode,
}
impl Drop for Panel {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

pub(super) enum Action {
    Close,
    Open(InvestigationRow, Option<(String, Value)>),
}

impl Panel {
    pub fn for_screen(config: &Config, state: &AppState) -> Option<Self> {
        let (subject, mode) = match &state.screen {
            Screen::Transaction { txid, mode, .. } => {
                (Investigation::Inclusions(txid.clone()), *mode)
            }
            Screen::Block { target, mode, .. } => {
                let hash = match target {
                    BlockTarget::Hash(h) => h.clone(),
                    _ => state.store.block_for_target(target)?.summary.hash.clone(),
                };
                (Investigation::Children(hash), *mode)
            }
            _ => return None,
        };
        let mut panel = Self {
            subject,
            system: config.chain_cache_key(),
            path: config.cache_path.clone(),
            page: 0,
            rows: Vec::new(),
            more: false,
            selected: 0,
            message: String::new(),
            loaded_at: None,
            receiver: None,
            task: None,
            opening: false,
            revision: state.store.chain.reconciled.as_ref().map(|s| s.revision),
            mode,
        };
        panel.refresh();
        Some(panel)
    }
    fn refresh(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        // A new channel makes cancelled/obsolete completions unable to update this panel.
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.receiver = Some(receiver);
        self.opening = false;
        self.message = "Loading stored observations…".into();
        let (path, system, subject, page) = (
            self.path.clone(),
            self.system.clone(),
            self.subject.clone(),
            self.page,
        );
        self.task = Some(tokio::spawn(async move {
            let result = match Cache::open(&path) {
                Ok(cache) => cache
                    .investigate(system, subject, page)
                    .await
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e.to_string()),
            };
            let _ = sender.send(Update::Page(result));
        }));
    }
    pub fn poll(&mut self, state: &AppState) -> (bool, Option<Action>) {
        let revision = state.store.chain.reconciled.as_ref().map(|s| s.revision);
        // Reload after a committed branch change, preserving selection by (hash,source).
        // Opening a hash remains valid across a reorg; it never changes object identity.
        if revision != self.revision && !self.opening {
            self.revision = revision;
            self.refresh();
        }
        let Some(receiver) = &mut self.receiver else {
            return (false, None);
        };
        let update = match receiver.try_recv() {
            Ok(update) => update,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return (false, None),
            Err(_) => {
                self.receiver = None;
                self.task = None;
                self.opening = false;
                self.message = "Investigation task stopped; r retries".into();
                return (true, None);
            }
        };
        self.receiver = None;
        self.task = None;
        self.opening = false;
        match update {
            Update::Page(Ok(page)) => {
                let previous = self
                    .rows
                    .get(self.selected)
                    .map(|r| (r.hash.clone(), r.source.clone()));
                self.rows = page.rows;
                self.more = page.more;
                self.selected = self
                    .rows
                    .iter()
                    .position(|r| Some((r.hash.clone(), r.source.clone())) == previous)
                    .unwrap_or_else(|| self.selected.min(self.rows.len().saturating_sub(1)));
                self.loaded_at = Some(current_unix_timestamp());
                self.message = if self.rows.is_empty() {
                    "No stored observations. This does not establish absence from the chain.".into()
                } else {
                    "Stored observations only; no live membership or completeness claim.".into()
                };
            }
            Update::Page(Err(e)) | Update::Open(_, Err(e)) => {
                self.message = format!("Unable to load: {e}; r retries")
            }
            Update::Open(row, Ok(raw)) => return (true, Some(Action::Open(row, raw))),
        }
        (true, None)
    }
    pub fn key(&mut self, code: KeyCode) -> Option<Action> {
        match code {
            KeyCode::Esc
            | KeyCode::F(7)
            | KeyCode::Left
            | KeyCode::Char('h')
            | KeyCode::Backspace => return Some(Action::Close),
            KeyCode::Char('r') | KeyCode::F(5) => self.refresh(),
            _ if self.receiver.is_some() => {}
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.rows.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.rows.len().saturating_sub(1),
            KeyCode::PageDown => {
                self.selected = (self.selected + 10).min(self.rows.len().saturating_sub(1))
            }
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(10),
            KeyCode::Char(']') if self.more => {
                self.page += 1;
                self.rows.clear();
                self.selected = 0;
                self.refresh();
            }
            KeyCode::Char('[') if self.page > 0 => {
                self.page -= 1;
                self.rows.clear();
                self.selected = 0;
                self.refresh();
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(row) = self.rows.get(self.selected).cloned() {
                    let (sender, receiver) = tokio::sync::oneshot::channel();
                    self.receiver = Some(receiver);
                    self.opening = true;
                    self.message = "Opening selected block… Esc cancels".into();
                    let (path, system) = (self.path.clone(), self.system.clone());
                    self.task = Some(tokio::spawn(async move {
                        let selected = row.clone();
                        let result = tokio::task::spawn_blocking(
                            move || -> Result<Option<(String, Value)>> {
                                let cache = Cache::open(&path)?;
                                let params = json!([selected.hash, 1]).to_string();
                                if let Some(raw) = cache.raw_rpc_response(
                                    &selected.source,
                                    &system,
                                    "getblock",
                                    &params,
                                )? {
                                    let body = rpc_result_value(&raw)?;
                                    if body["tx"].is_array() {
                                        return Ok(Some((selected.source, body)));
                                    }
                                }
                                cache.bookmark_response(
                                    &system,
                                    "getblock",
                                    &params,
                                    &selected.hash,
                                )
                            },
                        )
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| e.to_string()));
                        let _ = sender.send(Update::Open(row, result));
                    }));
                }
            }
            _ => {}
        }
        None
    }
    pub fn open(
        &self,
        state: &mut AppState,
        config: &Config,
        tx: &UnboundedSender<AppEvent>,
        row: InvestigationRow,
        raw: Option<(String, Value)>,
    ) {
        let source = raw.as_ref().map(|(source, _)| source.clone());
        if let Some((_, raw_json)) = raw
            && let Ok(summary) = serde_json::from_value::<BlockSummary>(raw_json.clone())
            && summary.hash == row.hash
            && raw_json["tx"].is_array()
        {
            // Install immutable evidence without altering the active height mapping.
            state
                .store
                .blocks_by_hash
                .entry(row.hash.clone())
                .or_insert(BlockRecord {
                    summary,
                    raw_json,
                    detail_loaded: false,
                });
        }
        let target = BlockTarget::Hash(row.hash);
        let screen = Screen::Block {
            target: target.clone(),
            selected_tx: 0,
            mode: self.mode,
            json: JsonState::new(),
        };
        push_screen(state, screen);
        request_block_detail(state, config, tx, target);
        state.notice = Some(TransientNotice {
            message: source
                .map(|s| {
                    format!("Opened cached block from {s}; membership remains source-observed")
                })
                .unwrap_or_else(|| "Loading selected hash through configured sources".into()),
            until: Instant::now() + Duration::from_secs(5),
        });
    }
    pub fn render(&self, frame: &mut Frame<'_>, theme: &Theme) {
        let (title, target) = match &self.subject {
            Investigation::Inclusions(id) => ("Transaction inclusion history — F7", id),
            Investigation::Children(id) => ("Child branches — F7", id),
        };
        let inner = render_workspace(
            frame,
            theme,
            title,
            "j/k select | Enter open block | F5/r reload cache | [/] pages | Esc close | q quit",
        );
        let header = Paragraph::new(vec![
            identity_property_line(theme, "System", self.system.to_string()),
            identity_property_line(theme, "Object", target.to_string()),
        ])
        .style(theme.base())
        .wrap(Wrap { trim: false });
        let selected = self.rows.get(self.selected);
        let details = Paragraph::new(format!(
            "Selected: {}\nSource: {}\n{} | observed {}\nPage {}{} | snapshot {}\n{}",
            selected.map(|r| r.hash.as_str()).unwrap_or("none"),
            selected.map(|r| r.source.as_str()).unwrap_or("none"),
            selected.map(status).unwrap_or("No known inclusion/child"),
            timestamp(selected.and_then(|r| r.observed_at)),
            self.page + 1,
            if self.more { " (more: ])" } else { "" },
            timestamp(self.loaded_at),
            self.message,
        ))
        .style(theme.base().bg(theme.panel))
        .wrap(Wrap { trim: false });
        let header_height = (header.line_count(inner.width.max(1)) as u16).min(inner.height / 3);
        let detail_height = (details.line_count(inner.width.max(1)) as u16).min(inner.height / 2);
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Min(1),
                Constraint::Length(detail_height),
            ])
            .split(inner);
        frame.render_widget(header, sections[0]);
        let items = self
            .rows
            .iter()
            .map(|r| {
                let height = r
                    .height
                    .map(|h| {
                        if theme.heights.borrow().dates {
                            r.timestamp
                                .map(|t| theme.format_time(t))
                                .unwrap_or_else(|| format!("{h} (date unavailable)"))
                        } else {
                            h.to_string()
                        }
                    })
                    .unwrap_or_else(|| "unknown".into());
                let status_color = match r.membership.as_str() {
                    "member" => theme.positive,
                    "orphan" => theme.warning,
                    _ => theme.muted,
                };
                vec![
                    Line::from(vec![
                        Span::styled(
                            format!("Height {height}  ·  "),
                            Style::default().fg(theme.focus),
                        ),
                        Span::styled(status(r), Style::default().fg(status_color)),
                        Span::raw(if r.cached {
                            "  ·  block cached"
                        } else {
                            "  ·  block not cached"
                        }),
                    ]),
                    Line::styled(r.hash.as_str(), Style::default().fg(theme.link)),
                    Line::styled(
                        format!("Source: {}", r.source),
                        Style::default().fg(theme.muted),
                    ),
                    Line::default(),
                ]
            })
            .collect::<Vec<_>>();
        scroll_view::list(frame, theme, items, sections[1], self.selected);
        frame.render_widget(details, sections[2]);
    }
}
fn status(row: &InvestigationRow) -> &'static str {
    match row.membership.as_str() {
        "member" => "Last observed active",
        "orphan" => "Last observed orphaned",
        _ => "Unverified membership",
    }
}
fn timestamp(value: Option<i64>) -> String {
    value
        .and_then(|v| chrono::DateTime::from_timestamp(v, 0))
        .map(|v| v.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn competing_children(state: &AppState) -> bool {
    let Screen::Block { target, .. } = &state.screen else {
        return false;
    };
    let Some(block) = state.store.block_for_target(target) else {
        return false;
    };
    let mut children = HashSet::new();
    if let Some(chain) = &state.store.chain.reconciled {
        children.extend(
            chain
                .canonical
                .values()
                .chain(chain.detached.values())
                .filter(|h| h.parent.as_deref() == Some(block.summary.hash.as_str()))
                .map(|h| h.hash.as_str()),
        );
    }
    children.extend(
        state
            .store
            .blocks_by_hash
            .values()
            .filter(|b| b.summary.previousblockhash.as_deref() == Some(block.summary.hash.as_str()))
            .map(|b| b.summary.hash.as_str()),
    );
    children.len() > 1
}

#[cfg(test)]
mod tests {
    use super::super::tests::{regression_config, regression_state};
    use super::*;

    fn row(hash: &str, source: &str, membership: &str) -> InvestigationRow {
        InvestigationRow {
            hash: hash.into(),
            source: source.into(),
            height: Some(10),
            timestamp: Some(1000),
            membership: membership.into(),
            observed_at: Some(123),
            cached: true,
        }
    }
    fn panel(rows: Vec<InvestigationRow>) -> Panel {
        Panel {
            subject: Investigation::Inclusions("same".into()),
            system: "iTest".into(),
            path: PathBuf::new(),
            page: 0,
            rows,
            more: false,
            selected: 0,
            message: String::new(),
            loaded_at: None,
            receiver: None,
            task: None,
            opening: false,
            revision: None,
            mode: ViewMode::Json,
        }
    }

    #[test]
    fn refreshed_history_keeps_source_selection_and_does_not_navigate() {
        let mut state = regression_state();
        state.screen = transaction_screen("same".into());
        let depth = state.screen_stack.len();
        let mut panel = panel(vec![row("a", "api", "member"), row("a", "local", "orphan")]);
        panel.selected = 1;
        let (tx, rx) = tokio::sync::oneshot::channel();
        panel.receiver = Some(rx);
        tx.send(Update::Page(Ok(InvestigationPage {
            rows: vec![row("a", "local", "member"), row("a", "api", "orphan")],
            more: false,
        })))
        .ok()
        .unwrap();
        assert!(panel.poll(&state).0);
        assert_eq!(panel.selected, 0);
        assert_eq!(status(&panel.rows[0]), "Last observed active");
        assert!(matches!(state.screen,Screen::Transaction { ref txid,.. } if txid=="same"));
        assert!(matches!(panel.key(KeyCode::Esc), Some(Action::Close)));
        assert_eq!(state.screen_stack.len(), depth);
    }

    #[test]
    fn selected_cached_block_preserves_mode_back_and_active_mapping() {
        let mut state = regression_state();
        let config = regression_config();
        let raw = json!({"hash":"orphan","height":10,"time":123,"tx":[]});
        let block = BlockRecord {
            summary: serde_json::from_value(raw.clone()).unwrap(),
            raw_json: raw.clone(),
            detail_loaded: true,
        };
        state.store.blocks_by_hash.insert("orphan".into(), block);
        state.store.block_hash_by_height.insert(10, "active".into());
        state.screen = transaction_screen("same".into());
        let panel = panel(vec![]);
        let (tx, _) = unbounded_channel();
        panel.open(
            &mut state,
            &config,
            &tx,
            row("orphan", "local", "orphan"),
            Some(("local".into(), raw)),
        );
        assert!(
            matches!(state.screen,Screen::Block { mode:ViewMode::Json,target:BlockTarget::Hash(ref hash),.. } if hash=="orphan")
        );
        assert_eq!(state.store.block_hash_by_height[&10], "active");
        pop_screen(&mut state);
        assert!(matches!(state.screen,Screen::Transaction { ref txid,.. } if txid=="same"));
    }

    #[test]
    fn narrow_history_keeps_selected_observation_visible() {
        use ratatui::backend::TestBackend;
        let mut panel = panel(
            (0..40)
                .map(|i| row(&format!("{i:064x}"), "local-daemon", "orphan"))
                .collect(),
        );
        panel.key(KeyCode::End);
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        for (width, height) in [(80, 24), (160, 40), (40, 16)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| panel.render(frame, &theme)).unwrap();
            let contents = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|c| c.symbol())
                .collect::<String>();
            assert!(
                contents.contains("local-daemon"),
                "{width}x{height}: {contents}"
            );
            assert!(
                contents.contains("Last observed orphaned"),
                "{width}x{height}: {contents}"
            );
            if width >= 80 {
                assert!(contents.contains(&format!("{:064x}", 39)));
            }
        }
    }
}
