# Carbide Silica for Linux: the port

The Linux app is Carbide Silica as it is on macOS: the same functions and the same display.
It is GTK 4 through gtk4-rs, in Rust, and it calls the bridge crate (`carbide_bridge::api`)
directly. This document is for the engineers who port the remaining pages. Read it before
you change the app. The rules of `macos/CONVENTIONS.md` (text, layout, read-only bridge)
apply here too.

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
WAIT=30 KEYS="ctrl+k" ./shot.sh "" /tmp/palette.png
WAIT=30 MOUSE="100 145" ./shot.sh "" /tmp/hover.png
```

`shot.sh` runs the app on Xvfb at 1281x1084 (the logical size of `macos/docs/images`), with
`--network host` and the node configuration mounted read-only as `/conf/VRSC.conf`, passed
with `-conf=`. HOME, settings and cache are in the container's `/tmp` and are gone after the
run. No public API is enabled. The app takes Carbide's start-up arguments (`-chain=`,
`-testnet`, `-conf=`, `-datadir=`, `-rpcconnect=`, `-rpcport=`, `-rpcuser=`,
`-rpcpassword=`, `-pollinterval=`, `-open=`); GTK does not read them.

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
linux/src/shared/            parts that more than one page uses, each with one owner (section 6)
linux/src/pages/mod.rs       the registry: places and routes to build functions (fixed)
linux/src/pages/<page>/      one folder for each page; mod.rs has the fixed build function
macos/bridge/rust/           the bridge as a Rust library (section 9)
```

## 3. Inventory

Status: **Done** (ported and in the app), **Partial**, **Stub** (a "not ported yet" panel),
**Replaced**. Lines are Swift lines.

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
| Views/Components.swift | 418 | page/components.rs, page/load.rs, shared/json_tree.rs | Done; JSONSection Partial |
| Views/SettingsView.swift | 206 | shell/settings_window.rs | Done |
| Views/SourcePrompts.swift | 96 | shell/dialogs.rs, page/components.rs | Done |
| Views/TransferTrace.swift | 604 | pages/transaction; ConfidenceMark in shared/confidence.rs | Stub; ConfidenceMark Done |
| Views/IdentitySupport.swift | 168 | shared/identity_support.rs | Partial (SectionLoader not) |
| Views/CurrencyStores.swift | 155 | shared/currency_stores.rs | Partial (two stores not) |
| Views/JSONTree.swift | 527 | shared/json_tree.rs | Stub (JSON as text) |
| Views/InclusionCard.swift | 277 | shared/inclusion.rs | Stub |
| Views/NodeDiagrams.swift | 181 | shared/branches.rs | Stub |
| Views/BlockEstimator.swift | 148 | shared/estimator.rs | Stub |
| Views/MempoolView.swift | 1320 | pages/mempool (with `background`) | Stub |
| Views/ChartsView.swift | 985 | pages/charts | Stub |
| Views/ChartStack.swift | 1136 | pages/charts | Stub |
| Views/ChartPlot.swift | 385 | pages/charts | Stub |
| Views/FeePoolChart.swift | 409 | pages/charts | Stub |
| Views/CurrenciesView.swift | 401 | pages/currencies | Stub |
| Views/MarketplaceView.swift | 998 | pages/marketplace | Stub |
| Views/BookmarksView.swift | 359 | pages/bookmarks | Stub |
| Views/NodeView.swift | 676 | pages/node | Stub |
| Views/PeerControls.swift | 143 | pages/node | Stub |
| Views/PeerMap.swift | 430 | pages/node | Stub |
| Views/PeerOutlineMap.swift | 533 | pages/node | Stub |
| Views/SourcesView.swift | 390 | pages/sources | Stub |
| Views/BlockView.swift | 186 | pages/block | Stub |
| Views/TransactionView.swift | 815 | pages/transaction | Stub |
| Views/TransactionStatement.swift | 1156 | pages/transaction | Stub |
| Views/TransactionExtras.swift | 427 | pages/transaction | Stub |
| Views/TraceRoute.swift | 297 | pages/transaction | Stub |
| Views/IdentityView.swift | 419 | pages/identity | Stub |
| Views/IdentityControl.swift | 487 | pages/identity | Stub |
| Views/IdentityEvidence.swift | 391 | pages/identity | Stub |
| Views/IdentityExtras.swift | 142 | pages/identity | Stub |
| Views/IdentityOrigin.swift | 209 | pages/identity | Stub |
| Views/RegistrationPayments.swift | 141 | pages/identity | Stub |
| Views/CurrencyView.swift | 258 | pages/currency | Stub |
| Views/CurrencyOverTime.swift | 397 | pages/currency | Stub |
| Views/CurrencyStateSeek.swift | 515 | pages/currency | Stub |
| Views/NotarizationLanes.swift | 561 | pages/currency | Stub |
| Views/NotarizationPicture.swift | 361 | pages/currency | Stub |
| Views/PoolBatch.swift | 121 | pages/currency | Stub |
| Views/AddressView.swift | 337 | pages/address | Stub |
| Views/EstimateView.swift | 17 | pages/estimate | Stub |

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

