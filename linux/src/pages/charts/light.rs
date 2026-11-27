//! Light on the chart panels (`ChartLight` in ChartStack.swift). It shows parts of a picture
//! of the panels that has more light. There are three lights, and each has its own mask, so
//! that one does not change another:
//!
//! - The scan: a quiet front that goes from the right to the left in 10 seconds. The next
//!   front starts at the right at the moment when the front gets to the left.
//! - The pointer: the stretch that the pointer moved across keeps its light for a short time
//!   and then loses it, as a phosphor does. There is no light ahead of the pointer.
//! - The wave of a click: two fronts go out from the selected block to the two edges. Each is
//!   strong at the start, loses light with the square of its distance, and loses speed.
//!
//! The Swift app gives this to Core Animation. Here it is one small canvas above the panels
//! that redraws on the frame clock: each frame draws the two pictures (textures made once
//! for each range and width) through gradient masks. The panels below do not redraw.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gdk, glib, gsk};

use crate::theme::draw;
use crate::widgets::canvas::Canvas;

/// The length of the light behind a front.
const TAIL: f32 = 300.0;
/// The time that a place keeps its light after the pointer leaves it.
const AFTERGLOW: f64 = 0.9;
/// The number of marks of light that the pointer can leave at one time.
const MARKS: usize = 48;
/// The time of one scan across the plot.
const SCAN: f64 = 10.0;

#[derive(Clone, Copy)]
struct Mark {
    x0: f32,
    x1: f32,
    /// Frame time in seconds.
    at: f64,
}

#[derive(Clone, Copy)]
struct Wave {
    origin: f32,
    at: f64,
}

#[derive(Default)]
struct LightState {
    lit: Option<gdk::Texture>,
    hot: Option<gdk::Texture>,
    /// The size of the pictures, in logical pixels.
    size: (f32, f32),
    left: f32,
    right: f32,
    /// When the scan started, in seconds of the frame clock.
    scan_start: Option<f64>,
    marks: Vec<Mark>,
    next_mark: usize,
    last_pointer: Option<f32>,
    wave: Option<Wave>,
}

pub struct ChartLight {
    pub canvas: Canvas,
    state: Rc<RefCell<LightState>>,
}

fn still() -> bool {
    !gtk::Settings::default().map(|s| s.is_gtk_enable_animations()).unwrap_or(true)
}

fn seconds(widget: &impl IsA<gtk::Widget>) -> f64 {
    widget.frame_clock().map(|c| c.frame_time() as f64 / 1e6).unwrap_or_else(|| glib::monotonic_time() as f64 / 1e6)
}

/// The alpha of a gradient with `stops` at `u` (0 to 1).
fn ramp(stops: &[(f32, f32)], u: f32) -> f32 {
    if u <= stops[0].0 {
        return stops[0].1;
    }
    for pair in stops.windows(2) {
        let ((a, va), (b, vb)) = (pair[0], pair[1]);
        if u <= b {
            return va + (vb - va) * ((u - a) / (b - a).max(1e-6));
        }
    }
    stops[stops.len() - 1].1
}

/// A band of a gradient of black, `stops` of (place, alpha), with its anchor at `x`.
fn band(snapshot: &gtk::Snapshot, x: f32, anchor: f32, height: f32, stops: &[(f32, f32)], opacity: f32) {
    if opacity <= 0.0 {
        return;
    }
    let width = TAIL / 0.94;
    let left = x - anchor * width;
    let stops: Vec<gsk::ColorStop> =
        stops.iter().map(|(at, alpha)| gsk::ColorStop::new(*at, gdk::RGBA::new(0.0, 0.0, 0.0, alpha * opacity))).collect();
    snapshot.append_linear_gradient(
        &draw::rect(left, 0.0, width, height),
        &draw::point(left, 0.0),
        &draw::point(left + width, 0.0),
        &stops,
    );
}

