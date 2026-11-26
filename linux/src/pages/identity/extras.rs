//! For an identity whose control is a token: where the token was issued, and who holds it
//! now (`TokenControlCard` in `IdentityExtras.swift`). Carbide follows the token in passes
//! of 16 transfers. A pass that reaches the limit stops, and the next pass continues from
//! the same output.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge;
use crate::bridge::api::{Confidence, Route, TokenControl, TokenControlState, TokenFrontier};
use crate::model::Topic;
use crate::page::{self, load::clear, Card, Page};
use crate::shared::confidence::confidence_mark;
use crate::shared::identity_support::{self as support, small_progress};
use crate::theme::{self, font, tone, Color};
use crate::widgets::icons::{self, Icon};
use crate::widgets::canvas::Canvas;
use crate::widgets::display;
use crate::widgets::lamp;


pub fn hop_text(hops: u32) -> String {
    format!("{hops} transfer{}", if hops == 1 { "" } else { "s" })
}

pub fn end_word(state: &TokenControlState) -> &'static str {
    match state {
        TokenControlState::Unspent { holder } => {
            if holder.is_none() { "unspent, no single holder" } else { "current holder" }
        }
        TokenControlState::Paused => "not finished",
        TokenControlState::Unresolved { .. } => "stopped",
    }
}

fn end_color(state: &TokenControlState) -> Color {
    match state {
        TokenControlState::Unspent { .. } => tone::GOOD,
        TokenControlState::Paused => tone::MINED,
        TokenControlState::Unresolved { .. } => tone::ALERT,
    }
}

pub struct TokenControlCard {
    pub card: Card,
    page: std::rc::Weak<Page>,
    identity_address: String,
    past_note: gtk::Box,
    body: gtk::Box,
    footer: gtk::Box,
    result: RefCell<Option<TokenControl>>,
    failure: RefCell<Option<String>>,
    running: Cell<bool>,
    /// Discards an answer to an earlier request after a new connection.
    attempt: Cell<u64>,
}

impl TokenControlCard {
    pub fn new(page: &Rc<Page>, identity_address: &str, viewing_past: bool) -> Rc<Self> {
        let card = page.card("Tokenized control", None);
        card.append(&page::text("The holder of the control token controls this identity.", &["secondary"]));
        let past_note = page::note("The trail shows the holder now, not at the viewed version.", false);
        card.append(&past_note);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        card.append(&body);
        let footer = gtk::Box::new(gtk::Orientation::Vertical, 12);
        card.append(&footer);
        let this = Rc::new(Self {
            card,
            page: Rc::downgrade(page),
            identity_address: identity_address.to_string(),
            past_note,
            body,
            footer,
            result: RefCell::new(None),
            failure: RefCell::new(None),
            running: Cell::new(false),
            attempt: Cell::new(0),
        });
        this.set_viewing_past(viewing_past);
        let weak = Rc::downgrade(&this);
        page.model.watch(&[Topic::Connection], &this.card.root, move |_| {
            if let Some(this) = weak.upgrade() {
                this.result.replace(None);
                this.run(None);
            }
        });
        let weak = Rc::downgrade(&this);
        page.model.watch(&[Topic::Names, Topic::Display], &this.card.root, move |_| {
            if let Some(this) = weak.upgrade() {
                this.render();
            }
        });
        this.run(None);
        this
    }

    pub fn set_viewing_past(&self, past: bool) {
        self.past_note.set_visible(past);
    }

