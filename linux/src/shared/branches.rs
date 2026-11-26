//! Owner: the Node page (`pages/node`). Side branches: the ends of the chains that the node
//! knows, drawn against the active chain (`BranchesCard` in `NodeDiagrams.swift`). Used by
//! the Node page and the Block page.
//!
//! The active chain runs along the top. Each side branch leaves it at the block where the
//! two diverged and runs for its own length. This is the daemon's own list of chain tips.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, Card, Page};
use crate::theme::draw::{self, Anchor};
use crate::theme::{tone, Color};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;
use crate::widgets::{lamp, loader};

/// How far back from the tip to draw.
const WINDOW: u64 = 300;
const MAX_DRAWN: usize = 8;

#[derive(Default)]
struct State {
    tips: Vec<api::ChainTip>,
    failure: Option<String>,
    loaded: bool,
    hovered: Option<String>,
}

fn active(tips: &[api::ChainTip]) -> Option<&api::ChainTip> {
    tips.iter().find(|t| t.status == "active")
}

/// A syncing node lists its downloaded headers as a tip far ahead; that is not a fork.
fn headers_ahead(tips: &[api::ChainTip], active: &api::ChainTip) -> Option<u64> {
    tips.iter().find(|t| t.status == "headers-only" && t.height > active.height + WINDOW).map(|t| t.height)
}

/// Side branches that diverged within the window behind the tip.
pub fn recent(tips: &[api::ChainTip]) -> Vec<api::ChainTip> {
    let Some(active) = active(tips) else { return vec![] };
    let start = active.height.saturating_sub(WINDOW);
    tips.iter()
        .filter(|t| t.status != "active" && t.height >= start && t.height <= active.height + 2 && t.branch_length <= WINDOW)
        .take(MAX_DRAWN)
        .cloned()
        .collect()
}

/// Side branches the node has on record, not counting headers it has merely downloaded ahead.
pub fn known(tips: &[api::ChainTip]) -> String {
    let ahead = if active(tips).and_then(|a| headers_ahead(tips, a)).is_none() { 0 } else { 1 };
    let count = (tips.len() as i64 - 1 - ahead).max(0);
    format!("{count} side branch{} known", if count == 1 { "" } else { "es" })
}

pub fn summary(tips: &[api::ChainTip]) -> String {
    let recent = recent(tips);
    if recent.is_empty() {
        return format!("The last {WINDOW} blocks have no side branches.");
    }
    let longest = recent.iter().map(|t| t.branch_length).max().unwrap_or(0);
    format!(
        "{} side branch{} in the last {WINDOW} blocks. The longest side branch has {longest} block{}.",
        recent.len(),
        if recent.len() == 1 { "" } else { "es" },
        if longest == 1 { "" } else { "s" }
    )
}

fn meaning(status: &str) -> String {
    match status {
        "valid-fork" => "validated, then replaced".into(),
        "valid-headers" => "headers valid, some blocks not downloaded".into(),
        "headers-only" => "headers only, blocks not downloaded".into(),
        "invalid" => "contains an invalid block".into(),
        other => other.into(),
    }
}

fn caption(recent: &[api::ChainTip], hovered: Option<&str>) -> String {
    let Some(tip) = recent.iter().find(|t| Some(t.hash.as_str()) == hovered) else {
        return "The top line is the active chain. Each lower line is a side branch. Click a side branch to open its last block.".into();
    };
    format!(
        "Branch ending at block {} · {} block{} long · {}",
        format::integer(tip.height),
        tip.branch_length,
        if tip.branch_length == 1 { "" } else { "s" },
        meaning(&tip.status)
    )
}

/// The row of the canvas under a point, as the Swift app finds it.
fn row_at(y: f64, count: usize) -> Option<usize> {
    let row = ((y - 44.0) / 26.0 + 0.5).trunc();
    (y > 30.0 && row >= 0.0 && (row as usize) < count).then_some(row as usize)
}

/// The card of side branches, for a page.
pub fn branches_card(page: &Rc<Page>) -> Card {
    let card = page.card("Branches", None);
    let state: Rc<RefCell<State>> = Rc::default();
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.append(&holder);
    let view = Rc::new(View { model: Rc::downgrade(&page.model), state, holder: holder.downgrade(), trailing: card.trailing.clone() });
    view.show();
    let weak = Rc::downgrade(&view);
    // Loaded once, and again on request. On a full node this request takes several
    // seconds and makes the node's other answers wait, so it is not repeated on a timer.
    page.model.watch(&[Topic::Connection], &holder, move |_| {
        if let Some(view) = weak.upgrade() {
            view.load();
        }
    });
    view.load();
    // The holder keeps the view as long as it lives.
    let keep = view.clone();
    holder.connect_destroy(move |_| {
        let _ = &keep;
    });
    card
}

