//! Changes to the node's peer connections (`PeerControls.swift`). Each change waits for a
//! confirmation. These are the only things the app can change on the node.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::feed::PeerFeed;
use crate::bridge::{self, api};
use crate::model::AppModel;
use crate::page::{self, Card, Page};
use crate::shell::dialogs;
use crate::widgets::{lamp, loader};

/// A change that waits for its confirmation.
#[derive(Clone)]
pub struct Pending {
    pub title: String,
    pub detail: String,
    pub button: String,
    pub action: api::PeerAction,
    pub done: String,
}

pub fn clear_bans_pending(count: usize) -> Pending {
    Pending {
        title: "Clear the ban list?".into(),
        detail: if count == 1 {
            "The banned address can connect to the node again.".into()
        } else {
            format!("All {count} banned addresses can connect to the node again.")
        },
        button: "Clear Ban List".into(),
        action: api::PeerAction::ClearBans,
        done: "The node cleared its ban list.".into(),
    }
}

pub fn disconnect_pending(address: &str) -> Pending {
    Pending {
        title: format!("Disconnect {address}?"),
        detail: "The node closes this connection. The peer can connect again.".into(),
        button: "Disconnect".into(),
        action: api::PeerAction::Disconnect { address: address.to_string() },
        done: format!("The node disconnects {address}."),
    }
}

pub fn ban_pending(address: &str) -> Pending {
    Pending {
        title: format!("Ban {address}?"),
        detail: "The node refuses connections from this IP address until the ban expires or you clear the ban list.".into(),
        button: "Ban".into(),
        action: api::PeerAction::Ban { ip: address.to_string() },
        done: format!("The node banned {address}."),
    }
}

pub const PING_DONE: &str = "The node sends a ping to all connected peers.";

/// The commands of the session's peer page and its peer windows.
pub struct PeerCommands {
    model: std::rc::Weak<AppModel>,
    feed: Rc<PeerFeed>,
    pub message: RefCell<Option<String>>,
    pub busy: Cell<bool>,
    /// Called when `busy` or `message` changes.
    changed: RefCell<Vec<Box<dyn Fn()>>>,
}

impl PeerCommands {
    pub fn new(model: &Rc<AppModel>, feed: &Rc<PeerFeed>) -> Rc<Self> {
        Rc::new(Self {
            model: Rc::downgrade(model),
            feed: feed.clone(),
            message: RefCell::new(None),
            busy: Cell::new(false),
            changed: RefCell::new(Vec::new()),
        })
    }

    pub fn on_change(&self, handler: impl Fn() + 'static) {
        self.changed.borrow_mut().push(Box::new(handler));
    }

    fn notify(&self) {
        for handler in self.changed.borrow().iter() {
            handler();
        }
    }

    /// Asks for the confirmation of a change, over `parent`.
    pub fn confirm(self: &Rc<Self>, parent: &gtk::Window, pending: Pending) {
        let commands = Rc::downgrade(self);
        let (action, done) = (pending.action.clone(), pending.done.clone());
        dialogs::confirm(parent, &pending.title, &pending.detail, &pending.button, move || {
            if let Some(commands) = commands.upgrade() {
                commands.run(action.clone(), done.clone());
            }
        });
    }

    /// The prompt for a node to add.
    pub fn start_add_node(self: &Rc<Self>, parent: &gtk::Window) {
        let commands = Rc::downgrade(self);
        dialogs::ask_text(
            parent,
            "Add a node",
            "The node tries to keep a connection to this address.",
            "Host name or IP address, with an optional port",
            "Add",
            move |text| {
                let node = text.trim().to_string();
                if node.is_empty() {
                    return;
                }
                if let Some(commands) = commands.upgrade() {
                    commands.run(api::PeerAction::AddNode { address: node.clone() }, format!("The node added {node} to its node list."));
                }
            },
        );
    }

    /// Sends a change to the node. Only a confirmation, or Ping All, calls it.
    pub fn run(self: &Rc<Self>, action: api::PeerAction, done: String) {
        if self.busy.get() {
            return;
        }
        let Some(model) = self.model.upgrade() else { return };
        self.busy.set(true);
        self.notify();
        let commands = self.clone();
        glib::spawn_future_local(async move {
            let result = match model.require_explorer() {
                Ok(explorer) => bridge::run(async move { explorer.peer_action(action).await }).await.map_err(|e| bridge::describe(&e)),
                Err(message) => Err(message),
            };
            *commands.message.borrow_mut() = Some(match result {
                Ok(()) => done,
                Err(message) => message,
            });
            commands.busy.set(false);
            commands.notify();
            commands.feed.load_bans(&model).await;
            model.refresh_node().await;
        });
    }

    pub fn ping_all(self: &Rc<Self>) {
        self.run(api::PeerAction::PingAll, PING_DONE.into());
    }
}

/// The window of a widget, for its dialogs.
pub fn window_of(widget: &impl IsA<gtk::Widget>) -> Option<gtk::Window> {
    widget.as_ref().root().and_downcast::<gtk::Window>()
}

/// The commands for all peers: add a node, ping all peers and clear the ban list
/// (`PeerControlsCard`).
pub struct ControlsCard {
    pub card: Card,
    keys: gtk::Box,
    clear: gtk::Button,
    busy: gtk::Box,
    message: gtk::Label,
    banned: Rc<Cell<usize>>,
}

impl ControlsCard {
    pub fn new(page: &Rc<Page>, commands: &Rc<PeerCommands>) -> Rc<Self> {
        let card = page.card("Manage peers", None);
        let keys = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let banned = Rc::new(Cell::new(0usize));
        let root = card.root.clone();
        let c = Rc::downgrade(commands);
        let r = root.downgrade();
        keys.append(&lamp::button("Add Node…", move || {
            if let (Some(c), Some(window)) = (c.upgrade(), r.upgrade().and_then(|r| window_of(&r))) {
                c.start_add_node(&window);
            }
        }));
        let c = Rc::downgrade(commands);
        keys.append(&lamp::button("Ping All", move || {
            if let Some(c) = c.upgrade() {
                c.ping_all();
            }
        }));
        let (c, r, count) = (Rc::downgrade(commands), root.downgrade(), banned.clone());
        let clear = lamp::button("Clear Ban List…", move || {
            if let (Some(c), Some(window)) = (c.upgrade(), r.upgrade().and_then(|r| window_of(&r))) {
                c.confirm(&window, clear_bans_pending(count.get()));
            }
        });
        keys.append(&clear);
        let busy = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        busy.append(&loader::small());
        busy.set_visible(false);
        keys.append(&busy);
        card.append(&keys);
        let message = page::text("", &["callout", "secondary"]);
        message.set_wrap(true);
        message.set_visible(false);
        card.append(&message);
        let view = Rc::new(Self { card, keys, clear, busy, message, banned });
        let weak = Rc::downgrade(&view);
        let c = Rc::downgrade(commands);
        commands.on_change(move || {
            if let (Some(view), Some(c)) = (weak.upgrade(), c.upgrade()) {
                view.update(&c);
            }
        });
        view.update(commands);
        view
    }

    pub fn set_banned(&self, count: usize, commands: &PeerCommands) {
        self.banned.set(count);
        self.update(commands);
    }

    fn update(&self, commands: &PeerCommands) {
        let busy = commands.busy.get();
        self.keys.set_sensitive(!busy);
        self.busy.set_visible(busy);
        self.clear.set_sensitive(self.banned.get() > 0);
        match commands.message.borrow().as_ref() {
            Some(text) => {
                self.message.set_text(text);
                self.message.set_visible(true);
            }
            None => self.message.set_visible(false),
        }
    }
}
