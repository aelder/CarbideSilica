//! The Currencies page: every currency the node lists, filtered as Carbide's currency
//! browser filters them (`CurrenciesView.swift`).
//!
//! Each option flag cycles through Any, Show and Hide. A currency matches when it has all
//! the Show flags and none of the Hide flags, and its launch state is included. The default
//! view is liquidity pools with known failed launches hidden. The filters and the selection
//! last for the session.

/// The wrapping row, which the Marketplace uses too.
pub(crate) mod flow;
mod logic;
mod state;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api};
use crate::bridge::api::{CurrencyListing, FlagState, LaunchState, Route};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::load::clear;
use crate::page::{self, Card, Page, SourceLine};
use crate::shared::currency_stores::{height_text, known_name};
use crate::theme::{self, draw, font, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::clamp::Clamp;
use crate::widgets::display::{self, PlateFill, Readout};
use crate::widgets::{lamp, loader};
use flow::Flow;
use state::{CurrencySort, Store, LAUNCH_STATES};

/// The widths of the columns of the table.
const RESERVES_WIDTH: i32 = 250;
const SUPPLY_WIDTH: i32 = 150;
const START_WIDTH: i32 = 86;

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>) -> Rc<Page> {
    install_css();
    let page = Page::scrolling(model);
    let view = View::new(&page);
    view.apply();

    let listener: Rc<dyn Fn()> = {
        let weak = Rc::downgrade(&view);
        Rc::new(move || {
            if let Some(view) = weak.upgrade() {
                view.apply();
            }
        })
    };
    view.store.listen(&listener);
    // The page keeps the listener for as long as it lives.
    *view.listener.borrow_mut() = Some(listener);

    {
        let weak = Rc::downgrade(&view);
        model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &view.list, move |_| {
            if let Some(view) = weak.upgrade() {
                view.cache.borrow_mut().clear();
                view.shown_ids.borrow_mut().clear();
                view.render();
            }
        });
        let weak = Rc::downgrade(&view);
        model.watch(&[Topic::Connection], &view.list, move |_| {
            if let Some(view) = weak.upgrade()
                && view.list.is_mapped()
            {
                view.load(false);
            }
        });
    }
    {
        let mut hooks = page.hooks();
        let weak = Rc::downgrade(&view);
        hooks.refresh = Some(Rc::new(move || {
            if let Some(view) = weak.upgrade() {
                view.load(true);
            }
        }));
        // The page owns the view through this hook; the other closures hold it weakly.
        let owner = view.clone();
        hooks.shown = Some(Rc::new(move || {
            owner.render();
            owner.load(false);
            owner.scroll_to_selected();
        }));
    }
    page
}

struct View {
    page: Weak<Page>,
    model: Weak<AppModel>,
    store: Rc<Store>,
    listener: RefCell<Option<Rc<dyn Fn()>>>,
    source: SourceLine,
    shown: Readout,
    launched: Readout,
    prelaunch: Readout,
    failed: Readout,
    fractional: Readout,
    spinner: Canvas,
    refresh: gtk::Button,
    find: gtk::Entry,
    /// True while the page sets the text of the Find field itself.
    syncing: Cell<bool>,
    launch_keys: Vec<(LaunchState, gtk::Button)>,
    any_key: gtk::Button,
    pools_key: gtk::Button,
    each_key: gtk::Button,
    flags: gtk::Box,
    flag_keys: Vec<(api::OptionFlag, gtk::Button)>,
    body: gtk::Stack,
    failure: gtk::Box,
    headings: Vec<(CurrencySort, Canvas)>,
    list: gtk::Box,
    empty: gtk::Box,
    /// The positions of the rows on screen, in their order.
    visible: RefCell<Vec<usize>>,
    /// The IDs of the rows in the list now, so that a change that keeps them does nothing.
    shown_ids: RefCell<Vec<String>>,
    /// A row of the table for each currency that was on screen: the rule and the tile.
    cache: RefCell<HashMap<String, (gtk::Box, gtk::Box)>>,
}

