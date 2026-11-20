//! Presentation-only height/date switching. Rendering queues bounded lookups;
//! no RPC is performed on the terminal thread and JSON values stay untouched.
use crate::{block_forecast::Calibration, display_time::TimeDisplay};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default)]
pub struct HeightDisplay {
    pub dates: bool,
    pub scope: String,
    pub source: String,
    pub timestamps: BTreeMap<u64, Option<u64>>,
    pub pending: BTreeSet<u64>,
    pub calibration: Option<Calibration>,
    pub foreign: BTreeMap<String, std::cell::RefCell<HeightDisplay>>,
}
impl HeightDisplay {
    pub fn format(&mut self, height: u64, known: Option<u64>, clock: TimeDisplay) -> String {
        if !self.dates {
            return height.to_string();
        }
        if let Some(time) = known.filter(|t| *t > 0) {
            return clock.format(time);
        }
        if let Some(time) = self.timestamps.get(&height) {
            return time
                .map(|t| clock.format(t))
                .unwrap_or_else(|| format!("{height} (date unavailable)"));
        }
        if let Some(c) = &self.calibration
            && height > c.tip.height
            && let Ok(time) = c.time_at_height(height)
        {
            return format!("est. {}", clock.format(time as u64));
        }
        if self.pending.len() < 256 {
            self.pending.insert(height);
        }
        format!("{height} (loading date)")
    }
    pub fn reset(&mut self, scope: String) {
        *self = Self {
            dates: self.dates,
            scope,
            ..Self::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block_time::TimeBlock;
    #[test]
    fn past_dates_are_observed_only_and_future_dates_are_marked_and_bounded() {
        let mut d = HeightDisplay::default();
        assert_eq!(d.format(50, Some(1000), TimeDisplay::Utc), "50");
        d.dates = true;
        assert_eq!(
            d.format(50, Some(1000), TimeDisplay::Utc),
            "1970-01-01T00:16:40Z"
        );
        let block = |height, timestamp| TimeBlock {
            height,
            timestamp,
            hash: format!("{height:064x}"),
            parent: None,
        };
        d.calibration = Some(
            Calibration::new(
                block(1, 1000),
                block(1001, 61000),
                61000,
                "s".into(),
                "c".into(),
            )
            .unwrap(),
        );
        assert!(d.format(1002, None, TimeDisplay::Utc).starts_with("est. "));
        assert!(
            d.format(500, None, TimeDisplay::Utc)
                .contains("loading date")
        );
        d.timestamps.insert(500, Some(30000));
        assert!(!d.format(500, None, TimeDisplay::Utc).contains("est."));
        for h in 0..10000 {
            d.format(h, None, TimeDisplay::Utc);
        }
        assert!(d.pending.len() <= 256);
        d.reset("other-chain".into());
        assert!(d.dates && d.timestamps.is_empty() && d.calibration.is_none());
    }
}
