use super::*;

#[derive(Debug, Default)]
pub(super) struct State {
    pub banned: Option<Result<usize, String>>,
    pub totals: Option<Result<NetTotals, String>>,
    pub prompt: Option<Prompt>,
    pub busy: bool,
    pub refresh_after_action: bool,
}

#[derive(Debug, serde::Deserialize)]
pub(super) struct NetTotals {
    totalbytesrecv: u64,
    totalbytessent: u64,
    timemillis: u64,
}

#[derive(Debug)]
pub(super) struct Refresh {
    pub peers: Result<Vec<Value>, String>,
    pub banned: Result<usize, String>,
    pub totals: Option<Result<NetTotals, String>>,
}

async fn read<T: serde::de::DeserializeOwned>(
    client: &Result<RpcClient>,
    method: &str,
) -> Result<T, String> {
    let value = client
        .as_ref()
        .map_err(ToString::to_string)?
        .call_local(method, &[])
        .await
        .map_err(|error| error.to_string())?;
    serde_json::from_value(value).map_err(|error| format!("Invalid {method} response: {error}"))
}

pub(super) async fn refresh(config: &Config, snapshot: bool) -> Refresh {
    let client = RpcClient::new(config.daemon.clone());
    let (peers, banned, totals) = tokio::join!(
        read(&client, "getpeerinfo"),
        read::<Vec<Value>>(&client, "listbanned"),
        async {
            if snapshot {
                Some(read(&client, "getnettotals").await)
            } else {
                None
            }
        },
    );
    Refresh {
        peers,
        banned: banned.map(|entries| entries.len()),
        totals,
    }
}

#[derive(Debug)]
pub(super) enum Action {
    Add(String),
    ClearBans,
    Disconnect(String),
    Ping,
    Ban(IpAddr),
}

impl Action {
    fn rpc(&self) -> (&'static str, Vec<Value>) {
        match self {
            Self::Add(node) => ("addnode", vec![json!(node), json!("add")]),
            Self::ClearBans => ("clearbanned", vec![]),
            Self::Disconnect(node) => ("disconnectnode", vec![json!(node)]),
            Self::Ping => ("ping", vec![]),
            Self::Ban(ip) => ("setban", vec![json!(ip.to_string()), json!("add")]),
        }
    }

    fn success(&self) -> String {
        match self {
            Self::Add(node) => format!("Added {node} to the daemon's node list"),
            Self::ClearBans => "Cleared the daemon's ban list".into(),
            Self::Disconnect(node) => format!("Disconnect requested for {node}"),
            Self::Ping => "Ping queued for all connected peers".into(),
            Self::Ban(ip) => format!("Banned {ip}"),
        }
    }
}

#[derive(Debug)]
pub(super) enum Prompt {
    Add { node: String, invalid: bool },
    Confirm(Action),
}

impl Prompt {
    pub fn controls(&self) -> &'static str {
        match self {
            Self::Add { .. } => "Enter add node | Esc cancel",
            Self::Confirm(_) => "y confirm | n/Esc cancel | q quit",
        }
    }

    pub fn paste(&mut self, text: &str) {
        if let Self::Add { node, invalid } = self {
            node.extend(text.chars().filter(|c| !c.is_control()));
            *invalid = false;
        }
    }
}

fn notify(state: &mut AppState, message: String) {
    state.notice = Some(TransientNotice {
        message,
        until: Instant::now() + Duration::from_secs(5),
    });
}

fn submit(state: &mut AppState, config: &Config, tx: &UnboundedSender<AppEvent>, action: Action) {
    state.peers.controls.busy = true;
    let config = config.clone();
    let tx = tx.clone();
    let generation = config.daemon.control.generation();
    tokio::spawn(async move {
        let (method, params) = action.rpc();
        let result = async {
            RpcClient::new(config.daemon.clone())?
                .call_local(method, &params)
                .await
        }
        .await
        .map(|_| action.success())
        .map_err(|error| format!("{method} failed: {error}"));
        let _ = tx.send(AppEvent::Guarded {
            control: config.daemon.control,
            generation,
            event: Box::new(AppEvent::PeerAction(result)),
        });
    });
}

pub(super) fn finish(state: &mut AppState, result: Result<String, String>) {
    state.peers.controls.busy = false;
    state.peers.controls.refresh_after_action = state.peers.loading;
    state.peers.last_refresh = None;
    notify(state, result.unwrap_or_else(|error| error));
}

