//! The curve of how many offers are available at or below each price, on a logarithmic
//! price scale, with its readout under the pointer (`drawSupply`, `supplyReadout`).
//!
//! The pointer moves only redraw this canvas and its line of text, not the page.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;

use super::logic::{self, CURVE_BOTTOM_MARGIN, CURVE_LEFT, CURVE_RIGHT_MARGIN, CURVE_TOP};
use crate::format;
use crate::theme::draw::{self, Anchor};
use crate::theme::{font, tone};
use crate::widgets::canvas::Canvas;

pub struct Curve {
    pub canvas: Canvas,
    pub readout: gtk::Label,
    /// The prices in ascending order.
    prices: Rc<RefCell<Vec<f64>>>,
    ticker: Rc<RefCell<String>>,
    hovered: Rc<Cell<Option<f64>>>,
}

impl Curve {
    pub fn new() -> Rc<Self> {
        let prices: Rc<RefCell<Vec<f64>>> = Rc::default();
        let hovered: Rc<Cell<Option<f64>>> = Rc::default();
        let canvas = {
            let (prices, hovered) = (prices.clone(), hovered.clone());
            Canvas::new(0, 190, move |canvas, snapshot, w, h| {
                draw_supply(canvas, snapshot, &prices.borrow(), hovered.get(), w as f64, h as f64);
            })
        };
        canvas.set_hexpand(true);
        canvas.update_property(&[gtk::accessible::Property::Label("The number of offers available at or below each price")]);
        let readout = gtk::Label::new(None);
        readout.set_xalign(0.0);
        readout.set_wrap(true);
        readout.add_css_class("caption-text");
        readout.add_css_class("tabular");
        readout.add_css_class("label");
        let curve = Rc::new(Self { canvas, readout, prices, ticker: Rc::default(), hovered });

        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&curve);
        motion.connect_motion(move |_, x, _| {
            if let Some(curve) = weak.upgrade() {
                let width = curve.canvas.width() as f64;
                let bounds = logic::price_bounds(&curve.prices.borrow());
                curve.hovered.set(Some(logic::price_at_x(x, width, bounds)));
                curve.changed();
            }
        });
        let weak = Rc::downgrade(&curve);
        motion.connect_leave(move |_| {
            if let Some(curve) = weak.upgrade() {
                curve.hovered.set(None);
                curve.changed();
            }
        });
        curve.canvas.add_controller(motion);
        curve
    }

    /// New prices, in ascending order, and the ticker of the price currency.
    pub fn set(&self, sorted: Vec<f64>, ticker: &str) {
        if *self.prices.borrow() == sorted && *self.ticker.borrow() == ticker {
            return;
        }
        *self.prices.borrow_mut() = sorted;
        *self.ticker.borrow_mut() = ticker.to_string();
        self.changed();
    }

    fn changed(&self) {
        self.canvas.queue_draw();
        let hovered = self.hovered.get();
        self.readout.set_text(&logic::supply_readout(&self.prices.borrow(), hovered, &self.ticker.borrow()));
        if hovered.is_some() {
            self.readout.remove_css_class("label");
            self.readout.add_css_class("value");
        } else {
            self.readout.remove_css_class("value");
            self.readout.add_css_class("label");
        }
    }
}

fn draw_supply(canvas: &Canvas, snapshot: &gtk::Snapshot, sorted: &[f64], hovered: Option<f64>, width: f64, height: f64) {
    if sorted.is_empty() {
        return;
    }
    let bounds = logic::price_bounds(sorted);
    let (left, right, top, base) = (CURVE_LEFT, width - CURVE_RIGHT_MARGIN, CURVE_TOP, height - CURVE_BOTTOM_MARGIN);
    let x = |price: f64| logic::x_of_price(price, width, bounds) as f32;
    for share in [0.0, 0.5, 1.0] {
        let count = (sorted.len() as f64 * share) as usize;
        let gy = logic::y_of_count(count, sorted.len(), height) as f32;
        draw::rule(snapshot, left as f32, gy, right as f32, gy, if share == 0.0 { tone::AXIS } else { tone::GRID }, 1.0, &[]);
        draw::text(snapshot, canvas, font::SMALL, &format::integer(count as u64), left as f32 - 6.0, gy, Anchor::Trailing, tone::LABEL);
    }
    let mut decade = bounds.0;
    while decade <= bounds.1 * 1.0001 {
        let last = decade * 10.0 > bounds.1 * 1.0001;
        draw::rule(snapshot, x(decade), base as f32, x(decade), base as f32 + 4.0, tone::AXIS, 1.0, &[]);
        draw::text(
            snapshot,
            canvas,
            font::SMALL,
            &format::amount(decade),
            x(decade),
            base as f32 + 11.0,
            if last { Anchor::Trailing } else { Anchor::Center },
            tone::LABEL,
        );
        decade *= 10.0;
    }
    let points = logic::curve_points(sorted, width, height);
    let curve = gtk::gsk::PathBuilder::new();
    let area = gtk::gsk::PathBuilder::new();
    for (index, (px, py)) in points.iter().enumerate() {
        if index == 0 {
            curve.move_to(*px as f32, *py as f32);
            area.move_to(*px as f32, *py as f32);
        } else {
            curve.line_to(*px as f32, *py as f32);
            area.line_to(*px as f32, *py as f32);
        }
    }
    area.line_to(right as f32, base as f32);
    area.close();
    draw::fill(snapshot, &area.to_path(), tone::RECORD.opacity(0.07));
    draw::stroke(snapshot, &curve.to_path(), tone::RECORD, 1.0);
    if let Some(price) = hovered {
        let hx = x(price);
        draw::lit(snapshot, &draw::rect_path(hx - 0.5, top as f32, 1.0, (base - top) as f32), tone::PRESENT, 0.6);
    }
}
