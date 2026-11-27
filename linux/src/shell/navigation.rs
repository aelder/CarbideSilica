//! The navigation stack (`NavigationStack` in `RootView` and `ShellState`): the place in
//! the sidebar with the pages of objects on top of it, Back and Forward.
//!
//! The pages below the top stay alive, so going back restores their selection and scroll
//! position. A place page is kept for the session, as the Swift app keeps its data in
//! shared stores and its scroll position in `PlaceMemory`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;

use crate::bridge::api::Route;
use crate::model::{AppModel, Place};
use crate::page::Page;
use crate::pages;

pub struct Navigator {
    model: Rc<AppModel>,
    pub stack: gtk::Stack,
    places: RefCell<HashMap<Place, Rc<Page>>>,
    pages: RefCell<Vec<(Route, Rc<Page>)>>,
    /// Pages left by going back, the next one last.
    forward: RefCell<Vec<Route>>,
    last_path: RefCell<Vec<Route>>,
    last_place: RefCell<Option<Place>>,
    current: RefCell<Option<Rc<Page>>>,
    serial: std::cell::Cell<u64>,
}

impl Navigator {
    pub fn new(model: &Rc<AppModel>) -> Rc<Self> {
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::None);
        stack.set_hexpand(true);
        stack.set_vexpand(true);
        Rc::new(Self {
            model: model.clone(),
            stack,
            places: Default::default(),
            pages: Default::default(),
            forward: Default::default(),
            last_path: Default::default(),
            last_place: Default::default(),
            current: Default::default(),
            serial: Default::default(),
        })
    }

    /// The page on screen.
    pub fn current(&self) -> Option<Rc<Page>> {
        self.current.borrow().clone()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.borrow().is_empty()
    }

    fn name(&self) -> String {
        let next = self.serial.get() + 1;
        self.serial.set(next);
        format!("page-{next}")
    }

    /// Builds again a place page the next time it shows, such as the charts at a new block.
    pub fn reset_place(&self, place: Place) {
        if let Some(page) = self.places.borrow_mut().remove(&place) {
            if self.current.borrow().as_ref().is_some_and(|c| Rc::ptr_eq(c, &page)) {
                *self.current.borrow_mut() = None;
            }
            self.stack.remove(&page.root);
        }
    }

    /// Brings the window in step with the model's place and path.
    pub fn sync(&self) {
        let path = self.model.path();
        let place = self.model.place();
        // Keeps the forward history in step with the path, however the path changed: a menu
        // command, a key, a link or a search.
        {
            let old = self.last_path.borrow().clone();
            let mut forward = self.forward.borrow_mut();
            if *self.last_place.borrow() != Some(place) {
                forward.clear();
            } else if path.len() < old.len() && old[..path.len()] == path[..] {
                forward.extend(old[path.len()..].iter().rev().cloned());
            } else if path.len() == old.len() + 1 && path[..old.len()] == old[..] && path.last() == forward.last() {
                forward.pop();
            } else if path != old {
                forward.clear();
            }
        }
        *self.last_path.borrow_mut() = path.clone();
        *self.last_place.borrow_mut() = Some(place);

        // Keep the pages of the common start of the path, build the rest.
        {
            let mut pages = self.pages.borrow_mut();
            let common = pages.iter().zip(path.iter()).take_while(|((a, _), b)| a == *b).count();
            for (_, page) in pages.drain(common..) {
                self.stack.remove(&page.root);
            }
            for route in &path[common..] {
                let page = pages::build_route(&self.model, route);
                self.stack.add_named(&page.root, Some(&self.name()));
                pages.push((route.clone(), page));
            }
        }
        let top = match self.pages.borrow().last() {
            Some((_, page)) => page.clone(),
            None => self.place_page(place),
        };
        let previous = self.current.replace(Some(top.clone()));
        if previous.as_ref().is_some_and(|p| Rc::ptr_eq(p, &top)) {
            return;
        }
        self.stack.set_visible_child(&top.root);
        if let Some(previous) = previous {
            let hidden = previous.hooks.borrow().hidden.clone();
            if let Some(hidden) = hidden {
                hidden();
            }
        }
        let shown = top.hooks.borrow().shown.clone();
        if let Some(shown) = shown {
            shown();
        }
    }

    fn place_page(&self, place: Place) -> Rc<Page> {
        if let Some(page) = self.places.borrow().get(&place) {
            return page.clone();
        }
        let page = (pages::spec(place).build)(&self.model);
        self.stack.add_named(&page.root, Some(&self.name()));
        self.places.borrow_mut().insert(place, page.clone());
        page
    }

    /// Goes back one page. At a place in the sidebar, goes to the dashboard, as Esc does in
    /// the terminal. Returns false when there is no page to go back to.
    pub fn go_back(&self) -> bool {
        let mut path = self.model.path();
        if !path.is_empty() {
            path.pop();
            self.model.set_path(path);
        } else if self.model.place() != Place::Dashboard {
            self.model.show(Place::Dashboard);
        } else {
            return false;
        }
        true
    }

    pub fn go_forward(&self) {
        let Some(next) = self.forward.borrow().last().cloned() else { return };
        let mut path = self.model.path();
        path.push(next);
        self.model.set_path(path);
    }
}
