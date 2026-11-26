<h1 align="center">Carbide Silica</h1>

<p align="center">
  A native macOS explorer for Verus and PBaaS chains.<br>
  It shows the chain as pictures and keeps the exact figures one step away.
</p>

<p align="center">
  <img alt="macOS 15 or later" src="https://img.shields.io/badge/macOS-15%2B-1c1c1c?style=flat-square&logo=apple&logoColor=white">
  <img alt="Swift 6" src="https://img.shields.io/badge/Swift-6-f05138?style=flat-square&logo=swift&logoColor=white">
  <img alt="Rust 1.88 or later" src="https://img.shields.io/badge/Rust-1.88%2B-8c4a2f?style=flat-square&logo=rust&logoColor=white">
  <img alt="Bridge made with UniFFI" src="https://img.shields.io/badge/bridge-UniFFI-55616d?style=flat-square">
  <img alt="MIT license" src="https://img.shields.io/badge/license-MIT-c9b27c?style=flat-square">
  <img alt="The app has no wallet access" src="https://img.shields.io/badge/wallet_access-none-3c8f6b?style=flat-square">
  <img alt="Status: experimental" src="https://img.shields.io/badge/status-experimental-a8842c?style=flat-square">
  <img alt="Last commit" src="https://img.shields.io/github/last-commit/aelder/CarbideSilica/carbide-silica?style=flat-square&color=55616d">
</p>

<p align="center">
  <a href="#what-you-see">What you see</a> &nbsp;
  <a href="#the-terminal-app-and-this-app">Terminal or app</a> &nbsp;
  <a href="#build-and-run">Build and run</a> &nbsp;
  <a href="#keys">Keys</a> &nbsp;
  <a href="#what-leaves-your-mac">Privacy</a> &nbsp;
  <a href="#how-it-is-built">How it is built</a> &nbsp;
  <a href="#known-limits">Known limits</a>
</p>

![The chain pulse on the Dashboard. Staked and mined blocks of the last 25 minutes are on a time axis, above a dot plot of the waits between blocks in the last 25 hours.](../macos/docs/images/chain-pulse.webp)