const SCAN_STOPS: [(f32, f32); 4] = [(0.0, 0.0), (0.06, 1.0), (0.45, 0.32), (1.0, 0.0)];
const LEFT_STOPS: [(f32, f32); 4] = [(0.0, 0.0), (0.06, 1.0), (0.4, 0.35), (1.0, 0.0)];
const RIGHT_STOPS: [(f32, f32); 4] = [(0.0, 0.0), (0.6, 0.35), (0.94, 1.0), (1.0, 0.0)];

/// The place and the light of one front of a wave at `elapsed` seconds, or None when it
/// has gone.
fn front(origin: f32, target: f32, width: f32, elapsed: f64) -> Option<(f32, f32)> {
    let distance = target - origin;
    let duration = 0.9 + 1.5 * (distance.abs() / (width + TAIL)) as f64;
    if elapsed >= duration || elapsed < 0.0 {
        return None;
    }
    let time = elapsed / duration;
    // Quick at the start and slower to the end.
    let gone = 1.0 - (1.0 - time).powf(2.4);
    let travelled = distance * gone as f32;
    let half = (width * 0.16).max(60.0);
    let light = 1.0 / (1.0 + (travelled.abs() / half).powi(2));
    Some((origin + travelled, light))
}

/// The opacity of a mark of the pointer: a strong light at the moment of the pointer, which
/// goes down quickly to the quiet afterglow and then to nothing.
fn glow(elapsed: f64) -> f32 {
    let t = (elapsed / AFTERGLOW) as f32;
    if !(0.0..1.0).contains(&t) {
        return 0.0;
    }
    ramp(&[(0.0, 1.0), (0.12, 0.34), (0.35, 0.11), (1.0, 0.0)], t)
}

impl ChartLight {
    pub fn new() -> Rc<Self> {
        let state: Rc<RefCell<LightState>> = Rc::default();
        let s = state.clone();
        let canvas = Canvas::new(0, 0, move |canvas, snapshot, _w, h| {
            let state = s.borrow();
            let now = seconds(canvas);
            let width = (state.right - state.left).max(0.0);
            if width <= 0.0 {
                return;
            }
            let picture = draw::rect(0.0, 0.0, state.size.0, state.size.1);
            snapshot.push_clip(&draw::rect(state.left, 0.0, width, h));
            snapshot.translate(&draw::point(state.left, 0.0));
            let shifted = draw::rect(-state.left, 0.0, picture.width(), picture.height());
            // The scan: two fronts, one plot width apart.
            if let (Some(lit), Some(start), false) = (&state.lit, state.scan_start, still()) {
                let phase = (((now - start) / SCAN).fract()) as f32;
                snapshot.push_mask(gsk::MaskMode::Alpha);
                for number in 0..2 {
                    let x = width - number as f32 * width - phase * width;
                    band(snapshot, x, 0.06, h, &SCAN_STOPS, 1.0);
                }
                snapshot.pop();
                snapshot.append_texture(lit, &shifted);
                snapshot.pop();
            }
            let hot = state.hot.as_ref().or(state.lit.as_ref());
            if let Some(hot) = hot {
                // The marks of the pointer.
                let shown: Vec<(Mark, f32)> =
                    state.marks.iter().map(|m| (*m, glow(now - m.at))).filter(|(_, o)| *o > 0.0).collect();
                if !shown.is_empty() {
                    snapshot.push_mask(gsk::MaskMode::Alpha);
                    for (mark, opacity) in shown {
                        draw::fill_rect(snapshot, mark.x0, 0.0, (mark.x1 - mark.x0).max(0.5), h, crate::theme::Color::rgb(0.0, 0.0, 0.0).opacity(opacity as f64));
                    }
                    snapshot.pop();
                    snapshot.append_texture(hot, &shifted);
                    snapshot.pop();
                }
                // The wave of a click.
                if let Some(wave) = state.wave {
                    let elapsed = now - wave.at;
                    let left = front(wave.origin, -TAIL, width, elapsed);
                    let right = front(wave.origin, width + TAIL, width, elapsed);
                    if left.is_some() || right.is_some() {
                        snapshot.push_mask(gsk::MaskMode::Alpha);
                        if let Some((x, light)) = left {
                            band(snapshot, x, 0.06, h, &LEFT_STOPS, light);
                        }
                        if let Some((x, light)) = right {
                            band(snapshot, x, 0.94, h, &RIGHT_STOPS, light);
                        }
                        snapshot.pop();
                        snapshot.append_texture(hot, &shifted);
                        snapshot.pop();
                    }
                }
            }
            snapshot.pop();
        });
        canvas.set_can_target(false);
        canvas.set_hexpand(true);
        canvas.set_vexpand(true);
        let light = Rc::new(Self { canvas, state });
        // The frame clock moves the light. The canvas redraws only while a light moves.
        let weak = Rc::downgrade(&light.state);
        light.canvas.add_tick_callback(move |canvas, _| {
            let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
            let mut state = state.borrow_mut();
            let now = seconds(canvas);
            let mut moving = false;
            if state.lit.is_some() && !still() {
                state.scan_start.get_or_insert(now);
                moving = true;
            }
            if state.marks.iter().any(|m| now - m.at < AFTERGLOW) {
                moving = true;
            }
            if let Some(wave) = state.wave {
                let width = state.right - state.left;
                let elapsed = now - wave.at;
                if front(wave.origin, -TAIL, width, elapsed).is_some() || front(wave.origin, width + TAIL, width, elapsed).is_some() {
                    moving = true;
                } else {
                    state.wave = None;
                    moving = true;
                }
            }
            if moving {
                canvas.queue_draw();
            }
            glib::ControlFlow::Continue
        });
        light
    }

