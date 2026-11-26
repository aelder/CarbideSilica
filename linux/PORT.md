# Carbide Silica for Linux: the port

The Linux app is Carbide Silica as it is on macOS: the same functions and the same display.
It is GTK 4 through gtk4-rs, in Rust, and it calls the bridge crate (`carbide_bridge::api`)
directly. All pages are ported. This document is for the engineers who change the app. Read
it before you change the app. The rules of `macos/CONVENTIONS.md` (text, layout, read-only
bridge) apply here too.

## 1. Build, run, picture

Build and run only in Docker. The cargo registry and the target folder are the named
volumes `carbide-linux-cargo` and `carbide-linux-target`.

```sh
cd linux
./build.sh                 # debug build
./build.sh test            # unit tests
./build.sh clippy          # any cargo command
./shot.sh "" docs/progress/dashboard.png          # the Dashboard
./shot.sh 4339600 /tmp/block.png                  # an object, as -open=
STEPS="key Tab; wait 3; shot json.png; scroll down 10 640 600" ./shot.sh 4339600 /tmp/shots/end.png
```

`shot.sh` runs the app on Xvfb at 1281x1084 (the logical size of `macos/docs/images`), with
`--network host` and the node configuration mounted read-only as `/conf/VRSC.conf`, passed
with `-conf=`. Each run takes its own X display, so runs can go at the same time. HOME,
settings, cache and the bookmark file are in the container's `/tmp` and are gone after the
run; the app runs in that home, so a file dialog does not open in the source tree. No
public API is enabled. The app takes Carbide's start-up arguments (`-chain=`, `-testnet`,
`-conf=`, `-datadir=`, `-rpcconnect=`, `-rpcport=`, `-rpcuser=`, `-rpcpassword=`,
`-pollinterval=`, `-open=`); GTK does not read them.

A run waits `WAIT` seconds (60 by default), then does the steps of `STEPS` in order, then
takes the picture `<out.png>`. Steps are separated by `;` or new lines:

| Step | Does |
| --- | --- |
| `wait 5` | waits 5 seconds |
| `key Tab ctrl+k` | presses keys (xdotool names), 0.4 seconds apart |
| `type text` | types a text |
| `move x y` | moves the pointer, to show a hover state |
| `click x y [button]`, `dclick x y` | clicks or double-clicks |
| `scroll down 5 [x y]` | turns the wheel (up, down, left, right), at x y when given |
| `shot name.png` | takes a picture now, into the folder of `<out.png>`; a name has no folder |

Coordinates are pixels of the display. Before the first key the window gets the keyboard
focus (Xvfb has no window manager), and each key goes to the window of the app that was
shown last, such as a file dialog. `KEYS`, `KEYWAIT` and `MOUSE` still work and come before
`STEPS`. The app's messages are printed after the run, a panic first, without any line
that looks like a credential. In a dialog, `key ctrl+a; type /out/name.json; key Return`
saves a file into the folder of the pictures. Do not click a key that changes the node
(the peer commands), the location data download or the public API confirmation.

Environment: `CARBIDE_SILICA_SETTINGS` (settings file), `CARBIDE_SILICA_RESOURCES` (fonts and
sounds folder; default `resources/` of the source tree or `<prefix>/share/carbide-silica`).

## 2. Structure

```
linux/src/main.rs            start-up: fonts, arguments, GTK application
linux/src/bridge.rs          tokio runtime; run() and run_blocking(); describe()
linux/src/model.rs           AppModel (AppModel.swift): state, Topic, watch/notify, navigation
linux/src/settings.rs        UserDefaults: settings.json, the same keys as macOS
linux/src/{format,display,launch,route,wait_record}.rs   Format, Display+HeightDates, Launch, Route, WaitRecord
linux/src/theme/             colours, type roles, style.css, draw helpers, fonts, sounds
linux/src/widgets/           design-system widgets (section 4)
linux/src/page/              Page, Card, cursor, LoadView, shared small components
linux/src/shell/             window, title bar, sidebar, path bar, navigation, keys, menus,
                             command list, dialogs, keyboard reference, settings
linux/src/shared/            parts that more than one page uses (section 6)
linux/src/pages/mod.rs       the registry: places and routes to build functions
linux/src/pages/<page>/      one folder for each page; mod.rs has the build function
macos/bridge/rust/           the bridge as a Rust library (section 9)
```

