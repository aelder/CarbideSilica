use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use directories::BaseDirs;

use crate::cli::{Cli, EmojiMode};

#[derive(Debug, Clone)]
pub struct Config {
    pub chain: ChainSelection,
    pub daemon: DaemonConfig,
    /// Optional hydration must not request permission, including in nested loaders.
    pub api_consent: bool,
    pub cache_path: PathBuf,
    pub emoji: EmojiMode,
    pub poll_interval_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct ChainSelection {
    pub name: String,
    pub testnet: bool,
    pub system_id: std::sync::Arc<std::sync::OnceLock<crate::world::SystemId>>,
}

#[derive(Debug, Clone)]
pub struct DaemonConfig {
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub password: Option<String>,
    pub config_file: Option<PathBuf>,
    pub background: bool,
    pub api_pool: std::sync::Arc<crate::sources::ApiPool>,
    pub control: std::sync::Arc<crate::sources::Control>,
    pub observations:
        Option<tokio::sync::mpsc::UnboundedSender<crate::chain_state::RpcObservation>>,
}

impl Config {
    pub fn load(cli: &Cli) -> Result<Self> {
        let network = crate::world::Network::from_testnet(
            cli.testnet || cli.chain.eq_ignore_ascii_case("vrsctest"),
        );
        let config_file = Some(resolve_config_path(cli, network)?);
        let file_values = match config_file.as_deref() {
            Some(path) if path.exists() => read_key_value_file(path)
                .with_context(|| format!("failed to read config file {}", path.display()))?,
            _ => HashMap::new(),
        };

        let host = cli
            .rpc_host
            .clone()
            .or_else(|| get_any(&file_values, &["rpcconnect"]))
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let port = cli
            .rpc_port
            .or_else(|| get_any(&file_values, &["rpcport"]).and_then(|value| value.parse().ok()))
            .unwrap_or_else(|| {
                default_rpc_port(&cli.chain, network == crate::world::Network::Testnet)
            });
        let poll_interval_seconds = cli
            .poll_interval_seconds
            .or_else(|| {
                get_any(
                    &file_values,
                    &["pollinterval", "poll_interval", "poll_interval_seconds"],
                )
                .and_then(|value| value.parse().ok())
            })
            .unwrap_or(3)
            .max(1);

        let config = Self {
            chain: ChainSelection {
                name: cli.chain.clone(),
                testnet: network == crate::world::Network::Testnet,
                system_id: std::sync::Arc::new(std::sync::OnceLock::from(crate::world::system_id(
                    &cli.chain, network,
                )?)),
            },
            daemon: DaemonConfig {
                host,
                port,
                user: cli
                    .rpc_user
                    .clone()
                    .or_else(|| get_any(&file_values, &["rpcuser"])),
                password: cli
                    .rpc_password
                    .clone()
                    .or_else(|| get_any(&file_values, &["rpcpassword"])),
                config_file,
                observations: None,
                control: Default::default(),
                api_pool: Default::default(),
                background: false,
            },
            api_consent: true,
            cache_path: default_cache_path(network),
            emoji: cli.emoji,
            poll_interval_seconds,
        };
        if let Some(value) = file_values.get("idindex") {
            config
                .daemon
                .control
                .set_identity_index(value == "1" || value.eq_ignore_ascii_case("true"));
        }
        if let Some(id) = config.chain.system_id.get() {
            let _ = config.daemon.control.expected.set((id.clone(), network));
        }
        Ok(config)
    }

    pub fn noninteractive(&self) -> Self {
        let mut config = self.clone();
        config.api_consent = false;
        config
    }

    pub fn chain_cache_key(&self) -> String {
        self.chain
            .system_id
            .get()
            .map(ToString::to_string)
            .unwrap_or_default()
    }

    pub fn daemon_cache_key(&self) -> String {
        let locator = format!("http://{}:{}", self.daemon.host, self.daemon.port);
        crate::rpc::source_id("local-daemon", &locator)
    }

