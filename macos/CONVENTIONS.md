# Conventions for Carbide Silica

Carbide Silica is a native macOS front end on Carbide's chain logic. These rules keep it
consistent. Read them before you change the app.

## What the app is for

The app is not a skin on the terminal app. Each view must make something easier to
understand than text can, and it must lose none of Carbide's depth:

- A view answers a question that the terminal needs several screens for.
- Each mark in a diagram is the real object. A click opens that object.
- Exact figures, raw JSON and Carbide's statements of uncertainty stay one step away.

The reference for what Carbide does is `docs/reference.md`, then the terminal source in
`src/app.rs` and `src/app/`. Parity means the app can do what the terminal does. It does
not mean the same keys or the same layout.

## Architecture

```
src/desktop_bridge.rs          Crate root: Carbide's modules, without the terminal UI
macos/bridge/src/ffi.rs        Shared bridge types, helpers and the Explorer object
macos/bridge/src/ffi_<area>.rs One module per area, with its own tests file
macos/Sources/CarbideSilica    The SwiftUI app
```

- **Do not change Carbide's files.** Only `src/desktop_bridge.rs` and files below `macos/`
  belong to this project. Upstream merges must stay free of conflicts.
- **Use Carbide's code.** Call the core modules (`rpc`, `defi`, `witnesses`,
  `relationships`, `currency_activity`, `block_forecast`, `block_time`, `bookmarks`,
  `sources`, `cache`, `chain_state`, `reconciliation`, `geoip`, `log_feed`) before you
  write new logic. Logic that lives in `src/app/` is tied to the terminal. Port only the
  pure part, and say in a comment where it came from.
- **The bridge is read-only.** The peer commands in `ffi.rs` are the only calls that
  change the node. Do not add a general "call any RPC method" entry. The node can hold a
  wallet. Never create, sign or send a transaction.
- **Each area adds to its own module.** Put new records and a new
  `#[uniffi::export(async_runtime = "tokio")] impl Explorer` block in `ffi_<area>.rs`.
  The module can use all of `ffi.rs` through `use super::*`. Keep the names and
  signatures of the shared helpers (`settle`, `fail`, `readable`, `text`, `strings`,
  `count`, `sats`, `addresses`, `amounts`, `pretty`, `without`, `fetch_block`,
  `block_row`, `labels`).
- **Requests through a public API can fail.** Pass results through `self.settle(...)`.
  Some methods are refused there. Say so plainly in the result; do not hide the limit.
- **Slow node requests.** `getchaintips` takes about 13 seconds on a full node and makes
  the node's other answers wait. Do not put a slow request on a timer.
- **Which client.** `self.client` is for automatic requests. It never asks the person
  anything. `self.primary` is only for the main object of a page or search that the
  person opened. It can ask for permission to use a disabled API.
- **Provenance.** Pass an object request through `self.observed(method, params, result)`
  and put its `Provenance` on the record. The page shows it with `ProvenanceNote`. When
  no source answers, `observed` serves the stored observation.
- **Bulk reads are not recorded.** Each recorded block adds approximately 17 KB to the
  cache, and Carbide does not prune block bodies. Use `self.unrecorded_client()` for a
  request that reads many blocks, such as a chart.
- **Other chains.** Use `client_for_system` for an object on another chain. Never send a
  request to a disabled source.
- **Requests from one page to another** go through `AppModel`: `showCharts(at:hash:)`,
  `openTrace(txid:output:)` and `openCurrency(_:at:)`.

## Tests

- Unit tests go in `ffi_<area>_tests.rs`, included with
  `#[cfg(test)] #[path = "ffi_<area>_tests.rs"] mod tests;`.
- Test the parsing and the logic with small JSON samples. Do not write tests that only
  check that a mock was called.
- Live tests are `#[ignore]`. Use `super::super::tests::live_explorer()`. Run them with:

```sh
CARBIDE_TEST_CONF="/Volumes/Extreme SSD/CarbideSilica-VRSC/VRSC.conf" \
  cargo test --manifest-path macos/bridge/Cargo.toml --lib <name> -- --ignored --nocapture
```

- Tests use their own cache folder. Do not point a test at the app's cache.
- Session source states are global to the process. Restore each state that a test changes.
- Do not print the contents of the `.conf` file. It holds the RPC password.

## Text

Write in the style of ASD-STE100 Simplified Technical English.

- One term for one thing:

| Use | Not |
| --- | --- |
| node | daemon, local daemon |
| side branch | fork, losing fork, orphaned branch |
| unconfirmed | waiting, pending (for a transaction) |
| synchronize, synchronization | sync, syncing |
| request | lookup, query, call |
| stored | cached, retained (for data shown to the person) |
| transaction | tx (in sentences) |

- No contractions. Write "cannot", "does not", "did not".
- Short sentences in the active voice. Maximum 20 words for an instruction, 25 for a
  description. One instruction in one sentence.
- Simple present tense, or simple past for a measured fact.
- "Possibly" and "approximately", not "probably" and "about".
- Say what the app cannot show, and why. Keep Carbide's statements of uncertainty.
- No filler and no devices: no colon reveals, no "not X but Y", no semicolon joins, no
  em-dash asides, no rhetorical triads, no "simply", "just", "actually".
- Carbide's own stage labels and evidence text come from Carbide's code. Leave them.
- Technical names are correct as they are: mempool, notarization, PBaaS, staked, UTXO.

## Layout

- A section is `Card(title) { ... }`: a rule and a heading. It has no fill. Use a fill
  only for content that is a separate object, such as a block of raw bytes.
- Do not open a page with large number tiles.
- An icon appears only when it means something. Use `Note(text:warning:)` for a sentence:
  it shows a triangle only for a warning. Do not put an icon before a neutral sentence.
- No large glyph on an empty state. The words are enough.
- State uses shape, colour and a word together. Never colour alone.
- Confidence uses `ConfidenceMark` with one of four levels: verified, observed, candidate,
  unknown.
- Show a name before an ID. Call `model.resolveNames(ids)` and use `RouteLink` or
  `FieldRow`, which show the name when it is known. A currency name has no trailing "@".
- Follow the display settings. `DisplayKey.showsNames` and `DisplayKey.showsDates` are in
  `Display.swift`. `RouteLink`, `FieldRow` and `JSONSection` follow them. Use
  `HeightDates.shared` for a height or a confirmation count.
- One scroll view on a page. Do not put a scrolling list or table inside the page scroll.
- A page that a menu command can act on publishes focused values (`ShellCommands.swift`):
  `placeActions`, `adjacentObjects` and `investigations`.
- Amounts always have a ticker. A hidden amount is shown as hidden, never as zero.
- Every hash, address and ID that names an object is a link to that object.
- A diagram has a caption that says what its marks mean, and a text or table form of the
  same data on the page.
- Use macOS conventions: segmented controls, standard dialogs, a confirmation before a
  change.

## Code

- Match the comment style in the file. A comment says what the code cannot.
- Swift views are `@MainActor` by default. Keep slow work off the main thread.
- A view that loads data uses `LoadView` or a `.task(id:)` keyed on `model.generation`,
  so it reloads when the connection changes.
- Do not start a `.task` on an empty `Group`. An empty group has no view to attach to.

## Build and check

```sh
cd macos && ./build.sh          # bridge, bindings, app, bundle
cargo test --manifest-path macos/bridge/Cargo.toml --lib ffi::
```

`macos/.cargo/config.toml` keeps release proc-macro libraries unstripped. Without it, a
clean build fails on recent macOS versions. Do not remove it.

Do not stop or restart the Verus node. Do not download anything.
