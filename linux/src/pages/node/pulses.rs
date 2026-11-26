//! The packets that go along the lines of the map, in the two directions at the same time
//! (`RoutePulses` in `PeerOutlineMap.swift`).
//!
//! The Swift app gives the packets to Core Animation. Here a small canvas of its own, over
//! the map, draws only the packets, on the frame clock of GTK. The map below it does not
//! redraw for a frame. The packets start again only when a line changes.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::geometry::{self, Packet, PeerRoute, Track};
use crate::theme::{draw, Color};
use crate::widgets::canvas::Canvas;

const OUT: Color = Color::rgb(0.96, 0.93, 0.82);
const BACK: Color = Color::rgb(0.6, 0.92, 0.86);

struct Line {
    forward: Track,
    reversed: Track,
    packets: Vec<Packet>,
}

#[derive(Default)]
struct State {
    shown: Vec<PeerRoute>,
    lines: Vec<Line>,
    /// The frame time of the start, in microseconds.
    start: Option<i64>,
}

fn animations_on() -> bool {
    gtk::Settings::default().map(|s| s.is_gtk_enable_animations()).unwrap_or(true)
}

pub struct Pulses {
    pub canvas: Canvas,
}

impl Pulses {
    /// `routes` are the lines of the map, which the map sets when it draws.
    pub fn new(routes: Rc<RefCell<Vec<PeerRoute>>>) -> Self {
        let state: Rc<RefCell<State>> = Rc::default();
        let s = state.clone();
        let canvas = Canvas::new(0, 0, move |canvas, snapshot, _, _| {
            let state = s.borrow();
            let (Some(start), Some(clock)) = (state.start, canvas.frame_clock()) else { return };
            if state.lines.is_empty() {
                return;
            }
            let elapsed = (clock.frame_time() - start) as f64 / 1e6;
            // One light for all packets of a colour: two shadow nodes in a frame.
            for outward in [true, false] {
                let color = if outward { OUT } else { BACK };
                draw::push_shadow(snapshot, color, 1.0, 4.0);
                for line in &state.lines {
                    let track = if outward { &line.forward } else { &line.reversed };
                    for packet in line.packets.iter().filter(|p| p.outward == outward) {
                        let Some(share) = packet.share(elapsed) else { continue };
                        let Some((at, angle)) = track.at(share) else { continue };
                        snapshot.save();
                        snapshot.translate(&draw::point(at.x as f32, at.y as f32));
                        snapshot.rotate(angle.to_degrees() as f32);
                        let rounded = gtk::gsk::RoundedRect::from_rect(draw::rect(-2.75, -1.3, 5.5, 2.6), 1.3);
                        snapshot.push_rounded_clip(&rounded);
                        draw::fill_rect(snapshot, -2.75, -1.3, 5.5, 2.6, color);
                        snapshot.pop();
                        snapshot.restore();
                    }
                }
                snapshot.pop();
            }
        });
        canvas.set_can_target(false);
        canvas.set_hexpand(true);
        canvas.set_vexpand(true);
        let weak = Rc::downgrade(&state);
        canvas.add_tick_callback(move |canvas, clock| {
            let Some(state) = weak.upgrade() else { return glib::ControlFlow::Break };
            if state.borrow().shown != *routes.borrow() {
                set_routes(&state, &routes.borrow());
            }
            let mut state = state.borrow_mut();
            if state.lines.is_empty() {
                return glib::ControlFlow::Continue;
            }
            if state.start.is_none() {
                state.start = Some(clock.frame_time());
            }
            canvas.queue_draw();
            glib::ControlFlow::Continue
        });
        Self { canvas }
    }
}

/// The lines of the map. The packets start again only when a line changed.
fn set_routes(state: &RefCell<State>, routes: &[PeerRoute]) {
    {
        let mut state = state.borrow_mut();
        state.shown = routes.to_vec();
        state.start = None;
        state.lines.clear();
        if animations_on() {
            for route in routes.iter().filter(|r| r.points.len() > 1) {
                let mut reversed = route.points.clone();
                reversed.reverse();
                state.lines.push(Line {
                    forward: Track::new(geometry::rounded(&route.points, geometry::CORNER)),
                    reversed: Track::new(geometry::rounded(&reversed, geometry::CORNER)),
                    packets: geometry::packets(route.id, route.round),
                });
            }
        }
    }
}
