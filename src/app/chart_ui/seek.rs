use super::*;
use crate::block_time::{self, RpcTimeSource, TimeBlock, TimeSource};
use chrono::{DateTime, TimeZone, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::app) enum Target {
    Number(u64),
    Height(u64),
    Time(i64),
    Latest,
}

pub(in crate::app) fn parse(input: &str, now: DateTime<Utc>) -> Result<Target> {
    let input = input.trim();
    anyhow::ensure!(
        !input.is_empty(),
        "Enter a block height, Unix seconds, or date/time"
    );
    if let Some(height) = input
        .strip_prefix("height:")
        .or_else(|| input.strip_prefix('#'))
    {
        return Ok(Target::Height(
            height
                .trim()
                .parse()
                .context("Enter a nonnegative block height")?,
        ));
    }
    if let Some(time) = input.strip_prefix("unix:") {
        return Ok(Target::Time(block_time::parse_at(time, now)?.timestamp()));
    }
    if input.bytes().all(|byte| byte.is_ascii_digit()) {
        let number = input.parse::<u64>().context("Number is out of range")?;
        anyhow::ensure!(
            number <= 99_999_999_999,
            "Use Unix seconds, or height: followed by a block height"
        );
        return Ok(Target::Number(number));
    }
    Ok(Target::Time(block_time::parse_at(input, now)?.timestamp()))
}

