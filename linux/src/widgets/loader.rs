//! The mark of the app for work in progress (`CircuitLoader.swift`): a pattern of circuit
//! traces in an octagon that draws itself from the middle column out, with a bright point
//! that goes from place to place, then clears and starts again.
//!
//! The pattern is made one time from a fixed start, so it is the same each time. Each
//! frame only sets the opacity and the dashes of the traces: the widget redraws itself on
//! the frame clock while it is on screen, and nothing else redraws with it. With
//! animations turned off in GTK's settings, the mark is the full pattern with no movement.

use std::rc::Rc;

use gtk::prelude::*;
use gtk::{gsk, glib};

use super::canvas::Canvas;
use crate::theme::{self, draw, font, Color};

/// One cycle: the pattern draws itself, stays, and goes out.
const CYCLE: f64 = 3.6;
const DRAWN: f64 = 2.3;
const COLORS: [Color; 3] = [
    Color::rgb(0.82, 0.93, 0.86),
    Color::rgb(0.5, 0.86, 0.62),
    Color::rgb(0.56, 0.74, 0.98),
];

/// A small generator with a fixed start, so that the pattern is the same each time.
struct Steps(u64);

impl Steps {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, limit: i64) -> i64 {
        (self.next() % (limit.max(1) as u64)) as i64
    }
}

struct Pattern {
    side: f32,
    step: f32,
    width: f32,
    /// One path for each band and colour, with the time that the band starts.
    traces: Vec<(f64, usize, gsk::Path)>,
    octagon: gsk::Path,
    /// The places of the bright point.
    places: Vec<(f32, f32)>,
}

