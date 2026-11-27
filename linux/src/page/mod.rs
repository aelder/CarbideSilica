//! A page: what the window shows for a place or an object.
//!
//! A page module builds a [`Page`] and fills its column with cards. The page gives the
//! shell its keyboard cursor and its functions ([`PageHooks`]): the shell's keys, menus and
//! command list use them, so a page never changes a shared file to add a key or a command.
//! See `linux/PORT.md`, "How a page module is written".

pub mod components;
pub mod cursor;
pub mod load;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

pub use components::*;
pub use cursor::PageCursor;
#[allow(unused_imports)]
pub use load::LoadView;

use crate::model::AppModel;
use crate::theme::{self, font};
use crate::widgets::{clamp::Clamp, display};

/// A function of the page with no argument.
pub type Action = Rc<dyn Fn()>;

/// A setting of a page that a menu command turns on and off, such as Stream Blocks.
#[derive(Clone)]
pub struct Toggle {
    pub get: Rc<dyn Fn() -> bool>,
    pub set: Rc<dyn Fn(bool)>,
}

/// The previous and next object of the same kind, such as the block before this block or
/// the identity version before this one (`AdjacentObjects`).
#[derive(Clone)]
pub struct Adjacent {
    pub noun: String,
    pub previous: Option<Action>,
    pub next: Option<Action>,
}

/// The investigations a transaction or identity page offers (`InvestigationActions`).
#[derive(Clone, Default)]
pub struct Investigations {
    /// Follow a transfer (the terminal's F8).
    pub follow_transfer: Option<Action>,
    /// Witnesses of notarization evidence (F4).
    pub witnesses: Option<Action>,
    /// References to and from other outputs (F3).
    pub protocol_links: Option<Action>,
    /// The blocks that hold the transaction, across side branches (F7).
    pub inclusions: Option<Action>,
    /// Earlier identities that shared addresses or authorities (the terminal's `v`).
    pub shared_authorities: Option<Action>,
}

/// Commands for the visible place: refresh, and the place's own toggles (`PlaceActions`).
#[derive(Clone, Default)]
pub struct PlaceActions {
    /// Dashboard: keep new blocks in view.
    pub stream_blocks: Option<Toggle>,
    /// Mempool: tiles or list.
    pub mempool_as_list: Option<Toggle>,
    /// Node: peer commands. Each opens the same confirmation as its button.
    pub add_node: Option<Action>,
    pub ping_all: Option<Action>,
    pub clear_bans: Option<Action>,
    pub show_map: Option<Toggle>,
}

/// A line that a page adds to the command list, in its group "This page".
#[derive(Clone)]
pub struct Command {
    pub title: String,
    /// The key that does the same, or an empty text.
    pub key: String,
    pub run: Action,
}

/// What a page gives to the keys, the menus and the command list.
#[derive(Default)]
pub struct PageHooks {
    /// Read the page again (r, F5, Ctrl-R). None: the shell refreshes the chain status.
    pub refresh: Option<Action>,
    pub adjacent: Option<Adjacent>,
    pub investigations: Investigations,
    pub place: PlaceActions,
    /// Keys of this page alone, by the character that the key types. They come before the
    /// keys of the window.
    pub keys: BTreeMap<String, (String, Action)>,
    /// More lines for the command list.
    pub commands: Vec<Command>,
    /// Shows or hides the raw JSON of the page (Tab). None for a page with no JSON.
    pub toggle_json: Option<Action>,
    /// Saves the raw JSON, while the page shows it (s).
    pub save_json: Option<Action>,
    /// Called each time the page comes on screen, and each time it goes off.
    pub shown: Option<Action>,
    pub hidden: Option<Action>,
}

/// One page of the window.
pub struct Page {
    pub model: Rc<AppModel>,
    /// The widget that the shell shows.
    pub root: gtk::Widget,
    /// The column of the page: cards go here, 20 apart.
    pub content: gtk::Box,
    pub scroller: Option<gtk::ScrolledWindow>,
    pub cursor: Rc<PageCursor>,
    pub hooks: RefCell<PageHooks>,
}

