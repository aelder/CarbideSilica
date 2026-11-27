//! The Currency page: a currency's definition and current state (`CurrencyView.swift`),
//! its values over time (`CurrencyOverTime.swift`), its state at a chosen block with
//! imports and exports (`CurrencyStateSeek.swift`), the last conversion batch
//! (`PoolBatch.swift`) and the notarization browser (`NotarizationLanes.swift`,
//! `NotarizationPicture.swift`).

mod basket;
mod graph;
mod notarizations;
mod over_time;
mod picture;
mod pool_batch;
mod state_seek;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{api, api::Route};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::{self, load::clear, LoadView, Page};
use crate::shared::currency_stores::{height_text, known_name};
use crate::shared::json_tree::json_section;
use crate::theme::{self, font};
use crate::widgets::display;
use crate::widgets::lamp;
use notarizations::NotarizationCard;
use over_time::OverTime;
use state_seek::StateCard;

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>, name: &str) -> Rc<Page> {
    let page = Page::scrolling(model);
    let route = name.to_string();
    let weak = Rc::downgrade(&page);
    let request = route.clone();
    let view = LoadView::new(
        model,
        move |explorer| {
            let name = request.clone();
            async move { explorer.currency(name).await }
        },
        move |holder, currency| {
            if let Some(page) = weak.upgrade() {
                show(&page, holder, &route, currency);
            }
        },
    );
    page.append(&view.root);
    let weak = Rc::downgrade(&view);
    page.hooks().refresh = Some(Rc::new(move || {
        if let Some(view) = weak.upgrade() {
            view.reload();
        }
    }));
    // The load view lives as long as the page.
    let keep = view.clone();
    page.root.connect_destroy(move |_| {
        let _ = &keep;
    });
    page
}

/// Body text in the size of SwiftUI's `.title3`: 15 against the 13 of the body.
pub fn title3(text: &str) -> gtk::Label {
    let label = theme::label(font::TITLE3, text, "value");
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label
}

/// A part that opens to show the data from the node, with a copy key at its right. The
/// contents are made the first time it opens, because the JSON of a large currency is long.
pub fn data_disclosure(
    open: bool,
    hint: fn(bool) -> &'static str,
    copy: &str,
    make: impl Fn() -> gtk::Widget + 'static,
    changed: impl Fn(bool) + 'static,
) -> gtk::Overlay {
    let label = page::text(hint(open), &["secondary"]);
    let content: Rc<RefCell<Option<gtk::Box>>> = Rc::default();
    let make = Rc::new(make);
    let fill = {
        let (content, make) = (content.clone(), make.clone());
        move |open: bool| {
            if let Some(holder) = content.borrow().as_ref()
                && open
                && holder.first_child().is_none()
            {
                holder.append(&make());
            }
        }
    };
    let disclosure = lamp::Disclosure::new("", open, {
        let (label, fill) = (label.clone(), fill.clone());
        move |open| {
            label.set_text(hint(open));
            fill(open);
            changed(open);
        }
    });
    disclosure.header.append(&label);
    *content.borrow_mut() = Some(disclosure.content.clone());
    fill(open);
    let copy = page::copy_button(copy);
    copy.set_halign(gtk::Align::End);
    copy.set_valign(gtk::Align::Start);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&disclosure.root));
    overlay.add_overlay(&copy);
    // The disclosure lives as long as its widgets.
    overlay.connect_destroy(move |_| {
        let _ = &disclosure;
    });
    overlay
}

/// Takes the keyboard focus from a key inside `part` before the part is built again. A
/// clicked key keeps the focus, and when it goes, the scroll view moves to the next key
/// that takes the focus: the page would jump under the pointer.
pub fn release_focus(part: &impl IsA<gtk::Widget>) {
    let Some(window) = part.as_ref().root().and_downcast::<gtk::Window>() else { return };
    if let Some(focus) = gtk::prelude::GtkWindowExt::focus(&window)
        && (focus == *part.as_ref() || focus.is_ancestor(part))
    {
        gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
    }
}

/// The JSON of the node, in the monospaced face, scrolling sideways.
pub fn json_text(json: &str) -> gtk::Widget {
    let text = page::text(json, &["mono", "callout"]);
    text.set_selectable(true);
    text.set_valign(gtk::Align::Start);
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
    scroller.set_propagate_natural_height(true);
    scroller.set_child(Some(&text));
    scroller.upcast()
}

