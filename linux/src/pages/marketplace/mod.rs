//! The Marketplace: open offers involving the chain's own coin, as Carbide requests them,
//! shown as a catalogue: an answer line, the prices as steps and as one curve, and a list
//! with filters that show their counts (`MarketplaceView.swift`).
//!
//! The offers on the chain are mostly identity names for sale at a few exact prices, so a
//! plot of points puts hundreds of offers on one spot. A person looks for a name, a kind of
//! name, a price or a seller. Each of those is a filter here, and each filter option shows
//! how many offers it leaves.

mod curve;
mod facet;
mod logic;
mod offer_line;

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::{Rc, Weak};

use gtk::glib;
use gtk::prelude::*;

use crate::bridge::{self, api};
use crate::bridge::api::{OfferDetail, OfferKey, OfferSort};
use crate::format;
use crate::model::{AppModel, Topic};
use crate::page::load::clear;
use crate::page::{self, Card, Page, SourceLine};
use crate::shared::currency_stores::known_name;
use crate::theme::{self, draw, font, lamp as lamp_tone, tone};
use crate::widgets::canvas::Canvas;
use crate::widgets::display::{self, Readout};
use crate::widgets::{lamp, loader};
use curve::Curve;
use logic::{Facet, Known, MarketItem, Values};

/// The sellers and listing dates of the session, so that a return to the page does not
/// read them again (`MarketStore`).
struct MarketStore {
    details: HashMap<String, OfferDetail>,
    generation: Option<u64>,
    loading: bool,
    problem: Option<String>,
}

thread_local! {
    static STORE: RefCell<MarketStore> =
        RefCell::new(MarketStore { details: HashMap::new(), generation: None, loading: false, problem: None });
}

fn with_store<T>(f: impl FnOnce(&mut MarketStore) -> T) -> T {
    STORE.with(|store| f(&mut store.borrow_mut()))
}

const SORTS: [(OfferSort, &str); 4] =
    [(OfferSort::AskAmount, "Ask amount"), (OfferSort::Name, "Name"), (OfferSort::Expiry, "Expiry"), (OfferSort::ListingAge, "Listing age")];

/// The rows that the catalogue shows first, and how many more each Show More adds.
const FIRST_ROWS: usize = 60;
const MORE_ROWS: usize = 100;

struct State {
    market: Option<api::Marketplace>,
    failure: Option<String>,
    loading: bool,
    items: Vec<MarketItem>,
    selected: BTreeMap<Facet, String>,
    search: String,
    sort: OfferSort,
    descending: bool,
    /// Positions in `items`, in the order of the list.
    ordered: Vec<usize>,
    shown: usize,
    /// Worked out once for each change of the data, not at each redraw: the option of each
    /// filter that each offer belongs to, the sellers, and how many offers have details.
    values: HashMap<String, Values>,
    sellers: Vec<(String, usize)>,
    detailed: usize,
}

/// The page. Its signature is fixed by `pages/mod.rs`.
pub fn build(model: &Rc<AppModel>) -> Rc<Page> {
    install_css();
    let page = Page::scrolling(model);
    let view = View::new(&page);
    view.render();
    {
        let weak = Rc::downgrade(&view);
        model.watch(&[Topic::Names, Topic::Display, Topic::HeightDates], &view.rows, move |_| {
            if let Some(view) = weak.upgrade() {
                view.render();
            }
        });
        // The expiry of each offer counts from the tip.
        let weak = Rc::downgrade(&view);
        model.watch(&[Topic::Status], &view.rows, move |model| {
            if let Some(view) = weak.upgrade() {
                let tip = model.state().status.as_ref().map(|s| s.blocks);
                if view.tip.replace(tip) != tip {
                    view.render_rows();
                }
            }
        });
        // A new connection: the page loads again, as the Swift view's task does.
        let weak = Rc::downgrade(&view);
        model.watch(&[Topic::Connection], &view.rows, move |_| {
            if let Some(view) = weak.upgrade()
                && view.rows.is_mapped()
            {
                view.start();
            }
        });
    }
    {
        let mut hooks = page.hooks();
        let weak = Rc::downgrade(&view);
        hooks.refresh = Some(Rc::new(move || {
            if let Some(view) = weak.upgrade() {
                glib::spawn_future_local(view.load());
            }
        }));
        // The page owns the view through this hook; the other closures hold it weakly.
        let owner = view.clone();
        hooks.shown = Some(Rc::new(move || owner.start()));
        let weak = Rc::downgrade(&view);
        hooks.hidden = Some(Rc::new(move || {
            // The reading of sellers and dates stops when the page goes, as the Swift
            // view's task is cancelled.
            if let Some(view) = weak.upgrade() {
                view.serial.set(view.serial.get() + 1);
            }
        }));
    }
    page
}

