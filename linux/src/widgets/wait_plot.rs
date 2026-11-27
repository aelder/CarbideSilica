//! Shared distribution of sampled waits for the dashboard and the Charts page.
use std::collections::BTreeMap;

use gtk::gsk;

use crate::theme::draw;

#[derive(Debug)]
enum Mark {
    Dot { x: f32, y: f32, radius: f32 },
    Bar { x: f32, y: f32, width: f32, height: f32 },
}

/// The bottom is the lower edge of the marks, just above the time axis.
pub struct WaitPlot {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
    pub diameter: f32,
    pub pitch: f32,
}

impl WaitPlot {
    fn marks(&self, samples: impl Iterator<Item = (f32, bool)>) -> Vec<(Mark, bool)> {
        let height = self.bottom - self.top;
        let width = self.right - self.left;
        if height <= 0.0 || width < self.diameter {
            return Vec::new();
        }
        let mut bins = BTreeMap::<i32, (usize, usize)>::new();
        let last = ((width / self.pitch).ceil() as i32 - 1).max(0);
        for (x, shorter) in samples {
            let index = (((x - self.left) / self.pitch) as i32).clamp(0, last);
            let (total, passed) = bins.entry(index).or_default();
            *total += 1;
            *passed += usize::from(shorter);
        }
        let mut marks = Vec::new();
        let Some(peak) = bins.values().map(|&(total, _)| total).max() else { return marks };
        let radius = self.diameter / 2.0;
        let fits = (peak - 1) as f32 * self.pitch + self.diameter <= height;
        for (index, (total, passed)) in bins {
            let x = (self.left + (index as f32 + 0.5) * self.pitch).min(self.right - radius);
            if fits {
                for level in 0..total {
                    let y = self.bottom - radius - level as f32 * self.pitch;
                    marks.push((Mark::Dot { x, y, radius }, level < passed));
                }
            } else {
                // One vertical scale across all bins preserves their relative frequencies.
                let y_at = |count: usize| self.bottom - height * (count as f32 / peak as f32);
                for (low, high, shorter) in [(0, passed, true), (passed, total, false)] {
                    if high > low {
                        marks.push((Mark::Bar {
                            x: x - radius, y: y_at(high), width: self.diameter,
                            height: y_at(low) - y_at(high),
                        }, shorter));
                    }
                }
            }
        }
        marks
    }

    /// Each of at most 100 samples represents an equal share of the sorted waits.
    pub fn paths<T: Copy + Into<f64>>(
        &self, waits: &[T], current: Option<f64>, position: impl Fn(f64) -> f32,
    ) -> (gsk::Path, gsk::Path) {
        let count = waits.len().min(100);
        let samples = (0..count).map(|index| {
            let rank = (2 * index + 1) * waits.len() / (2 * count);
            let value = waits[rank].into();
            (position(value), current.is_none_or(|w| value < w))
        });
        let passed = gsk::PathBuilder::new();
        let ahead = gsk::PathBuilder::new();
        for (mark, shorter) in self.marks(samples) {
            let path = if shorter { &passed } else { &ahead };
            match mark {
                Mark::Dot { x, y, radius } => path.add_circle(&draw::point(x, y), radius),
                Mark::Bar { x, y, width, height } => path.add_rect(&draw::rect(x, y, width, height)),
            }
        }
        (passed.to_path(), ahead.to_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLOT: WaitPlot = WaitPlot {
        left: 0.0, right: 52.0, top: 10.0, bottom: 50.0, diameter: 5.0, pitch: 7.0,
    };

    #[test]
    fn dense_column_stays_in_bounds_and_preserves_both_shares() {
        let samples = (0..100).map(|i| (52.0, i < 75)).chain([(4.0, true)]);
        let marks = PLOT.marks(samples);
        assert_eq!(marks.len(), 3);
        for (mark, _) in &marks {
            let Mark::Bar { x, y, width, height } = *mark else { panic!("expected a bar") };
            assert!(x >= PLOT.left && x + width <= PLOT.right);
            assert!(y >= PLOT.top && y + height <= PLOT.bottom && height > 0.0);
        }
        assert!(matches!(marks[1], (Mark::Bar { y: 20.0, height: 30.0, .. }, true)));
        assert!(matches!(marks[2], (Mark::Bar { y: 10.0, height: 10.0, .. }, false)));
    }

    #[test]
    fn sparse_column_keeps_the_individual_dots() {
        let marks = PLOT.marks([(8.0, true), (8.0, false)].into_iter());
        assert!(matches!(marks.as_slice(), [
            (Mark::Dot { y: 47.5, .. }, true), (Mark::Dot { y: 40.5, .. }, false)
        ]));
    }
}
