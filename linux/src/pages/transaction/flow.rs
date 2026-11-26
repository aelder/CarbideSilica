//! The statement as a flow (`StatementFlow` and `FlowPicture` in `TransactionStatement.swift`):
//! the parties that gave on the left, the transaction in the middle, the parties that
//! received on the right. The width of a band is the amount.
//!
//! All bands go through the transaction. The chain does not record which input paid which
//! output, thus the picture has no band from one party to a different party.
//!
//! The band of the joined addresses opens: a click goes one level in, where that group is
//! on the left and its next largest addresses are on the right, with the remainder as a
//! new group. The picture moves across to the new level in 0.6 seconds, on the frame clock
//! of the picture alone. The names change with a cross-fade of the same length.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::gsk;
use gtk::prelude::*;

use crate::page::{self, Page};
use crate::route as routes;
use crate::theme::{draw, tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::{lamp, link};

use super::flow_scene::{acts, choice, ease_in_out, Choice, Levels, Node, Scene, BAR, CORE};
use super::layout::{RowStyle, Selection, StatementLayout, StatementRow};
use crate::shared::statement_words as words;
use super::Ctx;

const LEFT_WIDTH: i32 = 236;
const RIGHT_WIDTH: i32 = 270;
const MOVE_SECONDS: f64 = 0.6;

struct State {
    layout: Rc<StatementLayout>,
    column: usize,
    unit: String,
    /// How many levels into the joined addresses the picture is. 0 is the transaction. The
    /// statement keeps it, so that the level stays when the names arrive.
    depth: Rc<Cell<usize>>,
    /// The level that the picture comes from, while it moves.
    origin: Cell<usize>,
    /// 0 at the start of a move between levels, 1 at its end.
    progress: Cell<f64>,
    /// The frame time at the start of the move.
    started: Cell<Option<i64>>,
    pointed: RefCell<Option<String>>,
    chosen: RefCell<Option<String>>,
    now: RefCell<Scene>,
    before: RefCell<Scene>,
}

impl State {
    fn levels(&self) -> Levels<'_> {
        Levels { layout: &self.layout, column: self.column }
    }

    fn level(&self) -> usize {
        self.depth.get().min(self.levels().max_depth())
    }

    /// The scenes of the level and the level before it. Each level has the height of the
    /// tallest one seen, so that the page below the picture does not move when the level
    /// changes.
    fn place_scenes(&self) {
        let levels = self.levels();
        let level = self.level();
        let origin = self.origin.get().min(levels.max_depth());
        let mut now = levels.scene(level);
        let mut before = levels.scene(origin);
        let tall = now.height.max(before.height).max(levels.scene(0).height);
        now.height = tall;
        before.height = tall;
        *self.now.borrow_mut() = now;
        *self.before.borrow_mut() = before;
    }
}

pub struct Flow {
    pub root: gtk::Box,
    state: Rc<State>,
    ctx: Weak<Ctx>,
    picture: Canvas,
    left: gtk::Stack,
    right: gtk::Stack,
    back: gtk::Revealer,
    back_keys: gtk::Box,
    caption: gtk::Label,
    /// A count that names the pages of the stacks.
    pages: Cell<u64>,
}

