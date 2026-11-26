//! Places to go, and the trail of what was opened this session (`Sidebar` in
//! `CarbideSilicaApp.swift`): lamp keys in the groups Watch, Explore, Library, Node and
//! Recent.

use std::rc::Rc;

use gtk::prelude::*;

use crate::display;
use crate::format;
use crate::model::{AppModel, Place, Topic};
use crate::page::load::clear;
use crate::pages::PLACES;
use crate::route;
use crate::widgets::display as parts;
use crate::widgets::lamp::LampKey;

/// The width of the sidebar, as the Swift app's ideal width.
pub const WIDTH: i32 = 210;

pub fn build(model: &Rc<AppModel>) -> gtk::ScrolledWindow {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.set_margin_top(6);
    let mut keys: Vec<(Place, Rc<LampKey>)> = Vec::new();
    let mut group_name = "";
    let mut group_box: Option<gtk::Box> = None;
    for spec in PLACES.iter() {
        if spec.group != group_name {
            group_name = spec.group;
            let (group, holder) = group(spec.group);
            column.append(&group);
            group_box = Some(holder);
        }
        let place = spec.place;
        let weak = Rc::downgrade(model);
        let key = Rc::new(LampKey::new(spec.key_title, 30, false, move || {
            if let Some(model) = weak.upgrade() {
                model.show(place);
            }
        }));
        key.button.set_margin_start(12);
        key.button.set_margin_end(12);
        key.button.set_margin_top(3);
        key.button.set_margin_bottom(3);
        key.button.set_tooltip_text(None);
        if let Some(holder) = &group_box {
            holder.append(&key.button);
        }
        keys.push((place, key));
    }
    let (recent_group, recent_holder) = group("Recent");
    column.append(&recent_group);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroller.set_child(Some(&column));
    scroller.add_css_class("sidebar");
    scroller.set_size_request(WIDTH, -1);
    // The rules in the groups expand. The sidebar keeps its width.
    scroller.set_hexpand(false);

    let keys = Rc::new(keys);
    // The open place has light. Nothing is lit while an object page is open on top of it.
    let update_keys = {
        let keys = keys.clone();
        move |model: &Rc<AppModel>| {
            let place = model.place();
            let path_empty = model.state().path.is_empty();
            for (key_place, key) in keys.iter() {
                key.set_chosen(path_empty && *key_place == place);
            }
            let state = model.state();
            let mempool = state.node.as_ref().and_then(|n| n.mempool_count).filter(|c| *c > 0).map(|c| c.to_string());
            let bookmarks = (!state.bookmarks.is_empty()).then(|| state.bookmarks.len().to_string());
            drop(state);
            let sync = model.sync_fraction().map(format::percent);
            for (key_place, key) in keys.iter() {
                match key_place {
                    Place::Mempool => key.set_figure(mempool.as_deref()),
                    Place::Bookmarks => key.set_figure(bookmarks.as_deref()),
                    Place::Node => key.set_figure(sync.as_deref()),
                    _ => {}
                }
            }
        }
    };
    update_keys(model);
    model.watch(&[Topic::Path, Topic::Node, Topic::Status, Topic::Bookmarks], &scroller, update_keys);

    // One object that was open in this session: a smaller key.
    let update_recent = {
        let (group, holder) = (recent_group.clone(), recent_holder.clone());
        move |model: &Rc<AppModel>| {
            clear(&holder);
            let recent = model.state().recent.clone();
            group.set_visible(!recent.is_empty());
            let names = if display::shows_names() { model.state().names.clone() } else { Default::default() };
            let last = model.state().path.last().cloned();
            for route in recent {
                let title = route::crumb_title(&route, &names);
                let weak = Rc::downgrade(model);
                let target = route.clone();
                let key = LampKey::new(&title, 24, true, move || {
                    if let Some(model) = weak.upgrade() {
                        model.set_path(vec![target.clone()]);
                    }
                });
                key.set_chosen(last.as_ref() == Some(&route));
                key.button.set_tooltip_text(Some(&route::title(&route)));
                key.button.set_margin_start(12);
                key.button.set_margin_end(12);
                key.button.set_margin_top(2);
                key.button.set_margin_bottom(2);
                holder.append(&key.button);
            }
        }
    };
    update_recent(model);
    model.watch(&[Topic::Recent, Topic::Names, Topic::Path, Topic::Display], &scroller, update_recent);
    scroller
}

/// A group of places, with its name above a thin rule. Returns the group and the box for
/// its keys.
fn group(name: &str) -> (gtk::Box, gtk::Box) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    group.set_margin_top(2);
    group.set_margin_bottom(16);
    let title = parts::name_text(name);
    title.set_margin_start(16);
    title.set_margin_end(16);
    title.set_margin_top(4);
    title.set_margin_bottom(6);
    group.append(&title);
    let rule = parts::rule();
    rule.set_margin_start(16);
    rule.set_margin_bottom(5);
    group.append(&rule);
    let keys = gtk::Box::new(gtk::Orientation::Vertical, 0);
    group.append(&keys);
    (group, keys)
}
