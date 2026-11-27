//! The window shell (`CarbideSilicaApp.swift`, `ShellCommands.swift`, `ShellLists.swift`):
//! the title bar, the sidebar, the path bar, the navigation stack, the keys, the menus, the
//! command list and the dialogs.

pub mod breadcrumb;
pub mod commands;
pub mod dialogs;
pub mod keyboard_reference;
pub mod keys;
pub mod navigation;
pub mod palette;
pub mod settings_window;
pub mod sidebar;
pub mod titlebar;

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api::Route};
use crate::launch;
use crate::model::{AppModel, Topic};
use crate::pages;
use crate::route;
use crate::theme::sound;
use crate::widgets::grain;
use navigation::Navigator;
use palette::Palette;

pub struct Shell {
    pub model: Rc<AppModel>,
    pub window: gtk::ApplicationWindow,
    pub navigator: Rc<Navigator>,
    palette: RefCell<Option<Rc<Palette>>>,
    keys_window: RefCell<Option<gtk::Window>>,
    settings_window: RefCell<Option<gtk::Window>>,
}

/// The default size of the window: the size of the pictures in `macos/docs/images`.
pub const DEFAULT_WIDTH: i32 = 1281;
pub const DEFAULT_HEIGHT: i32 = 1084;

pub fn build(app: &gtk::Application) -> Rc<Shell> {
    let model = AppModel::new();
    let window = gtk::ApplicationWindow::new(app);
    window.set_title(Some("Carbide Silica"));
    window.set_default_size(DEFAULT_WIDTH, DEFAULT_HEIGHT);
    window.set_size_request(980, 560);
    window.add_css_class("display");

    let navigator = Navigator::new(&model);
    let shell = Rc::new(Shell {
        model: model.clone(),
        window: window.clone(),
        navigator: navigator.clone(),
        palette: Default::default(),
        keys_window: Default::default(),
        settings_window: Default::default(),
    });

    // The navigation stack follows the model first, so that the menus and the keys that
    // watch the same topic see the new page.
    let nav = navigator.clone();
    model.watch(&[Topic::Path], &window, move |_| nav.sync());
    let nav = Rc::downgrade(&navigator);
    model.shell.borrow_mut().reset_place = Some(Box::new(move |place| {
        if let Some(nav) = nav.upgrade() {
            nav.reset_place(place);
        }
    }));

    let menu = commands::install(app, &shell);
    let weak = Rc::downgrade(&shell);
    let open_settings: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(shell) = weak.upgrade() {
            shell.show_settings();
        }
    });
    let weak = Rc::downgrade(&shell);
    let bookmark: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(shell) = weak.upgrade() {
            shell.bookmark_current();
        }
    });
    window.set_titlebar(Some(&titlebar::build(&model, &menu, open_settings, bookmark)));

    // Sidebar | path bar over the pages.
    let detail = gtk::Box::new(gtk::Orientation::Vertical, 0);
    detail.append(&breadcrumb::build(&model));
    detail.append(&navigator.stack);
    detail.set_size_request(560, -1);
    let split = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    split.append(&sidebar::build(&model));
    split.append(&detail);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&split));
    // Film grain over all of the display.
    overlay.add_overlay(&grain::film_grain());
    let palette = Palette::new(&shell);
    palette.root.set_visible(false);
    overlay.add_overlay(&palette.root);
    *shell.palette.borrow_mut() = Some(palette);
    window.set_child(Some(&overlay));

    keys::install(&shell);
    dialogs::watch_permissions(&model, &window);
    let weak = Rc::downgrade(&shell);
    model.watch(&[Topic::Search], &window, move |model| {
        if let Some(shell) = weak.upgrade()
            && let Some(message) = model.take_search_error()
        {
            dialogs::alert(&shell.window, "Search", &message, || {});
        }
    });
    let weak = Rc::downgrade(&shell);
    model.watch(&[Topic::Bookmarks], &window, move |model| {
        let Some(shell) = weak.upgrade() else { return };
        let error = model.state().bookmark_error.clone();
        if let Some(error) = error
            && model.place() != crate::model::Place::Bookmarks
        {
            let reload = Rc::downgrade(model);
            dialogs::alert(&shell.window, "Bookmarks", &error, move || {
                if let Some(model) = reload.upgrade() {
                    model.reload_bookmarks();
                }
            });
        }
    });

    navigator.sync();
    window.present();
    sound::warm();
    pages::start_background(&model);
    model.connect();

    if let Some(problem) = launch::arguments().problem.clone() {
        dialogs::alert(
            &window,
            "Start-up arguments",
            &format!("The app did not use the start-up arguments. {problem}"),
            || {},
        );
    }
    shell
}

