//! Currency discovery is one explicit list request, with retained source observations.
use super::*;
use crate::currency_status::{self, LaunchStatus};

#[derive(Debug, Clone)]
pub(super) struct Entry {
    pub id: String,
    pub name: String,
    pub definition: Value,
    pub state: Value,
}

#[derive(Debug, Clone)]
pub(super) struct View {
    pub request: u64,
    pub entries: Vec<Entry>,
    pub selected: usize,
    pub filter: String,
    pub editing: bool,
    pub options_open: bool,
    pub launch_open: bool,
    launch_mask: u8,
    launch_selected: usize,
    required: u64,
    excluded: u64,
    option_selected: usize,
    pub loading: bool,
    pub source: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct Loaded {
    pub request: u64,
    pub entries: Vec<Entry>,
    pub source: String,
    pub loading: bool,
    pub error: Option<String>,
}

pub(super) fn screen(store: &ObjectStore) -> Screen {
    let mut view = View {
        request: currency_ui::request_id(),
        entries: Vec::new(),
        selected: 0,
        filter: String::new(),
        editing: false,
        options_open: false,
        launch_open: false,
        launch_mask: LaunchStatus::DEFAULT_MASK,
        launch_selected: 0,
        required: 1, // OPTION_FRACTIONAL: liquidity pools.
        excluded: 0,
        option_selected: 0,
        loading: true,
        source: "Cached definitions — availability on this system unverified".into(),
        error: None,
    };
    view.entries = store
        .currencies_by_id
        .values()
        .map(|record| Entry {
            id: record.currency_id.clone(),
            name: record.name.clone(),
            definition: record.definition_json.clone(),
            state: Value::Null,
        })
        .collect();
    sort(&mut view.entries);
    Screen::Currencies(Box::new(view))
}

fn sort(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
}

fn decode(value: &Value) -> Result<Vec<Entry>> {
    let list = value
        .as_array()
        .context("listcurrencies did not return a list")?;
    let mut entries = BTreeMap::new();
    for record in list {
        let definition = record.get("currencydefinition").unwrap_or(record);
        let Some(id) = definition["currencyid"].as_str() else {
            continue;
        };
        let name = definition["fullyqualifiedname"]
            .as_str()
            .or_else(|| definition["name"].as_str())
            .unwrap_or(id);
        entries.insert(
            id.to_string(),
            Entry {
                id: id.into(),
                name: name.into(),
                definition: definition.clone(),
                state: currency_status::reported_state(record).clone(),
            },
        );
    }
    let mut entries: Vec<_> = entries.into_values().collect();
    sort(&mut entries);
    Ok(entries)
}

pub(super) fn spawn(config: Config, tx: UnboundedSender<AppEvent>, request: u64) {
    let tx = revision_sender(&config, tx);
    tokio::spawn(async move {
        let cached_config = config.clone();
        let cached =
            tokio::task::spawn_blocking(move || -> Result<Option<(Vec<Entry>, String)>> {
                let cache = Cache::open(&cached_config.cache_path)?;
                let sources = std::iter::once(cached_config.source_cache_key("local-daemon"))
                    .chain(
                        cached_config
                            .daemon
                            .api_pool
                            .endpoints
                            .iter()
                            .map(|endpoint| crate::rpc::source_id("public-api", &endpoint.url)),
                    );
                for source in sources {
                    if let Some(raw) = cache.raw_rpc_response(
                        &source,
                        &cached_config.chain_cache_key(),
                        "listcurrencies",
                        "[]",
                    )? {
                        let response: Value = serde_json::from_str(&raw)?;
                        return Ok(Some((decode(&response["result"])?, source)));
                    }
                }
                Ok(None)
            })
            .await;
        if let Ok(Ok(Some((entries, source)))) = cached {
            let _ = tx.send(AppEvent::CurrencyBrowserLoaded(Loaded {
                request,
                entries,
                source: format!("Retained list observation — {source}"),
                loading: true,
                error: None,
            }));
        }
        // Opening this browser must not cause API-consent prompts or enable a source.
        let result = async {
            let client = RpcClient::for_config(&config)?.noninteractive();
            let output = client.call_with_raw::<Value>("listcurrencies", &[]).await?;
            Ok::<_, anyhow::Error>((decode(&output.result)?, output.source_id))
        }
        .await;
        let loaded = match result {
            Ok((entries, source)) => Loaded {
                request,
                entries,
                source: format!("List observation — {source}"),
                loading: false,
                error: None,
            },
            Err(error) => Loaded {
                request,
                entries: Vec::new(),
                source: String::new(),
                loading: false,
                error: Some(format!("List unavailable; retained data kept: {error}")),
            },
        };
        let _ = tx.send(AppEvent::CurrencyBrowserLoaded(loaded));
    });
}

pub(super) fn apply(view: &mut View, loaded: &Loaded) {
    if view.request != loaded.request {
        return;
    }
    let selected = view
        .filtered()
        .get(view.selected)
        .map(|entry| entry.id.clone());
    if loaded.error.is_none() {
        view.entries = loaded.entries.clone();
        view.source = loaded.source.clone();
    }
    view.loading = loaded.loading;
    view.error = loaded.error.clone();
    view.selected = selected
        .and_then(|id| view.filtered().iter().position(|entry| entry.id == id))
        .unwrap_or(0);
}

impl View {
    pub fn filtered(&self) -> Vec<&Entry> {
        let filter = self.filter.to_lowercase();
        self.entries
            .iter()
            .filter(|entry| self.launch_mask & currency_status::classify(&entry.state).bit() != 0)
            .filter(|entry| match entry.definition["options"].as_u64() {
                Some(options) => {
                    options & self.required == self.required && options & self.excluded == 0
                }
                None => self.required == 0 && self.excluded == 0,
            })
            .filter(|entry| {
                entry.name.to_lowercase().contains(&filter)
                    || entry.id.to_lowercase().contains(&filter)
                    || currency_ui::option_tags(&entry.definition)
                        .join(" ")
                        .to_lowercase()
                        .contains(&filter)
            })
            .collect()
    }
    pub fn selected(&self) -> Option<Entry> {
        self.filtered()
            .get(self.selected)
            .map(|entry| (*entry).clone())
    }
    pub fn move_by(&mut self, delta: isize) {
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.filtered().len().saturating_sub(1));
    }
    pub fn paste(&mut self, text: &str) {
        if self.editing {
            self.filter.push_str(text);
            self.selected = 0;
        }
    }
    pub fn edit_key(&mut self, key: KeyCode) -> bool {
        if (self.options_open || self.launch_open) && matches!(key, KeyCode::Tab | KeyCode::BackTab)
        {
            self.options_open = !self.options_open;
            self.launch_open = !self.options_open;
            return true;
        }
        if self.launch_open {
            match key {
                KeyCode::Esc | KeyCode::Char('F') => self.launch_open = false,
                KeyCode::Down | KeyCode::Char('j') => {
                    self.launch_selected = (self.launch_selected + 1).min(3)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.launch_selected = self.launch_selected.saturating_sub(1)
                }
                KeyCode::Char(' ') | KeyCode::Enter => {
                    self.launch_mask ^= LaunchStatus::ALL[self.launch_selected].bit();
                    self.selected = 0;
                }
                KeyCode::Char('a') => {
                    self.launch_mask = LaunchStatus::ALL_MASK;
                    self.selected = 0;
                }
                KeyCode::Char('d') => {
                    self.launch_mask = LaunchStatus::DEFAULT_MASK;
                    self.selected = 0;
                }
                KeyCode::Char('l') => {
                    self.launch_mask = LaunchStatus::Launched.bit();
                    self.selected = 0;
                }
                KeyCode::Char('p') => {
                    self.launch_mask = LaunchStatus::Prelaunch.bit();
                    self.selected = 0;
                }
                KeyCode::Char('f') => {
                    self.launch_mask = LaunchStatus::Failed.bit();
                    self.selected = 0;
                }
                KeyCode::Char('u') => {
                    self.launch_mask = LaunchStatus::Unknown.bit();
                    self.selected = 0;
                }
                _ => {}
            }
            return true;
        }
        if self.options_open {
            match key {
                KeyCode::Esc | KeyCode::Char('F') => self.options_open = false,
                KeyCode::Down | KeyCode::Char('j') => {
                    self.option_selected =
                        (self.option_selected + 1).min(currency_ui::OPTION_TAGS.len() - 1)
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.option_selected = self.option_selected.saturating_sub(1)
                }
                KeyCode::Home => self.option_selected = 0,
                KeyCode::End => self.option_selected = currency_ui::OPTION_TAGS.len() - 1,
                KeyCode::Char(' ') | KeyCode::Enter => {
                    let bit = currency_ui::OPTION_TAGS[self.option_selected].0;
                    if self.required & bit != 0 {
                        self.required &= !bit;
                        self.excluded |= bit;
                    } else if self.excluded & bit != 0 {
                        self.excluded &= !bit;
                    } else {
                        self.required |= bit;
                    }
                    self.selected = 0;
                }
                KeyCode::Char('a') => {
                    self.required = 0;
                    self.excluded = 0;
                    self.selected = 0;
                }
                KeyCode::Char('p') => {
                    self.required = 1;
                    self.excluded = 0;
                    self.selected = 0;
                }
                _ => {}
            }
            return true;
        }
        if !self.editing {
            return false;
        }
        match key {
            KeyCode::Esc | KeyCode::Enter => self.editing = false,
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
            }
            KeyCode::Char(ch) => {
                self.filter.push(ch);
                self.selected = 0;
            }
            _ => {}
        }
        true
    }
}

