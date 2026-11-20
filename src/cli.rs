use std::env;
use std::ffi::OsString;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "vcarbide",
    version,
    about = "Terminal explorer for Verus and PBaaS chains",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Test terminal clipboard forwarding without a daemon connection.
    #[arg(long)]
    pub clipboard_test: bool,
    #[arg(value_name = "OBJECT", conflicts_with = "open_object")]
    pub start_object: Option<String>,

    #[arg(long = "open", visible_alias = "object", value_name = "OBJECT")]
    pub open_object: Option<String>,

    #[arg(long = "chain", value_name = "CHAIN", default_value = "VRSC")]
    pub chain: String,

    #[arg(long = "testnet")]
    pub testnet: bool,

    #[arg(long = "datadir", value_name = "PATH")]
    pub data_dir: Option<PathBuf>,

    #[arg(long = "conf", value_name = "PATH")]
    pub config_file: Option<PathBuf>,

    #[arg(long = "rpcconnect", value_name = "HOST")]
    pub rpc_host: Option<String>,

    #[arg(long = "rpcport", value_name = "PORT")]
    pub rpc_port: Option<u16>,

    #[arg(long = "rpcuser", value_name = "USER")]
    pub rpc_user: Option<String>,

    #[arg(long = "rpcpassword", value_name = "PASSWORD")]
    pub rpc_password: Option<String>,

    #[arg(long = "api-fallback")]
    pub api_fallback: bool,

    #[arg(long = "fork-api")]
    pub fork_api: bool,

    #[arg(long = "emoji", value_enum, default_value = "auto")]
    pub emoji: EmojiMode,

    #[arg(long = "probe")]
    pub probe: bool,

    /// Replace only this network's disposable cache with the current schema, then exit.
    #[arg(long)]
    pub reset_cache: bool,

    #[arg(
        long = "scan-auto-arbitrage",
        value_names = ["POOL", "START", "END"],
        num_args = 3,
        hide = true
    )]
    pub scan_auto_arbitrage: Option<Vec<String>>,

    #[arg(long = "poll-interval", value_name = "SECONDS")]
    pub poll_interval_seconds: Option<u64>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EmojiMode {
    Auto,
    On,
    Off,
}

impl Cli {
    pub fn parse_verus_style() -> Self {
        Self::parse_from(normalize_verus_style_args(env::args_os()))
    }

    pub fn requested_object(&self) -> Option<String> {
        self.open_object
            .clone()
            .or_else(|| self.start_object.clone())
    }
}

fn normalize_verus_style_args(args: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    args.into_iter()
        .map(|arg| match arg.to_str() {
            Some("-testnet") => OsString::from("--testnet"),
            Some("-api-fallback") => OsString::from("--api-fallback"),
            Some("-fork-api") => OsString::from("--fork-api"),
            Some("-chain") => OsString::from("--chain"),
            Some("-datadir") => OsString::from("--datadir"),
            Some(value) if value.starts_with("-datadir=") => {
                OsString::from(format!("--datadir={}", &value[9..]))
            }
            Some("-conf") => OsString::from("--conf"),
            Some("-rpcconnect") => OsString::from("--rpcconnect"),
            Some("-rpcport") => OsString::from("--rpcport"),
            Some("-rpcuser") => OsString::from("--rpcuser"),
            Some("-rpcpassword") => OsString::from("--rpcpassword"),
            Some("-probe") => OsString::from("--probe"),
            Some("-open") => OsString::from("--open"),
            Some("-pollinterval") => OsString::from("--poll-interval"),
            Some("-poll-interval") => OsString::from("--poll-interval"),
            Some(value) if value.starts_with("-chain=") => {
                OsString::from(format!("--chain={}", &value["-chain=".len()..]))
            }
            Some(value) if value.starts_with("-conf=") => {
                OsString::from(format!("--conf={}", &value["-conf=".len()..]))
            }
            Some(value) if value.starts_with("-rpcconnect=") => {
                OsString::from(format!("--rpcconnect={}", &value["-rpcconnect=".len()..]))
            }
            Some(value) if value.starts_with("-rpcport=") => {
                OsString::from(format!("--rpcport={}", &value["-rpcport=".len()..]))
            }
            Some(value) if value.starts_with("-rpcuser=") => {
                OsString::from(format!("--rpcuser={}", &value["-rpcuser=".len()..]))
            }
            Some(value) if value.starts_with("-rpcpassword=") => {
                OsString::from(format!("--rpcpassword={}", &value["-rpcpassword=".len()..]))
            }
            Some(value) if value.starts_with("-pollinterval=") => OsString::from(format!(
                "--poll-interval={}",
                &value["-pollinterval=".len()..]
            )),
            Some(value) if value.starts_with("-poll-interval=") => OsString::from(format!(
                "--poll-interval={}",
                &value["-poll-interval=".len()..]
            )),
            Some(value) if value.starts_with("-open=") => {
                OsString::from(format!("--open={}", &value["-open=".len()..]))
            }
            _ => arg,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::normalize_verus_style_args;
    use std::ffi::OsString;

    #[test]
    fn normalizes_verus_style_flags() {
        let args = vec![
            OsString::from("vex"),
            OsString::from("-chain=chips10sec"),
            OsString::from("-testnet"),
            OsString::from("-pollinterval=5"),
            OsString::from("-open=abc"),
            OsString::from("123"),
        ];

        let normalized = normalize_verus_style_args(args);

        assert_eq!(normalized[1].to_str(), Some("--chain=chips10sec"));
        assert_eq!(normalized[2].to_str(), Some("--testnet"));
        assert_eq!(normalized[3].to_str(), Some("--poll-interval=5"));
        assert_eq!(normalized[4].to_str(), Some("--open=abc"));
        assert_eq!(normalized[5].to_str(), Some("123"));
    }
}
