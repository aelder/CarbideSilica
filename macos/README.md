# Carbide Silica

A native macOS explorer for Verus, built in SwiftUI on the chain logic of
[Carbide](https://github.com/alexenglish/VerusCarbide). It is a separate front
end beside the terminal app. It does not change or replace `vcarbide`.

Like Carbide, it is read-only. It never creates or signs a transaction.

## What it does

The app can do what Carbide's terminal app does. `docs/reference.md` is the reference.
The app shows the same data as pictures where a picture is clearer, and keeps the exact
figures and the raw JSON one step away.

**Places**

- Dashboard: a status line (height against the longest chain, age of the last block, node
  and API state, peers), the chain pulse (bursts of blocks on a time axis, with the current
  wait set among a day of recorded waits), recent blocks, and recent confirmed
  transactions with their tags. Stream blocks keeps the newest block in view.
- Mempool: a waiting list on a time record. One sentence says what waits and how the
  wait since the last block compares with recent waits. The last 15 minutes are one
  axis with each block on it and a line for each transaction from its arrival to its
  block. Each waiting transaction is a row with its tags, the sentence of its
  statement, its wait, fee, size, expiry and waiting parents, in order of arrival. One
  bar shows the part of a block that the queue fills. Below, the transactions that the
  last 30 blocks took, with the wait of each one that the app saw. The second view is
  the terminal's list with arrival, fee, size, entry height, unconfirmed parents and
  tags. `design/mempool-research.html` has the study behind this form.
- Charts: block rate, mining and staking difficulty with MA100 lines, the staked share
  over 100 and 1,000 blocks, and the fee pool, as five panels on one time axis with one
  cursor through all of them. The block rate has process limits and the staked share has
  the limits of chance, so a signal is distinct from noise. Spans are 2 hours, 6 hours,
  1 day or 7 days of header time. You can page to older and newer ranges, go to a block or
  a date, and select a block to see its exact figures. `design/charts-research.html` has
  the reasons for the form.
- Future block estimates from Carbide's calibrated rate, with the calibration shown.
- Currencies: Carbide's filters. Each option flag is Any, Show or Hide, and there are
  launch-status filters. The default view is liquidity pools without failed launches.
- Marketplace: the open offers as a catalogue. An answer line (offers, sellers, the share
  of the largest seller, lowest and median price), the prices that many offers share as
  bars, one curve of the offers available at or below each price, and a list with a name
  search and filters that show their counts: kind of name, length, price, seller and
  listing period. Carbide's four sort orders and the links to each offer's identity,
  currencies, funding transaction and addresses stay. `design/market-research.html` has
  the reasons for the form.
- Bookmarks, in Carbide's own bookmark file, including notarization views with their
  perspective, manual entry, and Carbide's optional start-up population.
- Node and peers: synchronization progress, validation speed, peers and bans in one
  list, network totals, a peer page with the node log and raw JSON, a map of peers, and
  side branches. The peer list starts with one sentence and a count of the peers by
  network operator and by country. Each row has its measures on scales that all rows
  share: the response time as a dot with the range of the session, and the data received
  as a bar. The map opens on the area that holds all placed peers, joins near peers into
  one mark, and is drawn by the app from the coast points of the terminal's map: it
  makes no request to a map service. `design/peers-research.html` has the study behind
  this form.
- Sources: each node and public API with its state (Disabled, Enabled, Monitored), its
  traffic and its recorded activity.

**Objects**

- Search by block height or hash, transaction ID, date, address, identity (`name@`) or
  currency (`name@@`). Search also accepts `currencies`, and `estimate:` with a height or
  a date. A future height or date opens an estimate.
- Blocks: header, chain status, the tags of all transactions, stored child branches, and
  a link to the active block when the block is on a side branch.
- Transactions: Carbide's overview (status, totals, each fee variant, shielded and
  registration lines), a statement, inputs and outputs with Carbide's annotations and
  maturity badges, and a byte view for unrecognised output data. The statement is one
  sentence, then one row for each party with its net change in each currency: a hollow
  bar for an amount given, a solid bar for an amount received. The reserve deposits of a
  currency are one party. An amount that only passes through is a figure, outputs
  without value are named marks, and a reserve transfer shows what it asks for. A row
  marks its inputs and outputs in the lists. `design/txflow-research.html` has the study
  behind this form.
- Transfer trace: Carbide's DeFi trail with one verdict, then a receipt that reads from
  the amount sent to the amount received with the fee and the rate of the batch, the
  stages on an axis of blocks so that the wait for an export is a length, and the place
  of the transfer in its export batch. Below are live progress, leg budgets, execution
  rates, a comparison with the previous pool notarization, and searches of other
  destination windows. `design/trace-research.html` has the study behind this form.
- Witnesses and protocol references, forward and reverse, with Carbide's status for each.
- Identities at each version: the history as lanes for the keys, the revoke authority,
  the recover authority and the lock, with one step for each version and a time line of
  the true blocks. The control of the viewed version is one sentence, a table of the four
  powers, and the outcome of a lost key and of a stolen key in words.
  `design/identity-research.html` has the study behind this form. Registration evidence, funding candidates, the referral
  chain, verified registration payments, tokenized control, and historical reverse
  requests.
- Baskets in one unit: the Currency page of a basket has a control that selects one of
  its reserves as the unit. The reserves table then shows the value of each reserve and
  what one of it is worth in that unit. The lines over time show the basket and each
  reserve in that unit on one scale that starts at 100, and the last conversion batch is
  a statement with one bar for each reserve that moved. The values are the pool's own
  rates. `design/currency-research.html` has the study behind this form.
