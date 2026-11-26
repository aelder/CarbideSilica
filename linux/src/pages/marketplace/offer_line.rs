//! One offer on one line: the name, the price with a bar, when it was offered and when it
//! expires, the seller, and the links to each object that the offer refers to
//! (`OfferLine`).

use std::cell::RefCell;
use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;

use super::logic::{self, MarketItem};
use crate::bridge::{self, api::{self, OfferDetail, OfferKind, OfferLinkKind, Route}};
use crate::display;
use crate::format;
use crate::model::AppModel;
use crate::page::Page;
use crate::shared::currency_stores::{height_text, known_name};
use crate::theme::{draw, tone};
use crate::widgets::icons::{self, Icon};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{plate, PlateFill};
use crate::widgets::{link, row_light};

pub struct Line<'a> {
    pub item: &'a MarketItem,
    pub detail: Option<&'a OfferDetail>,
    pub bounds: (f64, f64),
    /// The ID of the cursor stop of the name.
    pub stop: String,
}

pub fn offer_line(page: &Rc<Page>, model: &Rc<AppModel>, line: Line) -> gtk::Box {
    let Line { item, detail, bounds, stop } = line;
    let offer = &item.offer;
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    row.add_css_class("market-offer");

    let open: Rc<dyn Fn()> = {
        let weak = Rc::downgrade(model);
        let (subject, identity, txid) = (offer.subject_id.clone(), item.is_identity(), offer.txid.clone());
        Rc::new(move || {
            let Some(model) = weak.upgrade() else { return };
            match (&subject, identity) {
                (Some(id), true) => model.open(Route::Identity { name: id.clone() }),
                _ => model.open(Route::Transaction { txid: txid.clone() }),
            }
        })
    };
    let ticker = |id: &str| known_name(id, model, None);
    let identity_name = |id: &str| identity_name(model, offer, id);

    // The name, its kind, the scripts it uses and what is exchanged.
    let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
    names.set_hexpand(true);
    names.set_valign(gtk::Align::Center);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    // The isolate marks keep the "@" after a name in a right-to-left script.
    let title = if item.is_identity() { format!("\u{2068}{}\u{2069}@", item.title) } else { item.title.clone() };
    let title_link = {
        let open = open.clone();
        link::link(&title, false, move || open())
    };
    title_link.add_css_class("market-title");
    title_link.add_css_class("value-link");
    if let Some(label) = link::label_of(&title_link) {
        label.set_wrap(true);
        label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        label.set_lines(2);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    }
    title_link.set_valign(gtk::Align::Center);
    title_link.set_tooltip_text(Some(offer.subject_id.as_deref().unwrap_or(&offer.txid)));
    {
        let open = open.clone();
        page.stop(&title_link, Some(stop), Some(offer.subject_id.clone().unwrap_or_else(|| offer.txid.clone())), move || open());
    }
    top.append(&title_link);
    if offer.kind != OfferKind::IdentityForSale {
        top.append(&plate(item.kind_label(), PlateFill::None));
    }
    names.append(&top);
    if let Some(traits) = item.traits.as_ref().filter(|t| matches!(t.kind, logic::NameKind::Other | logic::NameKind::Emoji)) {
        let words = if traits.scripts.is_empty() { "no letters".to_string() } else { traits.scripts.join(", ") };
        let popover = gtk::Popover::new();
        let points = link::link(&words, false, {
            let popover = popover.downgrade();
            move || {
                if let Some(popover) = popover.upgrade() {
                    if popover.is_visible() { popover.popdown() } else { popover.popup() }
                }
            }
        });
        points.add_css_class("label-link");
        if let Some(label) = link::label_of(&points) {
            label.add_css_class("caption-small");
        }
        points.set_tooltip_text(Some("Show the code points of this name"));
        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        column.set_margin_start(12);
        column.set_margin_end(12);
        column.set_margin_top(12);
        column.set_margin_bottom(12);
        let count = crate::page::text(
            &format!("{} character{}, code points:", traits.length, if traits.length == 1 { "" } else { "s" }),
            &["value"],
        );
        column.append(&count);
        let codes = crate::page::paragraph(&traits.code_points, &["mono", "value"]);
        codes.set_selectable(true);
        codes.set_max_width_chars(52);
        column.append(&codes);
        popover.set_child(Some(&column));
        popover.set_parent(&points);
        {
            // The popover goes with its link.
            let popover = popover.clone();
            points.connect_destroy(move |_| popover.unparent());
        }
        names.append(&points);
    }
    if offer.kind != OfferKind::IdentityForSale {
        let exchange = format!(
            "{} for {}",
            logic::asset(&offer.offered, identity_name, ticker),
            logic::asset(&offer.accepted, identity_name, ticker)
        );
        let text = crate::page::text(&exchange, &["caption-small", "secondary"]);
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.set_single_line_mode(true);
        names.append(&text);
    }
    row.append(&names);

    // The ask, with a bar on the scale of the price curve.
    let ask = gtk::Box::new(gtk::Orientation::Vertical, 3);
    ask.set_size_request(132, -1);
    ask.set_valign(gtk::Align::Center);
    let amount = crate::page::text(&logic::ask(offer, ticker), &["semibold", "tabular", "value"]);
    amount.set_ellipsize(gtk::pango::EllipsizeMode::End);
    amount.set_single_line_mode(true);
    ask.append(&amount);
    if let Some(price) = item.price {
        let share = logic::bar_share(price, bounds) as f32;
        let bar = Canvas::new(0, 4, move |_, snapshot, w, h| {
            draw::fill_rect(snapshot, 0.0, 0.0, (w * share).max(3.0), h, tone::RECORD);
        });
        bar.set_hexpand(true);
        ask.append(&bar);
    }
    row.append(&clamped(&ask, 132));

    // When it was offered and when it expires.
    let dates = gtk::Box::new(gtk::Orientation::Vertical, 1);
    dates.set_valign(gtk::Align::Center);
    if let Some(time) = detail.and_then(|d| d.block_time) {
        let offered = crate::page::text(&format!("offered {} ago", logic::age(time, format::now())), &["caption-small", "secondary"]);
        offered.set_tooltip_text(Some(&format::date(time)));
        offered.set_single_line_mode(true);
        offered.set_ellipsize(gtk::pango::EllipsizeMode::End);
        dates.append(&offered);
    }
    let expiry = crate::page::text(&expiry(model, offer.expiry_height), &["caption-small", "secondary"]);
    expiry.set_single_line_mode(true);
    expiry.set_ellipsize(gtk::pango::EllipsizeMode::End);
    dates.append(&expiry);
    row.append(&clamped(&dates, 166));

    // The seller.
    let seller: gtk::Widget = match detail.and_then(|d| d.holder.clone()) {
        Some(holder) => {
            let weak = Rc::downgrade(model);
            let target = holder.clone();
            let button = link::link(&format::short(&holder, 6, 4), false, move || {
                if let Some(model) = weak.upgrade() {
                    model.open(crate::route::holder(&target));
                }
            });
            button.set_tooltip_text(Some(&format!("The seller: {holder}")));
            button.set_valign(gtk::Align::Center);
            if let Some(label) = link::label_of(&button) {
                label.add_css_class("callout");
            }
            button.upcast()
        }
        None => crate::page::text(" ", &["callout"]).upcast(),
    };
    seller.set_valign(gtk::Align::Center);
    row.append(&clamped(&seller, 104));

    row.append(&links_menu(model, item));
    row_light::row_light(&row, Some(Box::new(move || open())));
    row
}