struct View {
    model: std::rc::Weak<AppModel>,
    state: Rc<RefCell<State>>,
    holder: glib::WeakRef<gtk::Box>,
    trailing: gtk::Label,
}

impl View {
    fn load(self: &Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(explorer) = model.explorer() else { return };
        let generation = model.generation();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.chain_tips().await }).await;
            let Some(view) = weak.upgrade() else { return };
            if view.model.upgrade().is_none_or(|m| m.generation() != generation) {
                return;
            }
            {
                let mut state = view.state.borrow_mut();
                match result {
                    Ok(tips) => {
                        state.tips = tips;
                        state.failure = None;
                    }
                    Err(error) => state.failure = Some(bridge::describe(&error)),
                }
                state.loaded = true;
            }
            view.show();
        });
    }

    fn set_trailing(&self, text: Option<&str>) {
        match text {
            Some(text) => {
                crate::theme::set_label(&self.trailing, crate::theme::font::SMALL, text);
                self.trailing.set_visible(true);
            }
            None => self.trailing.set_visible(false),
        }
    }

    fn show(self: &Rc<Self>) {
        let Some(holder) = self.holder.upgrade() else { return };
        clear(&holder);
        let state = self.state.borrow();
        self.set_trailing((state.loaded && state.failure.is_none()).then(|| known(&state.tips)).as_deref());
        if let Some(failure) = &state.failure {
            holder.append(&page::paragraph(failure, &["secondary"]));
            return;
        }
        if !state.loaded {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.append(&loader::small());
            row.append(&page::paragraph("Reading the chain tips from the node. This can take some seconds.", &["secondary"]));
            holder.append(&row);
            return;
        }
        let Some(tip) = active(&state.tips).cloned() else {
            holder.append(&page::paragraph("The node reported no active chain.", &["secondary"]));
            return;
        };
        holder.append(&page::paragraph(&summary(&state.tips), &["value"]));
        let recent = recent(&state.tips);
        if !recent.is_empty() {
            holder.append(&self.diagram(&tip, recent));
        }
        let weak = Rc::downgrade(self);
        holder.append(&lamp::button("Refresh", move || {
            if let Some(view) = weak.upgrade() {
                view.state.borrow_mut().loaded = false;
                view.show();
                view.load();
            }
        }));
        if let Some(ahead) = headers_ahead(&state.tips, &tip) {
            holder.append(&page::paragraph(
                &format!(
                    "The node has headers to block {}. It has validated blocks to block {}.",
                    format::integer(ahead),
                    format::integer(tip.height)
                ),
                &["callout", "secondary"],
            ));
        }
    }

    fn diagram(self: &Rc<Self>, active: &api::ChainTip, recent: Vec<api::ChainTip>) -> gtk::Overlay {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let (s, a, r) = (self.state.clone(), active.clone(), recent.clone());
        let canvas = Canvas::new(0, 52 + recent.len() as i32 * 26, move |canvas, snapshot, w, _| {
            draw_tree(canvas, snapshot, w, &a, &r, s.borrow().hovered.as_deref());
        });
        canvas.set_hexpand(true);
        column.append(&canvas);
        column.append(&display::rule());
        let caption_label = page::text(&caption(&recent, None), &["caption-text", "label"]);
        caption_label.set_size_request(-1, 20);
        caption_label.set_wrap(true);
        column.append(&caption_label);

        let motion = gtk::EventControllerMotion::new();
        let (state, r, c, label) = (self.state.clone(), recent.clone(), canvas.downgrade(), caption_label.downgrade());
        let set_hovered = Rc::new(move |hovered: Option<String>| {
            if state.borrow().hovered == hovered {
                return;
            }
            state.borrow_mut().hovered = hovered.clone();
            if let Some(c) = c.upgrade() {
                c.queue_draw();
            }
            if let Some(label) = label.upgrade() {
                label.set_text(&caption(&r, hovered.as_deref()));
                if hovered.is_some() {
                    label.remove_css_class("label");
                    label.add_css_class("value");
                } else {
                    label.remove_css_class("value");
                    label.add_css_class("label");
                }
            }
        });
        let (set, r) = (set_hovered.clone(), recent.clone());
        motion.connect_motion(move |_, _, y| set(row_at(y, r.len()).map(|row| r[row].hash.clone())));
        let set = set_hovered.clone();
        motion.connect_leave(move |_| set(None));
        canvas.add_controller(motion);
        let click = gtk::GestureClick::new();
        let (model, r) = (self.model.clone(), recent.clone());
        click.connect_released(move |_, _, _, y| {
            if let (Some(row), Some(model)) = (row_at(y, r.len()), model.upgrade()) {
                crate::theme::sound::link();
                model.open(api::Route::Block { id: r[row].hash.clone() });
            }
        });
        canvas.add_controller(click);
        canvas.set_cursor_from_name(Some("pointer"));
        display::panel(&column, 16)
    }
}