struct View {
    /// The page itself, for the keys made in a redraw.
    me: RefCell<Weak<View>>,
    page: Weak<Page>,
    model: Weak<AppModel>,
    state: RefCell<State>,
    /// Each start of the page's work has a number. Work with an older number stops.
    serial: Cell<u64>,
    tip: Cell<Option<u64>>,
    source: SourceLine,
    summary: gtk::Box,
    prices_card: Card,
    steps: gtk::Box,
    curve: Rc<Curve>,
    prices_note: gtk::Label,
    catalogue_card: Card,
    truncated: gtk::Box,
    facets: gtk::Box,
    search: gtk::Entry,
    result_count: gtk::Label,
    applied_row: gtk::Box,
    applied: gtk::Label,
    segments: lamp::Segments,
    reverse: gtk::Button,
    sort_note: gtk::Label,
    empty: gtk::Label,
    rows: gtk::Box,
    more: gtk::Box,
    smaller_card: Card,
    smaller_note: gtk::Label,
    smaller_rows: gtk::Box,
}

impl View {
    fn new(page: &Rc<Page>) -> Rc<Self> {
        let model = page.model.clone();
        let source = SourceLine::new();
        source.root.add_css_class("callout");
        page.append(&source.root);

        let open_card = Card::new("Open offers", None, false);
        page.cursor.add_card("Open offers", &open_card.root);
        let summary = gtk::Box::new(gtk::Orientation::Vertical, 12);
        open_card.append(&summary);
        page.append(&open_card.root);

        // The prices.
        let prices_card = page.card("Prices", Some("ask in"));
        let prices = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let columns = gtk::Box::new(gtk::Orientation::Horizontal, 26);
        let left = gtk::Box::new(gtk::Orientation::Vertical, 5);
        left.set_size_request(330, -1);
        left.set_hexpand(false);
        left.set_valign(gtk::Align::Start);
        left.append(&display::name_text("Prices that many offers share"));
        let rule = display::rule();
        rule.set_margin_bottom(3);
        left.append(&rule);
        let steps = gtk::Box::new(gtk::Orientation::Vertical, 5);
        left.append(&steps);
        columns.append(&left);
        let right = gtk::Box::new(gtk::Orientation::Vertical, 5);
        right.set_hexpand(true);
        right.append(&display::name_text("Offers at or below each price"));
        let rule = display::rule();
        rule.set_margin_bottom(3);
        right.append(&rule);
        let curve = Curve::new();
        right.append(&curve.canvas);
        right.append(&curve.readout);
        columns.append(&right);
        prices.append(&columns);
        prices.append(&display::rule());
        let prices_note = page::paragraph("", &["caption-text", "label"]);
        prices.append(&prices_note);
        prices_card.append(&display::panel(&prices, 16));
        page.append(&prices_card.root);

        // The catalogue: the filters at the left, the list at the right.
        let catalogue_card = page.card("Catalogue", None);
        let truncated = gtk::Box::new(gtk::Orientation::Vertical, 0);
        truncated.append(&page::note("The node returned more offers than the app keeps. The app shows the first 5,000.", true));
        catalogue_card.append(&truncated);
        let catalogue = gtk::Box::new(gtk::Orientation::Horizontal, 22);
        let facets = gtk::Box::new(gtk::Orientation::Vertical, 14);
        facets.set_size_request(232, -1);
        facets.set_valign(gtk::Align::Start);
        catalogue.append(&crate::widgets::clamp::Clamp::new(&facets, 232));
        let results = gtk::Box::new(gtk::Orientation::Vertical, 10);
        results.set_hexpand(true);
        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let search = gtk::Entry::new();
        search.add_css_class("display-field");
        search.set_placeholder_text(Some("Search the names"));
        search.set_size_request(240, -1);
        search_row.append(&search);
        let result_count = theme::label(font::FIGURE_SMALL, "", "value");
        result_count.set_valign(gtk::Align::Center);
        search_row.append(&result_count);
        results.append(&search_row);
        let applied_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let applied = page::text("", &["callout", "value"]);
        applied.set_wrap(true);
        applied.set_lines(2);
        applied.set_max_width_chars(120);
        applied.set_ellipsize(gtk::pango::EllipsizeMode::End);
        applied.set_valign(gtk::Align::Center);
        applied_row.append(&applied);
        let view_slot: Rc<RefCell<Weak<View>>> = Rc::new(RefCell::new(Weak::new()));
        let clear_key = {
            let slot = view_slot.clone();
            lamp::button("Clear Filters", move || {
                if let Some(view) = slot.borrow().upgrade() {
                    view.state.borrow_mut().selected.clear();
                    view.state.borrow_mut().shown = FIRST_ROWS;
                    view.search.set_text("");
                    view.reorder();
                    view.render_catalogue();
                }
            })
        };
        applied_row.append(&clear_key);
        results.append(&applied_row);
        let sort_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let sort_name = display::name_text("Sort by");
        sort_name.set_valign(gtk::Align::Center);
        sort_row.append(&sort_name);
        let titles: Vec<&str> = SORTS.iter().map(|s| s.1).collect();
        let segments = {
            let slot = view_slot.clone();
            lamp::Segments::new(&titles, 0, move |index| {
                if let Some(view) = slot.borrow().upgrade() {
                    {
                        let mut state = view.state.borrow_mut();
                        state.sort = SORTS[index].0;
                        state.shown = FIRST_ROWS;
                    }
                    view.reorder();
                    view.render_catalogue();
                }
            })
        };
        sort_row.append(&segments.root);
        let reverse = {
            let slot = view_slot.clone();
            lamp::toggle("Reverse", false, false, move || {
                if let Some(view) = slot.borrow().upgrade() {
                    {
                        let mut state = view.state.borrow_mut();
                        state.descending = !state.descending;
                        state.shown = FIRST_ROWS;
                    }
                    view.reorder();
                    view.render_catalogue();
                }
            })
        };
        reverse.set_margin_start(4);
        sort_row.append(&reverse);
        results.append(&sort_row);
        let sort_note = page::text("", &["caption-text", "label"]);
        results.append(&sort_note);
        let empty = page::text("", &["secondary"]);
        empty.set_margin_top(12);
        empty.set_margin_bottom(12);
        results.append(&empty);
        let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        results.append(&rows);
        let more = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        results.append(&more);
        catalogue.append(&results);
        catalogue_card.append(&catalogue);
        page.append(&catalogue_card.root);

        // The smaller sellers.
        let smaller_card = page.card("Offers of the smaller sellers", Some(""));
        let smaller_note = page::paragraph("", &["callout", "secondary"]);
        smaller_card.append(&smaller_note);
        let smaller_rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        smaller_card.append(&smaller_rows);
        page.append(&smaller_card.root);

        let view = Rc::new(Self {
            me: RefCell::new(Weak::new()),
            page: Rc::downgrade(page),
            model: Rc::downgrade(&model),
            state: RefCell::new(State {
                market: None,
                failure: None,
                loading: true,
                items: Vec::new(),
                selected: BTreeMap::new(),
                search: String::new(),
                sort: OfferSort::AskAmount,
                descending: false,
                ordered: Vec::new(),
                shown: FIRST_ROWS,
                values: HashMap::new(),
                sellers: Vec::new(),
                detailed: 0,
            }),
            serial: Cell::new(0),
            tip: Cell::new(None),
            source,
            summary,
            prices_card,
            steps,
            curve,
            prices_note,
            catalogue_card,
            truncated,
            facets,
            search,
            result_count,
            applied_row,
            applied,
            segments,
            reverse,
            sort_note,
            empty,
            rows,
            more,
            smaller_card,
            smaller_note,
            smaller_rows,
        });
        *view_slot.borrow_mut() = Rc::downgrade(&view);
        *view.me.borrow_mut() = Rc::downgrade(&view);
        {
            let weak = Rc::downgrade(&view);
            view.search.connect_changed(move |entry| {
                let Some(view) = weak.upgrade() else { return };
                let text = entry.text().to_string();
                if view.state.borrow().search == text {
                    return;
                }
                view.state.borrow_mut().search = text;
                view.reorder();
                view.render_catalogue();
            });
        }
        view
    }