    fn run(self: &Rc<Self>, resume: Option<TokenFrontier>) {
        let Some(page) = self.page.upgrade() else { return };
        let model = page.model.clone();
        let attempt = self.attempt.get() + 1;
        self.attempt.set(attempt);
        self.running.set(true);
        self.failure.replace(None);
        self.render();
        let explorer = match model.require_explorer() {
            Ok(explorer) => explorer,
            Err(message) => {
                self.running.set(false);
                self.failure.replace(Some(message));
                self.render();
                return;
            }
        };
        let id = self.identity_address.clone();
        let weak = Rc::downgrade(self);
        let generation = model.generation();
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.token_control(id, resume).await }).await;
            let Some(this) = weak.upgrade() else { return };
            if this.attempt.get() != attempt || model.generation() != generation {
                return;
            }
            this.running.set(false);
            match result {
                Ok(next) => {
                    if let TokenControlState::Unspent { holder: Some(holder) } = &next.state {
                        model.resolve_names([holder.clone()]);
                    }
                    this.result.replace(Some(next));
                }
                Err(error) => {
                    this.failure.replace(Some(bridge::describe(&error)));
                }
            }
            this.render();
        });
    }

    fn render(self: &Rc<Self>) {
        let Some(page) = self.page.upgrade() else { return };
        clear(&self.body);
        clear(&self.footer);
        let result = self.result.borrow().clone();
        if let Some(result) = &result {
            self.body.append(&trail(result));
            self.status(&page, result);
        }
        if self.running.get() {
            let row = small_progress("Checking…");
            row.set_spacing(10);
            self.footer.append(&row);
        } else if let Some(failure) = self.failure.borrow().clone() {
            self.footer.append(&page::note(&failure, true));
            let weak = Rc::downgrade(self);
            self.footer.append(&lamp::button("Try Again", move || {
                if let Some(this) = weak.upgrade() {
                    let resume = this.result.borrow().as_ref().and_then(|r| r.frontier.clone());
                    this.run(resume);
                }
            }));
        }
        self.body.set_visible(self.body.first_child().is_some());
        self.footer.set_visible(self.footer.first_child().is_some());
    }

    fn status(self: &Rc<Self>, page: &Rc<Page>, result: &TokenControl) {
        let running = self.running.get();
        let resume_key = |title: &str| {
            let weak = Rc::downgrade(self);
            let frontier = result.frontier.clone();
            let key = lamp::button(title, move || {
                if let Some(this) = weak.upgrade() {
                    this.run(frontier.clone());
                }
            });
            key.set_sensitive(!running);
            key
        };
        match &result.state {
            TokenControlState::Unspent { holder: Some(holder) } => {
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                row.append(&confidence_mark(Confidence::Verified, Some("Current token holder")));
                row.append(&support::id_link(page, holder, crate::route::holder(holder), false));
                self.body.append(&row);
                self.body.append(&page::text(&format!("{} checked.", hop_text(result.hops)), &["secondary"]));
            }
            TokenControlState::Unspent { holder: None } => {
                self.body.append(&confidence_mark(Confidence::Observed, Some("No single holder")));
                self.body.append(&page::paragraph(
                    &format!("The unspent output has no single holder address. {} checked.", hop_text(result.hops)),
                    &[],
                ));
            }
            TokenControlState::Paused => {
                self.body.append(&confidence_mark(Confidence::Observed, Some("Not finished")));
                self.body.append(&page::text(
                    &format!("{} checked. The trail continues past this pass.", hop_text(result.hops)),
                    &[],
                ));
                self.body.append(&resume_key(&format!("Follow up to {} more", result.hops_per_pass)));
            }
            TokenControlState::Unresolved { reason } => {
                self.body.append(&confidence_mark(Confidence::Unknown, Some("Unresolved")));
                let text = if result.frontier.is_none() {
                    reason.clone()
                } else {
                    format!("{} checked. {reason}.", hop_text(result.hops))
                };
                self.body.append(&page::paragraph(&text, &[]));
                self.body.append(&resume_key("Retry"));
            }
        }
        if let Some(frontier) = &result.frontier {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            row.add_css_class("callout");
            let name = display::name_text("Last output checked");
            name.set_valign(gtk::Align::Baseline);
            row.append(&name);
            row.append(&page::route_link(page, &frontier.txid, Route::Transaction { txid: frontier.txid.clone() }, true));
            let output = page::text(&format!("output {}", frontier.output), &["secondary", "callout"]);
            output.set_valign(gtk::Align::Baseline);
            row.append(&output);
            self.body.append(&row);
        }
    }
}

/// The trail as a line: where the token was issued, one tick per transfer followed, and
/// where it is now.
fn trail(result: &TokenControl) -> gtk::Overlay {
    let hops = result.hops as usize;
    let shown = hops.min(40);
    let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let issued = display::name_text("Issued");
    issued.set_valign(gtk::Align::Center);
    line.append(&issued);
    for _ in 0..shown {
        let dot = Canvas::new(6, 6, |_, snapshot, w, h| theme::draw::fill_rect(snapshot, 0.0, 0.0, w, h, tone::RECORD));
        dot.set_valign(gtk::Align::Center);
        line.append(&dot);
    }
    if hops > shown {
        let more = theme::label(font::FIGURE_SMALL, &format!("+{}", hops - shown), "label");
        more.set_valign(gtk::Align::Center);
        line.append(&more);
    }
    line.append(&icons::icon(Icon::ArrowRight, 13, tone::AXIS));
    let end = gtk::Label::new(None);
    end.set_markup(&super::role_markup(font::SMALL, end_word(&result.state), end_color(&result.state)));
    end.set_valign(gtk::Align::Center);
    line.append(&end);
    column.append(&line);
    column.append(&display::rule());
    let caption = theme::label(font::CAPTION, "Each dot is one transfer of the token that the trail followed.", "label");
    caption.set_wrap(true);
    column.append(&caption);
    display::panel(&column, 12)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_of_the_trail() {
        assert_eq!(hop_text(1), "1 transfer");
        assert_eq!(hop_text(16), "16 transfers");
        assert_eq!(end_word(&TokenControlState::Unspent { holder: None }), "unspent, no single holder");
        assert_eq!(end_word(&TokenControlState::Unspent { holder: Some("R".into()) }), "current holder");
        assert_eq!(end_word(&TokenControlState::Paused), "not finished");
        assert_eq!(end_word(&TokenControlState::Unresolved { reason: "x".into() }), "stopped");
    }
}
