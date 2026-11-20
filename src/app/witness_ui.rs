use super::*;
use crate::witnesses::{self, Report, Signature};
use crate::world::{OutpointRef, SystemId};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Signers,
    Evidence,
}

#[derive(Debug)]
struct Task {
    abort: tokio::task::AbortHandle,
    response: Mutex<tokio::sync::oneshot::Receiver<Report>>,
}

impl Drop for Task {
    fn drop(&mut self) {
        self.abort.abort();
    }
}

#[derive(Debug, Clone)]
struct SourceRevision {
    control: Arc<crate::sources::Control>,
    generation: u64,
    branch: u64,
}

impl SourceRevision {
    fn new(control: &Arc<crate::sources::Control>) -> Self {
        Self {
            control: control.clone(),
            generation: control.generation(),
            branch: control.branch_epoch.load(Ordering::Acquire),
        }
    }

    fn current(&self) -> bool {
        self.control.generation() == self.generation
            && self.control.branch_epoch.load(Ordering::Acquire) == self.branch
    }
}

#[derive(Debug, Clone)]
pub(super) struct View {
    pub mode: ViewMode,
    pub json: JsonState,
    pub document: Arc<Value>,
    txid: Txid,
    system: SystemId,
    roots: Vec<(u32, String)>,
    selected_root: usize,
    signer: usize,
    origin: usize,
    focus: Focus,
    scroll: u16,
    pub names: HashMap<(SystemId, String), String>,
    task: Option<Arc<Task>>,
    revisions: Vec<SourceRevision>,
    report: Option<Arc<Report>>,
    message: Option<&'static str>,
    pub navigation_message: Option<String>,
}

pub(super) enum Action {
    Identity(SystemId, String),
    Evidence(OutpointRef),
}

impl View {
    pub fn new(system: SystemId, txid: Txid, raw: &Value, selected: Option<u32>) -> Self {
        let roots = witnesses::roots(raw);
        let selected_root = roots
            .iter()
            .position(|(index, _)| Some(*index) == selected)
            .unwrap_or(0);
        let mut view = Self {
            mode: ViewMode::Overview,
            json: JsonState::new(),
            document: Arc::new(Value::Null),
            txid,
            system,
            roots,
            selected_root,
            signer: 0,
            origin: 0,
            focus: Focus::Signers,
            scroll: 0,
            names: HashMap::new(),
            task: None,
            revisions: Vec::new(),
            report: None,
            message: None,
            navigation_message: None,
        };
        view.rebuild_document();
        view
    }

    pub fn filename(&self) -> String {
        format!("witnesses-{}.json", self.txid)
    }

    fn root(&self) -> Option<OutpointRef> {
        Some(OutpointRef {
            system_id: self.system.clone(),
            txid: self.txid.clone(),
            vout: self.roots.get(self.selected_root)?.0,
        })
    }

    fn rebuild_document(&mut self) {
        let (status, message) = if self.task.is_some() {
            ("loading", None)
        } else if self.report.is_some() {
            ("complete", None)
        } else if let Some(message) = self.message {
            ("unavailable", Some(message))
        } else {
            ("ready", None)
        };
        self.document = Arc::new(json!({
            "root": self.root(),
            "status": status,
            "message": message,
            "report": self.report.as_deref(),
        }));
        self.json = JsonState::new();
    }