fn draw_tree(widget: &Canvas, snapshot: &gtk::Snapshot, width: f32, active: &api::ChainTip, recent: &[api::ChainTip], hovered: Option<&str>) {
    let start = active.height.saturating_sub(WINDOW);
    let left = 8.0f32;
    let right = width - 44.0;
    let x = |height: u64| left + ((height - height.min(start)) as f64 / WINDOW as f64) as f32 * (right - left);
    let top = 18.0f32;
    let tip_x = x(active.height);
    // The active chain, and its tip: the present.
    draw::rule(snapshot, left, top, tip_x, top, tone::VALUE, 1.5, &[]);
    draw::lit(snapshot, &draw::rect_path(tip_x - 0.75, top - 7.0, 1.5, 14.0), tone::PRESENT, 0.7);
    draw::name(snapshot, widget, "tip", tip_x + 6.0, top, Anchor::Leading, tone::PRESENT);
    draw::name(snapshot, widget, &format!("{WINDOW} blocks ago"), left, top - 12.0, Anchor::Leading, tone::LABEL);

    for (row, tip) in recent.iter().enumerate() {
        let y = top + 26.0 + row as f32 * 26.0;
        let fork = tip.height.saturating_sub(tip.branch_length);
        let fork_x = x(fork.max(start));
        let end_x = x(tip.height).max(fork_x + 6.0);
        let validated = tip.status == "valid-fork";
        let color: Color = if tip.status == "invalid" { tone::ALERT } else if validated { tone::MINED } else { tone::AXIS };
        let selected = hovered == Some(tip.hash.as_str());
        let builder = gtk::gsk::PathBuilder::new();
        builder.move_to(fork_x, top);
        builder.line_to(fork_x, y);
        builder.line_to(end_x, y);
        let stroke = gtk::gsk::Stroke::new(if selected { 1.5 } else { 1.0 });
        stroke.set_line_join(gtk::gsk::LineJoin::Miter);
        if !validated {
            stroke.set_dash(&[4.0, 3.0]);
        }
        snapshot.append_stroke(&builder.to_path(), &stroke, &(if selected { tone::PRESENT } else { color }).rgba());
        let end = draw::rect_path(end_x - 3.5, y - 3.5, 7.0, 7.0);
        draw::lit(snapshot, &end, color, if selected { 0.9 } else { 0.0 });
        if selected {
            draw::stroke(snapshot, &end, tone::PRESENT, 1.5);
        }
        draw::figure(snapshot, widget, &tip.branch_length.to_string(), end_x + 9.0, y, Anchor::Leading, if selected { tone::PRESENT } else { tone::VALUE });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tip(height: u64, length: u64, status: &str) -> api::ChainTip {
        api::ChainTip { height, hash: format!("h{height}"), branch_length: length, status: status.into() }
    }

    #[test]
    fn branches_in_the_window() {
        let tips = vec![tip(1000, 0, "active"), tip(990, 2, "valid-fork"), tip(500, 1, "valid-fork"), tip(5000, 4000, "headers-only")];
        assert_eq!(recent(&tips).len(), 1);
        assert_eq!(known(&tips), "2 side branches known");
        assert_eq!(summary(&tips), "1 side branch in the last 300 blocks. The longest side branch has 2 blocks.");
        assert_eq!(summary(&[tip(10, 0, "active")]), "The last 300 blocks have no side branches.");
        assert_eq!(known(&[tip(10, 0, "active"), tip(9, 1, "valid-fork")]), "1 side branch known");
        assert_eq!(row_at(44.0, 2), Some(0));
        assert_eq!(row_at(70.0, 2), Some(1));
        assert_eq!(row_at(20.0, 2), None);
        assert_eq!(row_at(110.0, 2), None);
        assert_eq!(caption(&recent(&tips), Some("h990")), "Branch ending at block 990 · 2 blocks long · validated, then replaced");
    }
}