fn option_summary(view: &View) -> String {
    match (view.required, view.excluded) {
        (1, 0) => "Liquidity pools (Show Fractional)".into(),
        (0, 0) => "Any option flags".into(),
        _ => format!(
            "Show {} / Hide {}",
            view.required.count_ones(),
            view.excluded.count_ones()
        ),
    }
}

fn render_options(frame: &mut Frame<'_>, theme: &Theme, view: &View, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Currency option flags ");
    let inner = block.inner(area);
    frame.render_widget(block.border_style(theme.focused()), area);
    let header_rows = inner.height.saturating_sub(1).min(3);
    let visible = usize::from(inner.height - header_rows);
    let first = view
        .option_selected
        .saturating_sub(visible / 2)
        .min(currency_ui::OPTION_TAGS.len().saturating_sub(visible));
    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "Matches {} of {} · {}",
                view.filtered().len(),
                view.entries.len(),
                option_summary(view)
            ),
            theme.focused(),
        )),
        Line::from("Matches have all Show flags and none of the Hide flags."),
        Line::from(""),
    ];
    lines.truncate(usize::from(header_rows));
    for (index, (bit, label)) in currency_ui::OPTION_TAGS
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
    {
        let (state, style) = if view.required & bit != 0 {
            ("[+] Show", theme.positive())
        } else if view.excluded & bit != 0 {
            ("[-] Hide", theme.negative())
        } else {
            ("[ ] Any", theme.muted())
        };
        lines.push(Line::from(vec![
            theme.selection_marker(index == view.option_selected),
            Span::raw(" "),
            Span::styled(format!("{state:<10}"), style),
            Span::styled(
                format!(
                    "{label}{}",
                    if *bit == 1 { " (liquidity pool)" } else { "" }
                ),
                if index == view.option_selected {
                    theme.focused()
                } else {
                    theme.base()
                },
            ),
            Span::styled(format!("  0x{bit:04x}"), theme.muted()),
        ]));
    }
    frame.render_widget(
        Paragraph::new(lines).style(theme.base()),
        scroll_view::content_area(inner, currency_ui::OPTION_TAGS.len() > visible),
    );
    let rail = Rect::new(
        inner.x,
        inner.y.saturating_add(header_rows),
        inner.width,
        visible as u16,
    );
    scroll_view::render(
        frame,
        theme,
        rail,
        currency_ui::OPTION_TAGS.len(),
        first,
        visible,
    );
}