fn pattern(side: f32) -> Pattern {
    let cells: i64 = if side < 24.0 { 5 } else if side < 48.0 { 9 } else { ((side / 4.5) as i64).min(30) };
    let step = side / cells as f32;
    let width = (side / 110.0).max(if side < 24.0 { 1.0 } else { 0.9 });
    let bands: usize = if side < 24.0 { 3 } else { 7 };
    let mut random = Steps(0x5EED_C0DE);
    let middle = cells as f32 / 2.0;
    // Polylines for each band and colour.
    let mut lines: Vec<Vec<Vec<Vec<(f32, f32)>>>> = vec![vec![Vec::new(); COLORS.len()]; bands];
    let band_of = |x: f32| -> usize { (((x - middle).abs() / middle * bands as f32) as usize).min(bands - 1) };
    let pt = |x: f32, y: f32| (x * step, y * step);
    let clamp = |v: f32| v.clamp(0.0, cells as f32);

    // Bundles of parallel lines from the middle to the four corners.
    if side >= 24.0 {
        for (dx, dy) in [(1.0f32, 1.0f32), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
            for lane in -1..=1 {
                let shift = lane as f32 * 0.8;
                let from = 1.2 + random.below(3) as f32;
                let to = middle * (0.75 + 0.25 * random.below(100) as f32 / 100.0);
                let band = band_of(middle + dx * from);
                lines[band][0].push(vec![
                    pt(middle + dx * from - dy * shift, middle + dy * from + dx * shift),
                    pt(middle + dx * to - dy * shift, middle + dy * to + dx * shift),
                ]);
            }
        }
    }

    // Traces: a run along the grid, then a turn, as the lines of a circuit board.
    let straight = [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)];
    let count = if side < 24.0 { 7 } else { (cells as f64 * 2.2) as i64 };
    for index in 0..count {
        let mut x = random.below(cells + 1) as f32;
        let mut y = random.below(cells + 1) as f32;
        let (band, color) = (band_of(x), index as usize % COLORS.len());
        let mut line = vec![pt(x, y)];
        let mut direction = straight[random.below(4) as usize];
        let turns = 2 + random.below(3);
        for turn in 0..turns {
            let length = (2 + random.below((cells / 3).max(2))) as f32;
            x = clamp(x + direction.0 * length);
            y = clamp(y + direction.1 * length);
            line.push(pt(x, y));
            if turn == 0 && random.below(4) == 0 {
                // A short step at 45 degrees between two runs.
                let slant = if random.below(2) == 0 { 1.0 } else { -1.0 };
                x = clamp(x + 1.0);
                y = clamp(y + slant);
                line.push(pt(x, y));
            }
            // Turn a quarter.
            direction = if direction.0 == 0.0 {
                if random.below(2) == 0 { (1.0, 0.0) } else { (-1.0, 0.0) }
            } else if random.below(2) == 0 {
                (0.0, 1.0)
            } else {
                (0.0, -1.0)
            };
        }
        lines[band][color].push(line);
    }

    // Rows of short parallel lines, as the legs of a part.
    if side >= 24.0 {
        for index in 0..(cells / 2) {
            let x = random.below(cells - 3) as f32;
            let y = (1 + random.below(cells - 2)) as f32;
            let upright = random.below(2) == 0;
            let (band, color) = (band_of(x), (index as usize + 1) % COLORS.len());
            let legs = 4 + random.below(4);
            for leg in 0..legs {
                let offset = leg as f32 * 0.5;
                let line = if upright {
                    vec![pt(x + offset, y), pt(x + offset, y + 1.0)]
                } else {
                    vec![pt(y, x + offset), pt(y + 1.0, x + offset)]
                };
                lines[band][color].push(line);
            }
        }
    }

    let mut traces = Vec::new();
    for (band, colours) in lines.into_iter().enumerate() {
        let start = DRAWN * band as f64 / bands as f64 * 0.8;
        for (color, polylines) in colours.into_iter().enumerate() {
            if polylines.is_empty() {
                continue;
            }
            let builder = gsk::PathBuilder::new();
            for line in polylines {
                for (i, (px, py)) in line.into_iter().enumerate() {
                    if i == 0 { builder.move_to(px, py) } else { builder.line_to(px, py) }
                }
            }
            traces.push((start, color, builder.to_path()));
        }
    }

    let cut = side * 0.29;
    let octagon = gsk::PathBuilder::new();
    let corners = [
        (cut, 0.0), (side - cut, 0.0), (side, cut), (side, side - cut),
        (side - cut, side), (cut, side), (0.0, side - cut), (0.0, cut),
    ];
    for (i, (x, y)) in corners.iter().enumerate() {
        if i == 0 { octagon.move_to(*x, *y) } else { octagon.line_to(*x, *y) }
    }
    octagon.close();

    let places = (0..9)
        .map(|_| {
            let x = side * (0.12 + 0.76 * random.below(100) as f32 / 100.0);
            let y = side * (0.12 + 0.76 * random.below(100) as f32 / 100.0);
            (x, y)
        })
        .collect();
    Pattern { side, step, width, traces, octagon: octagon.to_path(), places }
}

/// A linear interpolation of keyframes (`times` in seconds of the cycle).
fn keyframes(t: f64, frames: &[(f64, f64)]) -> f64 {
    for pair in frames.windows(2) {
        let ((t0, v0), (t1, v1)) = (pair[0], pair[1]);
        if t >= t0 && t <= t1 {
            return if t1 > t0 { v0 + (v1 - v0) * (t - t0) / (t1 - t0) } else { v1 };
        }
    }
    frames.last().map(|f| f.1).unwrap_or(1.0)
}

fn animations_on() -> bool {
    gtk::Settings::default().map(|s| s.is_gtk_enable_animations()).unwrap_or(true)
}