- Currencies: definition, reserves, supply and prices over time, the state at an exact
  block, imports and exports in 1,000-block windows, the conversion batch, and the
  notarization browser with checks of each foreign proof root. The browser gives the
  state of the link first: the wait since the last confirmed notarization against the
  earlier intervals, on one scale. The entries that the node has are a graph: the best
  chain is the straight line, and a side branch is a second line.
  `design/notarization-research.html` has the study behind this form.
- Addresses: balances, first and latest activity, the identities that use the address as
  a primary address, and transactions as input and output tiles.
- Raw JSON for each object as a tree with links, names, dates, copy and save.

**Sources and stored data**

- Each page shows the source of its data, and shows if the data is live or stored.
- The app records the answers for the pages you open in Carbide's cache. When no source
  answers, a page that you opened before shows the stored observation, marked as stored.
- Charts read blocks without adding them to the cache. A chart load does not make the
  cache grow.
- When the node cannot answer a page or a search, the app asks before it sends the
  request to a disabled public API. Automatic requests never ask.
- Block history and the child branches of a block come from stored observations. A
  background check keeps the stored chain current for the connected node and for each
  monitored node.

**Display and keyboard**

- View > Show Names for IDs, Show Dates for Heights and Show Times in UTC. The same
  settings are in Settings.
- The keys of the terminal work in the main window when no text field is in use.
  Each page has a cursor: j and k (or the arrow keys) move it through the rows and the
  links of the page, Return opens, Esc goes back. One letter goes to a place (d m o c),
  n and p step to the next and previous block in place, and F8 and F4 start on the
  output under the cursor. `design/power-user-review.html` has the review behind this.
- A menu command and a shortcut for each function. Help > Keyboard Shortcuts lists them
  beside the terminal keys, with the cache location and size.
- A path bar shows how you reached the open page. Back and Forward move through it.

## Start-up arguments

The arguments follow Carbide: `-chain=`, `-testnet`, `-conf=`, `-datadir=`,
`-rpcconnect=`, `-rpcport=`, `-rpcuser=`, `-rpcpassword=`, `-pollinterval=` and `-open=`,
or the same names with two dashes. They apply to one run and do not change the saved
settings.

## Known limits

- One chain is shown at a time. A block or transaction on another chain is shown as text,
  with a link to switch chain when that chain's node is configured.
- Another chain's node that the app finds on this Mac starts Disabled, as in Carbide. A
  transfer trace into that chain needs the node enabled in Sources.
- Reverse references include only transactions in Carbide's cache or opened in this run.
- A chart cannot read a newer range from a block on a side branch. The node does not
  supply it.
- Through a public API, charts load at approximately 10 blocks each second. Use the
  2-hour span.
- The local node needs the identity index (`-idindex=1`) for requests by address or
  authority. Without it, the app says so.
- The place and the network operator of a peer need location data (DB-IP City Lite and
  ASN Lite, CC BY 4.0). You install it from the Node page. The app does not download it without your confirmation.
- Not verified on live data, because no such data was available: a second PBaaS node for
  the other notarization view, a block on a side branch, a tokenized identity, and a
  mempool with transactions.

Peer commands (add node, ping, disconnect, ban, clear bans) are the only requests that
change the node. The app asks for confirmation before each one. It never creates or signs
a transaction.

## The cache

Carbide's cache is an SQLite file. The default location is
`~/Library/Caches/verus-carbide`. Settings shows its size, can clear it, and can move it
to a folder on another disk. The cache has no total size limit. Carbide keeps each block
and transaction that you open as evidence.

## Build and run

Requires macOS 15, Xcode (Swift 6) and Rust 1.88 or newer.

```sh
./build.sh
open "Carbide Silica.app"
```

To work on the Swift code in Xcode, run `./build.sh bridge` once, then open
`Package.swift`. Re-run it whenever the Rust side changes.

## Connecting

The app reads the node's RPC settings from the standard macOS location,
`~/Library/Application Support/Komodo/VRSC/VRSC.conf` (PBaaS chains below
`~/Library/Application Support/Verus/pbaas`). You can select a different file in Settings.

Sources lists each node and public API from Carbide's settings file
(`~/Library/Application Support/verus-explorer-tui/config.toml`, which the terminal app
also reads) and from the standard macOS locations. Save writes the states to that file.

A Mac with no node can allow one public API without a question, in Settings. The
operator of that API can then see the addresses and identities that you open.

## How it is put together

```
src/desktop_bridge.rs      Crate root for the bridge: Carbide's modules without the terminal UI
macos/bridge/              Rust crate with the Swift-facing API (src/ffi.rs, src/ffi_<area>.rs) and its tests
macos/Sources/CarbideCore  Swift bindings generated by UniFFI (not checked in)
macos/Sources/CarbideSilica  The SwiftUI app
```

The bridge compiles Carbide's existing source files directly, so the app uses
the same RPC client, resolver and decoding as the terminal. `src/desktop_bridge.rs`
is the only file added outside `macos/`, and no upstream file is modified, which
keeps merges from upstream free of conflicts.

Two things need attention when pulling from upstream:

- `macos/bridge/Cargo.toml` repeats the `[dependencies]` of the root
  `Cargo.toml`. Copy across any dependency changes.
- If upstream adds or removes a module in `src/main.rs`, mirror that in
  `src/desktop_bridge.rs`.

## Tests

```sh
cargo test --manifest-path bridge/Cargo.toml --lib ffi::
```

The live tests send requests to a real node. They use their own cache folder.

```sh
CARBIDE_TEST_CONF=<path to the node's .conf> CARBIDE_TEST_API=https://api.verus.services \
  cargo test --manifest-path bridge/Cargo.toml --lib ffi:: -- --ignored --test-threads=1
```

`CONVENTIONS.md` has the rules for the architecture, the tests, the text and the layout.