#[derive(Debug, Clone)]
pub(super) struct Prompt {
    input: String,
    now: DateTime<Utc>,
    pub error: Option<String>,
    future: bool,
    resume: bool,
    progress: String,
}
impl Prompt {
    pub(super) fn new(resume: bool) -> Self {
        Self {
            input: String::new(),
            now: Utc::now(),
            error: None,
            future: false,
            resume,
            progress: "Locating requested block… Esc cancels".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::app) struct Outcome {
    block: TimeBlock,
    end: TimeBlock,
    tip: u64,
    source: String,
    note: String,
    future: bool,
}

pub(super) async fn resolve(
    config: &Config,
    target: Target,
    span: TimeSpan,
    mut progress: impl FnMut(&str),
) -> Result<Outcome> {
    let (mut source, tip, _) = RpcTimeSource::open(config).await?;
    let target = match target {
        Target::Number(number) if number <= tip.height => Target::Height(number),
        Target::Number(number) => {
            let genesis = source.block_at(0).await?;
            let time = i64::try_from(number).context("Number is out of range")?;
            anyhow::ensure!(
                time >= genesis.timestamp,
                "Enter a height from 0 to {} or Unix seconds from {} onward",
                tip.height,
                genesis.timestamp
            );
            Target::Time(time)
        }
        target => target,
    };
    let future = match target {
        Target::Height(height) => height > tip.height,
        Target::Time(time) => time > tip.timestamp,
        _ => false,
    };
    if future {
        return Ok(Outcome {
            block: tip.clone(),
            end: tip.clone(),
            tip: tip.height,
            source: source.source_id().into(),
            note: "No blocks available yet for this target. Press l for the latest block.".into(),
            future: true,
        });
    }
    let (block, note) = match target {
        Target::Height(height) => (
            source.block_at(height).await?,
            format!("Seek to block {height}"),
        ),
        Target::Latest => (tip.clone(), format!("Seek to latest block {}", tip.height)),
        Target::Time(time) => {
            let found = block_time::locate(&mut source, time, tip.clone()).await?;
            let requested = Utc
                .timestamp_opt(time, 0)
                .single()
                .context("Time is out of range")?;
            let matched = Utc
                .timestamp_opt(found.block.timestamp, 0)
                .single()
                .context("Block time is out of range")?;
            let note = format!(
                "Requested {} · matched {} · difference {:+}s · {}",
                requested.to_rfc3339(),
                matched.to_rfc3339(),
                found.block.timestamp - time,
                found.description()
            );
            (found.block, note)
        }
        Target::Number(_) => unreachable!(),
    };
    let end_time = block.timestamp.saturating_add((span.seconds() / 2) as i64);
    progress("Locating centered chart range… Esc cancels");
    let end = if end_time >= tip.timestamp {
        tip.clone()
    } else {
        let found = block_time::locate(&mut source, end_time, tip.clone()).await?;
        if found.block.height >= block.height {
            found.block
        } else {
            block.clone()
        }
    };
    for checked in [&tip, &block, &end] {
        source.check(checked).await?;
    }
    Ok(Outcome {
        block,
        end,
        tip: tip.height,
        source: source.source_id().into(),
        note,
        future: false,
    })
}

fn loaded(view: &View, target: Target) -> Option<(String, String)> {
    let page = view.page.as_ref()?;
    if let Target::Number(number) = target
        && number > page.tip
    {
        return loaded(view, Target::Time(i64::try_from(number).ok()?));
    }
    let sample = match target {
        Target::Height(height) | Target::Number(height) if height <= page.tip => {
            page.samples.iter().find(|sample| sample.height == height)?
        }
        Target::Time(time) => {
            let low = page.samples.iter().map(|sample| sample.time).min()? as i64;
            let high = page.samples.iter().map(|sample| sample.time).max()? as i64;
            if !(low..=high).contains(&time) {
                return None;
            }
            page.samples.iter().min_by_key(|sample| {
                (
                    (sample.time as i64).abs_diff(time),
                    sample.time,
                    sample.height,
                )
            })?
        }
        _ => return None,
    };
    let note = if let Target::Time(time) = target {
        format!(
            "Requested {} · matched {} · difference {:+}s · nearest of {} loaded blocks",
            DateTime::<Utc>::from_timestamp(time, 0)?.to_rfc3339(),
            DateTime::<Utc>::from_timestamp(sample.time as i64, 0)?.to_rfc3339(),
            sample.time as i64 - time,
            page.samples.len()
        )
    } else {
        format!("Seek to block {} · loaded range", sample.height)
    };
    Some((sample.hash.clone(), note))
}

pub(in crate::app) fn prompt_active(state: &AppState) -> bool {
    matches!(&state.screen, Screen::Charts(view) if view.seek.is_some())
}

pub(in crate::app) fn paste_prompt(state: &mut AppState, text: &str) {
    if let Screen::Charts(view) = &mut state.screen
        && !view.loading
        && let Some(prompt) = &mut view.seek
    {
        prompt.input.push_str(text);
        prompt.error = None;
        prompt.future = false;
    }
}

pub(in crate::app) fn prompt_key(state: &mut AppState, key: event::KeyEvent) -> bool {
    let Screen::Charts(view) = &mut state.screen else {
        return false;
    };
    let Some(prompt) = &mut view.seek else {
        return false;
    };
    match key.code {
        KeyCode::Esc => {
            let resume = prompt.resume || view.page.is_none();
            view.seek = None;
            view.seek_action = None;
            view.request = next_request();
            view.loading = resume;
        }
        KeyCode::Enter | KeyCode::Char('l') if key.code == KeyCode::Enter || prompt.future => {
            if view.loading {
                return true;
            }
            let target = if key.code == KeyCode::Char('l') {
                Ok(Target::Latest)
            } else {
                parse(&prompt.input, prompt.now)
            };
            match target {
                Ok(target) => {
                    let resume = view.seek.as_ref().is_some_and(|prompt| prompt.resume);
                    if let Some((hash, note)) = loaded(view, target) {
                        let needs_pool = view.metric == Metric::FeePool
                            && view.page.as_ref().is_some_and(|page| {
                                page.samples
                                    .iter()
                                    .any(|sample| sample.hash == hash && !sample.pool_requested)
                            });
                        view.selected = Some(hash);
                        view.seek_note = Some(note);
                        view.seek = None;
                        view.loading = resume || needs_pool;
                        view.forward = false;
                        view.request = next_request();
                    } else {
                        view.seek_action = Some(target);
                        view.loading = true;
                        view.request = next_request();
                        view.error = None;
                        let prompt = view.seek.as_mut().unwrap();
                        prompt.error = None;
                        prompt.progress = "Locating requested block… Esc cancels".into();
                    }
                }
                Err(error) => prompt.error = Some(error.to_string()),
            }
        }
        KeyCode::Backspace | KeyCode::Delete if !view.loading => {
            prompt.input.pop();
            prompt.error = None;
            prompt.future = false;
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) && !view.loading => {
            prompt.input.clear();
            prompt.error = None;
            prompt.future = false;
        }
        KeyCode::Char(ch)
            if !view.loading
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            prompt.input.push(ch);
            prompt.error = None;
            prompt.future = false;
        }
        KeyCode::F(_) => return false,
        _ => {}
    }
    true
}

pub(in crate::app) fn apply(
    state: &mut AppState,
    request: u64,
    result: std::result::Result<Outcome, String>,
) {
    let Screen::Charts(view) = &mut state.screen else {
        return;
    };
    if view.request != request || view.seek.is_none() {
        return;
    }
    view.loading = false;
    view.seek_action = None;
    match result {
        Err(error) => view.seek.as_mut().unwrap().error = Some(error),
        Ok(result) if result.future => {
            let prompt = view.seek.as_mut().unwrap();
            prompt.error = Some(result.note);
            prompt.future = true;
        }
        Ok(result) => {
            view.selected = Some(result.block.hash.clone());
            view.end = Some(result.end.height);
            view.anchor = Some(result.end.hash);
            view.tip = Some(result.tip);
            view.focus = Some(result.block);
            view.seek_source = Some(result.source);
            view.seek_note = Some(result.note);
            view.seek = None;
            view.request = next_request();
            view.refresh = false;
            view.forward = false;
            view.loading = true;
        }
    }
}

pub(in crate::app) fn progress(state: &mut AppState, request: u64, message: String) {
    if let Screen::Charts(view) = &mut state.screen
        && view.request == request
        && let Some(prompt) = &mut view.seek
    {
        prompt.progress = message;
    }
}

pub(super) fn render(frame: &mut Frame<'_>, theme: &Theme, view: &View, area: Rect) {
    let Some(prompt) = &view.seek else { return };
    let width = area.width.saturating_sub(4).min(110);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + area.height.saturating_sub(8) / 2,
        width,
        8.min(area.height),
    );
    let preview = match parse(&prompt.input, prompt.now) {
        Ok(Target::Height(height)) => format!("Block {height}"),
        Ok(Target::Number(number))
            if number
                <= view
                    .page
                    .as_ref()
                    .map_or(view.tip.unwrap_or(0), |page| page.tip) =>
        {
            format!("Block {number}")
        }
        Ok(Target::Time(time)) => DateTime::<Utc>::from_timestamp(time, 0).map_or_else(
            || "Time out of range".into(),
            |time| format!("Date/time {}", time.to_rfc3339()),
        ),
        Ok(Target::Number(number)) => i64::try_from(number)
            .ok()
            .and_then(|time| DateTime::<Utc>::from_timestamp(time, 0))
            .map_or_else(
                || "Number out of range".into(),
                |time| format!("Unix time {} · range checked on Enter", time.to_rfc3339()),
            ),
        _ => "Enter a block height, Unix seconds, or date/time (UTC by default)".into(),
    };
    let lines = vec![
        Line::styled(format!("> {}", prompt.input), theme.focused()),
        Line::from(preview),
        Line::styled(
            if view.loading {
                prompt.progress.clone()
            } else {
                prompt.error.clone().unwrap_or_default()
            },
            theme.warning(),
        ),
        Line::styled(
            "Optional overrides height:4000000 or unix:1793491200",
            theme.muted(),
        ),
    ];
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme.base())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Go to block or date/time ")
                    .border_style(theme.focused()),
            )
            .wrap(Wrap { trim: false }),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_input_preserves_numeric_inference_and_explicit_overrides() {
        let now = DateTime::from_timestamp(1_793_491_200, 0).unwrap();
        assert_eq!(parse("4000000", now).unwrap(), Target::Number(4_000_000));
        assert_eq!(
            parse("00000004000000", now).unwrap(),
            Target::Number(4_000_000)
        );
        assert_eq!(
            parse("1793491200", now).unwrap(),
            Target::Number(1_793_491_200)
        );
        assert_eq!(parse("#0", now).unwrap(), Target::Height(0));
        assert_eq!(
            parse("height:4000000", now).unwrap(),
            Target::Height(4_000_000)
        );
        assert_eq!(parse("unix:0", now).unwrap(), Target::Time(0));
        assert_eq!(
            parse("2026-11-01", now).unwrap(),
            Target::Time(1_793_491_200)
        );
        assert_eq!(
            parse("November 1 2026 14:30 UTC", now).unwrap(),
            Target::Time(1_793_543_400)
        );
        for input in ["", "height:-1", "unix:bad", "1793491200000", "bad date"] {
            assert!(parse(input, now).is_err(), "{input}");
        }
    }

    #[test]
    fn cancelled_seek_rejects_delayed_results() {
        let mut state = crate::app::tests::regression_state();
        state.screen = screen(&state);
        let Screen::Charts(view) = &mut state.screen else {
            unreachable!()
        };
        view.page = Some(Page {
            first: 0,
            last: 1,
            tip: 10,
            samples: Vec::new(),
            source: "local".into(),
            status: "Retained observations".into(),
            span: TimeSpan::Hours2,
            complete: true,
            pool: None,
        });
        view.seek = Some(Prompt::new(false));
        view.loading = true;
        let request = view.request;
        assert!(prompt_key(
            &mut state,
            event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)
        ));
        apply(&mut state, request, Err("late lookup failure".into()));
        progress(&mut state, request, "late progress".into());
        let Screen::Charts(view) = &state.screen else {
            unreachable!()
        };
        assert!(view.seek.is_none());
        assert!(!view.loading);
        assert!(view.error.is_none());
        assert_eq!(view.page.as_ref().unwrap().last, 1);
    }
}