impl Flow {
    pub fn new(ctx: &Rc<Ctx>, layout: Rc<StatementLayout>, column: usize, unit: String, depth: Rc<Cell<usize>>) -> Rc<Self> {
        let state = Rc::new(State {
            layout,
            column,
            unit,
            origin: Cell::new(depth.get()),
            depth,
            progress: Cell::new(1.0),
            started: Cell::new(None),
            pointed: RefCell::new(None),
            chosen: RefCell::new(ctx.marked().map(|s| s.id)),
            now: RefCell::new(Scene::default()),
            before: RefCell::new(Scene::default()),
        });
        state.place_scenes();

        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let stack = |width: i32| {
            let stack = gtk::Stack::new();
            stack.set_transition_type(gtk::StackTransitionType::Crossfade);
            stack.set_transition_duration((MOVE_SECONDS * 1000.0) as u32);
            stack.set_size_request(width, -1);
            // The names keep their width: the picture takes the rest.
            stack.set_hexpand(false);
            stack.set_hhomogeneous(true);
            stack.set_vhomogeneous(true);
            stack.set_valign(gtk::Align::Start);
            stack
        };
        let left = stack(LEFT_WIDTH);
        let right = stack(RIGHT_WIDTH);
        let picture = {
            let state = state.clone();
            Canvas::new(0, 0, move |_, snapshot, w, h| {
                // The levels that move in and out stay inside the picture.
                snapshot.push_clip(&draw::rect(0.0, 0.0, w, h));
                paint(snapshot, &state, w as f64);
                snapshot.pop();
            })
        };
        picture.set_hexpand(true);
        picture.set_valign(gtk::Align::Start);
        picture.update_property(&[gtk::accessible::Property::Label(&format!(
            "The flow of {} through the transaction.",
            state.unit
        ))]);
        row.append(&left);
        row.append(&picture);
        row.append(&right);

        // The keys to go back are on the picture, in the empty corner above the group, so
        // that the picture stays in its place when they come.
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&row));
        let back_keys = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let back = gtk::Revealer::new();
        back.set_transition_type(gtk::RevealerTransitionType::Crossfade);
        back.set_transition_duration((MOVE_SECONDS * 1000.0) as u32);
        back.set_child(Some(&back_keys));
        back.set_halign(gtk::Align::Start);
        back.set_valign(gtk::Align::Start);
        overlay.add_overlay(&back);
        root.append(&overlay);

        let caption = page::paragraph("", &["caption-text", "label"]);
        root.append(&caption);

        let flow = Rc::new(Self {
            root,
            state,
            ctx: Rc::downgrade(ctx),
            picture,
            left,
            right,
            back,
            back_keys,
            caption,
            pages: Cell::new(0),
        });
        flow.show_level(false);

        // One pointer area for the names and the bands. With an area for each, the light
        // of a band went off and on when the pointer went from its name to the band.
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&flow);
        motion.connect_motion(move |_, x, y| {
            if let Some(flow) = weak.upgrade() {
                flow.hover(Some((x, y)));
            }
        });
        let weak = Rc::downgrade(&flow);
        motion.connect_leave(move |_| {
            if let Some(flow) = weak.upgrade() {
                flow.hover(None);
            }
        });
        overlay.add_controller(motion);

        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&flow);
        click.connect_released(move |_, presses, x, y| {
            let Some(flow) = weak.upgrade() else { return };
            if presses != 1 {
                return;
            }
            let width = flow.picture.width() as f64;
            let found = flow.state.now.borrow().band(x, y, width).map(|n| n.row.clone());
            if let Some(row) = found {
                flow.choose(&row);
            }
        });
        flow.picture.add_controller(click);
        flow
    }

    /// The row that the statement marks.
    pub fn set_chosen(self: &Rc<Self>, chosen: Option<&str>) {
        if self.state.chosen.borrow().as_deref() == chosen {
            return;
        }
        *self.state.chosen.borrow_mut() = chosen.map(str::to_string);
        self.picture.queue_draw();
        self.fill_labels(false);
    }

    fn hover(&self, point: Option<(f64, f64)>) {
        let found = point.and_then(|(x, y)| {
            let now = self.state.now.borrow();
            // In the picture, the band under the pointer. At the names, the row.
            let picture_left = LEFT_WIDTH as f64 + 10.0;
            let picture_width = self.picture.width() as f64;
            if x >= picture_left && x <= picture_left + picture_width {
                now.band(x - picture_left, y, picture_width).map(|n| n.row.id.clone())
            } else {
                now.row_at(x < picture_left, y).map(|n| n.row.id.clone())
            }
        });
        if *self.state.pointed.borrow() != found {
            *self.state.pointed.borrow_mut() = found;
            self.picture.queue_draw();
        }
    }

    fn choose(self: &Rc<Self>, row: &StatementRow) {
        let level = self.state.level();
        match choice(row, level, !self.state.layout.joined.is_empty()) {
            Choice::Level(next) => self.go(next),
            Choice::Mark => {
                let Some(ctx) = self.ctx.upgrade() else { return };
                let marked = ctx.marked();
                ctx.set_marked(if marked.is_some_and(|m| m.id == row.id) { None } else { Some(Selection::of(row)) });
            }
            Choice::Nothing => {}
        }
    }

    /// Goes to a level. The picture moves across in the direction of the level.
    fn go(self: &Rc<Self>, level: usize) {
        let level = level.min(self.state.levels().max_depth());
        if level == self.state.depth.get() {
            return;
        }
        self.state.origin.set(self.state.depth.get());
        self.state.depth.set(level);
        *self.state.pointed.borrow_mut() = None;
        self.state.place_scenes();
        let still = !self.picture.settings().is_gtk_enable_animations();
        self.state.progress.set(if still { 1.0 } else { 0.0 });
        self.state.started.set(None);
        self.show_level(!still);
        if still {
            return;
        }
        let state = self.state.clone();
        self.picture.add_tick_callback(move |canvas, clock| {
            let now = clock.frame_time();
            let start = match state.started.get() {
                Some(start) => start,
                None => {
                    state.started.set(Some(now));
                    now
                }
            };
            let part = (now - start) as f64 / (MOVE_SECONDS * 1_000_000.0);
            state.progress.set(ease_in_out(part));
            canvas.queue_draw();
            if part >= 1.0 {
                state.progress.set(1.0);
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    /// Sizes the picture, the names, the keys to go back and the caption for the level.
    fn show_level(self: &Rc<Self>, animate: bool) {
        let level = self.state.level();
        let height = self.state.now.borrow().height.ceil() as i32;
        self.picture.set_size(0, height);
        self.picture.queue_draw();
        self.fill_labels(animate);
        // The keys to go back.
        page::load::clear(&self.back_keys);
        if level > 0 {
            let weak = Rc::downgrade(self);
            self.back_keys.append(&lamp::button("Up one level", move || {
                if let Some(flow) = weak.upgrade() {
                    flow.go(flow.state.level().saturating_sub(1));
                }
            }));
            if level > 1 {
                let weak = Rc::downgrade(self);
                self.back_keys.append(&lamp::button("To the transaction", move || {
                    if let Some(flow) = weak.upgrade() {
                        flow.go(0);
                    }
                }));
            }
        }
        self.back.set_reveal_child(level > 0);
        let caption = self.state.levels().caption(level, &self.state.now.borrow());
        self.caption.set_text(&caption);
        self.caption.set_visible(!caption.is_empty());
    }

    /// The names of both sides, in a new page of each stack.
    fn fill_labels(self: &Rc<Self>, animate: bool) {
        let Some(ctx) = self.ctx.upgrade() else { return };
        let Some(page) = ctx.page() else { return };
        let now = self.state.now.borrow().clone();
        let n = self.pages.get() + 1;
        self.pages.set(n);
        for (stack, nodes, leading, width) in [(&self.left, &now.left, false, LEFT_WIDTH), (&self.right, &now.right, true, RIGHT_WIDTH)] {
            let fixed = gtk::Fixed::new();
            fixed.set_size_request(width, now.height.ceil() as i32);
            for node in nodes {
                let line = self.label(&page, node, leading);
                line.set_size_request(width, node.slot.round() as i32);
                fixed.put(&line, 0.0, node.top.round());
            }
            let old = stack.visible_child();
            let name = format!("level-{n}");
            stack.add_named(&fixed, Some(&name));
            stack.set_transition_type(if animate { gtk::StackTransitionType::Crossfade } else { gtk::StackTransitionType::None });
            stack.set_visible_child_name(&name);
            if let Some(old) = old {
                if animate {
                    let (stack, old) = (stack.downgrade(), old.downgrade());
                    glib::timeout_add_local_once(std::time::Duration::from_millis((MOVE_SECONDS * 1000.0) as u64 + 50), move || {
                        if let (Some(stack), Some(old)) = (stack.upgrade(), old.upgrade())
                            && old.parent().as_ref() == Some(stack.upcast_ref())
                            && stack.visible_child().as_ref() != Some(&old)
                        {
                            stack.remove(&old);
                        }
                    });
                } else {
                    stack.remove(&old);
                }
            }
        }
    }

    /// One row of names: the amount and the name of the party, at the middle of its slot.
    fn label(self: &Rc<Self>, page: &Rc<Page>, node: &Node, leading: bool) -> gtk::Box {
        let outer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.set_valign(gtk::Align::Center);
        line.set_vexpand(true);
        line.set_hexpand(true);
        line.set_halign(if leading { gtk::Align::Start } else { gtk::Align::End });
        let chosen = self.state.chosen.borrow().as_deref() == Some(node.row.id.as_str());
        let amount = page::text(&words::brief(node.sats), &["callout", "tabular", if chosen { "present" } else { "value" }]);
        amount.set_xalign(1.0);
        amount.set_size_request(78, -1);
        amount.set_tooltip_text(Some(&format!("{} {}", crate::format::coins(node.sats), self.state.unit)));
        let name: gtk::Widget = if let Some(address) = &node.row.address {
            page::route_link(page, address, routes::holder(address), true).upcast()
        } else if acts(&node.row) {
            // The name does what a click on its band does.
            let weak = Rc::downgrade(self);
            let row = node.row.clone();
            let choose = Rc::new(move || {
                if let Some(flow) = weak.upgrade() {
                    flow.choose(&row);
                }
            });
            let run = choose.clone();
            let key = link::link(&node.row.title, false, move || run());
            key.add_css_class(if node.row.style == RowStyle::Rest { "tx-tone-label" } else { "tx-tone-value" });
            if let Some(label) = link::label_of(&key) {
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            }
            key.set_valign(gtk::Align::Center);
            page.stop(&key, None, None, move || choose());
            key.upcast()
        } else {
            let text = page::text(&node.row.title, &["label"]);
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.upcast()
        };
        name.set_valign(gtk::Align::Center);
        if leading {
            line.append(&amount);
            line.append(&name);
        } else {
            line.append(&name);
            line.append(&amount);
        }
        outer.append(&line);
        outer
    }
}

/// The colour of a band.
fn color(row: &StatementRow, gave: bool, chosen: Option<&str>) -> Color {
    if chosen == Some(row.id.as_str()) {
        return tone::PRESENT;
    }
    match row.style {
        RowStyle::Closing => tone::MINED,
        RowStyle::Rest => tone::LABEL,
        RowStyle::Hidden => tone::STAKED,
        _ => {
            if gave {
                tone::VALUE
            } else {
                tone::RECORD
            }
        }
    }
}

/// The bands of the flow. While the picture moves between two levels, the level it leaves
/// goes out to one side and the new level comes in from the other side. The bar of the
/// group goes across with them: it is the right end of one level and the left end of the
/// next.
fn paint(snapshot: &gtk::Snapshot, state: &State, width: f64) {
    let chosen = state.chosen.borrow().clone();
    let pointed = state.pointed.borrow().clone();
    let now = state.now.borrow();
    let p = state.progress.get().clamp(0.0, 1.0);
    if p >= 1.0 {
        return scene(snapshot, &now, width, 0.0, 1.0, chosen.as_deref(), pointed.as_deref());
    }
    let before = state.before.borrow();
    let inward = state.level() >= state.origin.get().min(state.levels().max_depth());
    let way = if inward { 1.0 } else { -1.0 };
    scene(snapshot, &before, width, -way * p * width, 1.0 - p, chosen.as_deref(), pointed.as_deref());
    scene(snapshot, &now, width, way * (1.0 - p) * width, p, chosen.as_deref(), pointed.as_deref());
    // The bar of the group, from its place in one level to its place in the other.
    let end = |node: Option<&Node>, right: bool| -> Option<(f64, f64, f64)> {
        node.map(|n| (if right { width - BAR } else { 0.0 }, n.middle() - n.thick / 2.0, n.thick))
    };
    let (start, stop) = if inward {
        (end(before.group(), true), end(now.left.first(), false))
    } else {
        (end(before.left.first(), false), end(now.group(), true))
    };
    if let (Some(start), Some(stop)) = (start, stop) {
        let x = start.0 + (stop.0 - start.0) * p;
        let y = start.1 + (stop.1 - start.1) * p;
        let h = start.2 + (stop.2 - start.2) * p;
        draw::lit(snapshot, &draw::rect_path(x as f32, y as f32, BAR as f32, h as f32), tone::PRESENT, 0.8);
    }
}

fn scene(snapshot: &gtk::Snapshot, scene: &Scene, width: f64, shift: f64, opacity: f64, chosen: Option<&str>, pointed: Option<&str>) {
    snapshot.push_opacity(opacity);
    snapshot.save();
    snapshot.translate(&draw::point(shift as f32, 0.0));
    let mid = width / 2.0;
    let (core_top, core_height) = scene.core();
    let band = |node: &Node, y: f64, gave: bool| {
        let outer = if gave { BAR } else { width - BAR };
        let inner = if gave { mid - CORE / 2.0 } else { mid + CORE / 2.0 };
        let from_y = node.middle() - node.thick / 2.0;
        let bend = (outer + inner) / 2.0;
        let path = gsk::PathBuilder::new();
        path.move_to(outer as f32, from_y as f32);
        path.cubic_to(bend as f32, from_y as f32, bend as f32, y as f32, inner as f32, y as f32);
        path.line_to(inner as f32, (y + node.thick) as f32);
        path.cubic_to(
            bend as f32,
            (y + node.thick) as f32,
            bend as f32,
            (from_y + node.thick) as f32,
            outer as f32,
            (from_y + node.thick) as f32,
        );
        path.close();
        let path = path.to_path();
        let tone = color(&node.row, gave, chosen);
        let strong = chosen == Some(node.row.id.as_str()) || pointed == Some(node.row.id.as_str());
        draw::fill(snapshot, &path, tone.opacity(if strong { 0.6 } else if gave { 0.16 } else { 0.34 }));
        if gave || node.row.style == RowStyle::Hidden {
            // What a party gave is an outline, as in the table.
            let stroke = gsk::Stroke::new(1.0);
            if node.row.style == RowStyle::Hidden {
                stroke.set_dash(&[4.0, 3.0]);
            }
            snapshot.append_stroke(&path, &stroke, &tone.opacity(0.8).rgba());
        }
        // The end of the band at its party.
        let end = draw::rect_path(if gave { 0.0 } else { (width - BAR) as f32 }, from_y as f32, BAR as f32, node.thick as f32);
        if strong {
            draw::lit(snapshot, &end, tone, 0.6);
        } else {
            draw::fill(snapshot, &end, tone);
        }
    };
    let left_thick: f64 = scene.left.iter().map(|n| n.thick).sum();
    let right_thick: f64 = scene.right.iter().map(|n| n.thick).sum();
    let mut y = core_top + (core_height - left_thick) / 2.0;
    for node in &scene.left {
        band(node, y, true);
        y += node.thick;
    }
    y = core_top + (core_height - right_thick) / 2.0;
    for node in &scene.right {
        band(node, y, false);
        y += node.thick;
    }
    // The middle: all that came in is all that went out.
    draw::lit(
        snapshot,
        &draw::rect_path((mid - CORE / 2.0) as f32, core_top as f32, CORE as f32, core_height as f32),
        tone::PRESENT,
        0.5,
    );
    snapshot.restore();
    snapshot.pop();
}