impl View {
    fn new(page: &Rc<Page>) -> Rc<Self> {
        let model = page.model.clone();
        let store = state::store();

        let top = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let source = SourceLine::new();
        source.root.add_css_class("callout");
        top.append(&source.root);
        page.append(&top);

        let card = Card::new("Currencies", None, false);
        page.cursor.add_card("Currencies", &card.root);
        page.append(&card.root);

        // The console: the figures of the list and the keys that filter it.
        let console = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let figures = gtk::Box::new(gtk::Orientation::Horizontal, 26);
        figures.set_margin_start(18);
        figures.set_margin_end(18);
        figures.set_margin_top(12);
        figures.set_margin_bottom(12);
        let shown = Readout::new("Shown", "–", "", None);
        let launched = Readout::new("Launched", "–", "", None);
        let prelaunch = Readout::new("Prelaunch", "–", "", None);
        let failed = Readout::new("Failed", "–", "", None);
        let fractional = Readout::new("Fractional", "–", "", None);
        for readout in [&shown, &launched, &prelaunch, &failed, &fractional] {
            readout.root.set_valign(gtk::Align::Baseline);
            figures.append(&readout.root);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        figures.append(&spacer);
        let spinner = loader::small();
        spinner.set_valign(gtk::Align::Center);
        figures.append(&spinner);
        let refresh_slot: Rc<RefCell<Option<Weak<View>>>> = Rc::default();
        let refresh = {
            let slot = refresh_slot.clone();
            lamp::button("Refresh", move || {
                if let Some(view) = slot.borrow().as_ref().and_then(Weak::upgrade) {
                    view.load(true);
                }
            })
        };
        figures.append(&refresh);
        console.append(&figures);
        console.append(&display::rule());

        let grid = gtk::Grid::new();
        grid.set_column_spacing(16);
        grid.set_row_spacing(12);
        grid.set_margin_start(18);
        grid.set_margin_end(18);
        grid.set_margin_top(14);
        grid.set_margin_bottom(14);
        let grid_name = |text: &str, row: i32, top: bool| {
            let name = display::name_text(text);
            if top {
                // The first line of keys: the name sits at its middle.
                name.set_valign(gtk::Align::Start);
                name.set_margin_top(8);
            } else {
                name.set_valign(gtk::Align::Center);
            }
            grid.attach(&name, 0, row, 1, 1);
        };
        grid_name("Find", 0, false);
        let find = gtk::Entry::new();
        find.add_css_class("display-field");
        find.set_placeholder_text(Some("Name, ID or option"));
        find.set_size_request(320, -1);
        find.set_halign(gtk::Align::Start);
        grid.attach(&find, 1, 0, 1, 1);

        grid_name("Launch", 1, false);
        let launch_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let mut launch_keys = Vec::new();
        for launch in LAUNCH_STATES {
            let bit = api::launch_state_bit(launch);
            let store_ref = store.clone();
            let key = lamp::toggle(state::launch_word(launch), false, false, move || {
                store_ref.state.borrow_mut().filter.launch_mask ^= bit;
                store_ref.changed();
            });
            key.set_tooltip_text(Some("Click to include or to leave out the currencies in this state."));
            launch_row.append(&key);
            launch_keys.push((launch, key));
        }
        grid.attach(&launch_row, 1, 1, 1, 1);

        grid_name("Options", 2, true);
        let options = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let option_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let any_key = {
            let store = store.clone();
            lamp::toggle("Any", false, false, move || {
                {
                    let mut state = store.state.borrow_mut();
                    state.filter.required = 0;
                    state.filter.excluded = 0;
                }
                store.changed();
            })
        };
        let pools_key = {
            let store = store.clone();
            lamp::toggle("Liquidity pools", false, false, move || {
                {
                    let mut state = store.state.borrow_mut();
                    state.filter.required = 1;
                    state.filter.excluded = 0;
                }
                store.changed();
            })
        };
        let each_key = {
            let store = store.clone();
            lamp::toggle("Each option", false, false, move || {
                {
                    let mut state = store.state.borrow_mut();
                    state.show_flags = !state.show_flags;
                }
                store.changed();
            })
        };
        option_row.append(&any_key);
        option_row.append(&pools_key);
        option_row.append(&each_key);
        options.append(&option_row);

        // One key for each option. A click changes it: any, then show, then hide.
        let flags = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let flag_flow = Flow::new(4);
        let mut flag_keys = Vec::new();
        for flag in api::currency_option_flags() {
            let (store, bit) = (store.clone(), flag.bit);
            let key = lamp::toggle(&flag.name, false, false, move || {
                {
                    let mut state = store.state.borrow_mut();
                    state.filter = api::cycle_currency_flag(state.filter.clone(), bit);
                }
                store.changed();
            });
            flag_flow.append(&key);
            flag_keys.push((flag, key));
        }
        flags.append(&flag_flow);
        flags.append(&page::paragraph(
            "A lit key shows only the currencies with the option. A key with a line through it hides them. Click a key to change it.",
            &["caption-text", "label"],
        ));
        options.append(&flags);
        grid.attach(&options, 1, 2, 1, 1);
        console.append(&grid);
        card.append(&display::panel(&console, 0));

        // The list, or its progress.
        let body = gtk::Stack::new();
        body.set_vhomogeneous(false);
        body.set_hhomogeneous(false);
        let loading = loader::progress("Loading currencies…");
        loading.set_size_request(-1, 200);
        body.add_named(&loading, Some("loading"));
        let table = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let failure = gtk::Box::new(gtk::Orientation::Vertical, 0);
        failure.set_margin_bottom(12);
        table.append(&failure);
        let heading_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        heading_row.set_margin_start(8);
        heading_row.set_margin_end(8);
        heading_row.set_margin_bottom(8);
        table.append(&heading_row);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        table.append(&list);
        let empty = gtk::Box::new(gtk::Orientation::Vertical, 0);
        empty.append(&display::rule());
        let none = page::text("No currency is in these filters.", &["label"]);
        none.set_margin_top(8);
        none.set_margin_bottom(8);
        none.set_margin_start(8);
        none.set_margin_end(8);
        empty.append(&none);
        table.append(&empty);
        body.add_named(&table, Some("table"));
        card.append(&body);

        let mut headings = Vec::new();
        for column in [CurrencySort::Name, CurrencySort::Reserves, CurrencySort::Supply, CurrencySort::Start] {
            let (heading, arrow) = heading(column, store.clone());
            match column {
                CurrencySort::Name => {
                    heading.set_hexpand(true);
                    heading.set_halign(gtk::Align::Start);
                    heading_row.append(&heading);
                }
                CurrencySort::Reserves => heading_row.append(&fixed(&heading, RESERVES_WIDTH, gtk::Align::Start)),
                CurrencySort::Supply => heading_row.append(&fixed(&heading, SUPPLY_WIDTH, gtk::Align::End)),
                CurrencySort::Start => heading_row.append(&fixed(&heading, START_WIDTH, gtk::Align::End)),
            }
            headings.push((column, arrow));
        }

        let view = Rc::new(Self {
            page: Rc::downgrade(page),
            model: Rc::downgrade(&model),
            store: store.clone(),
            listener: RefCell::new(None),
            source,
            shown,
            launched,
            prelaunch,
            failed,
            fractional,
            spinner,
            refresh,
            find,
            syncing: Cell::new(false),
            launch_keys,
            any_key,
            pools_key,
            each_key,
            flags,
            flag_keys,
            body,
            failure,
            headings,
            list,
            empty,
            visible: RefCell::new(Vec::new()),
            shown_ids: RefCell::new(Vec::new()),
            cache: RefCell::new(HashMap::new()),
        });
        *refresh_slot.borrow_mut() = Some(Rc::downgrade(&view));
        {
            let weak = Rc::downgrade(&view);
            view.find.connect_changed(move |entry| {
                let Some(view) = weak.upgrade() else { return };
                if view.syncing.get() {
                    return;
                }
                view.store.state.borrow_mut().filter.text = entry.text().to_string();
                view.store.changed();
            });
        }
        view
    }

    /// Works out the rows in the filters and their order, then shows them.
    fn apply(&self) {
        let found = {
            let state = self.store.state.borrow();
            let found: Vec<usize> =
                api::filter_currencies(state.rows.clone(), state.filter.clone()).into_iter().map(|i| i as usize).collect();
            logic::order(&state.rows, found, state.sort, state.descending)
        };
        *self.visible.borrow_mut() = found;
        self.render();
    }

    fn render(&self) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(page) = self.page.upgrade() else { return };
        let state = self.store.state.borrow();
        let rows = &state.rows;
        let visible = self.visible.borrow().clone();

        self.source.set(state.provenance.as_ref(), Some(&model.current_chain()), true);

        self.shown.set("Shown", &logic::shown_text(visible.len(), rows.len()));
        self.launched.set("Launched", &logic::launch_count(rows, LaunchState::Launched));
        self.prelaunch.set("Prelaunch", &logic::launch_count(rows, LaunchState::Prelaunch));
        let failed = logic::launch_count(rows, LaunchState::Failed);
        self.failed.set("Failed", &failed);
        let alert = failed != "0";
        set_class(&self.failed.value, "alert", alert);
        set_class(&self.failed.value, "value", !alert);
        self.fractional.set("Fractional", &logic::fractional_count(rows));
        self.spinner.set_visible(state.loading);
        self.refresh.set_sensitive(!state.loading);

        if self.find.text().as_str() != state.filter.text {
            self.syncing.set(true);
            self.find.set_text(&state.filter.text);
            self.syncing.set(false);
        }
        for (launch, key) in &self.launch_keys {
            let bit = api::launch_state_bit(*launch);
            set_toggle(key, state::launch_word(*launch), state.filter.launch_mask & bit != 0, false);
        }
        set_toggle(&self.any_key, "Any", state.filter.required == 0 && state.filter.excluded == 0, false);
        set_toggle(&self.pools_key, "Liquidity pools", state.filter.required == 1 && state.filter.excluded == 0, false);
        set_toggle(&self.each_key, "Each option", state.show_flags, false);
        self.flags.set_visible(state.show_flags);
        if state.show_flags {
            for (flag, key) in &self.flag_keys {
                let flag_state = api::currency_flag_state(state.filter.clone(), flag.bit);
                set_toggle(key, &flag.name, flag_state == FlagState::Show, flag_state == FlagState::Hide);
            }
        }

        if state.loading && rows.is_empty() {
            self.body.set_visible_child_name("loading");
        } else {
            self.body.set_visible_child_name("table");
        }
        clear(&self.failure);
        if let Some(failure) = &state.failure {
            let text = if rows.is_empty() {
                failure.clone()
            } else {
                format!("The list did not refresh. This is the list from the earlier request. {failure}")
            };
            self.failure.append(&page::note(&text, true));
        }
        self.failure.set_visible(state.failure.is_some());

        for (column, arrow) in &self.headings {
            let active = state.sort == Some(*column);
            arrow.set_visible(active);
            let descending = state.descending;
            arrow.set_draw(move |_, snapshot, w, h| {
                let builder = gtk::gsk::PathBuilder::new();
                if descending {
                    builder.move_to(0.0, 0.0);
                    builder.line_to(w, 0.0);
                    builder.line_to(w / 2.0, h);
                } else {
                    builder.move_to(0.0, h);
                    builder.line_to(w, h);
                    builder.line_to(w / 2.0, 0.0);
                }
                builder.close();
                draw::fill(snapshot, &builder.to_path(), tone::PRESENT);
            });
        }

        // The rows. A row that was on screen before is the same widget, so a change of
        // filter does not move the page.
        let ids: Vec<String> = visible.iter().map(|&index| rows[index].currency_id.clone()).collect();
        if *self.shown_ids.borrow() != ids {
            clear(&self.list);
            let mut cache = self.cache.borrow_mut();
            for &index in &visible {
                let row = &rows[index];
                let (rule, tile) = cache
                    .entry(row.currency_id.clone())
                    .or_insert_with(|| {
                        let rule = display::rule();
                        let tile = currency_tile(&page, &model, &self.store, row);
                        (rule, tile)
                    })
                    .clone();
                self.list.append(&rule);
                self.list.append(&tile);
            }
            *self.shown_ids.borrow_mut() = ids;
        }
        for (id, (_, tile)) in self.cache.borrow().iter() {
            set_class(tile, "selected", state.selected.as_deref() == Some(id.as_str()));
        }
        self.empty.set_visible(visible.is_empty() && !rows.is_empty());
    }

