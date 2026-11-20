//! Active-chain, source-generation and reorg-guarded date hydration.
use super::*;
use crate::block_time::{RpcTimeSource, TimeSource};

#[derive(Debug)]
struct Update {
    source: String,
    dates: Vec<(u64, Option<u64>)>,
    calibration: Option<crate::block_forecast::Calibration>,
}
pub(super) struct Driver {
    scope: String,
    task: Option<tokio::task::JoinHandle<()>>,
    receiver: Option<UnboundedReceiver<Update>>,
    tip: Option<u64>,
}
impl Drop for Driver {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
impl Driver {
    pub(super) fn new() -> Self {
        Self {
            scope: String::new(),
            task: None,
            receiver: None,
            tip: None,
        }
    }
    fn drive(
        &mut self,
        cell: &std::cell::RefCell<crate::height_display::HeightDisplay>,
        config: &Config,
        tip: Option<u64>,
    ) -> bool {
        let scope = format!(
            "{}:{}:{}:{}:{}:{:?}",
            config.chain_cache_key(),
            config.daemon.host,
            config.daemon.port,
            config.daemon.control.generation(),
            config.daemon.control.branch_epoch.load(Ordering::Acquire),
            config
                .daemon
                .api_pool
                .endpoints
                .iter()
                .map(|e| e.control.generation())
                .collect::<Vec<_>>()
        );
        let mut display = cell.borrow_mut();
        if scope != self.scope || display.scope != scope {
            if let Some(task) = self.task.take() {
                task.abort();
            }
            self.receiver = None;
            self.scope = scope.clone();
            self.tip = tip;
            display.reset(scope);
        }
        if !display.dates {
            if let Some(task) = self.task.take() {
                task.abort();
            }
            self.receiver = None;
            display.pending.clear();
            return false;
        }
        if self.tip != tip
            || display
                .calibration
                .as_ref()
                .is_some_and(|c| current_unix_timestamp() - c.observed_at > 3600)
        {
            self.tip = tip;
            display.calibration = None;
            // A future/unavailable height may have become historical since last observation.
            display.timestamps.retain(|_, time| time.is_some());
        }
        let mut changed = false;
        if let Some(update) = self.receiver.as_mut().and_then(|rx| rx.try_recv().ok()) {
            if !update.source.is_empty() && display.source != update.source {
                display.timestamps.clear();
                display.calibration = None;
                display.source = update.source;
            }
            display.timestamps.extend(update.dates);
            if let Some(c) = update
                .calibration
                .filter(|c| self.tip.is_none_or(|h| c.tip.height >= h))
            {
                display.calibration = Some(c);
            }
            self.task = None;
            self.receiver = None;
            changed = true;
        }
        if self.task.is_none() && !display.pending.is_empty() {
            let heights: Vec<_> = std::mem::take(&mut display.pending)
                .into_iter()
                .filter(|h| !display.timestamps.contains_key(h))
                .filter(|h| {
                    display
                        .calibration
                        .as_ref()
                        .is_none_or(|c| *h <= c.tip.height)
                })
                .take(8)
                .collect();
            if !heights.is_empty() {
                let config = config.noninteractive();
                let (tx, rx) = unbounded_channel();
                self.receiver = Some(rx);
                self.task = Some(tokio::spawn(async move {
                    let fallback = heights.iter().map(|h| (*h, None)).collect();
                    let update = load(&config, &heights).await.unwrap_or(Update {
                        source: String::new(),
                        dates: fallback,
                        calibration: None,
                    });
                    let _ = tx.send(update);
                }));
            }
        }
        changed
    }
}

pub(super) struct Manager {
    active: Driver,
    foreign: BTreeMap<String, Driver>,
}
impl Manager {
    pub(super) fn new() -> Self {
        Self {
            active: Driver::new(),
            foreign: BTreeMap::new(),
        }
    }
    pub(super) fn drive(
        &mut self,
        theme: &Theme,
        config: &Config,
        tip: Option<u64>,
        configs: &[Config],
        tips: &HashMap<String, u64>,
    ) -> bool {
        let mut changed = self.active.drive(&theme.heights, config, tip);
        let display = theme.heights.borrow();
        self.foreign
            .retain(|system, _| display.foreign.contains_key(system));
        for (system, cell) in &display.foreign {
            cell.borrow_mut().dates = display.dates;
            if let Some(config) = configs.iter().find(|c| c.chain_cache_key() == *system) {
                changed |= self
                    .foreign
                    .entry(system.clone())
                    .or_insert_with(Driver::new)
                    .drive(cell, config, tips.get(system).copied());
            } else {
                let mut foreign = cell.borrow_mut();
                let pending = std::mem::take(&mut foreign.pending);
                foreign
                    .timestamps
                    .extend(pending.into_iter().map(|h| (h, None)));
            }
        }
        changed
    }
}

async fn load(config: &Config, heights: &[u64]) -> Result<Update> {
    let (mut source, tip, info) = RpcTimeSource::open(config).await?;
    let mut update = Update {
        source: source.source_id().into(),
        dates: vec![],
        calibration: None,
    };
    for h in heights.iter().copied().filter(|h| *h <= tip.height) {
        let timestamp = match source.block_at(h).await {
            Ok(block) if source.check(&block).await.is_ok() => u64::try_from(block.timestamp).ok(),
            _ => None,
        };
        update.dates.push((h, timestamp));
    }
    if heights.iter().any(|h| *h > tip.height) {
        update.calibration =
            crate::block_forecast::from_source(&mut source, config, tip.clone(), &info)
                .await
                .ok();
        if update.calibration.is_none() {
            update.dates.extend(
                heights
                    .iter()
                    .filter(|h| **h > tip.height)
                    .map(|h| (*h, None)),
            );
        }
    }
    source.check(&tip).await?;
    Ok(update)
}

pub(super) fn render(
    frame: &mut Frame<'_>,
    theme: &Theme,
    estimate: &crate::block_forecast::Estimate,
    area: Rect,
) {
    use crate::block_forecast::Target;
    let c = &estimate.calibration;
    let direction = match estimate.target {
        Target::Height(_) => format!(
            "Block {} — estimated {}",
            estimate.height,
            theme.format_time(estimate.timestamp as u64)
        ),
        Target::Time(_) => format!(
            "{} — estimated height {}",
            theme.format_time(estimate.timestamp as u64),
            estimate.height
        ),
    };
    let lines = vec![
        Line::styled("Future block estimate", theme.focused()),
        Line::from(theme.value(
            "estimate",
            direction,
            theme.warning().add_modifier(Modifier::BOLD),
        )),
        Line::default(),
        Line::styled("Observed production", theme.focused()),
        identity_property_line(
            theme,
            "Average",
            format!("{:.6} seconds per block", c.seconds_per_block()),
        ),
        identity_property_line(
            theme,
            "Calibration",
            format!(
                "#{} ({}) to #{} ({})",
                c.start.height,
                theme.format_time(c.start.timestamp as u64),
                c.tip.height,
                theme.format_time(c.tip.timestamp as u64)
            ),
        ),
        identity_property_line(
            theme,
            "Observed tip",
            format!(
                "#{} at {}",
                c.tip.height,
                theme.format_time(c.observed_at as u64)
            ),
        ),
        identity_property_line(theme, "Chain", c.system.to_string()),
        identity_property_line(theme, "Source", c.source_label.clone()),
        Line::default(),
        Line::styled(
            "This estimate assumes ordinary block production continues. Outages, changes in participation, and protocol updates can change the result.",
            theme.muted(),
        ),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme.base())
            .wrap(Wrap { trim: false }),
        detail_header_content(area),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_revisions_drop_queued_dates_before_they_can_reach_another_branch() {
        let config = super::super::tests::regression_config();
        let cell = std::cell::RefCell::new(crate::height_display::HeightDisplay::default());
        let mut driver = Driver::new();
        driver.drive(&cell, &config, Some(1000));
        cell.borrow_mut().dates = true;
        let (tx, rx) = unbounded_channel();
        driver.receiver = Some(rx);
        tx.send(Update {
            source: "old".into(),
            dates: vec![(1, Some(1000))],
            calibration: None,
        })
        .unwrap();
        config
            .daemon
            .control
            .branch_epoch
            .fetch_add(1, Ordering::AcqRel);
        driver.drive(&cell, &config, Some(1000));
        assert!(cell.borrow().timestamps.is_empty());
        assert!(driver.receiver.is_none());
    }
}