    /// New pictures of the panels, for a new range or width. `size` is the size of the
    /// pictures; `left` and `right` are the edges of the plot.
    pub fn set_pictures(&self, lit: Option<gdk::Texture>, hot: Option<gdk::Texture>, size: (f32, f32), left: f32, right: f32) {
        let mut state = self.state.borrow_mut();
        state.lit = lit;
        state.hot = hot;
        state.size = size;
        state.left = left;
        state.right = right;
        drop(state);
        self.canvas.queue_draw();
    }

    /// The place of the block under the pointer, in the coordinates of the panels, or None.
    pub fn pointer(&self, x: Option<f32>) {
        let now = seconds(&self.canvas);
        let mut state = self.state.borrow_mut();
        let Some(x) = x.filter(|_| state.lit.is_some()) else {
            state.last_pointer = None;
            return;
        };
        let at = x - state.left;
        if let Some(before) = state.last_pointer
            && (before - at).abs() >= 0.5
        {
            let mark = Mark { x0: before.min(at), x1: before.max(at), at: now };
            if state.marks.len() < MARKS {
                state.marks.push(mark);
            } else {
                let index = state.next_mark;
                state.marks[index] = mark;
            }
            state.next_mark = (state.next_mark + 1) % MARKS;
        }
        state.last_pointer = Some(at);
        drop(state);
        self.canvas.queue_draw();
    }

    /// A click: two fronts go out from `x`, the place of the selected block.
    pub fn wave(&self, x: f32) {
        if still() {
            return;
        }
        let now = seconds(&self.canvas);
        let mut state = self.state.borrow_mut();
        if state.lit.is_none() {
            return;
        }
        state.wave = Some(Wave { origin: x - state.left, at: now });
        drop(state);
        self.canvas.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramps_and_fronts() {
        assert_eq!(ramp(&SCAN_STOPS, 0.0), 0.0);
        assert_eq!(ramp(&SCAN_STOPS, 0.06), 1.0);
        assert!((ramp(&SCAN_STOPS, 0.45) - 0.32).abs() < 1e-6);
        assert_eq!(ramp(&SCAN_STOPS, 1.0), 0.0);
        // A wave starts at its origin with full light and ends.
        let (x, light) = front(100.0, -TAIL, 1000.0, 0.0).unwrap();
        assert_eq!((x, light), (100.0, 1.0));
        let (x, light) = front(100.0, -TAIL, 1000.0, 0.5).unwrap();
        assert!(x < 100.0 && light < 1.0);
        assert!(front(100.0, -TAIL, 1000.0, 5.0).is_none());
        assert_eq!(glow(0.0), 1.0);
        assert_eq!(glow(1.0), 0.0);
    }
}