## 3. Inventory

Status: **Done** (ported and in the app), **Replaced**. Lines are Swift lines. Every Swift
file is ported.

| Swift file | Lines | Linux module | Status |
| --- | ---: | --- | --- |
| AppModel.swift | 591 | model.rs, settings.rs, route.rs | Done |
| Breadcrumb.swift | 69 | shell/breadcrumb.rs, route.rs | Done |
| CarbideSilicaApp.swift | 461 | main.rs, shell/mod.rs, sidebar.rs, titlebar.rs | Done |
| CircuitLoader.swift | 267 | widgets/loader.rs | Done |
| ClickSound.swift | 152 | theme/sound.rs, widgets/lamp.rs (key_sounds) | Done (see 10) |
| CommandPalette.swift | 219 | shell/palette.rs | Done |
| Display.swift | 143 | display.rs, launch.rs | Done |
| DisplayTheme.swift | 766 | theme/*, widgets/* | Done |
| Format.swift | 66 | format.rs | Done |
| KeyboardReference.swift | 215 | shell/keyboard_reference.rs | Done |
| PageCursor.swift | 529 | page/cursor.rs, page/mod.rs, shell/keys.rs | Done |
| ShellCommands.swift | 297 | shell/commands.rs, shell/navigation.rs, page/mod.rs, pages/mod.rs | Done |
| ShellLists.swift | 171 | page/mod.rs, page/components.rs | Done |
| WaitRecord.swift | 70 | wait_record.rs | Done |
| WindowPicture.swift | 35 | shot.sh, tools/shot-inside.sh | Replaced |
| Views/ChainPulse.swift | 1124 | pages/dashboard/chain_pulse.rs | Done |
| Views/DashboardView.swift | 534 | pages/dashboard/{mod,feed,lists}.rs | Done |
| Views/Components.swift | 418 | page/components.rs, page/load.rs, shared/json_tree.rs | Done |
| Views/SettingsView.swift | 206 | shell/settings_window.rs | Done |
| Views/SourcePrompts.swift | 96 | shell/dialogs.rs, page/components.rs | Done |
| Views/TransferTrace.swift | 604 | pages/transaction/{trace,trace_route}.rs, shared/confidence.rs | Done |
| Views/IdentitySupport.swift | 168 | shared/identity_support.rs, model.rs (the AppModel extension) | Done |
| Views/CurrencyStores.swift | 155 | shared/currency_stores.rs | Done |
| Views/JSONTree.swift | 527 | shared/json_tree.rs | Done |
| Views/InclusionCard.swift | 277 | shared/inclusion.rs | Done |
| Views/NodeDiagrams.swift | 181 | shared/branches.rs | Done |
| Views/BlockEstimator.swift | 148 | shared/estimator.rs | Done |
| Views/MempoolView.swift | 1320 | pages/mempool | Done |
| Views/ChartsView.swift | 985 | pages/charts | Done |
| Views/ChartStack.swift | 1136 | pages/charts | Done |
| Views/ChartPlot.swift | 385 | pages/charts | Done |
| Views/FeePoolChart.swift | 409 | pages/charts | Done |
| Views/CurrenciesView.swift | 401 | pages/currencies | Done |
| Views/MarketplaceView.swift | 998 | pages/marketplace | Done |
| Views/BookmarksView.swift | 359 | pages/bookmarks | Done |
| Views/NodeView.swift | 676 | pages/node | Done |
| Views/PeerControls.swift | 143 | pages/node | Done |
| Views/PeerMap.swift | 430 | pages/node | Done |
| Views/PeerOutlineMap.swift | 533 | pages/node | Done |
| Views/SourcesView.swift | 390 | pages/sources | Done |
| Views/BlockView.swift | 186 | pages/block | Done |
| Views/TransactionView.swift | 815 | pages/transaction | Done |
| Views/TransactionStatement.swift | 1156 | pages/transaction, shared/statement_words.rs | Done |
| Views/TransactionExtras.swift | 427 | pages/transaction | Done |
| Views/TraceRoute.swift | 297 | pages/transaction | Done |
| Views/IdentityView.swift | 419 | pages/identity, shared/identity_support.rs (BalanceList) | Done |
| Views/IdentityControl.swift | 487 | pages/identity | Done |
| Views/IdentityEvidence.swift | 391 | pages/identity | Done |
| Views/IdentityExtras.swift | 142 | pages/identity | Done |
| Views/IdentityOrigin.swift | 209 | pages/identity | Done |
| Views/RegistrationPayments.swift | 141 | shared/registration_payments.rs | Done |
| Views/CurrencyView.swift | 258 | pages/currency | Done |
| Views/CurrencyOverTime.swift | 397 | pages/currency | Done |
| Views/CurrencyStateSeek.swift | 515 | pages/currency | Done |
| Views/NotarizationLanes.swift | 561 | pages/currency | Done |
| Views/NotarizationPicture.swift | 361 | pages/currency | Done |
| Views/PoolBatch.swift | 121 | pages/currency | Done |
| Views/AddressView.swift | 337 | pages/address, shared/address_activity.rs | Done |
| Views/EstimateView.swift | 17 | pages/estimate | Done |

## 4. Design system

### Colours (`theme::tone`, `theme::lamp`; CSS `@name`)

| Name | sRGB | Use |
| --- | --- | --- |
| SCREEN `@screen` | 0.012 0.012 0.016 | the black of the display |
| RULE `@rule` | 0.75 0.80 0.75 @ 0.15 | panel border, rule above a card |
| GRID `@grid` | same @ 0.09 | grid lines |
| AXIS `@axis` | same @ 0.36 | axes, outline of a key that is off |
| LABEL `@label` | 0.62 0.67 0.64 | names: axis labels, readout names |
| VALUE `@value` | 0.75 0.80 0.75 | figures and text |
| PRESENT `@present` | 0.80 0.84 0.77 | the present and the selection |
| STAKED `@staked` | 0.72 0.68 0.80 | staked |
| MINED `@mined` | 0.74 0.67 0.44 | mined |
| RECORD `@record` | 0.50 0.74 0.71 | links; a record that is done |
| GOOD `@good` | 0.26 0.65 0.48 | good state |
| ALERT `@alert` | 0.82 0.33 0.24 | alert |
| STAKED_GLOW / MINED_GLOW | 0.56 0.30 0.86 / 1 0.5 0.12 | light around staked / mined marks |
| SECONDARY / TERTIARY | VALUE @ 0.6 / @ 0.38 | SwiftUI `.secondary` / `.tertiary` |
| SERIES | record, mined, staked, good, (0.78 0.5 0.56), value | chart series in order |
| lamp BLUE_CENTRE / BLUE_EDGE | 0.62 0.73 0.73 / 0.42 0.57 0.60 | hover lamp |
| lamp AMBER_CENTRE / AMBER_EDGE | 0.71 0.74 0.64 / 0.62 0.53 0.44 | chosen or pressed lamp |
| lamp LEGEND | 0.33 0.24 0.20 | legend on a lit key |
| lamp HALATION | 0.95 0.33 0.14 | red-orange edge of a strong light |
| lamp FACE | 0.045 grey | face of a key that is off |

Text classes in `style.css`: `value value-soft label secondary tertiary present record
staked mined good alert warning`; sizes `title2` (17), `title3` (15), `callout` (12),
`subheadline` (11), `caption-text` (11.5, the panel caption), `caption-small` (10, SwiftUI's
`.caption`); `mono` (12, `.callout.monospaced()`), `mono-body` (13, `.body.monospaced()`),
`mono-caption` (10); `semibold medium tabular`.

### Type (`theme::font`; make labels with `theme::label(role, text, class)`)

Sizes are logical pixels, as the Swift sizes. Pango takes font sizes in points (4/3 of a
pixel at 96 dots an inch), so `theme::markup` and `theme::role_attributes` convert them
(`theme::pango_points`); letter spacing stays in pixels. Never write a Pango `font_size` by
hand: use `theme::markup`, `theme::text_markup(size, weight, text)` or `theme::text_label`.

| Role | Font | Size | Tracking | Case |
| --- | --- | ---: | ---: | --- |
| SMALL (names, axes) | Jost 500 | 9.5 | 1.1 | capitals |
| PLATE | Jost 700 | 8.5 | 0.8 | capitals |
| FIGURE | Jost 500, tabular | 17 | 0 | as is |
| FIGURE_SMALL | Jost 500, tabular | 11.5 | 0 | as is |
| HEADING (card) | Jost 500 | 13 | 2.2 | capitals |
| TITLE (page) | Jost 500 | 26 | 0 | as is |
| KEY (small key legend) | Jost 500 | 10.5 | 1.2 | capitals |
| PLACE (sidebar key) | Jost 500 | 11.5 | 1.6 | capitals |
| CAPTION | Inter 400 | 11.5 | 0 | as is |
| BODY / CALLOUT | Inter 400 | 13 / 12 | 0 | as is |
| TITLE2 / TITLE3 | Inter 400 | 17 / 15 | 0 | as is |
| SUBHEADLINE / TEXT_CAPTION | Inter 400 | 11 / 10 | 0 | as is |
| MONO_CALLOUT (IDs) | IBM Plex Mono 400 | 12 | 0 | as is |

A role with another weight is `font::Role { weight: 600, ..font::CALLOUT }`. A label whose
text changes with `set_text` takes its role with `theme::set_role(&label, role)`.

Futura cannot ship on Linux. Jost (SIL OFL 1.1) takes the Futura roles, Inter (OFL) takes
the system face, IBM Plex Mono (OFL) takes the monospaced face. They are in
`resources/fonts` with their licences and are given to fontconfig at start
(`theme/fonts.rs`); nothing is installed on the system. Jost is a little smaller than Futura
at one size (capitals 0.70 of the size against 0.76, lettering 3.5% narrower), so the Jost
roles are drawn at 1.05 of their size (`theme::DISPLAY_SCALE`). Its line is taller (1.445
of the size against 1.328), so the Jost roles have a line height of 0.92
(`theme::DISPLAY_LINE_HEIGHT`). With both, names and figures are within 1% to 4% of the
widths in the macOS pictures.

Glyphs that the three fonts lack (emoji, Chinese, Japanese, Korean, Arabic) come from the
fonts of the system through fontconfig. A Linux user needs a colour emoji font and a CJK
font, on Debian and Ubuntu `fonts-noto-color-emoji` and `fonts-noto-cjk`
(`fonts-noto-core` for other scripts); on Fedora `google-noto-color-emoji-fonts` and
`google-noto-sans-cjk-fonts`. Without them those names show as boxes. The Docker image has
them.

### Components

| Swift | Linux API | Use |
| --- | --- | --- |
| `Canvas` | `widgets::canvas::Canvas::new(w, h, \|c, snap, w, h\| …)` | custom drawing with GSK; `queue_draw()` |
| `GraphicsContext.rule/name/figure/plate/lit` | `theme::draw::{rule, name, figure, plate, lit, push_glow, push_shadow, text, fill, stroke}` | inside a canvas |
| `DisplayPanel` | `widgets::display::panel(&child, padding)` | frame of a chart or diagram |
| `CornerMarks` | `widgets::display::corner_marks()` | overlay of a framed area |
| `PanelRule` | `widgets::display::rule()` / `vertical_rule()` | thin rule |
| `Readout` | `Readout::new(label, value, class, reserve)`; `.set(label, value)` | "TARGET 25" |
| `Plate`, `TagChip` | `display::plate(text, PlateFill::Staked)`; `signal_tag_fill(tag)` | tags; `PlateFill::Present` for a chosen kind |
| outline | `display::outlined(&child, color, h, v)` | `.padding().overlay(Rectangle().strokeBorder(…))` |
| `FlowLayout` | `widgets::flow::Flow::new(spacing)` | a row that wraps |
| `NameText` | `display::name_text(text)` | field and column names |
| `LampKey` | `widgets::lamp::LampKey::new(title, 30, recent, action)`; `set_chosen`, `set_figure` | sidebar keys |
| `LampButtonStyle` | `lamp::button(title, action)`; `lamp::button_with(title, Some(Icon::Chart), action)`; `set_button_title` | every button of a page; a mark before the legend follows the light |
| `LampSegments` | `lamp::Segments::new(&titles, selected, changed)` | segmented control |
| `LampToggle` | `lamp::toggle(title, lit, struck, action)`; `lamp::set_toggle(&key, title, struck)`, `lamp::set_chosen` | on/off key |
| `LampMenu` | `lamp::Menu::new(&titles, selected, changed)` | choice list |
| `LampSlider` | `lamp::Slider::new(value, low, high, changed)` | slider |
| `DisplayDisclosureStyle` | `lamp::Disclosure::new(title, expanded, changed)` | a part that opens; its contents are hidden while it is closed |
| `PressSoundStyle` | `lamp::key_sounds(&widget, hover)` | sounds and the pressed class |
| `DisplayLinkStyle` | `widgets::link::link(text, mono, action)` | a link |
| `RowLight` | `widgets::row_light::row_light(&row, Some(action))` | hover light and sound of a row |
| `FilmGrain` | `widgets::grain::film_grain()` | the shell adds it once |
| `CircuitLoader`, `ProgressView` | `widgets::loader::{circuit(size), progress(text), small(), fraction_bar(f)}` | work in progress |
| SF Symbols | `icons::icon(Icon::Warning, size, color)`, `icons::text_icon(kind, size)` (in the text colour), `icons::label(kind, text, color, classes)` (SwiftUI `Label`) | marks that mean something |
| `Card` | `page.card(title, trailing)` → `Card { root, content }` | a section; registers with the cursor |
| `Note` | `page::note(text, warning)` | a sentence; triangle only for a warning |
| `SourceLine` | `page::SourceLine::new()`; `.set(prov, short, labelled)` | live or stored |
| `ProvenanceNote` | `page::provenance_note(&prov)` | where the data came from |
| `RouteLink` | `page::route_link(&page, id, route, shorten)` | a name-or-ID link and cursor stop |
| `FieldRow` | `page::field_row(&page, label, value, route, mono)` | label, value, copy |
| `CopyButton` | `page::copy_button(value)` | copy |
| `PageHeader`, `TagRow` | `page::page_header(title, subtitle, &tags)`, `page::tag_row(&tags)` | heading of an object page |
| `CoinAmount` | `page::coin_markup(sats, coin, color)` | amount with ticker (Pango markup) |
| `LoadView` | `page::LoadView::new(&model, \|ex\| async move { ex.x().await }, \|holder, v\| …)` | load with progress, error and Try Again |
| `SectionLoader` | `shared::identity_support::SectionLoader::new(&model, text, load, content)` | load one section |
| `JSONSection` | `shared::json_tree::json_section(&page, json, filename)`; `json_section_keeping(…, open)` | the Raw JSON card; Tab and s |
| `JSONTreeView` | `shared::json_tree::json_tree(&page, json, filename)` | the tree of any JSON; s saves it while it shows |
| `ConfidenceMark` | `shared::confidence::confidence_mark(level, label)` | four levels |
| `Divider` | `page::divider()` | line between rows |
| check box | `page::check_box(title, on, changed)` | `.toggleStyle(.checkbox)` |

Hover, press and chosen states of keys and links are CSS states in `theme/style.css`
(`:hover`, `.pressed`, `.chosen`), with transitions that are quick up and slower down.

All styles are in `theme/style.css`; a page does not load a CSS provider of its own. Shared
classes: `table-row` (a selectable row: hover field, `.selected`), `lamp facet` and
`lamp small compact` (filter keys), `display-link value-link` and `label-link` (links in
the tones of the text), `plain-key` (a key with no frame, `.buttonStyle(.plain)`),
`frameless-menu` (a menu key with no frame), `json-row`, `address-tile`, and the page
classes `currencies-tile`, `market-offer`, `market-title` and `tx-*`.

Icons (`widgets::icons::Icon`), each drawn once: Search, Dot, Ring, RingDashed, RingDotted,
HalfFilled, Stored, Clock, Hourglass, Warning, WarningFill, ExclamationCircle,
QuestionCircle, Copy, Save, Check, CheckCircle, CheckCircleFill, Seal, Cross, CrossCircle,
CrossCircleFill, CrossOctagonFill, MinusCircle, SlashCircle, NoSign, Diamond, Chevron,
ChevronLeft, ChevronRight, ChevronUp, Down, TriangleRight, TriangleDown, TriangleUp,
ArrowLeft, ArrowRight, ArrowsLeftRight, Refresh, Circlepath, ArrowUpCircle,
ArrowDownCircleFill, Branch, Chart, Link, Bookmark, Cube, PersonCircle, HexagonGrid,
CreditCard, Tray, TrayFull, ServerRack, Disconnected, Menu. Each names its SF Symbol in
its comment. Add a mark there, not in a page.

## 5. How a page module is written

**Registry.** `pages/mod.rs` maps each place (`PLACES`) and each `Route` to the `build`
function of a module, with fixed signatures: `build(&Rc<AppModel>) -> Rc<Page>` for a place,
`build(&Rc<AppModel>, &str) -> Rc<Page>` for an object. A page's keys, commands, toggles,
refresh and background work are declared by the page itself (below). A part that two pages
use goes in `shared/` or `widgets/`, once; a page does not keep a copy.

**Building.** `let page = Page::scrolling(model);` gives one scroll view, a column 1,100 wide
at most, 24 from the edges, 20 between parts. Add parts with `page.card(...)` and
`page.append(...)`. A page with its own layout and keys (the charts) uses `Page::fixed`. A
place page is built once and kept for the session; an object page lives while it is on the
path. `model.show_charts` builds the Charts page again.

**Data.** Never block the main thread. Run a bridge call on the bridge runtime and await it
on the GTK main loop:

```rust
let ex = model.explorer()?;            // or model.require_explorer()
glib::spawn_future_local(async move {
    let result = bridge::run(async move { ex.block(id).await }).await;
    // change widgets here
});
```

For the usual "load, then show", use `LoadView` (progress, error, Try Again, reload on a new
connection). Discard an answer when `model.generation()` changed while it was on its way.
Redraw on change with `model.watch(&[Topic::Names, Topic::Display], &widget, |model| …)`:
the watcher ends with the widget. Topics: Connection, Status, Node, Path, Recent, Names,
Bookmarks, Search, Permission, Display, HeightDates. Timers: `page.every(Duration, …)` runs
only while the page is on screen.

**Performance.** Continuous motion (pulses, the loader, sweeps) must not redraw a page: put
it in a small dedicated `Canvas` and drive it with `add_tick_callback`, and only while it
moves (see `chain_pulse.rs`: the plot redraws at 1 Hz, the trace only during its one-second
front, the wait mark only when it moved a quarter pixel). Ordinary data redraws at about 1 Hz.

**Navigation.** Through the model only: `model.open(route)` (push), `model.step(route)`
(replace the top, for next/previous), `model.set_path(vec)`, `model.show(place)`,
`model.show_charts(height, hash)`, `model.open_trace(txid, output)` then
`model.take_pending_trace()` in the transaction page, `model.open_currency_at(name, height)`
then `model.take_pending_currency_state()`. Back, Forward and the path bar are the shell's.

**Keys and the command list.** Fill `page.hooks()`:
`refresh`, `adjacent` (noun, previous, next: n/p, Ctrl+Alt+[ ]), `investigations` (F8, F4,
F3, F7, v and the Investigate menu), `place` (Stream Blocks, mempool list, peer map, peer
commands), `keys` (one character → title and action; the page's keys come before the
window's), `commands` (more lines for the command list), `toggle_json` (Tab), `save_json`
(s), `shown`/`hidden`. Make each row and link a cursor stop: `page.selectable_row(id, &row,
copy, open)` or `page.stop(&widget, id, copy, open)`; `route_link` does it for you. A part
that takes the arrow keys sets `page.cursor.suspended`; Left and Right are then the
part's. A page that builds a part again wraps it in `page.cursor.keeping_selection(|| …)`,
so the cursor stays on the same link or row, as SwiftUI keeps a view that is built again.

**Sounds.** Use the widgets: `lamp::button`, `LampKey`, `Segments`, `toggle`, `link::link`,
`row_light`. For your own key, call `lamp::key_sounds`. The setting "Play a sound for keys
and buttons" (`interfaceSounds`) turns them off.

**Text.** Copy every string verbatim from the Swift file. New Linux-only text: short whole
sentences, active voice, no dashes as punctuation, no figurative language.

## 6. Shared parts

`src/shared/` holds parts that several pages use. Keep the public signatures.

| File | Swift | Used by |
| --- | --- | --- |
| json_tree.rs `json_section`, `json_section_keeping`, `json_tree` | JSONSection, JSONTreeView | block, transaction, identity, currency, address |
| inclusion.rs `inclusion_card`, `block_branches_card` | InclusionCard | transaction, block |
| branches.rs `branches_card(page)` | BranchesCard | node, block |
| estimator.rs `block_estimator(page, query)`, `relative` | BlockEstimator, `.relative(presentation: .named)` | estimate, charts |
| identity_support.rs | IDLink, IDRow, ValueRow, SectionLoader, BalanceList, heightAndTime | identity, address |
| address_activity.rs `ActivityList::new(page, address, on_stale)` | AddressActivityList, AddressTileView, FlowRow | address, identity |
| registration_payments.rs `RegistrationPaymentsCard::new`, `for_transaction` | RegistrationPaymentsCard | identity, transaction |
| statement_words.rs `StatementWords`, `Namer`, `brief`, `list`, `capital`, `span` | StatementWords, StatementText, MempoolText.span | transaction, mempool |
| currency_stores.rs | knownName, observationSource, heightText, OtherChainNote, LaunchState.word, CurrencyBrowserState, CurrencySort, NotarizationRequests | currency pages, currencies, bookmarks, marketplace |
| confidence.rs `confidence_mark(level, label)` | ConfidenceMark | block, transaction, identity |

The AppModel extension of `IdentitySupport.swift` is on `AppModel` (`model.rs`):
`currency_name`, `display_id`, `height_text`, `amount_text`. `model::currency_name_in` and
`display_id_in` are the same rules over a set of names, for `Namer`.

`pages/mempool::background(model)` is called once at start by `pages::start_background`: it
runs `MempoolFeed.refresh` for the whole session, as the Swift sidebar does.

## 7. Shortcuts: macOS to Linux

Command is Ctrl, Option is Alt, Shift is Shift. Control-Command (the Investigate menu)
is Ctrl+Shift. The table is `SHORTCUTS` in `shell/commands.rs`.

| macOS | Linux | Command |
| --- | --- | --- |
| ⌘K | Ctrl+K | command list |
| ⌘F | Ctrl+F | search |
| ⌘1…⌘8 | Ctrl+1…Ctrl+8 | places |
| ⌘[ ⌘] | Ctrl+[ Ctrl+] | back, forward |
| ⌥⌘[ ⌥⌘] | Ctrl+Alt+[ Ctrl+Alt+] | previous, next |
| ⇧⌘E | Ctrl+Shift+E | estimate |
| ⌘R | Ctrl+R | refresh |
| ⌘D | Ctrl+D | bookmark |
| ⇧⌘I ⇧⌘D ⌥⌘U | Ctrl+Shift+I, Ctrl+Shift+D, Ctrl+Alt+U | names, dates, UTC |
| ⌥⌘S ⌥⌘L ⌥⌘G | Ctrl+Alt+S, Ctrl+Alt+L, Ctrl+Alt+G | stream blocks, mempool list, peer map |
| ⌃⌘T ⌃⌘W ⌃⌘L ⌃⌘B ⌃⌘A | Ctrl+Shift+T, W, L, B, A | follow transfer, witnesses, protocol links, block history, shared authorities |
| ⌥⌘N ⌥⌘P | Ctrl+Alt+N, Ctrl+Alt+P | add node, ping all |
| ⌘/ | Ctrl+/ (also F1, ?) | keyboard shortcuts |
| ⌘, | Ctrl+, | settings |
| ⌘Q | Ctrl+Q | quit |
| ⌘C | Ctrl+C | copy the value under the cursor when no text is selected |

The terminal keys (j k h l y s d m o c ` / ? n p [ ] r v B I D, F1–F10, Tab, Esc, Return,
arrows, Page Up/Down, Home/End) are the same as on macOS (`shell/keys.rs`).

## 8. The menus

macOS has a menu bar. GTK 4 has none in this window, so the menus (View, Go, Investigate,
Peers, Bookmark, Keyboard Shortcuts, Settings, Quit) are in the menu key at the left of the
title bar. Each item shows its Linux shortcut and is off when the open page has no such
function.

## 9. Changes in `macos/bridge`

- `src/ffi.rs`: a new public module `rust_api` at the end, which re-exports the records and
  functions of the area modules. Swift is not affected (no UniFFI item changed).
- `src/ffi_transaction.rs`: `mod statement` is `pub(super) mod statement`, so `rust_api`
  can name the statement records.
- `rust/Cargo.toml`, `rust/lib.rs` (new): the bridge as a Rust library named `carbide_bridge`.
  `src/desktop_bridge.rs` keeps `ffi` private and cannot change (it is in Carbide's `src`),
  so `lib.rs` declares the same modules in an inline module whose `#[path]` is `src`, and
  exports `ffi` and `api`. Keep its module list and its dependencies in step with
  `src/desktop_bridge.rs` and `macos/bridge/Cargo.toml`.

## 10. Deviations from the macOS app, and open problems

- Fonts: Jost, Inter and IBM Plex Mono in place of Futura, SF Pro and SF Mono (section 4).
  Inter is approximately 4% wider than SF Pro, so a long sentence can wrap one word earlier.
- Menus are in a menu key (section 8). Window buttons follow the desktop's layout.
- The sidebar is 180 wide, as in the macOS pictures (Swift: 180 to 280, 210 at first). The
  title bar is 36 high; the search field is centred nearer the page column than the
  window, but the keys at the right of the bar keep it approximately 40 to the left of the
  macOS place.
- Linux words for macOS words: "Chains on this computer" (the chain menu), "this computer"
  in Settings, "Show in Files" for "Show in Finder".
- Sounds: decoded once with rodio and Symphonia (MP3) and played through ALSA (PipeWire and
  PulseAudio serve ALSA). The decoding is tested. Playback was not heard: the container has
  no audio device, and the app then says once on standard error that it is silent.
- Film grain: a GTK widget cannot blend with what is below it. The tile is white with the
  noise in its alpha, which adds light as "plus lighter" does on black.
- Glow and halation are GSK shadow nodes (canvases) and CSS box-shadows (keys). They show in
  the headless pictures. The transitions of the lamps (100 ms up, 340 ms down) are CSS
  transitions and are not verified: a picture has one frame.
- Reduced motion follows GTK's `gtk-enable-animations` setting.
- Place pages stay alive for the session, so they keep their scroll position without
  `PlaceMemory`.
- The two dashboard lists are a FlowBox of two columns of at least 430, one column when
  narrow (`ViewThatFits`).
- Relative dates follow `Date.RelativeFormatStyle` with the named presentation in English
  and the local time zone, with weeks that begin on Sunday, as the English calendar of the
  macOS app. Byte counts follow `ByteCountFormatter` (binary) in English.
- Open: `HeightDates`, the permission dialog, the settings window and the keyboard reference
  window are written but not exercised against a public API or pictured; `known_chains` on
  Linux depends on Carbide's search paths for node configurations and was not checked;
  Ctrl+Alt+L can be taken by a desktop for its lock screen; the peer map needs the location
  data, which the pictures do not download; the witness card's decoded report was not
  pictured, because no transaction with signers was opened.