fn launch_summary(view: &View) -> String {
    if view.launch_mask == LaunchStatus::DEFAULT_MASK {
        return "All except failed".into();
    }
    if view.launch_mask == LaunchStatus::ALL_MASK {
        return "All statuses".into();
    }
    if view.launch_mask == 0 {
        return "None selected".into();
    }
    LaunchStatus::ALL
        .iter()
        .filter(|status| view.launch_mask & status.bit() != 0)
        .map(|status| status.label())
        .collect::<Vec<_>>()
        .join(" + ")
}

fn render_launch_filter(frame: &mut Frame<'_>, theme: &Theme, view: &View, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Currency launch status ");
    let inner = block.inner(area);
    frame.render_widget(block.border_style(theme.focused()), area);
    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "Matches {} of {} · {}",
                view.filtered().len(),
                view.entries.len(),
                launch_summary(view)
            ),
            theme.focused(),
        )),
        Line::from("Option flags and text filtering still apply."),
        Line::from(""),
    ];
    let header_rows = inner.height.saturating_sub(1).min(3);
    lines.truncate(usize::from(header_rows));
    let visible = usize::from(inner.height - header_rows).min(LaunchStatus::ALL.len());
    let first = view
        .launch_selected
        .saturating_sub(visible / 2)
        .min(4usize.saturating_sub(visible));
    for (index, status) in LaunchStatus::ALL
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
    {
        lines.push(Line::from(vec![
            theme.selection_marker(view.launch_selected == index),
            Span::raw(" "),
            Span::styled(
                if view.launch_mask & status.bit() != 0 {
                    "[x] "
                } else {
                    "[ ] "
                },
                if view.launch_mask & status.bit() != 0 {
                    theme.positive()
                } else {
                    theme.muted()
                },
            ),
            currency_ui::launch_badge(theme, *status),
        ]));
    }
    lines.push(Line::from(
        "Refunding flag proves failure; zero reserves do not.",
    ));
    lines.push(Line::from(
        "Unknown state is kept separate, including retained definitions without state.",
    ));
    frame.render_widget(
        Paragraph::new(lines).style(theme.base()),
        scroll_view::content_area(inner, LaunchStatus::ALL.len() > visible),
    );
    let rail = Rect::new(
        inner.x,
        inner.y.saturating_add(header_rows),
        inner.width,
        visible as u16,
    );
    scroll_view::render(frame, theme, rail, LaunchStatus::ALL.len(), first, visible);
}