Carbide Silica is a second front end for [Carbide](https://github.com/alexenglish/VerusCarbide), the terminal explorer by [alexenglish](https://github.com/alexenglish). The app compiles Carbide's own source files, so it uses the same RPC client, the same decoding and the same cache as the terminal app. Only the screen is different.

The app is read-only. It does not read a wallet, and it cannot create, sign or send a transaction.

## What you see

Each picture below is the app with a live VRSC node. No picture is a mock-up.

### One sentence for each transaction

A transaction page starts with a statement in words. Below it, the flow shows each party as a band, and the width of a band is its amount. This payout has 414 outputs. The app shows the five largest and puts the other 407 addresses in one band that you can open.

![A LuckPool payout as a flow. One band on the left is the pool. On the right are the five largest recipients, one band for 407 more addresses, the shielded pool and the fee.](../macos/docs/images/statement.webp)

### Five charts on one time axis

Block rate, mining difficulty, staking difficulty, staked share and the fee pool share one time axis and one cursor. The block rate has process limits, so a red point is a period outside the usual range. The staked share has the limits of chance. The dot plot at the bottom puts the selected wait among all waits of the range.

![The Charts page for one day. Five panels share a time axis. One block rate point is red because it is outside the limits.](../macos/docs/images/charts.webp)

### The history of an identity as lanes

Each step is one version of the identity. The lanes show who held the keys, who can revoke, who can recover, and the lock. The Control card then says in words what happens when a key is lost and when a key is stolen.

![The identity vETH at version 7 of 7. Four lanes show the keys, the revoke authority, the recover authority and the lock for each version.](../macos/docs/images/identity.webp)

### More pages

| | |
|---|---|
| ![The Marketplace page with a price curve and filters that show their counts](../macos/docs/images/marketplace.webp) | ![The Bridge.vETH basket with its reserves in one unit and their values over one year](../macos/docs/images/basket.webp) |
| **Marketplace.** 1,837 open offers as a catalogue. One curve shows the number of offers at or below each price. Each filter shows its count. | **Baskets in one unit.** Select a reserve as the unit. The table and the lines then show each reserve in that unit on one scale. |
| ![The Currencies page with filter keys and a table of liquidity pools](../macos/docs/images/currencies.webp) | ![The Dashboard with the chain pulse, recent blocks and recent transactions](../macos/docs/images/dashboard.webp) |
| **Currencies.** Lamp keys filter by launch state and by option. The default view is the liquidity pools without failed launches. | **Dashboard.** The chain pulse, the recent blocks and the recent confirmed transactions with their tags. |

The app also has the Mempool, Bookmarks, Sources, a Node page with a map of the connected peers, block and address pages, the transfer trace, witnesses, the notarization browser and the raw JSON of each object. [macos/docs/features.md](../macos/docs/features.md) lists each function.

## The terminal app and this app

The two apps use one engine. They read the same node, write the same cache file and share one bookmark file and one settings file. You can use the two apps on one Mac.

### The same object in each app

The left pictures are from Carbide's own readme. The right pictures show the same transaction and the same identity in this app.

| Terminal | App |
|---|---|
| ![The coinbase transaction of block 4,226,272 in the terminal app](../docs/images/transaction.png) | ![The same coinbase transaction in Carbide Silica, with a statement and a flow](../macos/docs/images/coinbase.webp) |
| ![The identity vETH in the terminal app](../docs/images/identity.png) | ![The identity vETH in Carbide Silica](../macos/docs/images/identity.webp) |
| ![The block interval chart in the terminal app](../docs/images/charts.png) | ![The five chart panels in Carbide Silica](../macos/docs/images/charts.webp) |

### What the app does better

| Subject | Terminal | App |
|---|---|---|
| Transactions | Inputs and outputs as lists | A statement in words, a table of each party's net change, and a flow |
| Large transactions | Each output is a row | Lists above 40 rows show 20 rows first, and the flow groups the small amounts |
| Charts | One measure at a time, selected with the keys 1 to 5 | Five panels on one time axis with one cursor, process limits and limits of chance |
| Block production | A list of recent blocks | The chain pulse: bursts, gaps and the present wait against one day of waits |
| Identity history | Each version as text | All versions as lanes, and the result of a lost key and a stolen key in words |
| Marketplace | A list with four sort orders | The same list, a price curve, shared prices as bars, and filters with counts |
| Peers | A list and a world map of text characters | A drawn map with one line for each peer, and counts by network operator and country |
| Pointer | Scroll, and click to copy | Each row, link, band and chart point reacts to the pointer |
| Finding a command | Help page and footer | Command-K lists each command and each card of the open page with its key |

### What the terminal does better

| Subject | Terminal | App |
|---|---|---|
| Remote use | Runs on the server beside the node, through SSH and tmux | Runs only on a Mac. It must reach the node's RPC port from that Mac |
| Platforms | Linux. Carbide is tested on Debian 13 | macOS 15 or later only |
| Installation | One `cargo build`, one binary | Rust and Xcode, a bridge build and a Swift build. No signed download exists |
| Size and memory | One small binary in a terminal | A 31 MB app. It used approximately 300 MB of memory on the Dashboard in one measurement |
| New chain logic | Arrives here first | Arrives after a merge from Carbide, and a new function needs new bridge code |
| Tests of the interface | Terminal tests operate the real screens | The tests cover the bridge. No test operates the windows |
| PoW Ratio chart | Has it | Does not have it. The staked share panel shows a related measure |
| Diagnostics | `--probe`, `--clipboard-test` and a UI timing log | None of these |
| Text workflow | Select, copy and pipe text with terminal tools | Copy one value at a time, or save the JSON |

### No difference

- The two apps are read-only. The peer commands are the only requests that change the node, and each one asks for confirmation.
- The two apps use the same RPC client, resolver and decoding, because the app compiles Carbide's source files.
- The two apps share the cache, the bookmark file and the settings file.
- The terminal keys work in the app: `j` and `k` move the cursor, Return opens, Esc goes back, and one letter goes to a place.

### Which one to use

- The node is on a remote server, and you work through SSH. Use the terminal app.
- The node is on your Mac, or its RPC port is reachable from your Mac, and you want to see a transaction, an identity or a trend as a picture. Use this app.
- You are not sure. Use the two apps. They do not conflict.

## Build and run

You need macOS 15, Xcode with Swift 6, and Rust 1.88 or later.

```sh
git clone https://github.com/aelder/CarbideSilica.git
cd CarbideSilica/macos
./build.sh
open "Carbide Silica.app"
```

`build.sh` builds the Rust bridge, generates the Swift bindings, builds the app and signs it for local use. The first build takes some minutes.

To work on the Swift code in Xcode, run `./build.sh bridge` once, then open `Package.swift`. Run it again each time the Rust side changes.

### Connect to a node

The app reads the RPC settings of the node from the standard macOS location, `~/Library/Application Support/Komodo/VRSC/VRSC.conf`. PBaaS chains are below `~/Library/Application Support/Verus/pbaas`. You can select a different file in Settings.

A Mac without a node can use one public API. You turn this on in Settings. The operator of that API can then see the addresses and identities that you open.

### Start-up arguments

The arguments are the same as Carbide's, with one dash or two.

| Argument | Function |
|---|---|
| `-chain=` | The chain to open |
| `-testnet` | Use the testnet |
| `-conf=`, `-datadir=` | The configuration file or the data folder of the node |
| `-rpcconnect=`, `-rpcport=`, `-rpcuser=`, `-rpcpassword=` | RPC settings that replace those of the file |
| `-pollinterval=` | The time between refreshes |
| `-open=` | A block, transaction, address, identity or currency to open at start |

The arguments apply to one run. They do not change the saved settings.

## Keys

Each function has a menu command and a shortcut. Help > Keyboard Shortcuts lists all of them beside the terminal keys.

| Key | Function | Terminal key |
|---|---|---|
| `⌘K` | Open the command list | none |
| `⌘F` or `/` | Search | `/` |
| `j` `k` or the arrow keys | Move the cursor through the rows and links of the page | `j` `k` |
| Return | Open the row or link under the cursor | Enter |
| Esc | Go back one page | Esc |
| Tab | Show the raw JSON of the page | Tab |
| `d` `m` `o` `c` | Dashboard, Mempool, Marketplace, Currencies | `d` `m` `o` `c` |
| `n` `p` | Next or previous block, in place | `n` `p` |
| `F8` `F4` `F3` `F7` | Follow a transfer, witnesses, protocol links, block history | `F8` `F4` `F3` `F7` |
| `⇧⌘I` `⇧⌘D` | Names or IDs, dates or heights | `Shift-I` `Shift-D` |

## What leaves your Mac

- The app sends RPC requests to your node. With a local node, nothing leaves the Mac.
- The public API is off at the start. When the node cannot answer a page that you open, the app asks before it sends that request to the API. The app never sends background requests to an API.
- The peer map uses location data that you install from the Node page. The app downloads it from DB-IP only after you confirm. The app draws the map itself and makes no request to a map service.
- The app has no analytics and no update check.

## How it is built

```mermaid
flowchart LR
    subgraph app["Carbide Silica.app"]
        ui["SwiftUI views<br>macos/Sources/CarbideSilica"]
        core["CarbideCore<br>Swift bindings from UniFFI"]
        bridge["Bridge crate<br>macos/bridge"]
        engine["Carbide modules<br>src/, not modified"]
        ui --> core --> bridge --> engine
    end
    engine -->|RPC| node["Verus node"]
    engine -.->|only with your approval| api["Public API"]
    engine <--> cache[("SQLite cache<br>shared with the terminal app")]
```

| Path | Contents |
|---|---|
| `src/desktop_bridge.rs` | The crate root for the bridge. It has Carbide's modules without the terminal interface |
| `macos/bridge/` | The Rust crate with the API for Swift (`src/ffi.rs`, `src/ffi_<area>.rs`) and its tests |
| `macos/Sources/CarbideCore` | The Swift bindings that UniFFI generates. They are not checked in |
| `macos/Sources/CarbideSilica` | The SwiftUI app |
| `macos/design/` | The study behind each picture, as HTML pages |

This project adds two files outside `macos/`: `src/desktop_bridge.rs` and this page, `.github/README.md`. It changes no file of Carbide, so a merge from Carbide has no conflicts. Carbide's own readme is [README.md](../README.md).

Two things need attention after a merge from Carbide:

- `macos/bridge/Cargo.toml` repeats the `[dependencies]` of the root `Cargo.toml`. Copy each dependency change.
- If Carbide adds or removes a module in `src/main.rs`, make the same change in `src/desktop_bridge.rs`.

### Tests

Run the tests from the `macos` folder.

```sh
cargo test --manifest-path bridge/Cargo.toml --lib ffi::
```

323 tests pass at this commit. The live tests send requests to a real node and use their own cache folder:

```sh
CARBIDE_TEST_CONF=<path to the node's .conf> CARBIDE_TEST_API=https://api.verus.services \
  cargo test --manifest-path bridge/Cargo.toml --lib ffi:: -- --ignored --test-threads=1
```

[macos/CONVENTIONS.md](../macos/CONVENTIONS.md) has the rules for the architecture, the tests, the text and the layout.

### The cache

Carbide's cache is an SQLite file. The default location is `~/Library/Caches/verus-carbide`. Settings shows its size, can clear it, and can move it to a folder on another disk. The cache has no size limit, because Carbide keeps each block and transaction that you open as evidence. Chart loads do not add to it.

## Known limits

- The app shows one chain at a time. A block or transaction on another chain is text, with a link to change the chain when that chain's node is configured.
- A node for another chain that the app finds on the Mac starts in the Disabled state, as in Carbide. A transfer trace into that chain needs the node enabled in Sources.
- Reverse references include only the transactions in Carbide's cache or opened in this run.
- A chart cannot read a newer range from a block on a side branch. The node does not supply it.
- Through a public API, charts load at approximately 10 blocks each second. Use the 2-hour span.
- Requests by address or by authority need the identity index of the node (`-idindex=1`). Without it, the app says so.
- The place and the network operator of a peer need the location data from DB-IP.
- These cases are not verified on live data, because no such data was available: a second PBaaS node for the other notarization view, a block on a side branch, and a tokenized identity.

## Credits and license

- [Carbide](https://github.com/alexenglish/VerusCarbide) by [alexenglish](https://github.com/alexenglish) supplies all chain logic. Carbide Silica adds the macOS interface.
- Peer locations come from [DB-IP](https://db-ip.com) City Lite and ASN Lite, under CC BY 4.0.
- The interface sounds are under the MIT license. `macos/Resources/Sounds/LICENSE-button-sounds.txt` has the notice.

Carbide Silica has the same license as Carbide, the [MIT License](../LICENSE). The software is experimental and has no warranty.
