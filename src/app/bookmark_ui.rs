//! Bookmark workspace and asynchronous, source-qualified object opening.
use super::*;
use crate::bookmarks::{self, Book, Bookmark, Kind, Snapshot};
use crate::world::{Network, SystemId};

const TILE_HEIGHT: u16 = 4;

type Key = (SystemId, Kind, String);
enum Message {
    Cached(Key, Result<Option<Snapshot>, String>),
    Open(u64, Bookmark, Result<Snapshot, String>),
    Permission(u64, Bookmark, Config),
    Startup(String),
}
struct Editor {
    fields: [String; 3],
    field: usize,
    kind: Kind,
    editing: bool,
}
pub(super) struct Workspace {
    book: Option<Book>,
    pub open: bool,
    selected: usize,
    page_size: Cell<usize>,
    editor: Option<Editor>,
    deleting: bool,
    permission: Option<(Bookmark, Config)>,
    message: String,
    snapshots: HashMap<Key, Result<Option<Snapshot>, String>>,
    tx: UnboundedSender<Message>,
    rx: UnboundedReceiver<Message>,
    lookup: Option<tokio::task::JoinHandle<()>>,
    scan: Option<tokio::task::JoinHandle<()>>,
    startup: Option<tokio::task::JoinHandle<()>>,
    generation: u64,
    cache_path: PathBuf,
    options: bookmarks::Options,
    started: bool,
}
impl Drop for Workspace {
    fn drop(&mut self) {
        for task in [&mut self.lookup, &mut self.scan, &mut self.startup]
            .into_iter()
            .filter_map(Option::take)
        {
            task.abort();
        }
    }
}
impl Workspace {
    pub fn show_error(&mut self, error: String) {
        self.open = true;
        self.message = error;
    }
    pub fn new(config: &Config, options: bookmarks::Options) -> Self {
        let (tx, rx) = unbounded_channel();
        let loaded =
            bookmarks::path(Network::from_testnet(config.chain.testnet)).and_then(Book::load);
        let (book, message) = match loaded {
            Ok(b) => (Some(b), String::new()),
            Err(e) => (None, format!("Unable to load bookmarks: {e}")),
        };
        Self {
            book,
            open: false,
            selected: 0,
            page_size: Cell::new(1),
            editor: None,
            deleting: false,
            permission: None,
            message,
            snapshots: HashMap::new(),
            tx,
            rx,
            lookup: None,
            scan: None,
            startup: None,
            generation: 0,
            cache_path: config.cache_path.clone(),
            options,
            started: false,
        }
    }
    fn entries(&self) -> Vec<Bookmark> {
        self.book
            .as_ref()
            .map(|b| b.entries.iter().map(|(_, b)| b.clone()).collect())
            .unwrap_or_default()
    }
    fn selected(&self) -> Option<Bookmark> {
        self.book
            .as_ref()?
            .entries
            .get(self.selected)
            .map(|(_, b)| b.clone())
    }
    fn len(&self) -> usize {
        self.book.as_ref().map_or(0, |book| book.entries.len())
    }
    pub fn can_copy(&self) -> bool {
        self.editor.is_none()
            && !self.deleting
            && self.permission.is_none()
            && self.lookup.is_none()
    }
    pub fn show(&mut self) {
        self.open = true;
        self.scan();
    }
    pub fn close(&mut self) {
        self.open = false;
        self.editor = None;
        self.deleting = false;
        self.permission = None;
        self.generation = self.generation.wrapping_add(1);
        if let Some(task) = self.lookup.take() {
            task.abort();
        }
    }
    pub fn paste(&mut self, text: &str) {
        if self.permission.is_none()
            && self.lookup.is_none()
            && let Some(editor) = &mut self.editor
        {
            let field = &mut editor.fields[editor.field];
            field.extend(text.chars().take(4096usize.saturating_sub(field.len())));
        }
    }
    pub fn editing(&self) -> bool {
        self.editor.is_some()
    }
    fn scan(&mut self) {
        if let Some(task) = self.scan.take() {
            task.abort();
        }
        let entries = self.entries();
        let path = self.cache_path.clone();
        let tx = self.tx.clone();
        self.scan = Some(tokio::spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                entries
                    .into_iter()
                    .map(|b| {
                        let value = Cache::open(&path)
                            .and_then(|cache| bookmarks::cached(&cache, &b))
                            .map_err(|e| e.to_string());
                        (b.key(), value)
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            if let Ok(values) = result {
                for (key, value) in values {
                    let _ = tx.send(Message::Cached(key, value));
                }
            }
        }));
    }
    /// Called only after the terminal has drawn. This never opens a permission modal.
    pub fn start_after_draw(&mut self, configs: &[Config]) {
        if self.started {
            return;
        }
        self.started = true;
        if !self.options.fetch_missing_on_startup {
            return;
        }
        let entries = self.entries();
        let configs = configs.to_vec();
        let tx = self.tx.clone();
        let path = self.cache_path.clone();
        let limit = self.options.startup_limit.min(100);
        self.startup = Some(tokio::spawn(async move {
            let mut attempted = 0;
            let mut fetched = 0;
            for b in entries {
                let cached = Cache::open(&path).and_then(|cache| bookmarks::cached(&cache, &b));
                match cached {
                    Ok(Some(snapshot)) => {
                        let _ = tx.send(Message::Cached(b.key(), Ok(Some(snapshot))));
                        continue;
                    }
                    Err(e) => {
                        let _ = tx.send(Message::Cached(b.key(), Err(e.to_string())));
                        continue;
                    }
                    _ => {}
                }
                if attempted >= limit {
                    break;
                }
                let Some(mut config) = configs
                    .iter()
                    .find(|c| c.chain.system_id.get() == Some(&b.system_id))
                    .cloned()
                else {
                    continue;
                };
                if config.daemon.control.state() == crate::sources::ConnectionState::Disabled
                    && !config
                        .daemon
                        .api_pool
                        .endpoints
                        .iter()
                        .any(|e| e.control.state() != crate::sources::ConnectionState::Disabled)
                {
                    continue;
                }
                attempted += 1;
                config.daemon.background = true;
                let result = tokio::time::timeout(
                    Duration::from_secs(70),
                    bookmarks::fetch(config, &b, false),
                )
                .await
                .map_err(|_| "Startup bookmark fetch timed out".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
                if result.is_ok() {
                    fetched += 1;
                }
                let _ = tx.send(Message::Cached(b.key(), result.map(Some)));
            }
            let _ = tx.send(Message::Startup(format!(
                "Startup bookmarks: {fetched} fetched, {attempted} attempted (limit {limit})"
            )));
        }));
    }
    pub fn has_updates(&self) -> bool {
        !self.rx.is_empty()
    }

    pub fn poll(&mut self) -> Option<(Bookmark, Snapshot)> {
        let mut opened = None;
        while let Ok(message) = self.rx.try_recv() {
            match message {
                Message::Cached(key, value) => {
                    // A cache-only scan must not erase a startup fetch failure.
                    if !matches!(&value, Ok(None))
                        || !matches!(self.snapshots.get(&key), Some(Err(_)))
                    {
                        self.snapshots.insert(key, value);
                    }
                }
                Message::Startup(message) => self.message = message,
                Message::Permission(g, b, c) if g == self.generation => {
                    self.lookup = None;
                    self.permission = Some((b, c));
                }
                Message::Open(g, b, result) if g == self.generation => {
                    self.lookup = None;
                    match result {
                        Ok(snapshot) => {
                            self.snapshots.insert(b.key(), Ok(Some(snapshot.clone())));
                            opened = Some((b, snapshot));
                            self.close();
                        }
                        Err(error) => self.message = error,
                    }
                }
                _ => {}
            }
        }
        opened
    }
    fn lookup(&mut self, b: Bookmark, config: Option<Config>, once: bool) {
        if let Some(task) = self.lookup.take() {
            task.abort();
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        let tx = self.tx.clone();
        let path = self.cache_path.clone();
        self.message = "Loading bookmark… Esc cancels".into();
        self.lookup = Some(tokio::spawn(async move {
            let cached = Cache::open(&path).and_then(|cache| bookmarks::cached(&cache, &b));
            match cached {
                Ok(Some(snapshot)) => {
                    let _ = tx.send(Message::Open(generation, b, Ok(snapshot)));
                    return;
                }
                Err(e) => {
                    let _ = tx.send(Message::Open(generation, b, Err(e.to_string())));
                    return;
                }
                _ => {}
            }
            let Some(mut config) = config else {
                let _=tx.send(Message::Open(generation,b,Err("Uncached; no connection configured for this system. Configure a source, then retry.".into())));
                return;
            };
            if !once
                && config.daemon.config_file.is_some()
                && config.daemon.control.state() == crate::sources::ConnectionState::Disabled
            {
                let _ = tx.send(Message::Permission(generation, b, config));
                return;
            }
            if once {
                config.daemon.control = Arc::default();
                let _ = config.daemon.control.expected.set((
                    b.system_id.clone(),
                    Network::from_testnet(config.chain.testnet),
                ));
            }
            let result = bookmarks::fetch(config, &b, true)
                .await
                .map_err(|e| e.to_string());
            let _ = tx.send(Message::Open(generation, b, result));
        }));
    }
    pub fn add_current(&mut self, config: &Config, state: &AppState) -> Result<()> {
        let bookmark = current_bookmark(config, state)?;
        self.editor = Some(Editor {
            fields: [
                bookmark.system_id.to_string(),
                bookmark.target,
                bookmark.description,
            ],
            field: 2,
            kind: bookmark.kind,
            editing: false,
        });
        self.message.clear();
        Ok(())
    }

    pub fn key(&mut self, code: KeyCode, config: &Config, configs: &[Config]) {
        if let Some((b, c)) = self.permission.take() {
            match code {
                KeyCode::Char('1') => self.lookup(b, Some(c), true),
                KeyCode::Char('2') => {
                    c.daemon
                        .control
                        .set(crate::sources::ConnectionState::Enabled);
                    self.lookup(b, Some(c), false);
                }
                KeyCode::Esc => self.message = "Bookmark opening cancelled".into(),
                _ => self.permission = Some((b, c)),
            }
            return;
        }
        if self.lookup.is_some() {
            if matches!(code, KeyCode::Esc | KeyCode::F(6)) {
                self.close();
            }
            return;
        }
        if let Some(editor) = &mut self.editor {
            match code {
                KeyCode::Esc => self.editor = None,
                KeyCode::Tab | KeyCode::Down if !editor.editing => {
                    editor.field = (editor.field + 1) % 3
                }
                KeyCode::BackTab | KeyCode::Up if !editor.editing => {
                    editor.field = (editor.field + 2) % 3
                }
                KeyCode::Backspace => {
                    editor.fields[editor.field].pop();
                }
                KeyCode::Char(c) if !c.is_control() && editor.fields[editor.field].len() < 4096 => {
                    editor.fields[editor.field].push(c)
                }
                KeyCode::Enter => {
                    let result = (|| -> Result<()> {
                        let book = self.book.as_mut().context("bookmark file unavailable")?;
                        if editor.editing {
                            book.edit(self.selected, editor.fields[2].clone())
                        } else {
                            book.add(Bookmark::new(
                                SystemId::try_from(editor.fields[0].trim().to_owned())?,
                                editor.kind,
                                editor.fields[1].clone(),
                                editor.fields[2].clone(),
                            )?)
                        }
                    })();
                    match result {
                        Ok(()) => {
                            if !editor.editing {
                                self.selected = self.len().saturating_sub(1);
                            }
                            self.editor = None;
                            self.message = "Bookmark saved".into();
                            self.scan();
                        }
                        Err(e) => self.message = e.to_string(),
                    }
                }
                _ => {}
            }
            return;
        }
        if self.deleting {
            match code {
                KeyCode::Char('y') => {
                    let result = self
                        .book
                        .as_mut()
                        .context("bookmark file unavailable")
                        .and_then(|b| b.remove(self.selected));
                    self.message = match result {
                        Ok(()) => "Bookmark removed".into(),
                        Err(e) => e.to_string(),
                    };
                    self.selected = self.selected.min(self.len().saturating_sub(1));
                    self.deleting = false;
                }
                KeyCode::Char('n') | KeyCode::Esc => self.deleting = false,
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Esc
            | KeyCode::F(6)
            | KeyCode::Left
            | KeyCode::Backspace
            | KeyCode::Char('h') => self.close(),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.len().saturating_sub(1),
            KeyCode::PageDown => {
                self.selected = self
                    .selected
                    .saturating_add(self.page_size.get())
                    .min(self.len().saturating_sub(1))
            }
            KeyCode::PageUp => self.selected = self.selected.saturating_sub(self.page_size.get()),
            KeyCode::Char('n') => {
                self.editor = Some(Editor {
                    fields: [config.chain_cache_key(), String::new(), String::new()],
                    field: 1,
                    kind: Kind::Auto,
                    editing: false,
                })
            }
            KeyCode::Char('e') => {
                if let Some(b) = self.selected() {
                    self.editor = Some(Editor {
                        fields: [b.system_id.to_string(), b.target, b.description],
                        field: 2,
                        kind: b.kind,
                        editing: true,
                    });
                }
            }
            KeyCode::Char('x') => self.deleting = self.selected().is_some(),
            KeyCode::Char('r') | KeyCode::F(5) => {
                let selected = self.selected().map(|b| b.key());
                let path = bookmarks::path(Network::from_testnet(config.chain.testnet));
                match path.and_then(Book::load) {
                    Ok(b) => {
                        self.selected = b
                            .entries
                            .iter()
                            .position(|(_, b)| Some(b.key()) == selected)
                            .unwrap_or(0);
                        self.book = Some(b);
                        self.message = "Bookmarks reloaded".into();
                        self.scan();
                    }
                    Err(e) => self.message = e.to_string(),
                }
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                if let Some(b) = self.selected() {
                    let config = configs
                        .iter()
                        .find(|c| c.chain.system_id.get() == Some(&b.system_id))
                        .cloned();
                    self.lookup(b, config, false);
                }
            }
            _ => {}
        }
    }

    pub fn render_add_popup(&self, frame: &mut Frame<'_>, theme: &Theme, body: Rect, footer: Rect) {
        frame.render_widget(Clear, footer);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("controls ", theme.muted()),
                Span::raw(self.controls().0),
            ]))
            .style(theme.base())
            .wrap(Wrap { trim: true }),
            footer,
        );
        self.render_dialog(frame, theme, body);
    }
    pub fn render(
        &self,
        frame: &mut Frame<'_>,
        theme: &Theme,
        mouse_capture: bool,
        configs: &[Config],
        source_names: &[String],
    ) {
        let area = frame.area();
        let (controls, close) = self.controls();
        let mut footer_lines = vec![Line::from(vec![
            Span::styled("controls ", theme.muted()),
            Span::raw(controls),
        ])];
        if let Some(close) = close {
            footer_lines.push(Line::from(vec![
                Span::raw(close),
                Span::raw(" | "),
                Span::styled(
                    format!("{} (Shift-M)", mouse_mode_label(mouse_capture)),
                    theme.warning(),
                ),
            ]));
        }
        let footer = Paragraph::new(footer_lines)
            .style(theme.base())
            .wrap(Wrap { trim: true });
        let [_, location, _, body, footer_area] = screen_layout(area, &footer);
        frame.render_widget(
            Clear,
            Rect {
                y: area.y.saturating_add(1),
                height: area.height.saturating_sub(1),
                ..area
            },
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("view ", theme.muted()),
                Span::styled("bookmarks", theme.focused()),
            ]))
            .style(theme.base()),
            location,
        );
        self.render_list(frame, theme, body, configs, source_names);
        frame.render_widget(footer, footer_area);
        self.render_dialog(frame, theme, body);
    }

    fn controls(&self) -> (&'static str, Option<&'static str>) {
        if self.permission.is_some() {
            (
                "1 connect once | 2 enable for session | Esc cancel | q quit",
                None,
            )
        } else if self.deleting {
            ("y remove | n/Esc cancel | q quit", None)
        } else if let Some(editor) = &self.editor {
            (
                if editor.editing {
                    "Enter save description | Esc cancel"
                } else {
                    "Tab/Shift-Tab or arrows select field | Enter save | Esc cancel"
                },
                None,
            )
        } else if self.lookup.is_some() {
            ("Opening bookmark | Esc/F6 cancel | q quit", None)
        } else {
            (
                "j/k select | Enter/Right/l open | Home/End first/last | PgUp/PgDn page | n new target | e edit description | x remove | F5/r reload",
                Some("Esc/F6 or Left/h back | q quit"),
            )
        }
    }

    fn render_list(
        &self,
        frame: &mut Frame<'_>,
        theme: &Theme,
        area: Rect,
        configs: &[Config],
        source_names: &[String],
    ) {
        let chain_name = |system: &SystemId| {
            configs
                .iter()
                .position(|config| config.chain.system_id.get() == Some(system))
                .and_then(|index| source_names.get(index))
                .cloned()
                .unwrap_or_else(|| system.to_string())
        };
        let details = Paragraph::new(self.details(theme, &chain_name, area.width))
            .style(theme.base())
            .wrap(Wrap { trim: true });
        let details_height = details.line_count(area.width.max(1)).min(usize::from(
            area.height
                .saturating_sub(SECTION_HEADING_HEIGHT + TILE_HEIGHT + 1),
        ));
        let [heading, list, _, details_area] = Layout::vertical([
            Constraint::Length(SECTION_HEADING_HEIGHT),
            Constraint::Min(1),
            Constraint::Length(u16::from(details_height > 0)),
            Constraint::Length(details_height as u16),
        ])
        .areas(area);

        let total = self.len();
        let visible = usize::from(list.height / TILE_HEIGHT).max(1);
        self.page_size.set(visible);
        let first = self
            .selected
            .saturating_sub(visible / 2)
            .min(total.saturating_sub(visible));
        let last = total.min(first + visible);
        let count = if total == 0 {
            "0 of 0".into()
        } else {
            format!("{}–{last} of {total}", first + 1)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("Bookmarks ", theme.focused()),
                Span::styled(count, theme.muted()),
            ]))
            .style(theme.base()),
            heading,
        );

        let content = scroll_view::content_area(list, total > visible);
        if let Some(book) = &self.book {
            for (offset, (_, bookmark)) in book.entries[first..last].iter().enumerate() {
                let y = content.y.saturating_add(offset as u16 * TILE_HEIGHT);
                let row = Rect::new(
                    content.x,
                    y,
                    content.width,
                    TILE_HEIGHT.min(content.bottom().saturating_sub(y)),
                );
                self.render_tile(
                    frame,
                    theme,
                    bookmark,
                    &chain_name(&bookmark.system_id),
                    first + offset == self.selected,
                    row,
                );
            }
        }
        if total == 0 {
            frame.render_widget(
                Paragraph::new(
                    "No bookmarks. Press n to enter a target, or Shift-B from a detail view.",
                )
                .style(theme.muted())
                .wrap(Wrap { trim: true }),
                content,
            );
        }
        scroll_view::render(frame, theme, list, total, first, visible);
        frame.render_widget(details, details_area);
    }

    fn render_tile(
        &self,
        frame: &mut Frame<'_>,
        theme: &Theme,
        bookmark: &Bookmark,
        chain: &str,
        selected: bool,
        area: Rect,
    ) {
        let width = tile_content_area(area).width;
        let kind = bookmark.kind.label();
        let description = if bookmark.description.is_empty() {
            match bookmark.kind {
                Kind::Auto => "Search target",
                Kind::Block => "Block",
                Kind::Transaction => "Transaction",
                Kind::Address => "Address",
                Kind::Identity => "Identity",
                Kind::Currency => "Currency",
                Kind::Notarizations => "Notarizations",
            }
        } else {
            &bookmark.description
        };
        let chain_label = truncate_middle(chain, usize::from(width / 3));
        let title_width =
            usize::from(width).saturating_sub(Line::raw(chain_label.as_str()).width() + 2);
        let title = justified_line(
            vec![theme.value_as(
                "description",
                truncate_end(description, title_width),
                description,
                theme.focused(),
            )],
            vec![theme.value_as(
                "system",
                chain_label,
                bookmark.system_id.as_ref(),
                theme.muted(),
            )],
            width,
        );
        let target = Line::from(vec![
            Span::styled(format!("{kind}  "), theme.muted()),
            theme.value_as(
                "bookmark target",
                truncate_middle(
                    &bookmark.target,
                    usize::from(width).saturating_sub(kind.len() + 2),
                ),
                bookmark.target.as_str(),
                theme.link(),
            ),
        ]);
        let (status, color, tags) = match self.snapshots.get(&bookmark.key()) {
            Some(Ok(Some(snapshot))) => (
                "Cached observation",
                theme.positive,
                snapshot_tags(snapshot),
            ),
            Some(Err(_)) => ("Unavailable (see details)", theme.warning, Vec::new()),
            Some(Ok(None)) => ("Uncached", theme.muted, Vec::new()),
            None => ("Checking cache…", theme.muted, Vec::new()),
        };
        let mut status_line = vec![Span::styled(
            format!(" {status} "),
            theme.status_badge_on(color),
        )];
        if !tags.is_empty() {
            status_line.push(Span::raw(" "));
            status_line.extend(transaction_tag_spans(theme, &tags));
        }
        // Keep each field on its own row so a long description can't hide the target.
        let content = render_tile_gutter(frame, theme, selected, area);
        frame.render_widget(
            tile_paragraph(
                theme,
                selected,
                vec![title, target, Line::from(status_line)],
            ),
            content,
        );
    }

    fn details(
        &self,
        theme: &Theme,
        chain_name: &impl Fn(&SystemId) -> String,
        width: u16,
    ) -> Vec<Line<'static>> {
        let mut details = Vec::new();
        if !self.message.is_empty() {
            details.push(Line::styled(self.message.clone(), theme.focused()));
        }
        if let Some(bookmark) = self.selected() {
            details.push(identity_property_pair(
                theme,
                "Created",
                theme.format_time(bookmark.created_at),
                "System",
                format!(
                    "{} ({})",
                    chain_name(&bookmark.system_id),
                    bookmark.system_id
                ),
                width,
            ));
            match self.snapshots.get(&bookmark.key()) {
                Some(Ok(Some(snapshot))) => details.push(identity_property_line(
                    theme,
                    "Observation source",
                    snapshot.source.clone(),
                )),
                Some(Err(error)) => details.push(Line::styled(error.clone(), theme.warning())),
                _ => {}
            }
        }
        if let Some(book) = &self.book {
            details.push(identity_property_line(
                theme,
                "Bookmark file",
                book.path.display().to_string(),
            ));
            if let Some(issue) = book.issues.first() {
                details.push(Line::styled(
                    format!("{} invalid line(s), preserved. {issue}", book.issues.len()),
                    theme.warning(),
                ));
            }
        }
        details
    }

    fn render_dialog(&self, frame: &mut Frame<'_>, theme: &Theme, area: Rect) {
        if let Some((bookmark, config)) = &self.permission {
            render_dialog(
                frame,
                theme,
                area,
                "Bookmark — disabled daemon",
                theme.warning(),
                vec![
                    identity_property_line(theme, "Target", bookmark.target.clone()),
                    identity_property_line(
                        theme,
                        "Source",
                        format!(
                            "{} · {}:{}",
                            config.chain.name, config.daemon.host, config.daemon.port
                        ),
                    ),
                ],
            );
        } else if let Some(editor) = &self.editor {
            let title = if editor.editing {
                "Edit bookmark description"
            } else {
                "Add bookmark"
            };
            let mut lines = vec![
                identity_property_line(theme, "Kind", editor.kind.label().into()),
                Line::default(),
            ];
            let width = usize::from(area.width.min(76));
            for (index, (label, value)) in ["System ID", "Target", "Description"]
                .into_iter()
                .zip(&editor.fields)
                .enumerate()
            {
                let active = index == editor.field;
                let mut display = truncate_middle(value, width.saturating_sub(label.len() + 8));
                display.push(if active { '_' } else { ' ' });
                lines.push(selection_line(theme, active, format!("{label}: {display}")));
            }
            if !self.message.is_empty() {
                lines.push(Line::styled(self.message.clone(), theme.muted()));
            }
            render_dialog(frame, theme, area, title, theme.focused(), lines);
        } else if self.deleting
            && let Some(bookmark) = self.selected()
        {
            render_dialog(
                frame,
                theme,
                area,
                "Remove selected bookmark?",
                theme.warning(),
                vec![identity_property_line(theme, "Target", bookmark.target)],
            );
        }
    }
}