impl Shell {
    pub fn palette(&self) -> Option<Rc<Palette>> {
        self.palette.borrow().clone()
    }

    pub fn palette_shown(&self) -> bool {
        self.palette().is_some_and(|p| p.root.is_visible())
    }

    /// Opens the command list. While it is open, the page behind it takes no keys (see
    /// `keys.rs`) and no clicks: the shade of the list covers the window.
    pub fn open_palette(&self) {
        if let Some(palette) = self.palette() {
            palette.open();
        }
    }

    /// The command list closed: the keyboard goes back to the page.
    pub fn palette_closed(&self) {
        gtk::prelude::GtkWindowExt::set_focus(&self.window, None::<&gtk::Widget>);
    }

    /// Reads the visible page again: the page's own refresh, or the chain status.
    pub fn refresh(&self) {
        let action = self.navigator.current().and_then(|p| p.hooks.borrow().refresh.clone());
        match action {
            Some(action) => action(),
            None => {
                let model = self.model.clone();
                glib::spawn_future_local(async move { model.refresh().await });
            }
        }
    }

    /// Replaces the open block with its neighbour, so Back still returns to the page before.
    pub fn step(&self, next: bool) {
        let adjacent = self.navigator.current().and_then(|p| p.hooks.borrow().adjacent.clone());
        if let Some(adjacent) = adjacent {
            if let Some(action) = if next { adjacent.next } else { adjacent.previous } {
                action();
            }
            return;
        }
        let Some(Route::Block { id }) = self.model.state().path.last().cloned() else { return };
        let Some(explorer) = self.model.explorer() else { return };
        let model = self.model.clone();
        glib::spawn_future_local(async move {
            let asked = id.clone();
            let result = bridge::run(async move { explorer.adjacent_block(asked, next).await }).await;
            match result {
                Ok(Some(hash)) => {
                    if model.state().path.last() == Some(&Route::Block { id: id.clone() }) {
                        model.step(Route::Block { id: hash });
                    }
                }
                Ok(None) => model.set_search_error(
                    if next { "This block is the tip of the chain." } else { "This is the first block." }.into(),
                ),
                Err(error) => model.set_search_error(bridge::describe(&error)),
            }
        });
    }

    /// Asks for a description and saves the open object as a bookmark.
    pub fn bookmark_current(&self) {
        let Some(current) = self.model.state().path.last().cloned().filter(route::is_bookmarkable) else { return };
        let model = Rc::downgrade(&self.model);
        dialogs::ask_text(
            &self.window,
            "Bookmark this page",
            "The app saves the bookmark in the Carbide bookmark file. The terminal app uses the same file.",
            "Description (optional)",
            "Save",
            move |note| {
                if let Some(model) = model.upgrade() {
                    model.add_bookmark(&current, &note);
                }
            },
        );
    }

    /// Asks for a future height or date and opens the estimate.
    pub fn estimate(&self) {
        let model = Rc::downgrade(&self.model);
        dialogs::ask_text(
            &self.window,
            "Estimate a future block",
            "Enter a future block height such as 5000000, or a date such as 2028-11-08 or in 2 hours.",
            "Height or date",
            "Estimate",
            move |text| {
                let target = text.trim().to_string();
                if target.is_empty() {
                    return;
                }
                if let Some(model) = model.upgrade() {
                    model.search(format!("estimate:{target}"));
                }
            },
        );
    }

    pub fn show_keys(&self) {
        if let Some(window) = self.keys_window.borrow().as_ref().filter(|w| w.is_visible()) {
            window.present();
            return;
        }
        let window = keyboard_reference::window(&self.model, &self.window);
        window.present();
        *self.keys_window.borrow_mut() = Some(window);
    }

    pub fn show_settings(&self) {
        if let Some(window) = self.settings_window.borrow().as_ref().filter(|w| w.is_visible()) {
            window.present();
            return;
        }
        let window = settings_window::window(&self.model, &self.window);
        window.present();
        *self.settings_window.borrow_mut() = Some(window);
    }
}