/// A column of a fixed width.
fn clamped(child: &impl IsA<gtk::Widget>, width: i32) -> crate::widgets::clamp::Clamp {
    let clamp = crate::widgets::clamp::Clamp::new(child, width);
    clamp.set_size_request(width, -1);
    clamp.set_valign(gtk::Align::Center);
    clamp
}

fn expiry(model: &Rc<AppModel>, height: u64) -> String {
    if height == 0 {
        return "no expiry".into();
    }
    let Some(tip) = model.state().status.as_ref().map(|s| s.blocks) else {
        return format!("expires at {}", height_text(height, None, model));
    };
    if height <= tip {
        return format!("expired at {}", height_text(height, None, model));
    }
    let left = logic::time_left((height - tip) as f64 / 60.0);
    if display::shows_dates() {
        format!("expires in {left}, {}", height_text(height, None, model))
    } else {
        format!("expires in {left}, at {}", format::integer(height))
    }
}

/// The objects that this offer refers to, in a menu: the funding transaction, each
/// identity and currency, and the addresses of the funding transaction once they are read.
fn links_menu(model: &Rc<AppModel>, item: &MarketItem) -> gtk::MenuButton {
    let button = gtk::MenuButton::new();
    button.add_css_class("frameless-menu");
    button.set_always_show_arrow(false);
    let mark = icons::icon(Icon::Link, 14, tone::RECORD);
    button.set_child(Some(&mark));
    button.set_tooltip_text(Some("The objects that this offer refers to"));
    button.update_property(&[gtk::accessible::Property::Label("Links")]);
    button.set_valign(gtk::Align::Center);
    let popover = gtk::Popover::new();
    popover.set_has_arrow(false);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    popover.set_child(Some(&list));
    button.set_popover(Some(&popover));

    let addresses: Rc<RefCell<Option<Vec<String>>>> = Rc::default();
    let offer = item.offer.clone();
    let fill: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::default();
    let fill_fn: Rc<dyn Fn()> = {
        let (weak, list, popover, addresses, fill) = (Rc::downgrade(model), list.clone(), popover.downgrade(), addresses.clone(), Rc::downgrade(&fill));
        Rc::new(move || {
            let Some(model) = weak.upgrade() else { return };
            crate::page::load::clear(&list);
            let item_button = |title: String, action: Box<dyn Fn(&Rc<AppModel>)>| {
                let entry = gtk::Button::with_label(&title);
                entry.add_css_class("flat");
                entry.add_css_class("menu-row");
                if let Some(label) = entry.child().and_downcast::<gtk::Label>() {
                    label.set_xalign(0.0);
                }
                let (weak, popover) = (Rc::downgrade(&model), popover.clone());
                entry.connect_clicked(move |_| {
                    if let Some(popover) = popover.upgrade() {
                        popover.popdown();
                    }
                    if let Some(model) = weak.upgrade() {
                        action(&model);
                    }
                });
                list.append(&entry);
            };
            let txid = offer.txid.clone();
            item_button("Funding Transaction".into(), Box::new(move |m| m.open(Route::Transaction { txid: txid.clone() })));
            for link in offer.links.iter().filter(|l| l.kind == OfferLinkKind::Identity) {
                let value = link.value.clone();
                let name = identity_name(&model, &offer, &value);
                item_button(format!("Identity {name}"), Box::new(move |m| m.open(Route::Identity { name: value.clone() })));
            }
            for link in offer.links.iter().filter(|l| l.kind == OfferLinkKind::Currency) {
                let value = link.value.clone();
                let name = known_name(&value, &model, None);
                item_button(format!("Currency {name}"), Box::new(move |m| m.open(Route::Currency { name: value.clone() })));
            }
            match addresses.borrow().clone() {
                Some(found) => {
                    for address in found {
                        let short = format::short(&address, 6, 4);
                        item_button(format!("Address {short}"), Box::new(move |m| m.open(crate::route::holder(&address))));
                    }
                }
                None => {
                    let (addresses, fill, txid) = (addresses.clone(), fill.clone(), offer.txid.clone());
                    item_button(
                        "Read Funding Addresses".into(),
                        Box::new(move |m| {
                            let Some(explorer) = m.explorer() else { return };
                            let (addresses, fill, txid) = (addresses.clone(), fill.clone(), txid.clone());
                            glib::spawn_future_local(async move {
                                let found = bridge::run(async move { explorer.offer_funding(vec![txid]).await }).await;
                                *addresses.borrow_mut() = Some(found.into_iter().next().map(|f| f.addresses).unwrap_or_default());
                                if let Some(fill) = fill.upgrade().and_then(|f| f.borrow().clone()) {
                                    fill();
                                }
                            });
                        }),
                    );
                }
            }
        })
    };
    *fill.borrow_mut() = Some(fill_fn.clone());
    fill_fn();
    // The menu keeps its function for as long as the key lives.
    button.connect_destroy(move |_| {
        fill.borrow_mut().take();
    });
    button
}

fn identity_name(model: &Rc<AppModel>, offer: &api::Offer, id: &str) -> String {
    if !display::shows_names() {
        return format::short(id, 6, 4);
    }
    for side in [&offer.offered, &offer.accepted] {
        if side.identity_id.as_deref() == Some(id)
            && let Some(name) = &side.identity_name
        {
            return format!("{name}@");
        }
    }
    model.state().names.get(id).cloned().unwrap_or_else(|| format::short(id, 6, 4))
}
