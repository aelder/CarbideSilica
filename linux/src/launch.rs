//! The start-up arguments, read once with Carbide's own rules (`src/cli.rs`) through the
//! bridge: `-chain=`, `-testnet`, `-conf=`, `-datadir=`, `-rpcconnect=`, `-rpcport=`,
//! `-rpcuser=`, `-rpcpassword=`, `-pollinterval=` and `-open=`, or the same flags with two
//! dashes. The Linux app adds no flags of its own.

use std::sync::OnceLock;
use std::time::Duration;

use crate::bridge::api;

static ARGUMENTS: OnceLock<api::StartArguments> = OnceLock::new();

pub fn arguments() -> &'static api::StartArguments {
    ARGUMENTS.get_or_init(|| api::start_arguments(std::env::args().collect()))
}

/// Seconds between dashboard refreshes. `-pollinterval` sets it, as in Carbide (minimum 1
/// second). Without it the app keeps its own default of 15 seconds.
pub fn refresh_interval() -> Duration {
    Duration::from_secs(arguments().poll_interval_seconds.map(|s| s.max(1)).unwrap_or(15))
}
