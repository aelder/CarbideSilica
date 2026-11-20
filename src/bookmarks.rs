//! Network-specific user bookmarks. Files are independent of config and cache.
use crate::{
    config::Config,
    domain::ObjectRoute,
    resolver::{classify_start_object, currency_query},
    world::{Network, SystemId},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{fs, io::Write, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub fetch_missing_on_startup: bool,
    pub startup_limit: usize,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            fetch_missing_on_startup: false,
            startup_limit: 20,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Auto,
    Block,
    Transaction,
    Address,
    Identity,
    Currency,
    Notarizations,
}
impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Block => "block",
            Self::Transaction => "transaction",
            Self::Address => "address",
            Self::Identity => "identity",
            Self::Currency => "currency",
            Self::Notarizations => "notarizations",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bookmark {
    pub version: u32,
    pub system_id: SystemId,
    pub kind: Kind,
    pub target: String,
    pub description: String,
    pub created_at: u64,
}
impl Bookmark {
    pub fn new(
        system_id: SystemId,
        kind: Kind,
        target: String,
        description: String,
    ) -> Result<Self> {
        let route = classify_start_object(Some(&target));
        let target = match route {
            ObjectRoute::Hash(hash) => hash,
            ObjectRoute::BlockHeight(h) => h.to_string(),
            _ => target.trim().to_owned(),
        };
        let mut value = Self {
            version: 1,
            system_id,
            kind,
            target,
            description,
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs(),
        };
        value.normalize_target();
        value.validate()?;
        Ok(value)
    }
    /// Store the RPC target separately from bookmark and search notation.
    /// Legacy bookmarks are normalized in memory when read.
    fn normalize_target(&mut self) {
        if self.kind == Kind::Auto
            && let Some(target) = self.target.strip_prefix("notarizations:")
        {
            self.kind = Kind::Notarizations;
            self.target = target.trim().to_owned();
        }
        if matches!(self.kind, Kind::Auto | Kind::Identity | Kind::Currency)
            && let Some(target) = currency_query(&self.target)
        {
            self.kind = Kind::Currency;
            self.target = target.to_owned();
        }
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported bookmark version {}",
            self.version
        );
        ensure!(
            self.description.len() <= 4096 && !self.description.chars().any(char::is_control),
            "description must be a single line of at most 4096 bytes"
        );
        ensure!(
            self.target.len() <= 256
                && self.target.trim() == self.target
                && !self.target.chars().any(char::is_control),
            "invalid bookmark target"
        );
        let route = classify_start_object(Some(&self.target));
        ensure!(
            match self.kind {
                Kind::Auto => !matches!(
                    route,
                    ObjectRoute::Tip
                        | ObjectRoute::Unknown(_)
                        | ObjectRoute::BlockTime(_)
                        | ObjectRoute::Estimate(_)
                ),
                Kind::Block => matches!(route, ObjectRoute::BlockHeight(_) | ObjectRoute::Hash(_)),
                Kind::Transaction => matches!(route, ObjectRoute::Hash(_)),
                Kind::Address => matches!(route, ObjectRoute::Address(_)),
                Kind::Identity => matches!(route, ObjectRoute::Identity(_)),
                Kind::Currency | Kind::Notarizations =>
                    !self.target.is_empty()
                        && !self.target.ends_with('@')
                        && currency_query(&self.target).is_none(),
            },
            "target does not match {}",
            self.kind.label()
        );
        Ok(())
    }
    pub fn key(&self) -> (SystemId, Kind, String) {
        (self.system_id.clone(), self.kind, self.target.clone())
    }
    pub fn requests(&self) -> Vec<(Kind, &'static str, Vec<Value>)> {
        let kind = if self.kind == Kind::Auto {
            match classify_start_object(Some(&self.target)) {
                ObjectRoute::BlockHeight(_) => Kind::Block,
                ObjectRoute::Address(_) => Kind::Address,
                ObjectRoute::Identity(_) => Kind::Identity,
                _ => Kind::Auto,
            }
        } else {
            self.kind
        };
        let tx = (
            Kind::Transaction,
            "getrawtransaction",
            vec![json!(self.target.to_ascii_lowercase()), json!(1)],
        );
        let block = (
            Kind::Block,
            "getblock",
            vec![json!(self.target.to_ascii_lowercase()), json!(1)],
        );
        match kind {
            Kind::Auto => vec![tx, block],
            Kind::Transaction => vec![tx],
            Kind::Block => vec![block],
            Kind::Address => vec![(
                kind,
                "getaddressbalance",
                vec![json!({"addresses":[self.target], "friendlynames":true})],
            )],
            Kind::Identity => vec![(kind, "getidentity", vec![json!(self.target)])],
            Kind::Currency | Kind::Notarizations => {
                vec![(kind, "getcurrency", vec![json!(self.target)])]
            }
        }
    }
}

pub fn path(network: Network) -> Result<PathBuf> {
    let dirs = directories::BaseDirs::new().context("data directory unavailable")?;
    Ok(dirs
        .data_local_dir()
        .join("verus-explorer-tui")
        .join(match network {
            Network::Mainnet => "bookmarks-mainnet.jsonl",
            Network::Testnet => "bookmarks-testnet.jsonl",
        }))
}