impl Page {
    /// A scrolling page: one scroll view for the whole page, a column 1,100 wide at most,
    /// 24 from the edges, with 20 between its parts (`Page` and `PlacePage`). A place page
    /// is kept for the session, so it keeps its scroll position.
    pub fn scrolling(model: &Rc<AppModel>) -> Rc<Self> {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 20);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let clamp = Clamp::new(&content, 1100 + 48);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_child(Some(&clamp));
        scroller.set_vexpand(true);
        scroller.set_hexpand(true);
        let cursor = PageCursor::new();
        cursor.attach(&content, Some(&scroller));
        Rc::new(Self {
            model: model.clone(),
            root: scroller.clone().upcast(),
            content,
            scroller: Some(scroller),
            cursor,
            hooks: RefCell::new(PageHooks::default()),
        })
    }

    /// A page that lays out its own area with no scroll view, such as the charts. The
    /// cursor keys are off: the page has its own keys.
    pub fn fixed(model: &Rc<AppModel>) -> Rc<Self> {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_vexpand(true);
        content.set_hexpand(true);
        let cursor = PageCursor::new();
        cursor.attach(&content, None);
        cursor.enabled.set(false);
        Rc::new(Self {
            model: model.clone(),
            root: content.clone().upcast(),
            content,
            scroller: None,
            cursor,
            hooks: RefCell::new(PageHooks::default()),
        })
    }

    pub fn append(&self, widget: &impl IsA<gtk::Widget>) {
        self.content.append(widget);
    }

    /// A titled section of the page, set off by a rule above its heading (`Card`).
    pub fn card(&self, title: &str, trailing: Option<&str>) -> Card {
        let card = Card::new(title, trailing, true);
        self.cursor.add_card(title, &card.root);
        card
    }

    /// Changes the hooks.
    pub fn hooks(&self) -> std::cell::RefMut<'_, PageHooks> {
        self.hooks.borrow_mut()
    }

    /// Makes `widget` a stop of the cursor. See [`PageCursor::add_stop`].
    pub fn stop(&self, widget: &impl IsA<gtk::Widget>, id: Option<String>, copy: Option<String>, open: impl Fn() + 'static) -> String {
        self.cursor.add_stop(widget, id, copy, Rc::new(open))
    }

    /// One row of a keyboard list (`SelectableRow`). A click selects it, a double-click
    /// opens it. Links in the row still open on one click.
    pub fn selectable_row(&self, id: &str, row: &impl IsA<gtk::Widget>, copy: Option<String>, open: impl Fn() + 'static) {
        let row = row.as_ref();
        row.set_margin_start(0);
        let open: Action = Rc::new(open);
        self.cursor.add_stop(row, Some(id.to_string()), copy, open.clone());
        let click = gtk::GestureClick::new();
        let (cursor, id) = (Rc::downgrade(&self.cursor), id.to_string());
        // The second press of a double-click is the row's: a list that holds the row, such
        // as a FlowBox, otherwise took it for its own activation and the row did not open.
        click.connect_pressed(|gesture, presses, _, _| {
            if presses == 2 {
                gesture.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        click.connect_released(move |_, presses, _, _| {
            if let Some(cursor) = cursor.upgrade() {
                cursor.select(Some(&id), false);
                if presses == 2 {
                    open();
                }
            }
        });
        row.add_controller(click);
    }

    /// Runs `tick` every `interval` while the page is on screen, and once at once.
    pub fn every(&self, interval: Duration, tick: impl Fn() + 'static) {
        every(&self.root, interval, tick);
    }
}

/// Runs `tick` every `interval` while `widget` is on screen. The timer stops when the
/// widget is gone. Ordinary data redraws use about one second.
pub fn every(widget: &impl IsA<gtk::Widget>, interval: Duration, tick: impl Fn() + 'static) {
    let weak = widget.upcast_ref::<gtk::Widget>().downgrade();
    tick();
    glib::timeout_add_local(interval, move || match weak.upgrade() {
        Some(widget) => {
            if widget.is_mapped() {
                tick();
            }
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
}

/// A titled section of a page, set off by a rule above its heading.
///
/// Sections are not boxed. A fill and a rounded border mark something as a separate
/// object, so they are kept for content that is one, such as a block of raw bytes.
pub struct Card {
    pub root: gtk::Box,
    /// The contents, 12 apart.
    pub content: gtk::Box,
    pub title: String,
    pub trailing: gtk::Label,
}

impl Card {
    pub fn new(title: &str, trailing: Option<&str>, shows_title: bool) -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_hexpand(true);
        root.append(&display::rule());
        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        heading.set_margin_top(2);
        let name = theme::label(font::HEADING, title, "value");
        name.set_hexpand(true);
        name.set_valign(gtk::Align::Baseline);
        let trailing_label = display::name_text(trailing.unwrap_or(""));
        trailing_label.set_valign(gtk::Align::Baseline);
        trailing_label.set_visible(trailing.is_some());
        heading.append(&name);
        heading.append(&trailing_label);
        heading.set_visible(shows_title);
        root.append(&heading);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.append(&content);
        Self { root, content, title: title.to_string(), trailing: trailing_label }
    }

    pub fn append(&self, widget: &impl IsA<gtk::Widget>) {
        self.content.append(widget);
    }

    pub fn set_trailing(&self, text: Option<&str>) {
        match text {
            Some(text) => {
                theme::set_label(&self.trailing, font::SMALL, text);
                self.trailing.set_visible(true);
            }
            None => self.trailing.set_visible(false),
        }
    }
}

/// The page of a place or an object that the Linux app does not show yet. It names the
/// Swift file that a port must follow.
pub fn not_ported(model: &Rc<AppModel>, title: &str, swift: &str) -> Rc<Page> {
    let page = Page::scrolling(model);
    let card = page.card(title, None);
    card.append(&note("This page is not ported yet.", false));
    let source = gtk::Label::new(Some(&format!("The macOS app shows this page in {swift}.")));
    source.set_xalign(0.0);
    source.add_css_class("secondary");
    source.add_css_class("callout");
    card.append(&source);
    page.append(&card.root);
    page
}
