# Carbide Silica

A native macOS explorer for [Verus](https://verus.io) and PBaaS chains.

Carbide Silica is a fork of [Carbide](https://github.com/alexenglish/VerusCarbide),
the terminal explorer and toolkit by Alex English. It puts a SwiftUI front end
on Carbide's chain logic: the same RPC client, search resolver and transaction
decoding, in a Mac app instead of a terminal.

Like Carbide, it is read-only. It connects to your local Verus daemon, uses no
wallet data, and never creates or signs transactions.

> **Status:** early and experimental. It comes with no warranty for any purpose.

## What it does

- Dashboard with chain status and a live list of recent blocks
- Search by block height or hash, transaction ID, date, address, identity
  (`name@`) or currency (`name@@`)
- Block, transaction, identity, currency and address views, linked to one
  another, each with the daemon's raw JSON

Carbide's charts, mempool, notarization, DeFi and bookmark views are not in the
Mac app yet.

## Build and run

Requires macOS 15, Xcode and Rust 1.88 or newer.

```sh
cd macos
./build.sh
open "Carbide Silica.app"
```

See the [macOS guide](../macos/README.md) for connection settings, project
layout and tests.

## Relationship to Carbide

This repository contains all of Carbide and tracks it. The terminal app still
builds and runs from here exactly as upstream documents in the
[Carbide README](../README.md):

```sh
cargo build --release --locked
./target/release/vcarbide
```

Carbide Silica's own code is in [`macos/`](../macos), plus one file,
`src/desktop_bridge.rs`. No Carbide source file is modified, so upstream changes
merge cleanly. Bugs in the shared chain logic belong upstream as
[Carbide issues](https://github.com/alexenglish/VerusCarbide/issues).

## License

MIT, the same as Carbide. See [LICENSE](../LICENSE).
