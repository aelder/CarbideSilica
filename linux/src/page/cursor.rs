//! The keyboard cursor of one page (`PageCursor.swift`). Each row and each link of the page
//! that the page marks as a stop is a place of the cursor, in the order that the page shows
//! them: card by card, and in a card from the top down and from the left to the right. The
//! keys move the cursor and open the stop under it, as j, k and Enter do in the terminal.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

/// One thing on a page that the keys can select and open: a row of a list or a link.
pub struct Stop {
    pub id: String,
    pub widget: glib::WeakRef<gtk::Widget>,
    /// The value that `y` and Ctrl-C copy.
    pub copy: Option<String>,
    pub open: Rc<dyn Fn()>,
}

/// A card of the page: its title and its widget, for Tab and for the command list.
pub struct CardMark {
    pub title: String,
    pub widget: glib::WeakRef<gtk::Widget>,
}

#[derive(Default)]
pub struct PageCursor {
    selection: RefCell<Option<String>>,
    stops: RefCell<Vec<Stop>>,
    cards: RefCell<Vec<CardMark>>,
    /// The column of the page, for positions.
    content: RefCell<Option<glib::WeakRef<gtk::Widget>>>,
    scroller: RefCell<Option<glib::WeakRef<gtk::ScrolledWindow>>>,
    /// False for a page that has its own keys, such as the charts.
    pub enabled: Cell<bool>,
    /// True while a part of the page has the arrow keys, such as the byte view.
    pub suspended: Cell<bool>,
    /// Called when the selection changes, with the new stop ID.
    on_select: RefCell<Vec<Rc<dyn Fn(Option<&str>)>>>,
    next_id: Cell<u64>,
}

struct Placed {
    index: usize,
    group: String,
    x: f32,
    y: f32,
    mid_y: f32,
}

impl PageCursor {
    pub fn new() -> Rc<Self> {
        let cursor = Rc::new(Self::default());
        cursor.enabled.set(true);
        cursor
    }

    pub fn attach(&self, content: &impl IsA<gtk::Widget>, scroller: Option<&gtk::ScrolledWindow>) {
        *self.content.borrow_mut() = Some(content.upcast_ref::<gtk::Widget>().downgrade());
        *self.scroller.borrow_mut() = scroller.map(|s| s.downgrade());
    }

    /// Makes `widget` a stop. `id` is the identity of a row that has one (for a selection
    /// that the page keeps); None gives the stop its own.
    pub fn add_stop(&self, widget: &impl IsA<gtk::Widget>, id: Option<String>, copy: Option<String>, open: Rc<dyn Fn()>) -> String {
        let id = id.unwrap_or_else(|| {
            let next = self.next_id.get() + 1;
            self.next_id.set(next);
            format!("stop-{next}")
        });
        let widget = widget.upcast_ref::<gtk::Widget>();
        let mut stops = self.stops.borrow_mut();
        stops.retain(|stop| stop.widget.upgrade().is_some() && stop.id != id);
        stops.push(Stop { id: id.clone(), widget: widget.downgrade(), copy, open });
        if self.selection.borrow().as_deref() == Some(id.as_str()) {
            widget.add_css_class("cursor-on");
        }
        id
    }

    pub fn add_card(&self, title: &str, widget: &impl IsA<gtk::Widget>) {
        let mut cards = self.cards.borrow_mut();
        cards.retain(|card| card.widget.upgrade().is_some() && card.title != title);
        cards.push(CardMark { title: title.to_string(), widget: widget.upcast_ref::<gtk::Widget>().downgrade() });
    }

    pub fn on_select(&self, handler: impl Fn(Option<&str>) + 'static) {
        self.on_select.borrow_mut().push(Rc::new(handler));
    }

    pub fn selection(&self) -> Option<String> {
        self.selection.borrow().clone()
    }

