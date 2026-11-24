//! Crate root for the desktop bridge (see `macos/bridge/Cargo.toml`).
//!
//! The terminal binary does not compile this file: its root is `main.rs`.
//! This root lives beside Carbide's modules so that `mod` declarations resolve
//! to the same source files, and it leaves out only `app`, the terminal UI.
//! The bridge's own code is in `macos/bridge/src`.
#![allow(dead_code, unused_imports)]

mod block_forecast;
mod block_time;
mod bookmarks;
mod cache;
mod chain_state;
mod cli;
mod clipboard;
mod config;
mod copy_values;
mod currency_activity;
mod currency_status;
mod defi;
mod display_time;
mod domain;
mod geoip;
mod height_display;
mod log_feed;
mod protocol;
mod reconciliation;
mod relationships;
mod resolver;
mod rpc;
mod sources;
mod theme;
mod witnesses;
mod world;

#[path = "../macos/bridge/src/ffi.rs"]
mod ffi;

uniffi::setup_scaffolding!();