/// The circuit mark of `size` logical pixels.
pub fn circuit(size: i32) -> Canvas {
    let pattern = Rc::new(pattern(size as f32));
    let started = Rc::new(std::cell::Cell::new(None::<i64>));
    let (p, s) = (pattern.clone(), started.clone());
    let canvas = Canvas::new(size, size, move |canvas, snapshot, _, _| {
        let still = !animations_on();
        let t = match (canvas.frame_clock(), still) {
            (Some(clock), false) => {
                let now = clock.frame_time();
                let begin = *s.get().get_or_insert(now);
                s.set(Some(begin));
                ((now - begin) as f64 / 1e6) % CYCLE
            }
            _ => 3.0,
        };
        draw_pattern(snapshot, &p, t, still);
    });
    canvas.set_halign(gtk::Align::Center);
    canvas.set_valign(gtk::Align::Center);
    canvas.add_tick_callback(|canvas, _| {
        canvas.queue_draw();
        glib::ControlFlow::Continue
    });
    canvas
}

fn draw_pattern(snapshot: &gtk::Snapshot, p: &Pattern, t: f64, still: bool) {
    snapshot.push_fill(&p.octagon, gsk::FillRule::Winding);
    for (start, color_index, path) in &p.traces {
        let early = start + *color_index as f64 * 0.12;
        let full = (early + 0.9).min(DRAWN);
        let opacity = if still {
            1.0
        } else {
            keyframes(t, &[(0.0, 0.0), (early, 0.0), (early + 0.05, 1.0), (3.1, 1.0), (3.45, 0.0), (CYCLE, 0.0)])
        };
        if opacity <= 0.0 {
            continue;
        }
        let color = COLORS[*color_index];
        let stroke = gsk::Stroke::new(p.width);
        if !still {
            // The spaces of the dashes close: the pieces become the full trace.
            let middle = (early + full) / 2.0;
            if t < middle {
                stroke.set_dash(&[p.step * 0.5, p.step * 1.4]);
            } else if t < full {
                stroke.set_dash(&[p.step * 1.2, p.step * 0.7]);
            }
        }
        snapshot.push_opacity(opacity);
        draw::push_shadow(snapshot, color, 0.7, (p.side / 60.0).max(0.8));
        snapshot.append_stroke(path, &stroke, &color.rgba());
        snapshot.pop();
        snapshot.pop();
    }
    snapshot.pop();
    // The bright point that goes from place to place while the pattern draws.
    if still || p.side < 24.0 {
        return;
    }
    let seen = keyframes(t, &[(0.0, 1.0), (DRAWN, 1.0), (DRAWN + 0.1, 0.0), (CYCLE, 0.0)]);
    if seen <= 0.0 {
        return;
    }
    let index = ((t / CYCLE) * p.places.len() as f64) as usize % p.places.len();
    let (x, y) = p.places[index];
    let radius = (p.side / 34.0).max(1.6);
    snapshot.push_opacity(seen);
    draw::push_shadow(snapshot, Color::rgb(0.5, 0.95, 0.75), 1.0, radius * 2.0);
    let builder = gsk::PathBuilder::new();
    builder.add_circle(&draw::point(x, y), radius);
    draw::fill(snapshot, &builder.to_path(), Color::rgb(0.9, 1.0, 0.94));
    snapshot.pop();
    snapshot.pop();
}

/// Work in progress on a page: the circuit mark, with the words below it in small capitals
/// (`CircuitProgressStyle` for a regular control).
pub fn progress(text: &str) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
    column.set_halign(gtk::Align::Center);
    column.set_valign(gtk::Align::Center);
    column.append(&circuit(84));
    if !text.is_empty() {
        let label = theme::label(font::SMALL, text, "label");
        label.set_xalign(0.5);
        column.append(&label);
    }
    column
}

/// The small mark, for a small control (`.controlSize(.small)`).
pub fn small() -> Canvas {
    circuit(16)
}

/// Progress with a known part: a thin bar.
pub fn fraction_bar(fraction: f64) -> gtk::ProgressBar {
    let bar = gtk::ProgressBar::new();
    bar.add_css_class("fraction-bar");
    bar.set_fraction(fraction.clamp(0.0, 1.0));
    bar
}