    pub fn has_stops(&self) -> bool {
        self.enabled.get() && self.stops.borrow().iter().any(|s| s.widget.upgrade().is_some_and(|w| w.is_drawable()))
    }

    /// The selected stop's copy value.
    pub fn selected_copy(&self) -> Option<String> {
        let selection = self.selection.borrow().clone()?;
        self.stops.borrow().iter().find(|s| s.id == selection).and_then(|s| s.copy.clone())
    }

    fn content(&self) -> Option<gtk::Widget> {
        self.content.borrow().as_ref().and_then(|w| w.upgrade())
    }

    fn scroller(&self) -> Option<gtk::ScrolledWindow> {
        self.scroller.borrow().as_ref().and_then(|w| w.upgrade())
    }

    /// The card of a widget: the nearest card that holds it.
    fn group_of(&self, widget: &gtk::Widget) -> String {
        let cards = self.cards.borrow();
        let mut current = Some(widget.clone());
        while let Some(node) = current {
            if let Some(card) = cards.iter().find(|c| c.widget.upgrade().as_ref() == Some(&node)) {
                return card.title.clone();
            }
            current = node.parent();
        }
        String::new()
    }

    /// The stops in the order of the page.
    fn ordered(&self) -> Vec<Placed> {
        let Some(content) = self.content() else { return vec![] };
        let stops = self.stops.borrow();
        let mut placed: Vec<Placed> = stops
            .iter()
            .enumerate()
            .filter_map(|(index, stop)| {
                let widget = stop.widget.upgrade()?;
                if !widget.is_drawable() {
                    return None;
                }
                let bounds = widget.compute_bounds(&content)?;
                Some(Placed {
                    index,
                    group: self.group_of(&widget),
                    x: bounds.x(),
                    y: bounds.y(),
                    mid_y: bounds.y() + bounds.height() / 2.0,
                })
            })
            .collect();
        // The place of each card is the place of its first stop.
        let mut origin: std::collections::HashMap<String, (i32, f32)> = Default::default();
        for p in &placed {
            let at = ((p.y / 24.0) as i32, p.x);
            let entry = origin.entry(p.group.clone()).or_insert(at);
            if at < *entry {
                *entry = at;
            }
        }
        placed.sort_by(|a, b| {
            if a.group != b.group {
                let (ga, gb) = (origin[&a.group], origin[&b.group]);
                return ga.partial_cmp(&gb).unwrap_or(std::cmp::Ordering::Equal).then(a.group.cmp(&b.group));
            }
            let (ra, rb) = ((a.mid_y / 8.0) as i32, (b.mid_y / 8.0) as i32);
            ra.cmp(&rb).then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
        });
        placed
    }

    /// Selects a stop by its ID. With `scroll`, the page scrolls to show it.
    pub fn select(&self, id: Option<&str>, scroll: bool) {
        let previous = self.selection.replace(id.map(str::to_string));
        if previous.as_deref() == id {
            return;
        }
        let stops = self.stops.borrow();
        for stop in stops.iter() {
            if let Some(widget) = stop.widget.upgrade() {
                if Some(stop.id.as_str()) == id {
                    widget.add_css_class("cursor-on");
                    if scroll {
                        self.scroll_to(&widget);
                    }
                } else {
                    widget.remove_css_class("cursor-on");
                }
            }
        }
        drop(stops);
        let handlers: Vec<_> = self.on_select.borrow().clone();
        for handler in handlers {
            handler(id);
        }
    }

    fn go(&self, placed: &[Placed], position: usize) {
        let id = self.stops.borrow()[placed[position].index].id.clone();
        self.select(Some(&id), true);
    }

    fn position(&self, placed: &[Placed]) -> Option<usize> {
        let selection = self.selection.borrow().clone()?;
        let stops = self.stops.borrow();
        placed.iter().position(|p| stops[p.index].id == selection)
    }

