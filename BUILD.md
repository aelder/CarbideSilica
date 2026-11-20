# Build Notes

Verus Carbide is initially distributed as source.

## Local Development

Use a Rust toolchain with `rustc` 1.88 or newer. The apt packages on some distributions may lag behind this and may also omit Cargo subcommands such as `cargo fmt`.

Recommended setup for this project:

```sh
rustup toolchain install stable
rustup default stable
rustup component add rustfmt clippy
```

If using distro packages instead, install the formatter package too:

```sh
sudo apt install rustfmt
```

That only fixes `cargo fmt`; it does not fix an older compiler. If `rustc --version` is below 1.88, use rustup or a newer distro package source.

```sh
cargo fmt
cargo check
cargo test
cargo run -- -chain=VRSC --poll-interval 3
cargo run -- --probe
```

## Personal builds for Debian 12 and 13

For personal use on x86-64 Debian 12 and 13 machines, build on Debian 12 with the Rust toolchain described above. From the repository root, run:

```sh
cargo build --release --locked
./target/release/vcarbide --version
```

Copy `target/release/vcarbide` to the Debian 13 machine and run `./vcarbide --version` there to check startup. Building on the older system keeps the glibc requirement within Debian 12’s baseline. A build made on Debian 13 can require newer glibc symbols than Debian 12 provides.

## If we distribute Linux binaries

Before publishing binaries, choose the oldest supported distribution and CPU architectures. Evaluate a dynamic glibc build on that baseline and a static musl build. Test the chosen artifact on each supported system, including startup, local daemon RPC and HTTPS API hostname resolution.

For an x86-64 musl evaluation:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

HTTP transport uses Rustls for HTTPS API endpoints and avoids native TLS/OpenSSL.
The current `public-api` Cargo feature is empty; API access is controlled by
runtime endpoint configuration and consent, not a build-time feature gate.

SQLite is configured through `rusqlite` with bundled SQLite so the release binary does not depend on the target host's SQLite installation.

## First Verification Matrix

- Plain SSH
- SSH inside tmux
- Truecolor terminal
- 256-color terminal
- Emoji-capable terminal
- Emoji fallback mode