    // MARK: Data

    /// The page's work when it comes on screen or the connection changes: the offers, then
    /// the sellers and dates.
    fn start(self: &Rc<Self>) {
        self.serial.set(self.serial.get() + 1);
        let view = self.clone();
        glib::spawn_future_local(async move {
            view.clone().load().await;
            view.load_details(false).await;
        });
    }

    async fn load(self: Rc<Self>) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(explorer) = model.explorer() else { return };
        self.state.borrow_mut().loading = true;
        self.render_summary();
        let result = bridge::run(async move { explorer.marketplace().await }).await;
        match result {
            Ok(found) => {
                // The currency that most offers ask for is the price currency.
                let main = logic::price_currency(&found.offers);
                let items: Vec<MarketItem> = found
                    .offers
                    .iter()
                    .cloned()
                    .map(|offer| MarketItem::new(offer, main.as_deref(), |id| known_name(id, &model, None)))
                    .collect();
                let currencies: Vec<String> = found
                    .offers
                    .iter()
                    .flat_map(|o| o.links.iter().filter(|l| l.kind == api::OfferLinkKind::Currency).map(|l| l.value.clone()))
                    .collect();
                {
                    let mut state = self.state.borrow_mut();
                    state.market = Some(found);
                    state.failure = None;
                    state.items = items;
                }
                self.rebuild();
                model.resolve_names(currencies);
            }
            Err(error) => self.state.borrow_mut().failure = Some(bridge::describe(&error)),
        }
        self.state.borrow_mut().loading = false;
        self.render();
    }

    /// Reads the seller and the listing block of each offer, in groups, and keeps them for
    /// the session. A public API sees each request, so the person starts it there.
    async fn load_details(self: &Rc<Self>, force: bool) {
        let Some(model) = self.model.upgrade() else { return };
        let Some(explorer) = model.explorer() else { return };
        if with_store(|s| s.loading) {
            return;
        }
        let generation = model.generation();
        with_store(|s| {
            if s.generation != Some(generation) {
                s.details.clear();
                s.generation = Some(generation);
            }
        });
        let public = model.state().status.as_ref().is_some_and(|s| s.source == api::DataSource::PublicApi);
        if !force && public {
            return;
        }
        let missing: Vec<OfferKey> = with_store(|s| {
            self.state
                .borrow()
                .items
                .iter()
                .filter(|item| !s.details.contains_key(&item.offer.txid))
                .map(|item| OfferKey { txid: item.offer.txid.clone(), identity_id: item.offer.offered.identity_id.clone() })
                .collect()
        });
        if missing.is_empty() {
            return;
        }
        with_store(|s| {
            s.loading = true;
            s.problem = None;
        });
        self.render_summary();
        let serial = self.serial.get();
        for group in missing.chunks(128) {
            if self.serial.get() != serial && !force {
                break;
            }
            let (keys, explorer) = (group.to_vec(), explorer.clone());
            let found = bridge::run(async move { explorer.offer_details(keys).await }).await;
            let problem = found.iter().find(|d| d.error.is_some() && d.holder.is_none() && d.block_height.is_none()).and_then(|d| d.error.clone());
            with_store(|s| {
                for detail in found {
                    s.details.insert(detail.txid.clone(), detail);
                }
            });
            if let Some(problem) = problem {
                // A source that refuses these requests refuses all of them.
                with_store(|s| s.problem = Some(format!("The source did not answer for the sellers and dates. {problem}")));
                break;
            }
            self.rebuild();
            self.render();
        }
        with_store(|s| s.loading = false);
        self.rebuild();
        self.render();
    }

    /// Works out the sellers and each offer's filter options again, then the list.
    fn rebuild(&self) {
        let now = format::now();
        with_store(|store| {
            let mut state = self.state.borrow_mut();
            let holders: Vec<&str> =
                state.items.iter().filter_map(|item| store.details.get(&item.offer.txid).and_then(|d| d.holder.as_deref())).collect();
            let sellers = logic::sellers(holders);
            let detailed = state.items.iter().filter(|item| store.details.contains_key(&item.offer.txid)).count();
            let named = logic::named_sellers(&sellers);
            let mut values = HashMap::new();
            for item in &state.items {
                let detail = store.details.get(&item.offer.txid);
                let known = Known { holder: detail.and_then(|d| d.holder.as_deref()), listed: detail.and_then(|d| d.block_time) };
                let own: Values = Facet::ALL.iter().filter_map(|f| logic::value(*f, item, known, &named, now).map(|v| (*f, v))).collect();
                values.insert(item.offer.txid.clone(), own);
            }
            state.sellers = sellers;
            state.detailed = detailed;
            state.values = values;
        });
        self.reorder();
    }

    fn reorder(&self) {
        let Some(model) = self.model.upgrade() else { return };
        // Resolved names for the name sort, without the trailing "@".
        let names: HashMap<String, String> = model
            .state()
            .names
            .iter()
            .map(|(id, name)| (id.clone(), name.strip_suffix('@').unwrap_or(name).to_string()))
            .collect();
        let times: HashMap<String, u64> =
            with_store(|s| s.details.iter().filter_map(|(txid, d)| d.block_time.map(|t| (txid.clone(), t))).collect());
        let mut state = self.state.borrow_mut();
        let visible: Vec<usize> = (0..state.items.len())
            .filter(|&i| {
                let item = &state.items[i];
                logic::matches(&item.title, state.values.get(&item.offer.txid), &state.selected, &state.search, None)
            })
            .collect();
        let offers: Vec<api::Offer> = visible.iter().map(|&i| state.items[i].offer.clone()).collect();
        let sorted = api::sort_offers(offers, state.sort, state.descending, names, times);
        let mut by_txid: HashMap<&str, usize> = HashMap::new();
        for &i in &visible {
            by_txid.entry(state.items[i].offer.txid.as_str()).or_insert(i);
        }
        let ordered: Vec<usize> = sorted.iter().filter_map(|o| by_txid.get(o.txid.as_str()).copied()).collect();
        state.ordered = ordered;
    }

    fn priced(&self) -> Vec<f64> {
        self.state.borrow().items.iter().filter_map(|i| i.price).collect()
    }

    fn price_ticker(&self, model: &Rc<AppModel>) -> String {
        self.state
            .borrow()
            .items
            .iter()
            .find(|i| i.price.is_some())
            .and_then(|i| i.offer.ask_currency.clone())
            .map(|c| known_name(&c, model, None))
            .unwrap_or_else(|| model.coin())
    }

    fn listed(&self, item: &MarketItem) -> Option<u64> {
        with_store(|s| s.details.get(&item.offer.txid).and_then(|d| d.block_time))
    }

    // MARK: Drawing

    fn render(&self) {
        self.render_summary();
        self.render_prices();
        self.render_catalogue();
        self.render_smaller_sellers();
    }

    /// The answer first: how many offers, from how many sellers, at what prices.
    fn render_summary(&self) {
        let Some(model) = self.model.upgrade() else { return };
        let state = self.state.borrow();
        self.source.set(state.market.as_ref().map(|m| &m.provenance), Some(&model.current_chain()), true);
        clear(&self.summary);
        if state.loading && state.market.is_none() {
            let progress = loader::progress("Loading offers…");
            progress.set_size_request(-1, 160);
            self.summary.append(&progress);
            return;
        }
        if let Some(failure) = &state.failure {
            self.summary.append(&page::note(failure, true));
            if state.market.is_none() {
                return;
            }
        }
        if state.market.is_none() {
            return;
        }
        let ticker = drop_then(state, || self.price_ticker(&model));
        let state = self.state.borrow();
        let mut prices = self.priced();
        prices.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let (store_loading, problem) = with_store(|s| (s.loading, s.problem.clone()));

        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 22);
        line.set_margin_start(18);
        line.set_margin_end(18);
        line.set_margin_top(12);
        line.set_margin_bottom(12);
        // The figures go on a second line when the window is too narrow for them, so
        // that the page does not get wider than the window.
        let figures = crate::widgets::flow::Flow::new(22);
        figures.set_line_spacing(8);
        figures.set_hexpand(true);
        figures.set_valign(gtk::Align::Center);
        line.append(&figures);
        let figure = |label: &str, value: &str| {
            let readout = Readout::new(label, value, "", None);
            figures.append(&readout.root);
        };
        figure("Offers", &format::integer(state.items.len() as u64));
        if let Some(top) = state.sellers.first() {
            figure("Sellers", &state.sellers.len().to_string());
            figure("Largest seller", &logic::percent(top.1, state.detailed));
        }
        if let Some(low) = prices.first() {
            figure("Lowest", &format!("{} {ticker}", format::amount(*low)));
            figure("Median", &format!("{} {ticker}", format::amount(prices[prices.len() / 2])));
        }
        if let Some(newest) = state.items.iter().filter_map(|i| self.listed(i)).max() {
            figure("Newest", &format!("{} ago", logic::age(newest, format::now())));
        }
        if state.loading {
            let spinner = loader::small();
            spinner.set_valign(gtk::Align::Center);
            line.append(&spinner);
        }
        let refresh = {
            let view = self.page.clone();
            let model = self.model.clone();
            lamp::button("Refresh", move || {
                if let (Some(page), Some(_)) = (view.upgrade(), model.upgrade()) {
                    let refresh = page.hooks.borrow().refresh.clone();
                    if let Some(refresh) = refresh {
                        refresh();
                    }
                }
            })
        };
        refresh.set_sensitive(!state.loading);
        line.append(&refresh);
        panel.append(&line);

        // The sellers and the listing dates need two requests for each offer.
        if store_loading || state.detailed < state.items.len() {
            panel.append(&display::rule());
            let progress = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            progress.set_margin_start(18);
            progress.set_margin_end(18);
            progress.set_margin_top(9);
            progress.set_margin_bottom(9);
            if store_loading {
                let spinner = loader::small();
                spinner.set_valign(gtk::Align::Center);
                progress.append(&spinner);
                let text = page::text(
                    &format!(
                        "Reading sellers and listing dates: {} of {}",
                        format::integer(state.detailed as u64),
                        format::integer(state.items.len() as u64)
                    ),
                    &["market-caption", "secondary", "tabular"],
                );
                text.set_valign(gtk::Align::Center);
                progress.append(&text);
            } else {
                let text = page::text(
                    problem
                        .as_deref()
                        .unwrap_or("The sellers and the listing dates are not read. They need two requests for each offer."),
                    &["market-caption", "secondary"],
                );
                text.set_wrap(true);
                text.set_valign(gtk::Align::Center);
                progress.append(&text);
                let weak = self.me.borrow().clone();
                let read = lamp::button("Read Sellers and Dates", move || {
                    if let Some(view) = weak.upgrade() {
                        glib::spawn_future_local(async move { view.load_details(true).await });
                    }
                });
                read.add_css_class("market-small-key");
                progress.append(&read);
            }
            panel.append(&progress);
        }
        self.summary.append(&display::panel(&panel, 0));
    }

    /// The exact prices that many offers share, as bars, and one curve of how many offers
    /// are available at or below each price.
    fn render_prices(&self) {
        let Some(model) = self.model.upgrade() else { return };
        let mut prices = self.priced();
        let has_market = self.state.borrow().market.is_some();
        self.prices_card.root.set_visible(has_market && !prices.is_empty());
        if !has_market || prices.is_empty() {
            return;
        }
        let ticker = self.price_ticker(&model);
        self.prices_card.set_trailing(Some(&format!("ask in {ticker}")));
        clear(&self.steps);
        let steps = logic::price_steps(&prices, &ticker);
        let top = steps.iter().map(|s| s.count).max().unwrap_or(1).max(1) as f32;
        for step in steps {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let label = page::text(&step.label, &["callout", "tabular", if step.other { "label" } else { "value" }]);
            label.set_xalign(1.0);
            label.set_size_request(104, -1);
            label.set_ellipsize(gtk::pango::EllipsizeMode::Start);
            row.append(&crate::widgets::clamp::Clamp::new(&label, 104));
            let share = step.count as f32 / top;
            let other = step.other;
            let bar = Canvas::new(0, 10, move |_, snapshot, w, h| {
                draw::fill_rect(snapshot, 0.0, 0.0, (w * share).max(2.0), h, tone::RECORD.opacity(if other { 0.4 } else { 1.0 }));
            });
            bar.set_hexpand(true);
            bar.set_valign(gtk::Align::Center);
            row.append(&bar);
            let count = theme::label(font::FIGURE_SMALL, &format::integer(step.count as u64), "value");
            count.set_size_request(44, -1);
            row.append(&count);
            self.steps.append(&row);
        }
        let unpriced = self.state.borrow().items.len() - prices.len();
        self.prices_note.set_text(&logic::prices_note(unpriced, &ticker));
        prices.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        self.curve.set(prices, &ticker);
    }

    fn render_catalogue(&self) {
        let has_market = self.state.borrow().market.is_some();
        self.catalogue_card.root.set_visible(has_market);
        if !has_market {
            return;
        }
        self.truncated.set_visible(self.state.borrow().market.as_ref().is_some_and(|m| m.truncated));
        self.render_facets();
        {
            let state = self.state.borrow();
            theme::set_label(
                &self.result_count,
                font::FIGURE_SMALL,
                &format!("{} of {} offers", format::integer(state.ordered.len() as u64), format::integer(state.items.len() as u64)),
            );
            let filtered = !state.selected.is_empty() || !state.search.trim().is_empty();
            self.applied_row.set_visible(filtered);
            self.applied.set_text(&logic::applied_text(&state.selected, &state.search));
            let index = SORTS.iter().position(|s| s.0 == state.sort).unwrap_or(0);
            if self.segments.selected() != index {
                self.segments.set_selected(index);
            }
            lamp::set_chosen(&self.reverse, state.descending);
            self.sort_note.set_text(&logic::sort_note(state.sort, state.descending, state.detailed, state.items.len()));
            self.empty.set_visible(state.ordered.is_empty());
            self.empty.set_text(if state.items.is_empty() {
                "The node returned no open offers for this currency."
            } else {
                "No offer matches the filters."
            });
            if self.search.text().as_str() != state.search {
                self.search.set_text(&state.search);
            }
        }
        self.render_rows();
    }

    /// Each option shows its count with the other filters applied, and a bar for it.
    fn render_facets(&self) {
        clear(&self.facets);
        let state = self.state.borrow();
        for facet in Facet::ALL {
            let options = logic::options(facet, &state.items, state.detailed, &state.sellers);
            if options.is_empty() {
                continue;
            }
            let group = gtk::Box::new(gtk::Orientation::Vertical, 3);
            let name = display::name_text(facet.title());
            name.set_margin_bottom(2);
            group.append(&name);
            let counts = logic::counts(&state.items, &state.values, &state.selected, &state.search, facet);
            let top = counts.values().copied().max().unwrap_or(1).max(1);
            for option in options {
                let count = counts.get(&option).copied().unwrap_or(0);
                let on = state.selected.get(&facet) == Some(&option);
                let weak = self.me.borrow().clone();
                let chosen = option.clone();
                group.append(&facet::facet_option(&option, count, count as f64 / top as f64, on, move || {
                    let Some(view) = weak.upgrade() else { return };
                    {
                        let mut state = view.state.borrow_mut();
                        if state.selected.get(&facet) == Some(&chosen) {
                            state.selected.remove(&facet);
                        } else {
                            state.selected.insert(facet, chosen.clone());
                        }
                        // A new filter starts with the first rows again. The page does not
                        // scroll: a person who clicks a filter keeps their place.
                        state.shown = FIRST_ROWS;
                    }
                    view.reorder();
                    view.render_catalogue();
                }));
            }
            self.facets.append(&group);
        }
        if state.detailed < state.items.len() && !with_store(|s| s.loading) {
            let note = page::paragraph("The Seller and Offered filters appear when the sellers and dates are read.", &["market-caption", "secondary"]);
            self.facets.append(&note);
        }
    }

    fn render_rows(&self) {
        let (Some(page), Some(model)) = (self.page.upgrade(), self.model.upgrade()) else { return };
        clear(&self.rows);
        clear(&self.more);
        let state = self.state.borrow();
        if state.market.is_none() {
            return;
        }
        let mut prices = self.priced();
        prices.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let bounds = logic::price_bounds(&prices);
        with_store(|store| {
            for &index in state.ordered.iter().take(state.shown) {
                let item = &state.items[index];
                self.rows.append(&display::rule());
                let line = offer_line::Line {
                    item,
                    detail: store.details.get(&item.offer.txid),
                    bounds,
                    stop: item.offer.txid.clone(),
                };
                self.rows.append(&offer_line::offer_line(&page, &model, line));
            }
        });
        if state.ordered.len() > state.shown {
            let more = (state.ordered.len() - state.shown).min(MORE_ROWS);
            let weak = self.me.borrow().clone();
            let key = lamp::button(&format!("Show {more} More"), move || {
                if let Some(view) = weak.upgrade() {
                    view.state.borrow_mut().shown += MORE_ROWS;
                    view.render_rows();
                }
            });
            key.set_margin_top(10);
            self.more.append(&key);
        }
    }

    /// When one seller has most of the offers, the offers of the others are the only
    /// independent prices. They get their own group so that they are not lost.
    fn render_smaller_sellers(&self) {
        let (Some(page), Some(model)) = (self.page.upgrade(), self.model.upgrade()) else { return };
        clear(&self.smaller_rows);
        let state = self.state.borrow();
        let show = state.market.is_some()
            && state.sellers.len() > 2
            && state.detailed > 0
            && (state.sellers[0].1 + state.sellers[1].1) as f64 / state.detailed as f64 >= 0.8;
        self.smaller_card.root.set_visible(show);
        if !show {
            return;
        }
        let large: Vec<&str> = state.sellers.iter().take(2).map(|s| s.0.as_str()).collect();
        let mut prices = self.priced();
        prices.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let bounds = logic::price_bounds(&prices);
        with_store(|store| {
            let mut rest: Vec<&MarketItem> = state
                .items
                .iter()
                .filter(|item| {
                    store.details.get(&item.offer.txid).and_then(|d| d.holder.as_deref()).is_some_and(|h| !large.contains(&h))
                })
                .collect();
            rest.sort_by(|a, b| {
                a.price.unwrap_or(f64::INFINITY).partial_cmp(&b.price.unwrap_or(f64::INFINITY)).unwrap_or(std::cmp::Ordering::Equal)
            });
            self.smaller_card.set_trailing(Some(&format!("{} from {} sellers", rest.len(), state.sellers.len() - 2)));
            self.smaller_note.set_text(&format!(
                "Two sellers have {} of the offers. These are the offers of the others.",
                logic::percent(state.sellers[0].1 + state.sellers[1].1, state.detailed)
            ));
            for item in rest.into_iter().take(60) {
                self.smaller_rows.append(&page::divider());
                let line = offer_line::Line {
                    item,
                    detail: store.details.get(&item.offer.txid),
                    bounds,
                    stop: format!("smaller-{}", item.offer.txid),
                };
                self.smaller_rows.append(&offer_line::offer_line(&page, &model, line));
            }
        });
    }
}