    /// Loads once per connection. A failed refresh keeps the earlier list, as Carbide does.
    fn load(&self, force: bool) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(explorer) = model.explorer() else { return };
        let generation = model.generation();
        {
            let mut state = self.store.state.borrow_mut();
            if !force && state.generation == Some(generation) && !state.rows.is_empty() {
                return;
            }
            if state.loading && !force {
                return;
            }
            if state.generation != Some(generation) {
                state.rows.clear();
                state.source = None;
                state.provenance = None;
            }
            state.loading = true;
        }
        self.store.changed();
        let store = self.store.clone();
        let weak = Rc::downgrade(&model);
        glib::spawn_future_local(async move {
            let result = bridge::run(async move { explorer.currency_list().await }).await;
            let Some(model) = weak.upgrade() else { return };
            match result {
                Ok(list) => {
                    if generation != model.generation() {
                        return;
                    }
                    let mut state = store.state.borrow_mut();
                    state.rows = list.rows;
                    state.source = Some(list.source);
                    state.provenance = Some(list.provenance);
                    state.failure = None;
                    state.generation = Some(generation);
                }
                Err(error) => store.state.borrow_mut().failure = Some(bridge::describe(&error)),
            }
            store.state.borrow_mut().loading = false;
            store.changed();
            let reserves: Vec<String> =
                store.state.borrow().rows.iter().flat_map(|row| row.reserve_ids.iter().cloned()).collect();
            model.resolve_names(reserves);
        });
    }

    /// The list shows where the person was: the currency opened last, in the middle.
    fn scroll_to_selected(&self) {
        let Some(selected) = self.store.state.borrow().selected.clone() else { return };
        let Some(page) = self.page.upgrade() else { return };
        let Some(tile) = self.cache.borrow().get(&selected).map(|(_, tile)| tile.clone()) else { return };
        let (content, scroller) = (page.content.clone(), page.scroller.clone());
        glib::idle_add_local_once(move || {
            let Some(scroller) = scroller else { return };
            if let Some(bounds) = tile.compute_bounds(&content)
                && tile.is_drawable()
            {
                let adjustment = scroller.vadjustment();
                let middle = (bounds.y() + bounds.height() / 2.0) as f64;
                adjustment.set_value((middle - adjustment.page_size() / 2.0).max(0.0));
            }
        });
    }
}