// Preserve unrecognized/malformed lines byte-for-byte even when editing a valid row.
pub struct Book {
    pub path: PathBuf,
    lines: Vec<String>,
    pub entries: Vec<(usize, Bookmark)>,
    pub issues: Vec<String>,
    original: String,
}
impl Book {
    pub fn load(path: PathBuf) -> Result<Self> {
        let original = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let lines: Vec<_> = original.split_inclusive('\n').map(str::to_owned).collect();
        let mut book = Self {
            path,
            lines,
            entries: vec![],
            issues: vec![],
            original,
        };
        for (line, text) in book.lines.iter().enumerate() {
            if text.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Bookmark>(text)
                .map_err(anyhow::Error::from)
                .and_then(|mut b| {
                    b.normalize_target();
                    b.validate()?;
                    Ok(b)
                }) {
                Ok(b) => book.entries.push((line, b)),
                Err(e) => book.issues.push(format!("Line {}: {e}", line + 1)),
            }
        }
        Ok(book)
    }
    pub fn add(&mut self, bookmark: Bookmark) -> Result<()> {
        bookmark.validate()?;
        ensure!(
            !self.entries.iter().any(|(_, b)| b.key() == bookmark.key()),
            "already bookmarked; edit its description instead"
        );
        let mut lines = self.lines.clone();
        if let Some(last) = lines.last_mut()
            && !last.ends_with('\n')
        {
            last.push('\n');
        }
        lines.push(format!("{}\n", serde_json::to_string(&bookmark)?));
        self.save(lines)
    }
    pub fn edit(&mut self, selected: usize, description: String) -> Result<()> {
        let (line, mut b) = self
            .entries
            .get(selected)
            .context("no bookmark selected")?
            .clone();
        b.description = description;
        b.validate()?;
        let mut lines = self.lines.clone();
        lines[line] = format!("{}\n", serde_json::to_string(&b)?);
        self.save(lines)
    }
    pub fn remove(&mut self, selected: usize) -> Result<()> {
        let line = self
            .entries
            .get(selected)
            .context("no bookmark selected")?
            .0;
        let mut lines = self.lines.clone();
        lines.remove(line);
        self.save(lines)
    }
    fn save(&mut self, lines: Vec<String>) -> Result<()> {
        let parent = self.path.parent().context("bookmark path has no parent")?;
        fs::create_dir_all(parent)?;
        let lock = self.path.with_extension("jsonl.lock");
        let _guard = FileGuard::create(lock)
            .context("bookmarks are being edited by another process; reload and retry")?;
        let disk = match fs::read_to_string(&self.path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        ensure!(
            disk == self.original,
            "bookmark file changed externally; press r to reload before editing"
        );
        let temporary = self
            .path
            .with_extension(format!("jsonl.{}.tmp", std::process::id()));
        let mut file = FileGuard::create(temporary)?;
        file.file.write_all(lines.concat().as_bytes())?;
        file.file.sync_all()?;
        fs::rename(&file.path, &self.path)?;
        *self = Self::load(self.path.clone())?;
        Ok(())
    }
}
struct FileGuard {
    path: PathBuf,
    file: fs::File,
}
impl FileGuard {
    fn create(path: PathBuf) -> Result<Self> {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self { path, file })
    }
}
impl Drop for FileGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub kind: Kind,
    pub raw: Value,
    pub source: String,
}

pub fn cached(cache: &crate::cache::Cache, bookmark: &Bookmark) -> Result<Option<Snapshot>> {
    bookmark.validate()?;
    for (kind, method, params) in bookmark.requests() {
        if let Some((source, raw)) = cache.bookmark_response(
            bookmark.system_id.as_ref(),
            method,
            &serde_json::to_string(&params)?,
            &bookmark.target,
        )? {
            return Ok(Some(Snapshot { kind, raw, source }));
        }
    }
    Ok(None)
}