    pub fn poll(&mut self) -> bool {
        if self.revisions.iter().any(|revision| !revision.current()) {
            self.task = None;
            self.report = None;
            self.revisions.clear();
            self.focus = Focus::Signers;
            self.signer = 0;
            self.origin = 0;
            self.scroll = 0;
            self.message = Some("Source changed. Press r to investigate again.");
            self.rebuild_document();
            return true;
        }
        let Some(task) = &self.task else { return false };
        let result = task.response.lock().unwrap().try_recv();
        match result {
            Ok(report) => self.report = Some(Arc::new(report)),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return false,
            Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                self.message = Some("Investigation stopped before producing a report.");
            }
        }
        self.task = None;
        self.rebuild_document();
        true
    }

    pub fn signer_ids(&self) -> impl Iterator<Item = (&SystemId, &str)> {
        self.report
            .iter()
            .flat_map(|report| report.signatures.iter())
            .map(|signature| (&signature.system, signature.identity.as_str()))
    }

    fn selected_signer(&self) -> Option<&Signature> {
        self.report.as_ref()?.signatures.get(self.signer)
    }

    pub fn json_action(&self, pointer: &str) -> Option<Action> {
        let parts: Vec<_> = pointer.strip_prefix('/')?.split('/').collect();
        match parts.as_slice() {
            ["root", "txid"] => self.root().map(Action::Evidence),
            ["report", "signatures", index, rest @ ..] => {
                let signer = self
                    .report
                    .as_ref()?
                    .signatures
                    .get(index.parse::<usize>().ok()?)?;
                match rest {
                    ["identity"] => Some(Action::Identity(
                        signer.system.clone(),
                        signer.identity.clone(),
                    )),
                    ["target", "txid"] => Some(Action::Evidence(signer.target.clone())),
                    ["origins", origin, "output", "txid"] => {
                        let origin = signer.origins.get(origin.parse::<usize>().ok()?)?;
                        Some(Action::Evidence(origin.output.clone()))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    pub fn controls(&self) -> &'static str {
        if self.task.is_some() {
            "Tab JSON | Esc back"
        } else if self.report.is_some() {
            "j/k select | a signers/evidence | [/] evidence | Enter/Right/l open | PgUp/PgDn scroll | Tab JSON | F5/r refresh | Esc back"
        } else {
            "j/k select output | Enter/Right/l investigate | Tab JSON | F5/r refresh | Esc back"
        }
    }

    fn start(&mut self, config: &Config, store: &ObjectStore, refresh: bool) {
        let Some(root) = self.root() else { return };
        let Some(transaction) = store.transaction(&self.txid) else {
            return;
        };
        let seed = transaction.raw_json.clone();
        let source = transaction
            .source_id
            .clone()
            .unwrap_or_else(|| "loaded transaction (source not recorded)".into());
        self.revisions = std::iter::once(&config.daemon.control)
            .chain(
                config
                    .daemon
                    .api_pool
                    .endpoints
                    .iter()
                    .filter(|api| api.control.state() != crate::sources::ConnectionState::Disabled)
                    .map(|api| &api.control),
            )
            .map(SourceRevision::new)
            .collect();
        let (send, response) = tokio::sync::oneshot::channel();
        let config = config.clone();
        let task = tokio::spawn(async move {
            let _ = send.send(witnesses::investigate(config, root, seed, source, refresh).await);
        });
        self.task = Some(Arc::new(Task {
            abort: task.abort_handle(),
            response: Mutex::new(response),
        }));
        self.report = None;
        self.message = None;
        self.navigation_message = None;
        self.signer = 0;
        self.origin = 0;
        self.focus = Focus::Signers;
        self.scroll = 0;
        self.rebuild_document();
    }

    pub fn key(
        &mut self,
        key: KeyCode,
        config: &Config,
        store: &ObjectStore,
        theme: &Theme,
        raw_names: bool,
        area: Rect,
    ) -> (bool, Option<Action>) {
        if matches!(key, KeyCode::Char('r') | KeyCode::F(5)) {
            self.start(config, store, true);
            return (true, None);
        }
        if self.mode == ViewMode::Json {
            return (false, None);
        }
        let origin_count = self
            .selected_signer()
            .map_or(0, |signer| signer.origins.len());
        if matches!(
            key,
            KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::Char('j' | 'k' | 'a' | '[' | ']')
        ) {
            self.navigation_message = None;
        }
        match key {
            key if is_open_key(key) => {
                if let Some(signer) = self.selected_signer() {
                    let action = if self.focus == Focus::Signers {
                        Some(Action::Identity(
                            signer.system.clone(),
                            signer.identity.clone(),
                        ))
                    } else {
                        signer
                            .origins
                            .get(self.origin)
                            .map(|origin| Action::Evidence(origin.output.clone()))
                    };
                    return (true, action);
                }
                if self.report.is_none() && self.task.is_none() {
                    self.start(config, store, false);
                }
                return (true, None);
            }
            KeyCode::Char('a') if origin_count > 0 => {
                self.focus = if self.focus == Focus::Signers {
                    Focus::Evidence
                } else {
                    Focus::Signers
                };
            }
            KeyCode::Char('[' | ']') if origin_count > 0 => {
                self.focus = Focus::Evidence;
                self.origin = if key == KeyCode::Char('[') {
                    self.origin.saturating_sub(1)
                } else {
                    (self.origin + 1).min(origin_count - 1)
                };
            }
            KeyCode::Up
            | KeyCode::Down
            | KeyCode::Char('j' | 'k')
            | KeyCode::Home
            | KeyCode::End => {
                let select = |index: usize, count: usize| {
                    let last = count.saturating_sub(1);
                    match key {
                        KeyCode::Home => 0,
                        KeyCode::End => last,
                        KeyCode::Up | KeyCode::Char('k') => index.saturating_sub(1),
                        _ => (index + 1).min(last),
                    }
                };
                if let Some(report) = &self.report {
                    if self.focus == Focus::Evidence {
                        self.origin = select(self.origin, origin_count);
                    } else {
                        let selected = select(self.signer, report.signatures.len());
                        if selected != self.signer {
                            self.signer = selected;
                            self.origin = 0;
                        }
                    }
                } else if self.task.is_none() {
                    let selected = select(self.selected_root, self.roots.len());
                    if selected != self.selected_root {
                        self.selected_root = selected;
                        self.rebuild_document();
                    }
                }
            }
            KeyCode::PageUp | KeyCode::PageDown => {
                let step = area.height.saturating_sub(1).max(1);
                if key == KeyCode::PageUp {
                    self.scroll = self.scroll.saturating_sub(step);
                } else {
                    let content = detail_header_content(area);
                    let max = wrapped_max_scroll(
                        &self.lines(theme, raw_names),
                        content.width,
                        content.height,
                    );
                    self.scroll = self
                        .scroll
                        .saturating_add(step)
                        .min(max.min(u16::MAX as usize) as u16);
                }
                return (true, None);
            }
            _ => return (false, None),
        }
        self.follow_selection(theme, raw_names, area);
        (true, None)
    }

    pub fn follow_selection(&mut self, theme: &Theme, raw_names: bool, area: Rect) {
        self.scroll = scroll_view::follow_selection(
            &self.lines(theme, raw_names),
            detail_header_content(area),
            self.scroll,
        );
    }

    fn lines(&self, theme: &Theme, raw_names: bool) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::styled("Witness investigation — F4", theme.focused()),
            identity_property_line(theme, "Transaction", self.txid.clone()),
            identity_property_line(theme, "Observing system", self.system.to_string()),
        ];
        if let Some((output, label)) = self.roots.get(self.selected_root) {
            lines.push(Line::styled(
                format!("Output {output}: {label}"),
                theme.muted(),
            ));
        }
        if let Some(message) = self.message {
            lines.push(Line::styled(message, theme.warning()));
        }
        if let Some(message) = &self.navigation_message {
            lines.push(Line::styled(message.clone(), theme.warning()));
        }
        lines.push(Line::default());
        let Some(report) = &self.report else {
            if self.task.is_some() {
                lines.push(Line::styled(
                    "Inspecting reachable evidence…",
                    theme.warning(),
                ));
            } else if self.roots.is_empty() {
                lines.push(Line::styled(
                    "No notarization, finalization, or evidence output in this transaction",
                    theme.muted(),
                ));
            } else {
                lines.extend(self.roots.iter().enumerate().map(|(i, (output, label))| {
                    selection_line(
                        theme,
                        i == self.selected_root,
                        format!("Output {output}: {label}"),
                    )
                }));
            }
            return lines;
        };
        lines.push(Line::styled(
            format!(
                "{} signature groups · {} transactions · {} outputs · {} multipart sets decoded",
                report.signatures.len(),
                report.transactions,
                report.outputs,
                report.multipart_sets,
            ),
            theme.muted(),
        ));
        if report.signatures.is_empty() {
            lines.push(Line::from(
                "No witness signatures found in the evidence reached.",
            ));
        } else {
            lines.push(Line::styled("Signers", theme.focused()));
        }
        for (index, signer) in report.signatures.iter().enumerate() {
            let active = self.signer == index && self.focus == Focus::Signers;
            let name = if raw_names {
                None
            } else {
                self.names
                    .get(&(signer.system.clone(), signer.identity.clone()))
            };
            lines.push(
                Line::from(vec![
                    theme.selection_marker(active),
                    Span::raw(" "),
                    theme.value_as(
                        "witness identity",
                        name.unwrap_or(&signer.identity).clone(),
                        signer.identity.clone(),
                        Style::default().fg(theme.link),
                    ),
                    Span::raw(format!("  ·  {}  ·  ", signer.system)),
                    Span::styled(
                        if signer.confirmed {
                            " Confirming "
                        } else {
                            " Rejecting "
                        },
                        if signer.confirmed {
                            theme.status_badge()
                        } else {
                            theme.warning()
                        },
                    ),
                    Span::raw(format!(
                        "  Signing height {}  ·  {} distinct key signature(s)",
                        signer
                            .height
                            .map_or_else(|| "unknown".into(), |height| height.to_string()),
                        signer.key_signatures.len()
                    )),
                ])
                .style(theme.tile_body(active)),
            );
        }
        if let Some(signer) = self.selected_signer() {
            lines.push(Line::default());
            lines.push(Line::styled("Selected signer evidence", theme.focused()));
            lines.push(identity_property_line(
                theme,
                "Signed target",
                format!(
                    "{} / {}:{}",
                    signer.target.system_id, signer.target.txid, signer.target.vout,
                ),
            ));
            for (index, origin) in signer.origins.iter().enumerate() {
                let active = self.focus == Focus::Evidence && index == self.origin;
                let outpoint = format!("{}:{}", origin.output.txid, origin.output.vout);
                lines.push(
                    Line::from(vec![
                        theme.selection_marker(active),
                        Span::raw(format!(" Evidence: {} / ", origin.output.system_id)),
                        theme.value(
                            "evidence outpoint",
                            outpoint,
                            Style::default().fg(theme.link),
                        ),
                        Span::raw(format!(" · {}", origin.source)),
                    ])
                    .style(theme.tile_body(active)),
                );
            }
        }
        if !report.finalizations.is_empty() {
            lines.push(Line::default());
            lines.push(Line::styled(
                "Observed finalization fields",
                theme.focused(),
            ));
            lines.extend(report.finalizations.iter().cloned().map(Line::from));
        }
        if !report.issues.is_empty() {
            lines.push(Line::default());
            lines.push(Line::styled("Investigation incomplete", theme.warning()));
            lines.extend(report.issues.iter().cloned().map(Line::from));
        }
        lines
    }

    pub fn render(&self, frame: &mut Frame<'_>, theme: &Theme, raw_names: bool, area: Rect) {
        scroll_view::paragraph(
            frame,
            theme,
            Paragraph::new(self.lines(theme, raw_names))
                .style(theme.base())
                .wrap(Wrap { trim: false }),
            detail_header_content(area),
            usize::from(self.scroll),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn source_revision_discards_a_queued_witness_report() {
        let config = crate::app::tests::regression_config();
        let system = config.chain.system_id.get().unwrap().clone();
        let txid = "b".repeat(64);
        let mut view = View::new(
            system,
            txid,
            &json!({"vout":[{"scriptPubKey":{"notaryevidence":{}}}]}),
            None,
        );
        let (send, response) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(std::future::pending::<()>());
        view.task = Some(Arc::new(Task {
            abort: task.abort_handle(),
            response: Mutex::new(response),
        }));
        view.revisions = vec![SourceRevision::new(&config.daemon.control)];
        send.send(Report::default()).unwrap();
        config
            .daemon
            .control
            .branch_epoch
            .fetch_add(1, Ordering::AcqRel);

        assert!(view.poll());
        assert!(view.report.is_none());
        assert!(view.task.is_none());
        assert_eq!(view.document["status"], "unavailable");
        assert!(task.await.unwrap_err().is_cancelled());
    }
}
