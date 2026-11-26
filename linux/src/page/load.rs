//! Loads a value from the bridge and shows progress, an error with Try Again, or the
//! content (`LoadView`). It loads again when the app connects again.

use std::cell::Cell;
use std::future::Future;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api, Explorer};
use crate::model::{AppModel, Topic};
use crate::theme::tone;
use crate::widgets::{icons, loader};

pub struct LoadView {
    /// The holder of the progress, the error or the content.
    pub root: gtk::Box,
    attempt: Cell<u64>,
    reload: std::cell::RefCell<Option<Rc<dyn Fn()>>>,
}

impl LoadView {
    /// Loads with `work` and shows the value with `show`. `show` gets the holder empty and
    /// fills it.
    pub fn new<T, Work, Fut, Show>(model: &Rc<AppModel>, work: Work, show: Show) -> Rc<Self>
    where
        T: Send + 'static,
        Work: Fn(Explorer) -> Fut + 'static,
        Fut: Future<Output = Result<T, api::CarbideError>> + Send + 'static,
        Show: Fn(&gtk::Box, T) + 'static,
    {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_hexpand(true);
        let view = Rc::new(Self { root, attempt: Cell::new(0), reload: Default::default() });
        let work = Rc::new(work);
        let show = Rc::new(show);
        let weak_view = Rc::downgrade(&view);
        let weak_model = Rc::downgrade(model);
        let reload: Rc<dyn Fn()> = Rc::new(move || {
            let (Some(view), Some(model)) = (weak_view.upgrade(), weak_model.upgrade()) else { return };
            let attempt = view.attempt.get() + 1;
            view.attempt.set(attempt);
            clear(&view.root);
            let progress = loader::progress("");
            progress.set_margin_top(40);
            progress.set_margin_bottom(40);
            view.root.append(&progress);
            let explorer = match model.require_explorer() {
                Ok(explorer) => explorer,
                Err(message) => {
                    view.show_failure(&message);
                    return;
                }
            };
            let (work, show) = (work.clone(), show.clone());
            let generation = model.generation();
            let weak_view = Rc::downgrade(&view);
            let weak_model = Rc::downgrade(&model);
            glib::spawn_future_local(async move {
                let result = bridge::run(work(explorer)).await;
                let (Some(view), Some(model)) = (weak_view.upgrade(), weak_model.upgrade()) else { return };
                if view.attempt.get() != attempt || model.generation() != generation {
                    return;
                }
                clear(&view.root);
                match result {
                    Ok(value) => show(&view.root, value),
                    Err(error) => view.show_failure(&bridge::describe(&error)),
                }
            });
        });
        *view.reload.borrow_mut() = Some(reload.clone());
        let again = reload.clone();
        model.watch(&[Topic::Connection], &view.root, move |_| again());
        reload();
        view
    }

    pub fn reload(&self) {
        let reload = self.reload.borrow().clone();
        if let Some(reload) = reload {
            reload();
        }
    }

    fn show_failure(self: &Rc<Self>, message: &str) {
        clear(&self.root);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 10);
        column.set_halign(gtk::Align::Center);
        column.set_margin_top(40);
        column.set_margin_bottom(40);
        let mark = icons::icon(icons::Icon::Warning, 28, tone::LABEL);
        mark.set_halign(gtk::Align::Center);
        column.append(&mark);
        let title = gtk::Label::new(Some("The data did not load"));
        title.add_css_class("dialog-title");
        column.append(&title);
        let detail = super::paragraph(message, &["secondary"]);
        detail.set_justify(gtk::Justification::Center);
        detail.set_xalign(0.5);
        detail.set_max_width_chars(70);
        column.append(&detail);
        let weak = Rc::downgrade(self);
        let again = crate::widgets::lamp::button("Try Again", move || {
            if let Some(view) = weak.upgrade() {
                view.reload();
            }
        });
        again.set_halign(gtk::Align::Center);
        column.append(&again);
        self.root.append(&column);
    }
}

pub fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}