/// Fills the page with the currency.
fn show(page: &Rc<Page>, holder: &gtk::Box, route: &str, currency: api::CurrencyDetail) {
    let model = page.model.clone();
    let column = gtk::Box::new(gtk::Orientation::Vertical, 20);
    holder.append(&column);
    let tags: Vec<(String, display::PlateFill)> = std::iter::once(currency.launch_status.clone())
        .chain(currency.option_tags.iter().cloned())
        .map(|tag| {
            let fill = display::signal_tag_fill(&tag);
            (tag, fill)
        })
        .collect();
    let header = gtk::Box::new(gtk::Orientation::Vertical, 6);
    header.append(&page::page_header(
        currency.fully_qualified_name.as_deref().unwrap_or(&currency.name),
        currency.currency_id.as_deref(),
        &tags,
    ));
    header.append(&page::provenance_note(&currency.provenance));
    column.append(&header);

    // The reserve that prices and values are shown in. None selects the default.
    let chosen_unit: Rc<RefCell<Option<String>>> = Rc::default();
    let unit: Rc<dyn Fn() -> Option<String>> = {
        let (chosen, weak, ids) = (
            chosen_unit.clone(),
            Rc::downgrade(&model),
            currency.reserves.iter().map(|r| r.currency_id.clone()).collect::<Vec<_>>(),
        );
        Rc::new(move || {
            let coin = weak.upgrade().and_then(|m| m.state().status.as_ref().and_then(|s| s.chain_id.clone()));
            basket::unit(&ids, chosen.borrow().as_deref(), coin.as_deref())
        })
    };

    // The definition, the current state and the reserves: built again when names, dates
    // or the unit change.
    let fixed = gtk::Box::new(gtk::Orientation::Vertical, 20);
    column.append(&fixed);
    let over_time_holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.append(&over_time_holder);

    let state_card = StateCard::new(page, &currency);
    let over_time = {
        let weak = Rc::downgrade(&state_card);
        OverTime::new(
            page,
            &currency,
            unit.clone(),
            Rc::new(move |height| {
                if let Some(card) = weak.upgrade() {
                    card.choose(height);
                }
            }),
        )
    };
    over_time_holder.append(&over_time.card.root);
    column.append(&state_card.card.root);
    let lanes = NotarizationCard::new(page, &currency, route);
    column.append(&lanes.card.root);
    let id = currency.currency_id.clone().unwrap_or_else(|| currency.name.clone());
    column.append(&json_section(page, &currency.json, &format!("currency-{id}.json")));

    // A new unit draws the reserves and the lines again, through a weak path to itself.
    let again: Rc<RefCell<Option<std::rc::Weak<dyn Fn()>>>> = Rc::default();
    let render: Rc<dyn Fn()> = {
        let (page, fixed, currency, unit, chosen) = (Rc::downgrade(page), fixed.downgrade(), currency.clone(), unit.clone(), chosen_unit.clone());
        let (over_time, again) = (Rc::downgrade(&over_time), again.clone());
        let menu_cache: Rc<MenuCache> = Rc::default();
        Rc::new(move || {
            let (Some(page), Some(fixed)) = (page.upgrade(), fixed.upgrade()) else { return };
            let changed: Rc<dyn Fn(String)> = {
                let (chosen, again, over_time) = (chosen.clone(), again.clone(), over_time.clone());
                Rc::new(move |id: String| {
                    *chosen.borrow_mut() = Some(id);
                    // Redraw after the menu has closed.
                    let (render, over_time) = (again.borrow().clone(), over_time.clone());
                    glib::idle_add_local_once(move || {
                        if let Some(render) = render.and_then(|r| r.upgrade()) {
                            render();
                        }
                        if let Some(over_time) = over_time.upgrade() {
                            over_time.render();
                        }
                    });
                })
            };
            fill_fixed(&page, &fixed, &currency, unit(), changed, &menu_cache);
        })
    };
    *again.borrow_mut() = Some(Rc::downgrade(&render));
    render();
    {
        let render = render.clone();
        model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &fixed, move |_| render());
    }
    // The chain's coin can be the default unit: draw again when the status first names it.
    {
        let (render, last) = (render.clone(), RefCell::new(None::<String>));
        model.watch(&[Topic::Status], &fixed, move |model| {
            let chain = model.state().status.as_ref().and_then(|s| s.chain_id.clone());
            if *last.borrow() != chain {
                *last.borrow_mut() = chain;
                render();
            }
        });
    }
    model.resolve_names(
        [currency.parent.clone(), currency.system_id.clone()]
            .into_iter()
            .flatten()
            .chain(currency.reserves.iter().map(|r| r.currency_id.clone()))
            .collect::<Vec<_>>(),
    );

    // The terminal's keys on a currency: g asks for a past state, n goes to the
    // notarizations.
    {
        let mut hooks = page.hooks();
        let (weak_card, cursor) = (Rc::downgrade(&state_card), page.cursor.clone());
        hooks.keys.insert(
            "g".into(),
            (
                "State at a Chosen Block".into(),
                Rc::new(move || {
                    if let Some(card) = weak_card.upgrade() {
                        card.focus();
                    }
                    cursor.scroll_to_card(state_seek::TITLE);
                }),
            ),
        );
        let cursor = page.cursor.clone();
        hooks.keys.insert(
            "n".into(),
            ("Go to Notarizations".into(), Rc::new(move || cursor.scroll_to_card(notarizations::TITLE))),
        );
    }

    // Another page asked for this currency's state at a height, such as a pool on a
    // transfer trace.
    let names: Vec<String> = [Some(route.to_string()), currency.currency_id.clone(), Some(currency.name.clone()), currency.fully_qualified_name.clone()]
        .into_iter()
        .flatten()
        .map(|n| request_key(&n))
        .collect();
    let take = {
        let (weak, page_weak, names) = (Rc::downgrade(&state_card), Rc::downgrade(page), names);
        move |model: &Rc<AppModel>| {
            let Some(request) = model.state().pending_currency_state.clone() else { return };
            if !names.contains(&request_key(&request.currency)) {
                return;
            }
            model.take_pending_currency_state();
            let Some(card) = weak.upgrade() else { return };
            card.choose(request.height);
            // Let the page lay out before it scrolls.
            let page_weak = page_weak.clone();
            glib::timeout_add_local_once(Duration::from_millis(150), move || {
                if let Some(page) = page_weak.upgrade() {
                    page.cursor.scroll_to_card(state_seek::TITLE);
                }
            });
        }
    };
    take(&model);
    model.watch(&[Topic::Path], &column, move |model| take(model));

    // The cards live as long as the column.
    let keep = (over_time, state_card, lanes, render);
    column.connect_destroy(move |_| {
        let _ = &keep;
    });
}