    pub fn move_by(&self, step: i32) {
        let placed = self.ordered();
        if placed.is_empty() {
            return;
        }
        match self.position(&placed) {
            Some(position) => {
                let next = (position as i32 + step).clamp(0, placed.len() as i32 - 1) as usize;
                self.go(&placed, next);
            }
            None => {
                // The first stop in view, or the last stop of the page.
                let offset = self.scroller().map(|s| s.vadjustment().value() as f32).unwrap_or(0.0);
                let first = placed.iter().position(|p| p.y >= offset).unwrap_or(placed.len() - 1);
                self.go(&placed, first);
            }
        }
    }

    pub fn go_to_end(&self, last: bool) {
        let placed = self.ordered();
        if !placed.is_empty() {
            self.go(&placed, if last { placed.len() - 1 } else { 0 });
        }
    }

    /// The first stop of the next card, or of the card before.
    pub fn move_group(&self, forward: bool) {
        let placed = self.ordered();
        if placed.is_empty() {
            return;
        }
        let Some(index) = self.position(&placed) else { return self.move_by(1) };
        let group = placed[index].group.clone();
        if forward {
            let next = placed[index..].iter().position(|p| p.group != group).map(|o| index + o).unwrap_or(index);
            self.go(&placed, next);
        } else {
            let start = placed[..=index].iter().rposition(|p| p.group != group).map(|i| i + 1).unwrap_or(0);
            if start < index {
                return self.go(&placed, start);
            }
            if start == 0 {
                return;
            }
            let before = placed[start - 1].group.clone();
            let first = placed[..start].iter().rposition(|p| p.group != before).map(|i| i + 1).unwrap_or(0);
            self.go(&placed, first);
        }
    }

    pub fn open_selected(&self) -> bool {
        let Some(selection) = self.selection.borrow().clone() else { return false };
        let open = self.stops.borrow().iter().find(|s| s.id == selection).map(|s| s.open.clone());
        match open {
            Some(open) => {
                open();
                true
            }
            None => false,
        }
    }

    /// The titles of the cards of the page, from the top down.
    pub fn card_titles(&self) -> Vec<String> {
        let Some(content) = self.content() else { return vec![] };
        let cards = self.cards.borrow();
        let mut placed: Vec<(i32, f32, String)> = cards
            .iter()
            .filter_map(|card| {
                let widget = card.widget.upgrade()?;
                let bounds = widget.compute_bounds(&content)?;
                Some(((bounds.y() / 24.0) as i32, bounds.x(), card.title.clone()))
            })
            .collect();
        placed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)));
        let mut seen = std::collections::HashSet::new();
        placed.into_iter().map(|p| p.2).filter(|t| seen.insert(t.clone())).collect()
    }

    pub fn scroll_to_card(&self, title: &str) {
        let widget = self.cards.borrow().iter().find(|c| c.title == title).and_then(|c| c.widget.upgrade());
        if let (Some(widget), Some(content), Some(scroller)) = (widget, self.content(), self.scroller())
            && let Some(bounds) = widget.compute_bounds(&content)
        {
            scroller.vadjustment().set_value(bounds.y() as f64);
        }
    }

    pub fn scroll_to_top(&self) {
        if let Some(scroller) = self.scroller() {
            scroller.vadjustment().set_value(0.0);
        }
    }

    /// Scrolls the page so that the widget is in view.
    pub fn scroll_to(&self, widget: &gtk::Widget) {
        let (Some(content), Some(scroller)) = (self.content(), self.scroller()) else { return };
        let Some(bounds) = widget.compute_bounds(&content) else { return };
        let adjustment = scroller.vadjustment();
        let (top, bottom) = (bounds.y() as f64, (bounds.y() + bounds.height()) as f64);
        let (value, page) = (adjustment.value(), adjustment.page_size());
        if top < value {
            adjustment.set_value((top - 8.0).max(0.0));
        } else if bottom > value + page {
            adjustment.set_value(bottom - page + 8.0);
        }
    }
}
