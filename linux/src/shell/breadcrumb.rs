//! The trail from the place in the sidebar to the open page, as the terminal's path line
//! (`Breadcrumb.swift`). Each step is a link that goes back to it.

use std::rc::Rc;

use gtk::prelude::*;

use crate::display;
use crate::model::{AppModel, Topic};
use crate::page::load::clear;
use crate::pages;
use crate::route;
use crate::theme::tone;
use crate::widgets::icons::{self, Icon};
use crate::widgets::link;

pub fn build(model: &Rc<AppModel>) -> gtk::Box {
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    bar.add_css_class("breadcrumb");
    let update = {
        let bar = bar.clone();
        move |model: &Rc<AppModel>| {
            clear(&bar);
            let path = model.path();
            let names = if display::shows_names() { model.state().names.clone() } else { Default::default() };
            let place = pages::spec(model.place()).title.to_string();
            let mut titles = vec![place];
            titles.extend(path.iter().map(|r| route::crumb_title(r, &names)));
            let last = titles.len() - 1;
            for (depth, title) in titles.into_iter().enumerate() {
                if depth > 0 {
                    bar.append(&icons::icon(Icon::Chevron, 12, tone::AXIS));
                }
                if depth == last {
                    let label = gtk::Label::new(Some(&title));
                    label.add_css_class("value");
                    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                    bar.append(&label);
                } else {
                    let weak = Rc::downgrade(model);
                    let button = link::link(&title, false, move || {
                        if let Some(model) = weak.upgrade() {
                            let mut path = model.path();
                            path.truncate(depth);
                            model.set_path(path);
                        }
                    });
                    bar.append(&button);
                }
            }
        }
    };
    update(model);
    model.watch(&[Topic::Path, Topic::Names, Topic::Display], &bar, update);
    bar
}