pub(super) fn render(
    frame: &mut Frame<'_>,
    theme: &Theme,
    view: &View,
    store: &ObjectStore,
    area: Rect,
) {
    if view.launch_open {
        render_launch_filter(frame, theme, view, area);
        return;
    }
    if view.options_open {
        render_options(frame, theme, view, area);
        return;
    }
    let entries = view.filtered();
    let heading = vec![
        Line::from(Span::styled(
            format!(
                "Currencies  {} of {}{}",
                entries.len(),
                view.entries.len(),
                if view.loading { " · loading…" } else { "" }
            ),
            theme.focused(),
        )),
        Line::from(Span::styled(view.source.clone(), theme.muted())),
        Line::from(format!(
            "Filter: {}{}  · f edit · Enter opens · F5/r refresh",
            view.filter,
            if view.editing { "_" } else { "" }
        )),
    ];
    let mut heading = heading;
    heading.push(Line::from(vec![
        Span::styled("Options: ", theme.muted()),
        Span::styled(option_summary(view), theme.positive()),
        Span::raw(" · Shift-F flags"),
    ]));
    heading.push(Line::from(vec![
        Span::styled("Launch: ", theme.muted()),
        Span::styled(launch_summary(view), theme.warning()),
        Span::raw(" · Shift-F then Tab"),
    ]));
    let header_height = 5 + u16::from(view.error.is_some());
    frame.render_widget(
        Paragraph::new(heading).style(theme.base()),
        Rect::new(area.x, area.y, area.width, 5.min(area.height)),
    );
    if let Some(error) = &view.error {
        frame.render_widget(
            Paragraph::new(error.as_str()).style(theme.warning()),
            Rect::new(
                area.x,
                area.y.saturating_add(5),
                area.width,
                u16::from(area.height > 5),
            ),
        );
    }
    let compact_height = 5;
    let available_height = area.height.saturating_sub(header_height);
    let visible = usize::from(available_height / compact_height).max(1);
    // A dedicated reserve row should not reduce the number of browsable tiles.
    let mut spare_rows = usize::from(available_height)
        .saturating_sub(entries.len().min(visible) * usize::from(compact_height));
    let first = view
        .selected
        .saturating_sub(visible / 2)
        .min(entries.len().saturating_sub(visible));
    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new("No matching currencies in this observation").style(theme.muted()),
            Rect::new(
                area.x,
                area.y.saturating_add(header_height),
                area.width,
                area.height.saturating_sub(header_height),
            ),
        );
    }
    let names: HashMap<_, _> = view
        .entries
        .iter()
        .filter(|entry| !entry.name.is_empty() && entry.name != entry.id)
        .map(|entry| (entry.id.as_str(), entry.name.as_str()))
        .collect();
    let list_area = Rect::new(
        area.x,
        area.y.saturating_add(header_height),
        area.width,
        available_height,
    );
    let body = scroll_view::content_area(list_area, entries.len() > visible);
    let content_width = usize::from(tile_content_area(body).width);
    let mut y = body.y;
    for (offset, entry) in entries.iter().skip(first).take(visible).enumerate() {
        if y >= area.bottom() {
            break;
        }
        let active = first + offset == view.selected;
        let tags = currency_ui::option_tags(&entry.definition);
        let mut tag_line = currency_ui::tag_line(theme, &tags);
        tag_line.spans.insert(0, Span::raw(" "));
        tag_line.spans.insert(
            0,
            currency_ui::launch_badge(theme, currency_status::classify(&entry.state)),
        );
        let reserves: Vec<_> = entry.definition["currencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|id| {
                entry.definition["currencynames"][id]
                    .as_str()
                    .filter(|name| !name.is_empty() && *name != id)
                    .or_else(|| names.get(id).copied())
                    .unwrap_or_else(|| currency_display_value(store, id))
            })
            .collect();
        let count = reserves.len();
        let unit = if count == 1 { "reserve" } else { "reserves" };
        let start = Line::from(vec![
            Span::raw("Start "),
            theme.value(
                "start block",
                entry.definition["startblock"].to_string(),
                Style::default(),
            ),
            Span::raw(" · "),
        ]);
        let mut summary = Line::from(vec![
            Span::raw("Supply "),
            theme.value(
                "supply",
                entry
                    .state
                    .get("supply")
                    .map_or("unavailable".into(), Value::to_string),
                Style::default(),
            ),
            Span::raw(" · "),
            theme.value("reserve count", count.to_string(), Style::default()),
            Span::raw(format!(" {unit}")),
        ]);
        if start.width() + summary.width() <= content_width {
            summary.spans.splice(..0, start.spans);
        }
        let mut reserve_row = None;
        if !reserves.is_empty() {
            let names = reserves.join(", ");
            if summary.width() + Line::from(names.as_str()).width() + 3 <= content_width {
                summary.spans.extend([
                    Span::raw(" · "),
                    theme.value("reserve currencies", names, Style::default()),
                ]);
            } else if spare_rows > 0 {
                let row = Line::from(vec![
                    Span::styled("Reserves: ", Style::default().fg(theme.muted)),
                    theme.value("reserve currencies", names, Style::default()),
                ]);
                if row.width() <= content_width {
                    reserve_row = Some(row);
                    spare_rows -= 1;
                }
            }
        }
        let mut lines = vec![
            Line::from(
                theme.value(
                    "currency name",
                    entry.name.clone(),
                    Style::default()
                        .fg(theme.focus)
                        .add_modifier(Modifier::BOLD),
                ),
            ),
            tag_line,
            Line::from(theme.value(
                "currency i-address",
                entry.id.clone(),
                Style::default().fg(theme.link),
            )),
            summary,
        ];
        lines.extend(reserve_row);
        let tile_height = lines.len() as u16 + 1;
        let content = render_tile_gutter(
            frame,
            theme,
            active,
            Rect::new(body.x, y, body.width, tile_height.min(area.bottom() - y)),
        );
        frame.render_widget(
            Paragraph::new(lines).style(theme.tile_body(active)),
            content,
        );
        y = y.saturating_add(tile_height);
    }
    scroll_view::render(frame, theme, list_area, entries.len(), first, visible);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn filter_view() -> Box<View> {
        let Screen::Currencies(mut view) = screen(&ObjectStore::default()) else {
            unreachable!()
        };
        view.entries = decode(&json!([
            {"currencyid":"i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV","name":"VRSC","options":256},
            {"currencyid":"i6j1rzjgrDhSmUYiTtp21J8Msiudv5hgt9","name":"Bridge.vDEX","options":545},
            {"currencyid":"iGBs4DWztRNvNEJBt4mqHszLxfKTNHTkhM","name":"DAI.vETH","options":32},
            {"currencyid":"i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X","name":"vETH"}
        ]))
        .unwrap();
        view
    }

    #[test]
    fn selection_marker_and_contrast_follow_the_currency_tile() {
        use ratatui::backend::TestBackend;

        let mut view = filter_view();
        view.required = 0;
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        let mut terminal = Terminal::new(TestBackend::new(90, 30)).unwrap();

        for (selected, unselected) in [(5, 10), (10, 5)] {
            terminal
                .draw(|frame| render(frame, &theme, &view, &ObjectStore::default(), frame.area()))
                .unwrap();
            let buffer = terminal.backend().buffer();
            assert_eq!(buffer[(0, selected)].symbol(), ">");
            assert_eq!(buffer[(0, selected)].bg, theme.accent);
            assert_eq!(buffer[(0, selected + 1)].symbol(), "│");
            assert_eq!(buffer[(0, unselected)].symbol(), " ");
            assert_eq!(buffer[(0, unselected)].bg, theme.panel);
            for (row, background) in [(selected, theme.panel), (unselected, theme.background)] {
                assert_eq!(buffer[(2, row)].bg, background); // Name
                assert_eq!(buffer[(2, row + 2)].bg, background); // Currency ID
                assert_eq!(buffer[(89, row + 3)].bg, background); // Trailing space
            }
            view.selected = 1;
        }
    }

    #[test]
    fn reserve_names_use_available_width_and_height() {
        use ratatui::backend::TestBackend;

        let mut view = filter_view();
        let pool = view
            .entries
            .iter_mut()
            .find(|entry| entry.name == "Bridge.vDEX")
            .unwrap();
        let vdex = "iHog9UCTrn95qpUBFCZ7kKz7qWdMA8MQ6N";
        pool.definition["currencies"] = json!([
            "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV",
            "iGBs4DWztRNvNEJBt4mqHszLxfKTNHTkhM",
            vdex,
            "i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X"
        ]);
        pool.definition["startblock"] = json!(1);
        pool.state = json!({"supply":100});
        let mut store = ObjectStore::default();
        store.currency_name_hints.insert(vdex.into(), "vDEX".into());
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        let reserves = "VRSC, DAI.vETH, vDEX, vETH";

        for (width, height, expected) in [
            (80, 10, Some(format!("4 reserves · {reserves}"))),
            (46, 11, Some(format!("Reserves: {reserves}"))),
            (46, 10, None),
            (35, 11, None),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| render(frame, &theme, &view, &store, frame.area()))
                .unwrap();
            let text: String = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect();
            assert!(text.contains("4 reserves"), "{width}x{height}: {text}");
            if let Some(expected) = expected {
                assert!(text.contains(&expected), "{width}x{height}: {text}");
            } else {
                assert!(!text.contains(reserves), "{width}x{height}: {text}");
            }
        }
    }

    #[test]
    fn defaults_to_pools_and_combines_required_excluded_and_text_filters() {
        let mut view = filter_view();
        assert_eq!(
            view.filtered()
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Bridge.vDEX"]
        );
        view.required |= 32;
        assert_eq!(view.filtered().len(), 1); // Both Fractional and Token are required.
        view.excluded = 512;
        assert!(view.filtered().is_empty());
        view.required = 0;
        assert_eq!(view.filtered().len(), 2); // Unknown options do not satisfy an exclusion.
        view.excluded = 0;
        assert_eq!(view.filtered().len(), 4); // All currencies includes missing metadata.
        view.filter = "dai".into();
        assert_eq!(view.filtered()[0].name, "DAI.vETH");
        view.required = 1;
        assert!(view.filtered().is_empty());
    }

    #[test]
    fn panel_cycles_flags_resets_selection_and_preserves_filters_on_refresh() {
        let mut view = filter_view();
        view.options_open = true;
        view.selected = 3;
        view.edit_key(KeyCode::Char(' '));
        assert_eq!((view.required, view.excluded, view.selected), (0, 1, 0));
        assert_eq!(view.filtered().len(), 2);
        view.edit_key(KeyCode::Enter);
        assert_eq!((view.required, view.excluded), (0, 0));
        view.edit_key(KeyCode::Char(' '));
        assert_eq!((view.required, view.excluded), (1, 0));
        view.edit_key(KeyCode::Char('a'));
        assert_eq!(view.filtered().len(), 4);
        view.edit_key(KeyCode::Char('p'));
        assert_eq!(view.filtered().len(), 1);
        let loaded = Loaded {
            request: view.request,
            entries: view.entries.clone(),
            source: "retained".into(),
            loading: false,
            error: None,
        };
        apply(&mut view, &loaded);
        assert_eq!((view.required, view.excluded), (1, 0));
        view.edit_key(KeyCode::Esc);
        assert!(!view.options_open);
    }

    #[test]
    fn option_panel_scrolls_to_the_selected_flag_in_small_terminals() {
        let mut view = filter_view();
        view.options_open = true;
        view.edit_key(KeyCode::End);
        let theme = Theme::desert(crate::cli::EmojiMode::Off);
        for (width, height) in [(45, 3), (45, 12), (80, 24), (120, 40)] {
            let mut terminal =
                Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            view.options_open = true;
            view.launch_open = false;
            for label in ["No ID issuance", "Launch status unknown"] {
                terminal
                    .draw(|frame| {
                        render(frame, &theme, &view, &ObjectStore::default(), frame.area())
                    })
                    .unwrap();
                let buffer = terminal.backend().buffer();
                let selected = buffer
                    .content
                    .chunks(usize::from(width))
                    .find(|line| line[1].symbol() == SELECTION_MARKER)
                    .unwrap();
                let text: String = selected.iter().map(|cell| cell.symbol()).collect();
                assert!(text.contains(label), "{text}");
                view.edit_key(KeyCode::Tab);
                view.launch_selected = 3;
            }
        }
    }

    #[test]
    fn launch_filters_exclude_refunding_by_default_and_preserve_unknown_as_separate() {
        let mut view = filter_view();
        view.required = 0;
        view.entries[0].state = json!({"flags":3});
        view.entries[1].state = json!({"flags":49});
        view.entries[2].state = json!({"flags":37});
        assert_eq!(view.filtered().len(), 3); // One failed launch hidden; unknown retained.
        view.launch_open = true;
        view.edit_key(KeyCode::Char('f'));
        assert_eq!(view.filtered().len(), 1);
        assert_eq!(
            currency_status::classify(&view.filtered()[0].state),
            LaunchStatus::Failed
        );
        view.edit_key(KeyCode::Char('l'));
        assert_eq!(view.filtered().len(), 1);
        assert_eq!(
            currency_status::classify(&view.filtered()[0].state),
            LaunchStatus::Launched
        );
        view.edit_key(KeyCode::Char('p'));
        assert_eq!(
            currency_status::classify(&view.filtered()[0].state),
            LaunchStatus::Prelaunch
        );
        view.edit_key(KeyCode::Char('u'));
        assert_eq!(view.filtered().len(), 1);
        assert_eq!(
            currency_status::classify(&view.filtered()[0].state),
            LaunchStatus::Unknown
        );
        view.edit_key(KeyCode::Char('a'));
        assert_eq!(view.filtered().len(), 4);
        view.edit_key(KeyCode::Char('d'));
        assert_eq!(view.filtered().len(), 3);
        view.launch_selected = 2;
        view.edit_key(KeyCode::Char(' ')); // Include failed as well.
        assert_eq!(view.filtered().len(), 4);
        let loaded = Loaded {
            request: view.request,
            entries: view.entries.clone(),
            source: "retained".into(),
            loading: false,
            error: None,
        };
        apply(&mut view, &loaded);
        assert_eq!(view.launch_mask, LaunchStatus::ALL_MASK);
    }

    #[test]
    fn nested_definitions_decode_deduplicate_and_filter_by_flags() {
        let entries = decode(&json!([
            {"currencydefinition":{"currencyid":"i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV","name":"VRSC","options":256}},
            {"currencydefinition":{"currencyid":"i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV","name":"VRSC","options":256}}
        ])).unwrap();
        assert_eq!(entries.len(), 1);
        let Screen::Currencies(mut view) = screen(&ObjectStore::default()) else {
            unreachable!()
        };
        view.entries = entries;
        view.required = 0;
        view.filter = "pbaas".into();
        assert_eq!(view.filtered().len(), 1);
        view.filter = "fractional".into();
        assert!(view.filtered().is_empty());
    }
}
