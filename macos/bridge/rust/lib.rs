//! Crate root of the bridge as a Rust library (see `Cargo.toml` beside this file).
//!
//! It declares the same modules as `src/desktop_bridge.rs`, in an inline module whose
//! `#[path]` is Carbide's `src` folder. Each module then finds its child modules where
//! Carbide keeps them. The `use` lines below give the modules the paths that their code
//! uses (`crate::rpc` and so on). Keep the list in step with `src/desktop_bridge.rs`.
#![allow(dead_code, unused_imports)]

#[path = "../../../src"]
mod upstream {
    pub(crate) mod block_forecast;
    pub(crate) mod block_time;
    pub(crate) mod bookmarks;
    pub(crate) mod cache;
    pub(crate) mod chain_state;
    pub(crate) mod cli;
    pub(crate) mod clipboard;
    pub(crate) mod config;
    pub(crate) mod copy_values;
    pub(crate) mod currency_activity;
    pub(crate) mod currency_status;
    pub(crate) mod defi;
    pub(crate) mod display_time;
    pub(crate) mod domain;
    pub(crate) mod geoip;
    pub(crate) mod height_display;
    pub(crate) mod log_feed;
    pub(crate) mod protocol;
    pub(crate) mod reconciliation;
    pub(crate) mod relationships;
    pub(crate) mod resolver;
    pub(crate) mod rpc;
    pub(crate) mod sources;
    pub(crate) mod theme;
    pub(crate) mod witnesses;
    pub(crate) mod world;

    #[path = "../macos/bridge/src/ffi.rs"]
    pub mod ffi;
}

pub(crate) use upstream::{block_forecast, block_time, bookmarks, cache, chain_state, cli, clipboard, config, copy_values, currency_activity, currency_status, defi, display_time, domain, geoip, height_display, log_feed, protocol, reconciliation, relationships, resolver, rpc, sources, theme, witnesses, world};

/// The bridge module, as in `src/desktop_bridge.rs`.
pub use upstream::ffi;
/// Every record, enum and function of the bridge in one public path.
pub use upstream::ffi::rust_api as api;

uniffi::setup_scaffolding!();