/// The name of a column. A click puts the list in the order of the column, a second click
/// turns the order, and a third click gives the order of the source again.
fn heading(column: CurrencySort, store: Rc<Store>) -> (gtk::Box, Canvas) {
    let heading = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    heading.append(&display::name_text(column.title()));
    let arrow = Canvas::new(7, 6, |_, _, _, _| {});
    arrow.set_valign(gtk::Align::Center);
    arrow.set_visible(false);
    heading.append(&arrow);
    heading.set_tooltip_text(Some("Click to put the list in the order of this column."));
    let click = gtk::GestureClick::new();
    click.connect_released(move |gesture, _, _, _| {
        gesture.set_state(gtk::EventSequenceState::Claimed);
        {
            let mut state = store.state.borrow_mut();
            let (sort, descending) = logic::heading_click(state.sort, state.descending, column);
            state.sort = sort;
            state.descending = descending;
        }
        store.changed();
    });
    heading.add_controller(click);
    (heading, arrow)
}

/// One currency as a row of the table: the name with its state and options, the ID, the
/// reserves, the supply and the start block (`CurrencyTile`).
fn currency_tile(page: &Rc<Page>, model: &Rc<AppModel>, store: &Rc<Store>, row: &CurrencyListing) -> gtk::Box {
    let tile = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    tile.add_css_class("currencies-tile");
    tile.set_tooltip_text(Some(&format!("Open {}", row.name)));

    let left = gtk::Box::new(gtk::Orientation::Vertical, 5);
    left.set_hexpand(true);
    left.set_valign(gtk::Align::Start);
    let names = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let name = page::text(&row.name, &["value", "semibold"]);
    name.set_valign(gtk::Align::Baseline);
    name.set_single_line_mode(true);
    names.append(&name);
    let id = page::text(&row.currency_id, &["currencies-id"]);
    id.set_valign(gtk::Align::Baseline);
    id.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    id.set_hexpand(true);
    names.append(&id);
    left.append(&names);
    let plates = Flow::new(5);
    // Launched is the usual state: only a different state gets a plate.
    if row.launch != LaunchState::Launched {
        let fill = if row.launch == LaunchState::Failed { PlateFill::Alert } else { PlateFill::Mined };
        plates.append(&display::plate(state::launch_word(row.launch), fill));
    }
    for tag in &row.option_tags {
        plates.append(&display::plate(tag, PlateFill::None));
    }
    left.append(&plates);
    tile.append(&left);

    let reserves = logic::reserves_text(row, |id, fallback| known_name(id, model, fallback));
    let reserves = page::text(&reserves, &["callout", "currencies-reserves"]);
    if row.reserve_ids.is_empty() {
        reserves.add_css_class("empty");
    }
    reserves.set_wrap(true);
    reserves.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    reserves.set_lines(2);
    reserves.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let reserves = fixed(&reserves, RESERVES_WIDTH, gtk::Align::Start);
    reserves.set_valign(gtk::Align::Start);
    tile.append(&reserves);

    let supply = page::text(&row.supply.map(format::amount).unwrap_or_else(|| "–".into()), &["callout", "tabular", "value"]);
    supply.set_xalign(1.0);
    fit(&supply, SUPPLY_WIDTH);
    let supply = fixed(&supply, SUPPLY_WIDTH, gtk::Align::End);
    supply.set_valign(gtk::Align::Start);
    tile.append(&supply);

    let start = page::text(
        &row.start_block.map(|height| height_text(height, None, model)).unwrap_or_else(|| "–".into()),
        &["callout", "tabular", "label"],
    );
    start.set_xalign(1.0);
    fit(&start, START_WIDTH);
    let start = fixed(&start, START_WIDTH, gtk::Align::End);
    start.set_valign(gtk::Align::Start);
    tile.append(&start);

    let open: Rc<dyn Fn()> = {
        let (store, model, id) = (store.clone(), Rc::downgrade(model), row.currency_id.clone());
        Rc::new(move || {
            store.state.borrow_mut().selected = Some(id.clone());
            store.changed();
            if let Some(model) = model.upgrade() {
                model.open(Route::Currency { name: id.clone() });
            }
        })
    };
    let click = gtk::GestureClick::new();
    {
        let open = open.clone();
        click.connect_released(move |gesture, presses, _, _| {
            if presses == 1 {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                open();
            }
        });
    }
    tile.add_controller(click);
    page.stop(&tile, Some(row.currency_id.clone()), Some(row.currency_id.clone()), move || open());
    tile
}