pub(super) fn prompt_key(
    state: &mut AppState,
    config: &Config,
    tx: &UnboundedSender<AppEvent>,
    key: KeyCode,
) {
    let Some(mut prompt) = state.peers.controls.prompt.take() else {
        return;
    };
    match (&mut prompt, key) {
        (_, KeyCode::Esc) | (Prompt::Confirm(_), KeyCode::Char('n')) => return,
        (Prompt::Confirm(_), KeyCode::Char('y')) => {
            if let Prompt::Confirm(action) = prompt {
                submit(state, config, tx, action);
            }
            return;
        }
        (Prompt::Add { node, invalid }, KeyCode::Enter) => {
            let target = node.trim();
            if !target.is_empty() && !target.chars().any(char::is_whitespace) {
                submit(state, config, tx, Action::Add(target.into()));
                return;
            }
            *invalid = true;
        }
        (Prompt::Add { node, invalid }, KeyCode::Backspace) => {
            node.pop();
            *invalid = false;
        }
        (Prompt::Add { node, invalid }, KeyCode::Char(ch)) => {
            node.push(ch);
            *invalid = false;
        }
        _ => {}
    }
    state.peers.controls.prompt = Some(prompt);
}

pub(super) fn key(
    state: &mut AppState,
    config: &Config,
    tx: &UnboundedSender<AppEvent>,
    key: KeyCode,
) -> bool {
    let action = match (&state.auxiliary, key) {
        (Some(AuxiliaryScreen::PeerBrowser), KeyCode::Char('a')) => {
            if !state.peers.controls.busy {
                state.peers.controls.prompt = Some(Prompt::Add {
                    node: String::new(),
                    invalid: false,
                });
            }
            return true;
        }
        (Some(AuxiliaryScreen::PeerBrowser), KeyCode::Char('p')) => {
            if !state.peers.controls.busy {
                submit(state, config, tx, Action::Ping);
            }
            return true;
        }
        (Some(AuxiliaryScreen::PeerBrowser), KeyCode::Char('c')) => Some(Action::ClearBans),
        (Some(AuxiliaryScreen::PeerDetail { peer_id, .. }), KeyCode::Char('x' | 'b')) => {
            let peer = state.peers.records.iter().find(|peer| peer.id == *peer_id);
            if key == KeyCode::Char('b') {
                peer.and_then(|peer| peer.ip).map(Action::Ban)
            } else {
                peer.and_then(|peer| peer.raw_json["addr"].as_str())
                    .filter(|addr| !addr.is_empty())
                    .map(|addr| Action::Disconnect(addr.into()))
            }
        }
        _ => return false,
    };
    if !state.peers.controls.busy {
        if let Some(action) = action {
            state.peers.controls.prompt = Some(Prompt::Confirm(action));
        } else {
            notify(state, "The selected peer has no usable address".into());
        }
    }
    true
}

pub(super) fn render_totals(frame: &mut Frame<'_>, theme: &Theme, state: &State, area: Rect) {
    let lines = match &state.totals {
        Some(Ok(totals)) => vec![
            Line::from(format!(" Received  {} bytes", totals.totalbytesrecv)),
            Line::from(format!(" Sent      {} bytes", totals.totalbytessent)),
            Line::from(format!(
                " Taken     {}",
                theme.format_time(totals.timemillis / 1000)
            )),
        ],
        Some(Err(_)) => vec![Line::styled(" Network totals unavailable", theme.muted())],
        None => vec![Line::styled(" Loading network totals…", theme.muted())],
    };
    frame.render_widget(
        Paragraph::new(lines).style(theme.base()).block(
            Block::default()
                .title(" Network totals snapshot ")
                .borders(Borders::ALL)
                .border_style(theme.muted()),
        ),
        area,
    );
}

pub(super) fn render_prompt(frame: &mut Frame<'_>, theme: &Theme, prompt: &Prompt, area: Rect) {
    let (title, lines) = match prompt {
        Prompt::Add { node, invalid } => {
            let mut lines = vec![
                Line::from("Hostname or IP address, with optional port."),
                Line::styled(format!("> {node}"), theme.focused()),
            ];
            if *invalid {
                lines.push(Line::styled("Enter one node address.", theme.warning()));
            }
            ("Add node", lines)
        }
        Prompt::Confirm(action) => {
            let message = match action {
                Action::ClearBans => "Clear all bans on this daemon?".into(),
                Action::Disconnect(node) => format!("Disconnect {node}?"),
                Action::Ban(ip) => format!("Ban {ip} for the daemon's default duration?"),
                _ => return,
            };
            ("Confirm peer action", vec![Line::from(message)])
        }
    };
    let width = area.width.min(64);
    let height = area.height.min(7);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme.base())
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .title(format!(" {title} "))
                    .borders(Borders::ALL)
                    .border_style(theme.focused()),
            ),
        popup,
    );
}