Text classes in `style.css`: `value label secondary tertiary present record staked mined
good alert warning callout caption-text mono semibold medium tabular`.

### Type (`theme::font`; make labels with `theme::label(role, text, class)`)

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
| MONO_CALLOUT (IDs) | IBM Plex Mono 400 | 12 | 0 | as is |

Futura cannot ship on Linux. Jost (SIL OFL 1.1) takes the Futura roles, Inter (OFL) takes
the system face, IBM Plex Mono (OFL) takes the monospaced face. They are in
`resources/fonts` with their licences and are given to fontconfig at start
(`theme/fonts.rs`); nothing is installed on the system.

### Components

| Swift | Linux API | Use |
| --- | --- | --- |
| `Canvas` | `widgets::canvas::Canvas::new(w, h, \|c, snap, w, h\| …)` | custom drawing with GSK; `queue_draw()` |
| `GraphicsContext.rule/name/figure/plate/lit` | `theme::draw::{rule, name, figure, plate, lit, push_glow, push_shadow, text, fill, stroke}` | inside a canvas |
| `DisplayPanel` | `widgets::display::panel(&child, padding)` | frame of a chart or diagram |
| `CornerMarks` | `widgets::display::corner_marks()` | overlay of a framed area |
| `PanelRule` | `widgets::display::rule()` / `vertical_rule()` | thin rule |
| `Readout` | `Readout::new(label, value, class, reserve)`; `.set(label, value)` | "TARGET 25" |
| `Plate`, `TagChip` | `display::plate(text, PlateFill::Staked)`; `signal_tag_fill(tag)` | tags |
| `NameText` | `display::name_text(text)` | field and column names |
| `LampKey` | `widgets::lamp::LampKey::new(title, 30, recent, action)`; `set_chosen`, `set_figure` | sidebar keys |
| `LampButtonStyle` | `lamp::button(title, action)` | every button of a page |
| `LampSegments` | `lamp::Segments::new(&titles, selected, changed)` | segmented control |
| `LampToggle` | `lamp::toggle(title, lit, struck, action)` | on/off key |
| `LampMenu` | `lamp::Menu::new(&titles, selected, changed)` | choice list |
| `LampSlider` | `lamp::Slider::new(value, low, high, changed)` | slider |
| `DisplayDisclosureStyle` | `lamp::Disclosure::new(title, expanded, changed)` | a part that opens |
| `PressSoundStyle` | `lamp::key_sounds(&widget, hover)` | sounds and the pressed class |
| `DisplayLinkStyle` | `widgets::link::link(text, mono, action)` | a link |
| `RowLight` | `widgets::row_light::row_light(&row, Some(action))` | hover light and sound of a row |
| `FilmGrain` | `widgets::grain::film_grain()` | the shell adds it once |
| `CircuitLoader`, `ProgressView` | `widgets::loader::{circuit(size), progress(text), small(), fraction_bar(f)}` | work in progress |
| SF Symbols | `widgets::icons::icon(Icon::Warning, size, color)` | marks that mean something |
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
| `ConfidenceMark` | `shared::confidence::confidence_mark(level, label)` | four levels |
| `Divider` | `page::divider()` | line between rows |
| check box | `page::check_box(title, on, changed)` | `.toggleStyle(.checkbox)` |