/// A column of a fixed width. A text wider than the column wraps or is cut.
fn fixed(child: &impl IsA<gtk::Widget>, width: i32, align: gtk::Align) -> Clamp {
    child.set_halign(align);
    let clamp = Clamp::new(child, width);
    clamp.set_size_request(width, -1);
    clamp.set_hexpand(false);
    clamp
}

/// Makes a one-line figure smaller to fit its column, down to seven tenths of its size,
/// as `minimumScaleFactor(0.7)`. A text that still does not fit is cut.
fn fit(label: &gtk::Label, width: i32) {
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.connect_map(move |label| {
        label.set_attributes(None);
        let (_, natural, _, _) = label.measure(gtk::Orientation::Horizontal, -1);
        if natural > width {
            let scale = (width as f64 / natural as f64).max(0.7);
            let attributes = gtk::pango::AttrList::new();
            attributes.insert(gtk::pango::AttrFloat::new_scale(scale));
            label.set_attributes(Some(&attributes));
        }
    });
}

/// Lights a key made by `lamp::toggle`, and puts a line through its legend or takes it off.
fn set_toggle(key: &gtk::Button, title: &str, lit: bool, struck: bool) {
    lamp::set_chosen(key, lit);
    set_class(key, "struck", struck);
    if let Some(label) = key.child().and_downcast::<gtk::Label>() {
        let markup = theme::markup(font::KEY, title);
        label.set_markup(&if struck { format!("<s>{markup}</s>") } else { markup });
    }
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.add_css_class(class);
    } else {
        widget.remove_css_class(class);
    }
}

/// The styles of this page that `style.css` does not have: the light of a row under the
/// pointer and of the selected row, the ID and the reserves.
fn install_css() {
    thread_local! {
        static INSTALLED: Cell<bool> = const { Cell::new(false) };
    }
    if INSTALLED.with(|done| done.replace(true)) {
        return;
    }
    let Some(display) = gtk::gdk::Display::default() else { return };
    let css = format!(
        ".currencies-tile {{ padding: 9px 8px; background-color: transparent; }}\n\
         .currencies-tile:hover {{ background-color: {grid}; }}\n\
         .currencies-tile.selected {{ background-color: {selected}; box-shadow: inset 2px 0 0 {present}; }}\n\
         .currencies-id {{ font-family: \"IBM Plex Mono\"; font-size: 10px; color: {label}; }}\n\
         .currencies-reserves {{ color: {reserves}; }}\n\
         .currencies-reserves.empty {{ color: {label}; }}\n",
        grid = tone::GRID.css(),
        selected = tone::PRESENT.opacity(0.1).css(),
        present = tone::PRESENT.css(),
        label = tone::LABEL.css(),
        reserves = tone::VALUE.opacity(0.85).css(),
    );
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
}