/// Ends a borrow of the state, then works out a value that borrows it again.
fn drop_then<T>(state: std::cell::Ref<'_, State>, f: impl FnOnce() -> T) -> T {
    drop(state);
    f()
}

/// The styles of this page that `style.css` does not have: the filter keys, the caption
/// size of macOS, the names of the offers and the links key.
fn install_css() {
    thread_local! {
        static INSTALLED: Cell<bool> = const { Cell::new(false) };
    }
    if INSTALLED.with(|done| done.replace(true)) {
        return;
    }
    let Some(display) = gtk::gdk::Display::default() else { return };
    let css = format!(
        ".lamp.market-facet {{ padding: 0; min-height: 0; color: {value}; }}\n\
         .lamp.market-facet .count {{ color: {label}; }}\n\
         .lamp.market-facet:hover, .lamp.market-facet.chosen, .lamp.market-facet.pressed {{ color: {legend}; }}\n\
         .lamp.market-facet:hover .count, .lamp.market-facet.chosen .count, .lamp.market-facet.pressed .count {{ color: {legend}; }}\n\
         .lamp.market-facet:disabled {{ color: {value}; opacity: 0.5; }}\n\
         .lamp.market-facet:disabled .count {{ color: {label}; }}\n\
         .lamp.small.market-small-key {{ padding: 3px 8px; }}\n\
         .market-caption {{ font-size: 10px; }}\n\
         .display-link.market-title {{ color: {value}; }}\n\
         .display-link.market-title label {{ font-size: 15px; font-weight: 500; }}\n\
         .display-link.market-title:hover {{ color: rgb(214, 242, 235); }}\n\
         .display-link.market-label-link {{ color: {label}; }}\n\
         .display-link.market-label-link:hover {{ color: rgb(214, 242, 235); }}\n\
         .market-offer {{ padding: 7px 8px; }}\n\
         menubutton.market-links > button {{ background: none; border: none; box-shadow: none; padding: 2px 4px; min-height: 0; min-width: 0; }}\n\
         menubutton.market-links > button:hover {{ background-color: {hover}; }}\n",
        value = tone::VALUE.css(),
        label = tone::LABEL.css(),
        legend = lamp_tone::LEGEND.css(),
        hover = tone::PRESENT.opacity(0.06).css(),
    );
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&css);
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
}