Hover, press and chosen states of keys and links are CSS states in `theme/style.css`
(`:hover`, `.pressed`, `.chosen`), with transitions that are quick up and slower down.

## 5. How a page module is written

**Registry.** `pages/mod.rs` maps each place (`PLACES`) and each `Route` to the `build`
function of a module, with fixed signatures: `build(&Rc<AppModel>) -> Rc<Page>` for a place,
`build(&Rc<AppModel>, &str) -> Rc<Page>` for an object. A page is finished by editing only
its own folder `pages/<page>/`: add files there and declare them in its `mod.rs`. Do not edit
`pages/mod.rs`, the shell, or another page. A page's keys, commands, toggles, refresh and
background work are declared by the page itself (below), so no shared file changes.

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
that takes the arrow keys sets `page.cursor.suspended`.

**Sounds.** Use the widgets: `lamp::button`, `LampKey`, `Segments`, `toggle`, `link::link`,
`row_light`. For your own key, call `lamp::key_sounds`. The setting "Play a sound for keys
and buttons" (`interfaceSounds`) turns them off.

**Text.** Copy every string verbatim from the Swift file. New Linux-only text: short whole
sentences, active voice, no dashes as punctuation, no figurative language.

## 6. Shared parts and their owners

`src/shared/` holds parts that several pages use. The owner finishes the file; other pages
call it and wait. Keep the public signatures.

| File | Swift | Owner | Used by | State |
| --- | --- | --- | --- | --- |
| json_tree.rs `json_section(page, json, filename)` | JSONSection, JSONTree | Block | block, transaction, identity, currency, address | Partial: text, Tab, copy |
| inclusion.rs `inclusion_card`, `block_branches_card` | InclusionCard | Transaction | transaction, block | Stub |
| branches.rs `branches_card(page)` | BranchesCard | Node | node, block | Stub |
| estimator.rs `block_estimator(page, query)` | BlockEstimator | Estimate | estimate, charts | Stub |
| identity_support.rs | IDLink, ValueRow, heightAndTime, SectionLoader | Identity | identity, address | Partial |
| currency_stores.rs | knownName, observationSource, heightText, OtherChainNote; CurrencyBrowserState, NotarizationRequests | Currency (browser state: Currencies) | currency pages, bookmarks, marketplace, identity, address | Partial |
| confidence.rs `confidence_mark(level, label)` | ConfidenceMark | done | block, transaction, identity | Done |

`pages/mempool::background(model)` is called once at start by `pages::start_background`: it
is where the Mempool engineer runs `MempoolFeed.refresh` for the whole session, as the Swift
sidebar does. It does nothing now.

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

- Fonts: Jost, Inter and IBM Plex Mono in place of Futura, SF Pro and SF Mono.
- Menus are in a menu key (section 8). Window buttons follow the desktop's layout.
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
- Open: the stubs in sections 3 and 6; `MempoolFeed` (the Mempool page's session record)
  does not run yet; `HeightDates`, the permission dialog, the settings window and the
  keyboard reference window are written but not exercised against a public API or pictured;
  `known_chains` on Linux depends on Carbide's search paths for node configurations and was
  not checked; Ctrl+Alt+L can be taken by a desktop for its lock screen.