    pub fn bind_system(&self, info: &serde_json::Value) -> Result<()> {
        if let Some(testnet) = info.get("testnet").and_then(serde_json::Value::as_bool) {
            anyhow::ensure!(
                testnet == self.chain.testnet,
                "daemon belongs to the other network"
            );
        }
        let id = crate::world::SystemId::try_from(
            info.get("chainid")
                .and_then(serde_json::Value::as_str)
                .context("getinfo omitted chainid")?
                .to_owned(),
        )?;
        if let Some(existing) = self.chain.system_id.get() {
            anyhow::ensure!(
                existing == &id,
                "daemon system ID changed from {existing} to {id}"
            );
        } else if let Err(id) = self.chain.system_id.set(id) {
            anyhow::ensure!(
                self.chain.system_id.get() == Some(&id),
                "conflicting daemon system ID"
            );
        }
        Ok(())
    }
}

fn get_any(values: &HashMap<String, String>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| values.get(*key).cloned())
}

pub(crate) fn read_key_value_file(path: &Path) -> Result<HashMap<String, String>> {
    let contents = fs::read_to_string(path)?;
    let mut values = HashMap::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };

        values.insert(
            key.trim().to_ascii_lowercase(),
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string(),
        );
    }

    Ok(values)
}

pub fn standard_config_path(
    home: &Path,
    chain: &str,
    network: crate::world::Network,
) -> Result<PathBuf> {
    let filename = crate::world::chain_directory(chain, network)?;
    let base = if filename == "VRSC" || filename == "vrsctest" {
        home.join(".komodo")
    } else {
        home.join(if network == crate::world::Network::Testnet {
            ".verustest"
        } else {
            ".verus"
        })
        .join("pbaas")
    };
    Ok(base.join(&filename).join(format!("{filename}.conf")))
}

fn resolve_config_path(cli: &Cli, network: crate::world::Network) -> Result<PathBuf> {
    let home = BaseDirs::new().context("home directory unavailable")?;
    let standard = standard_config_path(home.home_dir(), &cli.chain, network)?;
    let filename = crate::world::chain_directory(&cli.chain, network)?;
    let directory = if let Some(data_dir) = &cli.data_dir {
        anyhow::ensure!(
            data_dir.is_dir(),
            "-datadir must name an existing directory"
        );
        if filename == "VRSC" || filename == "vrsctest" {
            data_dir.clone()
        } else {
            data_dir.join(&filename)
        }
    } else {
        standard
            .parent()
            .context("config path has no parent")?
            .to_path_buf()
    };
    Ok(match &cli.config_file {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => directory.join(path),
        None => directory.join(format!("{filename}.conf")),
    })
}

fn default_cache_path(network: crate::world::Network) -> PathBuf {
    BaseDirs::new()
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".cache"))
        .join("verus-carbide")
        .join(network.database_name())
}

fn default_rpc_port(chain: &str, testnet: bool) -> u16 {
    match (chain.eq_ignore_ascii_case("vrsc"), testnet) {
        (true, false) => 27486,
        (true, true) => 27484,
        _ => 27486,
    }
}

#[cfg(test)]
mod tests {
    use super::read_key_value_file;
    use std::fs;

    #[test]
    fn parses_simple_env_style_config() {
        let path = std::env::temp_dir().join("vex-config-test.conf");
        fs::write(
            &path,
            "\n# comment\nrpcuser = alice\nrpcpassword=\"secret\"\nrpcport=1234\n",
        )
        .unwrap();

        let values = read_key_value_file(&path).unwrap();

        assert_eq!(values.get("rpcuser").unwrap(), "alice");
        assert_eq!(values.get("rpcpassword").unwrap(), "secret");
        assert_eq!(values.get("rpcport").unwrap(), "1234");

        let _ = fs::remove_file(path);
    }
}