fn render_dialog(
    frame: &mut Frame<'_>,
    theme: &Theme,
    area: Rect,
    title: &str,
    border: Style,
    lines: Vec<Line<'static>>,
) {
    let width = area.width.saturating_sub(2).min(76);
    let body = Paragraph::new(lines)
        .style(theme.base())
        .wrap(Wrap { trim: false });
    let height =
        (body.line_count(width.saturating_sub(2).max(1)) + 2).min(usize::from(area.height)) as u16;
    let popup = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        body.block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border)
                .title(title),
        ),
        popup,
    );
}

pub(super) fn supports_current(state: &AppState) -> bool {
    state.auxiliary.is_none()
        && matches!(
            state.screen,
            Screen::Block { .. }
                | Screen::Transaction { .. }
                | Screen::Address { .. }
                | Screen::Identity { .. }
                | Screen::Currency(_)
                | Screen::Notarizations(_)
        )
}

fn current_bookmark(config: &Config, state: &AppState) -> Result<Bookmark> {
    if let Screen::Notarizations(view) = &state.screen {
        let (observer, subject) = view.bookmark_target()?;
        return Bookmark::new(observer, Kind::Notarizations, subject, String::new());
    }
    let (kind, target) = match &state.screen {
        Screen::Block { target, .. } => (
            Kind::Block,
            state
                .store
                .block_for_target(target)
                .map(|b| b.summary.hash.clone())
                .unwrap_or_else(|| match target {
                    BlockTarget::Hash(h) => h.clone(),
                    BlockTarget::Height(h) => h.to_string(),
                }),
        ),
        Screen::Transaction { txid, .. } => (Kind::Transaction, txid.clone()),
        Screen::Address { address, .. } => (Kind::Address, address.clone()),
        Screen::Identity { target, .. } => (
            Kind::Identity,
            state
                .store
                .identity_for_target(target)
                .map(|v| v.fully_qualified_name.clone())
                .unwrap_or_else(|| target.query.clone()),
        ),
        Screen::Currency(view) => (
            Kind::Currency,
            view.id.as_ref().unwrap_or(&view.query).clone(),
        ),
        _ => anyhow::bail!(
            "Open a block, transaction, address, identity, currency, or notarization browser first."
        ),
    };
    Bookmark::new(
        config
            .chain
            .system_id
            .get()
            .context("unknown object system")?
            .clone(),
        kind,
        target,
        String::new(),
    )
}
fn snapshot_tags(snapshot: &Snapshot) -> Vec<String> {
    match snapshot.kind {
        Kind::Transaction => display_transaction_tags(
            &TransactionSummary {
                txid: snapshot.raw["txid"].as_str().unwrap_or_default().into(),
                raw: Some(snapshot.raw.clone()),
            },
            &ObjectStore::default(),
        ),
        Kind::Block => snapshot.raw["tx"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|v| v.is_object())
            .flat_map(|v| {
                display_transaction_tags(
                    &TransactionSummary {
                        txid: v["txid"].as_str().unwrap_or_default().into(),
                        raw: Some(v.clone()),
                    },
                    &ObjectStore::default(),
                )
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        Kind::Identity => vec!["Identity".into()],
        Kind::Address => vec!["Address".into()],
        Kind::Currency => vec!["Currency".into()],
        Kind::Notarizations => vec!["Notarizations".into()],
        Kind::Auto => vec![],
    }
}

pub(super) fn install(
    state: &mut AppState,
    config: &Config,
    configs: &[Config],
    bookmark: &Bookmark,
    snapshot: Snapshot,
) -> Result<()> {
    let screen = match snapshot.kind {
        Kind::Block => {
            let summary: BlockSummary = serde_json::from_value(snapshot.raw.clone())?;
            let target = BlockTarget::Hash(summary.hash.clone());
            state.store.insert_block(BlockRecord {
                prepared_transactions: Default::default(),
                summary,
                raw_json: snapshot.raw,
                detail_loaded: false,
            });
            block_screen(target)
        }
        Kind::Transaction => {
            let txid = snapshot.raw["txid"]
                .as_str()
                .context("transaction ID absent")?
                .to_owned();
            let mut record = transaction_record(&txid, snapshot.raw);
            record.source_id = Some(snapshot.source);
            state.store.insert_transaction(record);
            transaction_screen(txid)
        }
        Kind::Identity => {
            let raw = snapshot.raw;
            let identity_address = raw["identity"]["identityaddress"]
                .as_str()
                .context("identity address absent")?
                .to_owned();
            let fully_qualified_name = raw["fullyqualifiedname"]
                .as_str()
                .unwrap_or(&bookmark.target)
                .to_owned();
            let version = IdentityVersionRecord {
                identity_address,
                fully_qualified_name,
                block_height: raw["blockheight"].as_u64().unwrap_or_default(),
                block_time: 0,
                raw_json: raw,
            };
            state
                .store
                .insert_identity_version(&bookmark.target, version);
            identity_screen(bookmark.target.clone(), None)
        }
        Kind::Address => {
            state
                .store
                .addresses_by_address
                .entry(bookmark.target.clone())
                .or_insert_with(|| AddressRecord {
                    address: bookmark.target.clone(),
                    transaction_count: 0,
                    transaction_ids: vec![],
                    first_txid: None,
                    latest_txid: None,
                    unspent_outpoints: HashSet::new(),
                    balance_response: None,
                    utxos_complete: false,
                    history_complete: false,
                })
                .balance_response = Some(snapshot.raw);
            address_screen(bookmark.target.clone())
        }
        Kind::Currency | Kind::Notarizations => {
            let id = snapshot.raw["currencyid"]
                .as_str()
                .context("currency ID absent")?
                .to_owned();
            let name = snapshot.raw["fullyqualifiedname"]
                .as_str()
                .or_else(|| snapshot.raw["name"].as_str())
                .unwrap_or(&bookmark.target)
                .to_owned();
            state.store.learn_currency_data(&snapshot.raw);
            state
                .store
                .insert_currency_definition(CurrencyDefinitionRecord {
                    currency_id: id.clone(),
                    name: name.clone(),
                    definition_json: snapshot.raw.clone(),
                });
            let mut screen = currency_ui::screen(id.clone(), bookmark.system_id.to_string());
            if let Screen::Currency(view) = &mut screen {
                currency_ui::apply(
                    view,
                    currency_ui::Loaded {
                        query: id.clone(),
                        id: Some(id),
                        name: Some(name),
                        definition: Some(snapshot.raw),
                        snapshots: Vec::new(),
                        error: None,
                        activity: Vec::new(),
                    },
                );
                if snapshot.kind == Kind::Notarizations {
                    screen = notarization_ui::browser(view, config, configs)
                        .context("notarization browser unavailable for this currency")?;
                }
            }
            screen
        }
        Kind::Auto => anyhow::bail!("bookmark did not resolve to an object"),
    };
    state.auxiliary = None;
    state.help_return = None;
    cancel_search(state);
    state.search.active = false;
    push_screen(state, screen);
    Ok(())
}