/// The form of a currency name for comparing requests: no spaces, no "@", lower case.
fn request_key(text: &str) -> String {
    text.trim().trim_matches('@').to_lowercase()
}

fn reserve_name(model: &AppModel, reserve: &api::ReserveRow) -> String {
    if crate::display::shows_names() {
        reserve.name.clone().unwrap_or_else(|| known_name(&reserve.currency_id, model, None))
    } else {
        format::short(&reserve.currency_id, 6, 4)
    }
}

/// The definition, the current state and the reserves.
/// The "Show values in" menu that the reserves card keeps, with its titles.
type MenuCache = RefCell<Option<(Vec<String>, Rc<lamp::Menu>)>>;

fn fill_fixed(
    page: &Rc<Page>,
    holder: &gtk::Box,
    currency: &api::CurrencyDetail,
    unit: Option<String>,
    changed: Rc<dyn Fn(String)>,
    menu_cache: &MenuCache,
) {
    clear(holder);
    let model = page.model.clone();
    let definition = page.card("Definition", None);
    definition.append(&page::field_row(page, "Name", &currency.name, None, false));
    let optional = |label: &str, value: &Option<String>, route: fn(String) -> Route| {
        if let Some(value) = value {
            definition.append(&page::field_row(page, label, value, Some(route(value.clone())), true));
        }
    };
    optional("Currency ID", &currency.currency_id, |name| Route::Identity { name });
    optional("Parent", &currency.parent, |name| Route::Currency { name });
    optional("System", &currency.system_id, |name| Route::Currency { name });
    if let Some(options) = currency.options {
        definition.append(&page::field_row(page, "Options", &options.to_string(), None, false));
    }
    if let Some(protocol) = currency.proof_protocol {
        definition.append(&page::field_row(page, "Proof protocol", &protocol.to_string(), None, false));
    }
    if let Some(start) = currency.start_block.filter(|s| *s > 0) {
        definition.append(&page::field_row(page, "Start block", &height_text(start, None, &model), Some(Route::Block { id: start.to_string() }), false));
    }
    if let Some(end) = currency.end_block.filter(|e| *e > 0) {
        definition.append(&page::field_row(page, "End block", &height_text(end, None, &model), None, false));
    }
    optional("Definition transaction", &currency.definition_txid, |txid| Route::Transaction { txid });
    holder.append(&definition.root);

    let current = page.card("Current state", None);
    current.append(&page::field_row(page, "Launch status", &currency.launch_status, None, false));
    if let Some(supply) = currency.supply {
        current.append(&page::field_row(page, "Supply", &format::amount(supply), None, false));
    }
    if let Some(height) = currency.last_confirmed_height {
        current.append(&page::field_row(page, "Last confirmed height", &height_text(height, None, &model), None, false));
    }
    holder.append(&current.root);

    if currency.reserves.is_empty() {
        return;
    }
    let count = currency.reserves.len();
    let reserves = page.card("Reserves", Some(&format!("{count} currencies")));
    let unit_reserve = currency.reserves.iter().find(|r| Some(&r.currency_id) == unit.as_ref());
    let unit_price = basket::positive(unit_reserve.and_then(|r| r.price_in_reserve));
    let unit_name = unit_reserve.map(|r| reserve_name(&model, r)).unwrap_or_default();
    let weights: Vec<Option<f64>> = currency.reserves.iter().map(|r| r.weight).collect();
    if let Some(sentence) = basket::sentence(&currency.name, &weights, unit_price, currency.supply, &unit_name) {
        let label = title3(&sentence);
        label.set_selectable(true);
        reserves.append(&label);
    }
    if count > 1 {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.append(&display::name_text("Show values in"));
        let titles: Vec<String> = currency.reserves.iter().map(|r| reserve_name(&model, r)).collect();
        let ids: Vec<String> = currency.reserves.iter().map(|r| r.currency_id.clone()).collect();
        let selected = ids.iter().position(|id| Some(id) == unit.as_ref()).unwrap_or(0);
        // The menu is kept while its titles stay the same: it is the parent of its open
        // list, which is still closing when the reserves are drawn again.
        let mut cache = menu_cache.borrow_mut();
        let menu = match cache.as_ref() {
            Some((kept, menu)) if *kept == titles => menu.clone(),
            _ => {
                let names: Vec<&str> = titles.iter().map(String::as_str).collect();
                let menu = lamp::Menu::new(&names, selected, move |index| {
                    if let Some(id) = ids.get(index) {
                        changed(id.clone());
                    }
                });
                *cache = Some((titles.clone(), menu.clone()));
                menu
            }
        };
        menu.set_selected(selected);
        if menu.button.parent().is_some() {
            menu.button.unparent();
        }
        row.append(&menu.button);
        reserves.append(&row);
    }
    let grid = gtk::Grid::new();
    grid.set_column_spacing(20);
    grid.set_row_spacing(8);
    let mut titles = vec!["Currency".to_string(), "Weight".into(), "In reserve".into(), "Price in reserve".into()];
    if unit_price.is_some() {
        titles.push(format!("Value in {unit_name}"));
        titles.push(format!("One is worth, in {unit_name}"));
    }
    for (column, title) in titles.iter().enumerate() {
        let name = display::name_text(title);
        name.set_hexpand(true);
        if column > 0 {
            name.set_xalign(1.0);
        }
        grid.attach(&name, column as i32, 0, 1, 1);
    }
    grid.attach(&display::rule(), 0, 1, titles.len() as i32, 1);
    for (index, reserve) in currency.reserves.iter().enumerate() {
        let row = index as i32 + 2;
        let names = crate::display::shows_names();
        let text = if names { reserve.name.clone().unwrap_or_else(|| reserve.currency_id.clone()) } else { reserve.currency_id.clone() };
        let link = page::route_link(page, &text, Route::Currency { name: reserve.currency_id.clone() }, !names || reserve.name.is_none());
        grid.attach(&link, 0, row, 1, 1);
        let mut values = vec![
            reserve.weight.map(format::percent).unwrap_or_else(|| "–".into()),
            reserve.reserves.map(format::amount).unwrap_or_else(|| "–".into()),
            reserve.price_in_reserve.map(format::amount).unwrap_or_else(|| "–".into()),
        ];
        if let Some(unit_price) = unit_price {
            values.push(
                currency
                    .supply
                    .zip(reserve.weight)
                    .map(|(supply, weight)| basket::text(supply * unit_price * weight))
                    .unwrap_or_else(|| "–".into()),
            );
            values.push(if Some(&reserve.currency_id) == unit.as_ref() {
                "1".into()
            } else {
                basket::worth(reserve.price_in_reserve, Some(unit_price)).map(basket::text).unwrap_or_else(|| "–".into())
            });
        }
        for (offset, value) in values.iter().enumerate() {
            let label = page::text(value, &["tabular"]);
            label.set_xalign(1.0);
            grid.attach(&label, offset as i32 + 1, row, 1, 1);
        }
    }
    reserves.append(&grid);
    let caption = format!(
        "\"Price in reserve\" is the number of units of the reserve for one unit of {}. {}",
        currency.name,
        if unit_price.is_none() {
            String::new()
        } else {
            format!(
                "Each reserve holds its weight of the value of the pool. The values in {unit_name} are the pool's own rates between its reserves. They are not market quotes."
            )
        }
    );
    reserves.append(&page::paragraph(&caption, &["caption-text", "secondary"]));
    holder.append(&reserves.root);
}