pub async fn fetch(config: Config, bookmark: &Bookmark, interactive: bool) -> Result<Snapshot> {
    bookmark.validate()?;
    ensure!(
        config.chain.system_id.get() == Some(&bookmark.system_id),
        "bookmark source belongs to another system"
    );
    let client = crate::rpc::RpcClient::for_config(&config)?;
    let client = if interactive {
        client
    } else {
        client.noninteractive()
    };
    let mut error = anyhow::anyhow!("unrecognized bookmark");
    for (kind, method, params) in bookmark.requests() {
        match client.call_with_raw::<Value>(method, &params).await {
            Ok(output) => {
                if kind == Kind::Transaction {
                    ensure!(
                        output.result["txid"].as_str()
                            == Some(bookmark.target.to_ascii_lowercase().as_str()),
                        "source returned a different transaction"
                    );
                }
                if kind == Kind::Block {
                    if let Ok(height) = bookmark.target.parse::<u64>() {
                        ensure!(
                            output.result["height"].as_u64() == Some(height),
                            "source returned a different height"
                        );
                    } else {
                        ensure!(
                            output.result["hash"].as_str()
                                == Some(bookmark.target.to_ascii_lowercase().as_str()),
                            "source returned a different block"
                        );
                    }
                }
                if matches!(kind, Kind::Currency | Kind::Notarizations) {
                    ensure!(
                        output.result["currencyid"]
                            .as_str()
                            .is_some_and(|id| !id.is_empty()),
                        "source returned no currency definition"
                    );
                }
                return Ok(Snapshot {
                    kind,
                    raw: output.result,
                    source: output.source_id,
                });
            }
            Err(e) => {
                // A cancelled permission must not trigger another lookup or modal.
                if e.is::<crate::sources::ApiCancelled>() {
                    return Err(e);
                }
                error = e;
            }
        }
    }
    Err(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn currency_search_syntax_never_reaches_bookmark_rpc() {
        let bookmark = Bookmark::new(
            SystemId::try_from("iTest".to_owned()).unwrap(),
            Kind::Auto,
            "vETH@@".into(),
            String::new(),
        )
        .unwrap();
        assert_eq!(bookmark.kind, Kind::Currency);
        assert_eq!(bookmark.target, "vETH");
        let [(_, method, args)] = bookmark.requests().try_into().unwrap();
        assert_eq!(method, "getcurrency");
        assert_eq!(args, vec![json!("vETH")]);
    }
    fn bookmark() -> Bookmark {
        Bookmark::new(
            SystemId::try_from("iTest".to_owned()).unwrap(),
            Kind::Block,
            "123".into(),
            "Example".into(),
        )
        .unwrap()
    }
    fn directory(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bookmark-{name}-{}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        p
    }
    #[test]
    fn malformed_rows_survive_edits_and_external_changes_are_not_overwritten() {
        let dir = directory("editing");
        let path = dir.join("bookmarks-mainnet.jsonl");
        fs::write(&path, "not json\n").unwrap();
        let mut book = Book::load(path.clone()).unwrap();
        assert_eq!(book.issues.len(), 1);
        book.add(bookmark()).unwrap();
        book.edit(0, "A new description".into()).unwrap();
        assert!(fs::read_to_string(&path).unwrap().starts_with("not json\n"));
        assert_eq!(book.entries[0].1.description, "A new description");
        book.remove(0).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "not json\n");
        fs::write(&path, "externally changed\n").unwrap();
        assert!(book.add(bookmark()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "externally changed\n");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn networks_are_separate_and_duplicate_keys_include_system_and_kind() {
        let dir = directory("networks");
        let main = dir.join("bookmarks-mainnet.jsonl");
        let test = dir.join("bookmarks-testnet.jsonl");
        let mut book = Book::load(main).unwrap();
        book.add(bookmark()).unwrap();
        assert!(book.add(bookmark()).is_err());
        let mut foreign = bookmark();
        foreign.system_id = SystemId::try_from("iOther".to_owned()).unwrap();
        book.add(foreign).unwrap();
        assert_eq!(book.entries.len(), 2);
        assert!(Book::load(test).unwrap().entries.is_empty());
        assert!(!Options::default().fetch_missing_on_startup);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cached_bookmarks_resolve_by_system_and_preserve_observation_source() {
        let dir = directory("cache");
        let cache = crate::cache::Cache::open(&dir.join("mainnet.sqlite3")).unwrap();
        cache.bind_network(Network::Mainnet).unwrap();
        let hash = "a".repeat(64);
        let b = Bookmark::new(
            SystemId::try_from("iTest".to_owned()).unwrap(),
            Kind::Auto,
            hash.clone(),
            String::new(),
        )
        .unwrap();
        let raw = json!({"result":{"txid":hash,"vin":[{"coinbase":"00"}],"vout":[]},"error":null});
        cache
            .put_raw_rpc(
                "local-daemon:test",
                "iTest",
                "getrawtransaction",
                &json!([hash, 1]).to_string(),
                &raw.to_string(),
            )
            .unwrap();
        let snapshot = cached(&cache, &b).unwrap().unwrap();
        assert_eq!(snapshot.kind, Kind::Transaction);
        assert_eq!(snapshot.source, "local-daemon:test");
        let mut foreign = b.clone();
        foreign.system_id = SystemId::try_from("iOther".to_owned()).unwrap();
        assert!(cached(&cache, &foreign).unwrap().is_none());
        let mut block = b.clone();
        block.kind = Kind::Block;
        assert!(cached(&cache, &block).unwrap().is_none());
        let identity = Bookmark::new(
            b.system_id.clone(),
            Kind::Identity,
            "VRSC@".into(),
            String::new(),
        )
        .unwrap();
        cache.put_raw_rpc("local-daemon:test","iTest","getidentity","[\"i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV\"]", &json!({"result":{"identity":{"identityaddress":"i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV","name":"VRSC"},"fullyqualifiedname":"VRSC@","blockheight":1}}).to_string()).unwrap();
        assert_eq!(
            cached(&cache, &identity).unwrap().unwrap().raw["identity"]["identityaddress"],
            "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV"
        );
        drop(cache);
        fs::remove_dir_all(dir).unwrap();
    }
}
