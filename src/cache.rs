use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Map, Value};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
mod currency;
mod currency_activity;
pub mod defi;
pub mod identity;
pub mod investigation;
mod lifecycle;
mod reconciliation;
#[cfg(test)]
mod regression_tests;

pub const SCHEMA_VERSION: i64 = 6;
pub const DECODER_VERSION: i64 = 3;

#[derive(Debug, Clone)]
pub struct SourceEvent {
    pub key: String,
    pub message: String,
    pub observed_at: u64,
}

/// Rechecked by the writer after queueing, not only when HTTP finishes.
#[derive(Debug, Clone)]
pub struct Receipt {
    pub branch_epoch: u64,
    pub control: Arc<crate::sources::Control>,
    pub generation: u64,
    pub order: u64,
    pub epoch: u64,
    pub once: bool,
}
impl Receipt {
    pub fn current(&self) -> bool {
        if self.branch_epoch != self.control.branch_epoch.load(Ordering::Acquire) {
            return false;
        }
        if self.once {
            self.control.generation() == self.generation
        } else {
            self.control.accepts(self.generation)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BlockContext {
    pub height: u64,
    pub hash: String,
}

#[derive(Clone, Debug)]
pub struct Cache {
    worker: std::sync::Arc<Worker>,
}

#[derive(Debug)]
struct Worker {
    commands: std::sync::mpsc::SyncSender<Command>,
    epoch: Arc<AtomicU64>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let (reply, done) = std::sync::mpsc::sync_channel(1);
        if self.commands.send(Command::Shutdown(reply)).is_ok() {
            let _ = done.recv();
        }
    }
}

type Reply<T> = std::sync::mpsc::SyncSender<Result<T>>;
type CurrencyDefinition = (String, String, Value);

#[derive(Debug, Clone)]
pub struct CurrencySnapshot {
    pub source: String,
    pub kind: String,
    pub reported_height: Option<u64>,
    pub observed: Option<BlockContext>,
    pub current: bool,
    pub state: Value,
    pub response: Value,
}

enum Command {
    CurrencyActivityRead {
        system: String,
        currency: String,
        reply: Reply<Vec<crate::currency_activity::StoredPage>>,
    },
    CurrencyActivityWrite {
        page: crate::currency_activity::Page,
        receipt: Receipt,
        reply: Reply<()>,
    },
    DefiEdgesRead {
        root: crate::world::OutpointRef,
        reply: Reply<Vec<defi::StoredLifecycleEdge>>,
    },
    DefiEdgesWrite {
        root: crate::world::OutpointRef,
        edges: Vec<crate::defi::LifecycleEdge>,
        reply: Reply<()>,
    },
    IdentityEvidenceRead {
        system: String,
        identity: String,
        version_txid: String,
        version_height: u64,
        reply: Reply<Option<identity::EvidenceRecord>>,
    },
    IdentityEvidenceWrite {
        record: identity::EvidenceRecord,
        reply: Reply<()>,
    },
    Investigate {
        system: String,
        subject: investigation::Investigation,
        page: usize,
        reply: Reply<investigation::InvestigationPage>,
    },
    ReconciliationState {
        system: String,
        source: String,
        reply: Reply<crate::reconciliation::State>,
    },
    Reconcile {
        system: String,
        source: String,
        receipt: Receipt,
        expected: u64,
        change: crate::reconciliation::Change,
        reply: Reply<()>,
    },
    Shutdown(std::sync::mpsc::SyncSender<()>),
    Observe {
        network: crate::world::Network,
        receipt: Receipt,
        source: String,
        system: String,
        method: String,
        params: String,
        response: String,
        reply: Reply<bool>,
    },
    QualifyCurrency {
        source: String,
        system: String,
        response: Value,
        receipt: Receipt,
        block: BlockContext,
        reply: Reply<()>,
    },
    ReverseLinks {
        system: String,
        txid: String,
        reply: Reply<Vec<crate::relationships::Link>>,
    },
    Bookmark {
        system: String,
        method: String,
        params: String,
        target: String,
        reply: Reply<Option<(String, Value)>>,
    },
    #[cfg(test)]
    PutRaw {
        source: String,
        system: String,
        method: String,
        params: String,
        response: String,
        reply: Reply<()>,
    },
    GetRaw {
        source: String,
        system: String,
        method: String,
        params: String,
        reply: Reply<Option<String>>,
    },
    #[cfg(test)]
    PutCurrency {
        system: String,
        value: Value,
        reply: Reply<()>,
    },
    Definitions {
        system: String,
        reply: Reply<Vec<CurrencyDefinition>>,
    },
    Definition {
        system: String,
        currency: String,
        reply: Reply<Option<CurrencyDefinition>>,
    },
    CurrencySnapshots {
        system: String,
        currency: String,
        reply: Reply<Vec<CurrencySnapshot>>,
    },
    HistoricalCurrency {
        system: String,
        currency: String,
        source: String,
        block: BlockContext,
        state: Value,
        response: Value,
        receipt: Receipt,
        reply: Reply<()>,
    },
    #[allow(dead_code)] // Storage read interface for the forthcoming currency workspace.
    Current {
        source: String,
        block: BlockContext,
        system: String,
        currency: String,
        ttl: u64,
        reply: Reply<Option<Value>>,
    },
    RegisterSource {
        source: String,
        system: String,
        endpoint: String,
        reply: Reply<()>,
    },
    Clear(Reply<()>),
    Network(crate::world::Network, Reply<()>),
    Events(Reply<Vec<SourceEvent>>),
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self> {
        use std::sync::{Arc, Mutex, OnceLock, Weak};
        static WORKERS: OnceLock<Mutex<std::collections::HashMap<PathBuf, Weak<Worker>>>> =
            OnceLock::new();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let absolute = std::path::absolute(path)?;
        let parent = absolute
            .parent()
            .context("database path has no parent")?
            .canonicalize()?;
        let key = parent.join(
            absolute
                .file_name()
                .context("database path has no filename")?,
        );
        let mut workers = WORKERS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| anyhow::anyhow!("database worker registry poisoned"))?;
        workers.retain(|_, worker| worker.strong_count() > 0);
        if let Some(worker) = workers.get(&key).and_then(Weak::upgrade) {
            return Ok(Self { worker });
        }
        let (commands, receiver) = std::sync::mpsc::sync_channel(64);
        let (ready, started) = std::sync::mpsc::sync_channel(1);
        let db_path = key.clone();
        let epoch = Arc::new(AtomicU64::new(0));
        let writer_epoch = epoch.clone();
        std::thread::Builder::new()
            .name("explorer-database".into())
            .spawn(move || {
                let store = match StoreConnection::open(&db_path) {
                    Ok(store) => {
                        let _ = ready.send(Ok(()));
                        store
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                while let Ok(command) = receiver.recv() {
                    match command {
                        Command::CurrencyActivityRead {
                            system,
                            currency,
                            reply,
                        } => {
                            let _ = reply.send(store.currency_activity(&system, &currency));
                        }
                        Command::CurrencyActivityWrite {
                            page,
                            receipt,
                            reply,
                        } => {
                            let result = if receipt.current()
                                && receipt.epoch == writer_epoch.load(Ordering::Acquire)
                            {
                                store.record_currency_activity(&page, receipt.order)
                            } else {
                                Err(anyhow::anyhow!(
                                    "currency activity observation became obsolete"
                                ))
                            };
                            let _ = reply.send(result);
                        }
                        Command::DefiEdgesRead { root, reply } => {
                            let _ = reply.send(store.defi_edges(&root));
                        }
                        Command::DefiEdgesWrite { root, edges, reply } => {
                            let _ = reply.send(store.record_defi_edges(&root, &edges));
                        }
                        Command::IdentityEvidenceRead {
                            system,
                            identity,
                            version_txid,
                            version_height,
                            reply,
                        } => {
                            let _ = reply.send(store.identity_evidence(
                                &system,
                                &identity,
                                &version_txid,
                                version_height,
                            ));
                        }
                        Command::IdentityEvidenceWrite { record, reply } => {
                            let _ = reply.send(store.record_identity_evidence(&record));
                        }
                        Command::Investigate {
                            system,
                            subject,
                            page,
                            reply,
                        } => {
                            let _ = reply.send(store.investigate(&system, &subject, page));
                        }
                        Command::ReconciliationState {
                            system,
                            source,
                            reply,
                        } => {
                            let _ = reply.send(store.reconciliation_state(&system, &source));
                        }
                        Command::Reconcile {
                            system,
                            source,
                            receipt,
                            expected,
                            change,
                            reply,
                        } => {
                            let result = if receipt.current()
                                && receipt.epoch == writer_epoch.load(Ordering::Acquire)
                            {
                                store
                                    .commit_reconciliation(
                                        &system,
                                        &source,
                                        receipt.order,
                                        expected,
                                        &change,
                                    )
                                    .map(|()| {
                                        if !change.detached.is_empty() {
                                            receipt
                                                .control
                                                .branch_epoch
                                                .fetch_add(1, Ordering::AcqRel);
                                        }
                                    })
                            } else {
                                Err(anyhow::anyhow!(
                                    "reconciliation authorization or cache epoch changed"
                                ))
                            };
                            let _ = reply.send(result);
                        }
                        Command::Shutdown(reply) => {
                            drop(store);
                            let _ = reply.send(());
                            return;
                        }
                        Command::Observe {
                            network,
                            receipt,
                            source,
                            system,
                            method,
                            params,
                            response,
                            reply,
                        } => {
                            let result = if !receipt.current()
                                || receipt.epoch != writer_epoch.load(Ordering::Acquire)
                            {
                                Ok(false)
                            } else {
                                store.bind_network(network).and_then(|()| {
                                    store.observe(
                                        &source,
                                        &system,
                                        &method,
                                        &params,
                                        &response,
                                        receipt.order,
                                        receipt.generation,
                                    )
                                })
                            };
                            let _ = reply.send(result);
                        }
                        Command::QualifyCurrency {
                            source,
                            system,
                            response,
                            receipt,
                            block,
                            reply,
                        } => {
                            let result = if !receipt.current()
                                || receipt.epoch != writer_epoch.load(Ordering::Acquire)
                            {
                                Err(anyhow::anyhow!("currency observation became obsolete"))
                            } else {
                                store.qualify_currency(
                                    &source,
                                    &system,
                                    &response,
                                    &block,
                                    receipt.order,
                                )
                            };
                            let _ = reply.send(result);
                        }
                        Command::ReverseLinks {
                            system,
                            txid,
                            reply,
                        } => {
                            let _ = reply.send(store.reverse_links(&system, &txid));
                        }
                        Command::Bookmark {
                            system,
                            method,
                            params,
                            target,
                            reply,
                        } => {
                            let _ = reply
                                .send(store.bookmark_response(&system, &method, &params, &target));
                        }
                        #[cfg(test)]
                        Command::PutRaw {
                            source,
                            system,
                            method,
                            params,
                            response,
                            reply,
                        } => {
                            let _ =
                                reply
                                    .send(store.put_raw_rpc(
                                        &source, &system, &method, &params, &response,
                                    ));
                        }
                        Command::GetRaw {
                            source,
                            system,
                            method,
                            params,
                            reply,
                        } => {
                            let _ = reply
                                .send(store.raw_rpc_response(&source, &system, &method, &params));
                        }
                        #[cfg(test)]
                        Command::PutCurrency {
                            system,
                            value,
                            reply,
                        } => {
                            let _ = reply.send(store.put_currency_response(&system, &value));
                        }
                        Command::Definitions { system, reply } => {
                            let _ = reply.send(store.currency_definitions(&system));
                        }
                        Command::Definition {
                            system,
                            currency,
                            reply,
                        } => {
                            let _ = reply.send(store.currency_definition(&system, &currency));
                        }
                        Command::CurrencySnapshots {
                            system,
                            currency,
                            reply,
                        } => {
                            let _ = reply.send(store.currency_snapshots(&system, &currency));
                        }
                        Command::HistoricalCurrency {
                            system,
                            currency,
                            source,
                            block,
                            state,
                            response,
                            receipt,
                            reply,
                        } => {
                            let result = if receipt.current()
                                && receipt.epoch == writer_epoch.load(Ordering::Acquire)
                            {
                                store.record_historical_currency(
                                    &system,
                                    &currency,
                                    &source,
                                    &block,
                                    &state,
                                    &response,
                                    receipt.order,
                                )
                            } else {
                                Err(anyhow::anyhow!(
                                    "historical currency observation became obsolete"
                                ))
                            };
                            let _ = reply.send(result);
                        }
                        Command::Current {
                            source,
                            block,
                            system,
                            currency,
                            ttl,
                            reply,
                        } => {
                            let _ =
                                reply.send(store.current_currency_state(
                                    &source, &system, &currency, &block, ttl,
                                ));
                        }
                        Command::RegisterSource {
                            source,
                            system,
                            endpoint,
                            reply,
                        } => {
                            let _ = reply.send(
                                store
                                    .connection
                                    .execute(
                                        "insert or replace into source_endpoints values (?1,?2,?3)",
                                        params![source, system, endpoint],
                                    )
                                    .map(|_| ())
                                    .map_err(Into::into),
                            );
                        }
                        Command::Events(reply) => {
                            let _ = reply.send(store.events());
                        }
                        Command::Network(network, reply) => {
                            let _ = reply.send(store.bind_network(network));
                        }
                        Command::Clear(reply) => {
                            writer_epoch.fetch_add(1, Ordering::AcqRel);
                            let _ = reply.send(store.clear());
                        }
                    }
                }
            })?;
        started
            .recv()
            .context("database worker failed to start")??;
        let worker = Arc::new(Worker { commands, epoch });
        workers.insert(key, Arc::downgrade(&worker));
        Ok(Self { worker })
    }

    fn request<T>(&self, make: impl FnOnce(Reply<T>) -> Command) -> Result<T> {
        let (reply, result) = std::sync::mpsc::sync_channel(1);
        self.worker
            .commands
            .send(make(reply))
            .map_err(|_| anyhow::anyhow!("database worker stopped"))?;
        result.recv().context("database worker did not reply")?
    }

    pub fn epoch(&self) -> u64 {
        self.worker.epoch.load(Ordering::Acquire)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn observe(
        &self,
        network: crate::world::Network,
        receipt: Receipt,
        source: String,
        system: String,
        method: String,
        params: String,
        response: String,
    ) -> Result<bool> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::Observe {
                network,
                receipt,
                source,
                system,
                method,
                params,
                response,
                reply,
            })
        })
        .await?
    }

    pub async fn qualify_currency(
        &self,
        source: String,
        system: String,
        response: Value,
        receipt: Receipt,
        block: BlockContext,
    ) -> Result<()> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::QualifyCurrency {
                source,
                system,
                response,
                receipt,
                block,
                reply,
            })
        })
        .await?
    }

    pub async fn reverse_links(
        &self,
        system: String,
        txid: String,
    ) -> Result<Vec<crate::relationships::Link>> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::ReverseLinks {
                system,
                txid,
                reply,
            })
        })
        .await?
    }

    #[cfg(test)]
    pub fn put_raw_rpc(
        &self,
        source: &str,
        system: &str,
        method: &str,
        params: &str,
        response: &str,
    ) -> Result<()> {
        self.request(|reply| Command::PutRaw {
            source: source.into(),
            system: system.into(),
            method: method.into(),
            params: params.into(),
            response: response.into(),
            reply,
        })
    }
    pub fn bookmark_response(
        &self,
        system: &str,
        method: &str,
        params: &str,
        target: &str,
    ) -> Result<Option<(String, Value)>> {
        self.request(|reply| Command::Bookmark {
            system: system.into(),
            method: method.into(),
            params: params.into(),
            target: target.into(),
            reply,
        })
    }
    pub fn raw_rpc_response(
        &self,
        source: &str,
        system: &str,
        method: &str,
        params: &str,
    ) -> Result<Option<String>> {
        self.request(|reply| Command::GetRaw {
            source: source.into(),
            system: system.into(),
            method: method.into(),
            params: params.into(),
            reply,
        })
    }
    pub async fn raw_rpc_response_async(
        &self,
        source: &str,
        system: &str,
        method: &str,
        parameters: &str,
    ) -> Result<Option<String>> {
        let cache = self.clone();
        let (source, system, method, parameters) = (
            source.to_owned(),
            system.to_owned(),
            method.to_owned(),
            parameters.to_owned(),
        );
        tokio::task::spawn_blocking(move || {
            cache.raw_rpc_response(&source, &system, &method, &parameters)
        })
        .await?
    }

    #[cfg(test)]
    pub fn put_currency_response(&self, system: &str, value: &Value) -> Result<()> {
        self.request(|reply| Command::PutCurrency {
            system: system.into(),
            value: value.clone(),
            reply,
        })
    }
    pub fn currency_definitions(&self, system: &str) -> Result<Vec<CurrencyDefinition>> {
        self.request(|reply| Command::Definitions {
            system: system.into(),
            reply,
        })
    }
    pub fn currency_definition(
        &self,
        system: &str,
        currency: &str,
    ) -> Result<Option<CurrencyDefinition>> {
        self.request(|reply| Command::Definition {
            system: system.into(),
            currency: currency.into(),
            reply,
        })
    }
    pub fn currency_snapshots(
        &self,
        system: &str,
        currency: &str,
    ) -> Result<Vec<CurrencySnapshot>> {
        self.request(|reply| Command::CurrencySnapshots {
            system: system.into(),
            currency: currency.into(),
            reply,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn record_historical_currency(
        &self,
        system: String,
        currency: String,
        source: String,
        block: BlockContext,
        state: Value,
        response: Value,
        receipt: Receipt,
    ) -> Result<()> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::HistoricalCurrency {
                system,
                currency,
                source,
                block,
                state,
                response,
                receipt,
                reply,
            })
        })
        .await?
    }
    #[allow(dead_code)] // No currency view yet; callers must supply a source and observed block.
    pub fn current_currency_state(
        &self,
        source: &str,
        system: &str,
        currency: &str,
        block: &BlockContext,
        ttl: u64,
    ) -> Result<Option<Value>> {
        self.request(|reply| Command::Current {
            source: source.into(),
            block: block.clone(),
            system: system.into(),
            currency: currency.into(),
            ttl,
            reply,
        })
    }
    pub fn register_source(&self, source: &str, system: &str, endpoint: &str) -> Result<()> {
        self.request(|reply| Command::RegisterSource {
            source: source.into(),
            system: system.into(),
            endpoint: endpoint.into(),
            reply,
        })
    }

    pub fn events(&self) -> Result<Vec<SourceEvent>> {
        self.request(Command::Events)
    }

    pub fn bind_network(&self, network: crate::world::Network) -> Result<()> {
        self.request(|reply| Command::Network(network, reply))
    }

    pub fn clear_at(path: &Path) -> Result<u64> {
        let cache = Self::open(path)?;
        cache.request(Command::Clear)?;
        cache_disk_usage(path)
    }

    /// Explicit destructive reset of disposable data, never configuration/bookmarks.
    /// Ownership prevents resetting a database used by another current explorer.
    pub fn reset_at(path: &Path, network: crate::world::Network) -> Result<()> {
        let _ownership = lifecycle::acquire_ownership(path)?;
        let connection = Connection::open(path)?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        anyhow::ensure!(
            [0, 2, 3, 4, 5, SCHEMA_VERSION].contains(&version),
            "refusing to reset unknown schema {version}"
        );
        let allowed = [
            "currency_activity_pages",
            "reconciled_chain",
            "reconciliation_headers",
            "database_identity",
            "systems",
            "source_endpoints",
            "raw_blobs",
            "raw_rpc_cache",
            "objects",
            "object_observations",
            "relationships",
            "currency_definitions",
            "currency_definition_observations",
            "currency_current",
            "currency_states",
            "source_status",
            "observation_heads",
            "block_membership",
            "transaction_inclusions",
            "membership_history",
            "mempool_membership",
            "identity_definitions",
            "identity_observations",
            "identity_evidence_reports",
            "identity_evidence_edges",
            "defi_lifecycle_edges",
            "significant_events",
        ];
        let tables: Vec<String> = {
            let mut statement = connection.prepare(
                "select name from sqlite_master where type='table' and name not like 'sqlite_%'",
            )?;
            statement
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        anyhow::ensure!(
            tables.iter().all(|t| allowed.contains(&t.as_str())),
            "refusing to reset an unrecognized database"
        );
        let network = serde_json::to_string(&network)?;
        if tables.iter().any(|t| t == "database_identity") {
            let existing: Option<String> = connection
                .query_row(
                    "select network from database_identity where singleton=1",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            anyhow::ensure!(
                existing.as_ref().is_none_or(|n| n == &network),
                "database belongs to the other network"
            );
        }
        connection.execute_batch("pragma foreign_keys=off; pragma journal_mode=WAL;")?;
        let tx = connection.unchecked_transaction()?;
        for table in tables {
            tx.execute_batch(&format!("drop table \"{table}\";"))?;
        }
        tx.execute_batch(
            &include_str!("schema.sql")
                .replace("BEGIN IMMEDIATE;", "")
                .replace("COMMIT;", ""),
        )?;
        tx.execute("insert into database_identity values(1,?1)", [network])?;
        tx.commit()?;
        connection.execute_batch("vacuum; pragma wal_checkpoint(truncate);")?;
        Ok(())
    }
}

struct StoreConnection {
    connection: Connection,
    _ownership: Option<lifecycle::Ownership>,
    writes: std::cell::Cell<u64>,
}

impl StoreConnection {
    fn bookmark_response(
        &self,
        system: &str,
        method: &str,
        parameters: &str,
        target: &str,
    ) -> Result<Option<(String, Value)>> {
        let kind = match method {
            "getblock" => "block",
            "getrawtransaction" => "transaction",
            _ => "",
        };
        if kind == "transaction" {
            self.reproject(system, &target.to_ascii_lowercase())?;
        }
        let object: Option<(String, String)> = if !kind.is_empty() && target.len() == 64 {
            self.connection.query_row(
                "select o.source,b.payload from object_observations o join raw_blobs b on b.hash=o.payload_hash where o.system_id=?1 and o.kind=?2 and o.object_id=?3 and (?2!='block' or json_type(b.payload,'$.result.tx')='array') order by o.request_order desc limit 1",
                params![system,kind,target.to_ascii_lowercase()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?
        } else {
            None
        };
        let row = if object.is_some() {
            object
        } else {
            self.connection.query_row(
                "select r.source,b.payload from raw_rpc_cache r join raw_blobs b on b.hash=r.payload_hash
                 where r.system_id=?1 and r.method=?2 and (
                    r.params_json=?3
                    or (?2='getidentity' and (json_extract(b.payload,'$.result.identity.identityaddress')=?4 or lower(json_extract(b.payload,'$.result.fullyqualifiedname'))=lower(?4)))
                    or (?2='getcurrency' and (json_extract(b.payload,'$.result.currencyid')=?4 or lower(json_extract(b.payload,'$.result.name'))=lower(?4) or lower(json_extract(b.payload,'$.result.fullyqualifiedname'))=lower(?4)))
                 ) order by r.fetched_at desc limit 1",
                params![system,method,parameters,target], |r| Ok((r.get(0)?,r.get(1)?))).optional()?
        };
        row.map(|(source, raw)| {
            let response: Value = serde_json::from_str(&raw)?;
            let value = response
                .get("result")
                .filter(|v| v.is_object())
                .context("cached bookmark response has no object")?
                .clone();
            Ok((source, value))
        })
        .transpose()
    }
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create cache directory {}", parent.display())
            })?;
        }

        let ownership = lifecycle::acquire_ownership(path)?;
        let connection = Connection::open(path)
            .with_context(|| format!("failed to open cache database {}", path.display()))?;
        connection.busy_timeout(Duration::from_secs(60))?;
        connection.execute_batch("pragma journal_mode=WAL; pragma foreign_keys=ON;")?;
        let cache = Self {
            connection,
            _ownership: Some(ownership),
            writes: Default::default(),
        };
        cache.migrate()?;
        Ok(cache)
    }

    #[cfg(test)]
    pub fn put_raw_rpc(
        &self,
        source: &str,
        system: &str,
        method: &str,
        params_json: &str,
        response_json: &str,
    ) -> Result<()> {
        self.observe(
            source,
            system,
            method,
            params_json,
            response_json,
            crate::rpc::next_request_order(),
            0,
        )
        .map(|_| ())
    }

    #[allow(clippy::too_many_arguments)]
    fn observe(
        &self,
        source: &str,
        system: &str,
        method: &str,
        params_json: &str,
        response_json: &str,
        order: u64,
        generation: u64,
    ) -> Result<bool> {
        use crate::world::{ObjectKind, ObjectRef, SystemId};
        use sha2::{Digest, Sha256};
        let system_id = SystemId::try_from(system.to_string())?;
        let parameters: Value = serde_json::from_str(params_json)?;
        let parameters = serde_json::to_string(&parameters)?;
        let response: Value = serde_json::from_str(response_json)?;
        anyhow::ensure!(
            response.get("error").is_none_or(Value::is_null),
            "cannot cache an RPC error as an observation"
        );
        let normalized = serde_json::to_string(&response)?;
        let hash = format!("{:x}", Sha256::digest(normalized.as_bytes()));
        let tx = self.connection.unchecked_transaction()?;
        let order = i64::try_from(order)?;
        let generation = i64::try_from(generation)?;
        let managed: bool = tx.query_row(
            "select exists(select 1 from reconciled_chain where system_id=?1 and source=?2)",
            params![system, source],
            |r| r.get(0),
        )?;
        let prior_order: Option<i64> = tx.query_row("select request_order from raw_rpc_cache where source=?1 and system_id=?2 and method=?3 and params_json=?4",params![source,system,method,parameters],|r|r.get(0)).optional()?;
        if prior_order.is_some_and(|previous| previous >= order) {
            return Ok(false);
        }
        tx.execute(
            "insert or ignore into systems(system_id) values (?1)",
            [system],
        )?;
        tx.execute(
            "insert or ignore into raw_blobs(hash,payload) values (?1,?2)",
            params![hash, normalized],
        )?;
        tx.execute(
            "insert into raw_rpc_cache(source,system_id,method,params_json,payload_hash,fetched_at,request_order,generation,retention)
            values (?1,?2,?3,?4,?5,unixepoch(),?6,?7,?8) on conflict(source,system_id,method,params_json)
            do update set payload_hash=excluded.payload_hash,fetched_at=excluded.fetched_at,request_order=excluded.request_order,generation=excluded.generation,retention=excluded.retention",
            params![source, system, method, parameters, hash, order, generation, lifecycle::retention(method,&serde_json::from_str::<Value>(&parameters)?)],
        )?;
        if let Some(result) = response.get("result").filter(|v| v.is_object()) {
            let identity = match method {
                "getblock" | "getblockheader" => result
                    .get("hash")
                    .and_then(Value::as_str)
                    .map(|id| (ObjectKind::Block, id)),
                "getrawtransaction" => result
                    .get("txid")
                    .and_then(Value::as_str)
                    .map(|id| (ObjectKind::Transaction, id)),
                _ => None,
            };
            if let Some((kind, id)) = identity {
                // Parameter aliases must not let an older response replace a newer object.
                if !Self::advance_head(
                    &tx,
                    system,
                    source,
                    &format!("object:{kind:?}:{id}"),
                    order,
                )? {
                    return Ok(false);
                }
                let reference = ObjectRef {
                    system_id: system_id.clone(),
                    kind,
                    id: id.into(),
                };
                let kind = serde_json::to_value(&reference.kind)?
                    .as_str()
                    .context("invalid object kind")?
                    .to_owned();
                let mut body = result.clone();
                // These are source observations, not part of a transaction or block body.
                if let Some(fields) = body.as_object_mut() {
                    for field in [
                        "confirmations",
                        "blockhash",
                        "blocktime",
                        "in_active_chain",
                        "nextblockhash",
                    ] {
                        fields.remove(field);
                    }
                    if reference.kind == ObjectKind::Transaction {
                        Self::project_links(&tx, source, &system_id, result, &hash)?;
                        fields.remove("time");
                        fields.remove("height");
                        if let Some(outputs) = fields.get_mut("vout").and_then(Value::as_array_mut)
                        {
                            for output in outputs {
                                if let Some(fields) = output.as_object_mut() {
                                    for key in ["spentTxId", "spentIndex", "spentHeight"] {
                                        fields.remove(key);
                                    }
                                }
                            }
                        }
                    } else if let Some(transactions) =
                        fields.get_mut("tx").and_then(Value::as_array_mut)
                    {
                        for transaction in transactions {
                            if let Some(id) = transaction.get("txid").and_then(Value::as_str) {
                                *transaction = Value::String(id.into());
                            }
                        }
                    }
                }
                let body = serde_json::to_string(&body)?;
                let body_hash = format!("{:x}", Sha256::digest(body.as_bytes()));
                tx.execute(
                    "insert or ignore into raw_blobs(hash,payload) values (?1,?2)",
                    params![body_hash, body],
                )?;
                tx.execute("insert into objects(system_id,kind,object_id,payload_hash,decoder_version) values (?1,?2,?3,?4,?5) on conflict(system_id,kind,object_id) do update set payload_hash=excluded.payload_hash,decoder_version=excluded.decoder_version where ?6!='getblockheader' or json_type((select payload from raw_blobs where hash=objects.payload_hash),'$.tx') is null",params![system,kind,id,body_hash,DECODER_VERSION,method])?;
                tx.execute("insert into object_observations(system_id,kind,object_id,source,body_hash,payload_hash,observed_at,request_order)
                    values (?1,?2,?3,?4,?5,?6,unixepoch(),?7) on conflict(system_id,kind,object_id,source,body_hash)
                    do update set observed_at=excluded.observed_at,payload_hash=excluded.payload_hash,request_order=excluded.request_order",params![system,kind,id,source,body_hash,hash,order])?;
            }
        }
        if let Some(result) = response.get("result") {
            if method == "getcurrency" {
                Self::project_currency(&tx, source, system, result, &hash, order)?;
            }
            if method == "listcurrencies"
                && let Some(entries) = result.as_array()
            {
                for entry in entries {
                    let definition = entry.get("currencydefinition").unwrap_or(entry);
                    if definition["currencyid"].is_string() {
                        let mut definition = definition.clone();
                        if let Some(fields) = definition.as_object_mut() {
                            fields.remove("bestcurrencystate");
                            fields.remove("lastconfirmedcurrencystate");
                        }
                        Self::project_currency(&tx, source, system, &definition, &hash, order)?;
                    }
                }
            }
            Self::project_source_state(&tx, source, system, method, result, order, generation)?;
            if matches!(method, "getblock" | "getblockheader")
                && !managed
                && let (Some(block), Some(height), Some(confirmations)) = (
                    result.get("hash").and_then(Value::as_str),
                    result.get("height").and_then(Value::as_i64),
                    result.get("confirmations").and_then(Value::as_i64),
                )
                && Self::advance_head(&tx, system, source, &format!("height:{height}"), order)?
            {
                let membership = if confirmations > 0 {
                    "member"
                } else {
                    "orphan"
                };
                if membership == "member" {
                    let conflicts: Vec<(String, String)> = {
                        let mut statement=tx.prepare("select source,block_hash from block_membership where system_id=?1 and height=?2 and block_hash<>?3 and membership='member'")?;
                        statement
                            .query_map(params![system, height, block], |row| {
                                Ok((row.get(0)?, row.get(1)?))
                            })?
                            .collect::<rusqlite::Result<_>>()?
                    };
                    for (other_source, other_block) in conflicts {
                        if source == other_source {
                            Self::membership_transition(
                                &tx,
                                system,
                                source,
                                "block",
                                &other_block,
                                "orphan",
                                &height.to_string(),
                                order,
                            )?;
                        }
                        let message = if source == other_source {
                            format!(
                                "{system}: block at height {height} changed from {other_block} to {block} ({source})"
                            )
                        } else {
                            format!(
                                "{system}: sources disagree at height {height}: {other_source} reports {other_block}; {source} reports {block}"
                            )
                        };
                        tx.execute("insert or replace into significant_events(event_key,message,observed_at) values (?1,?2,unixepoch())",params![format!("{system}:{height}:{source}:{other_source}:{block}:{other_block}"),message])?;
                    }
                    tx.execute("update block_membership set membership='orphan' where system_id=?1 and source=?2 and height=?3 and block_hash<>?4",params![system,source,height,block])?;
                }
                // A block leaving this source's chain also invalidates its transaction inclusions.
                if membership == "orphan" {
                    tx.execute("update transaction_inclusions set confirmed=0 where system_id=?1 and source=?2 and block_hash=?3",params![system,source,block])?;
                } else {
                    tx.execute("update transaction_inclusions set confirmed=0 where system_id=?1 and source=?2 and block_hash in (select block_hash from block_membership where system_id=?1 and source=?2 and height=?3 and membership='orphan')",params![system,source,height])?;
                }
                Self::membership_transition(
                    &tx,
                    system,
                    source,
                    "block",
                    block,
                    membership,
                    &height.to_string(),
                    order,
                )?;
                tx.execute("insert into block_membership values (?1,?2,?3,?4,?5,unixepoch()) on conflict(system_id,source,block_hash) do update set membership=excluded.membership,height=excluded.height,observed_at=excluded.observed_at",params![system,source,block,height,membership])?;
            }
            if method == "getrawtransaction"
                && let Some(txid) = result.get("txid").and_then(Value::as_str)
                && Self::advance_head(&tx, system, source, &format!("transaction:{txid}"), order)?
            {
                let block = result
                    .get("blockhash")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let mut confirmed = result
                    .get("confirmations")
                    .and_then(Value::as_i64)
                    .is_some_and(|n| n > 0);
                if managed {
                    confirmed = tx.query_row("select exists(select 1 from block_membership where system_id=?1 and source=?2 and block_hash=?3 and membership='member')",params![system,source,block],|r|r.get(0))?;
                }
                if !managed {
                    tx.execute("update transaction_inclusions set confirmed=0 where system_id=?1 and source=?2 and txid=?3",params![system,source,txid])?;
                }
                tx.execute("insert into transaction_inclusions values (?1,?2,?3,?4,?5,?6,unixepoch()) on conflict(system_id,source,txid,block_hash) do update set height=excluded.height,confirmed=excluded.confirmed,observed_at=excluded.observed_at",params![system,source,txid,block,result.get("height").and_then(Value::as_i64),confirmed])?;
                Self::membership_transition(
                    &tx,
                    system,
                    source,
                    "transaction",
                    txid,
                    if confirmed {
                        "confirmed"
                    } else {
                        "unconfirmed"
                    },
                    block,
                    order,
                )?;
            }
            if method == "getidentity"
                && let (Some(name), Some(address)) = (
                    result.get("fullyqualifiedname").and_then(Value::as_str),
                    result
                        .pointer("/identity/identityaddress")
                        .and_then(Value::as_str),
                )
            {
                let name = name.to_lowercase();
                let conflict:Option<String>=tx.query_row("select name from identity_definitions where identity_address=?1 and name<>?2",params![address,name],|row|row.get(0)).optional()?;
                if let Some(other) = conflict {
                    let mut statement = tx.prepare(
                        "select distinct system_id,source from identity_observations where name=?1",
                    )?;
                    let origins = statement
                        .query_map([&other], |row| {
                            Ok(format!(
                                "{} on {}",
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(0)?
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                        .join(", ");
                    tx.execute("insert or replace into significant_events values (?1,?2,unixepoch())",params![format!("identity:{address}:{name}"),format!("Identity locator anomaly: {address}: {other} fetched from {origins}; {name} fetched from {source} on {system}. Both observations retained.")])?;
                }
                tx.execute(
                    "insert or ignore into identity_definitions values (?1,?2)",
                    params![name, address],
                )?;
                let identity_body = Self::blob(&tx, &result["identity"])?;
                tx.execute("insert into identity_observations values (?1,?2,?3,?4,?5,unixepoch(),?6) on conflict(system_id,source,name,height,body_hash) do update set observed_at=excluded.observed_at,payload_hash=excluded.payload_hash",params![system,source,name,result.get("blockheight").and_then(Value::as_i64).unwrap_or(0),hash,identity_body])?;
            }
        }
        tx.execute("delete from significant_events where event_key not in (select event_key from significant_events order by observed_at desc,event_key limit 200)",[])?;
        let writes = self.writes.get().wrapping_add(1);
        self.writes.set(writes);
        if writes.is_multiple_of(64) {
            Self::retain_snapshots(&tx)?;
            Self::collect_blobs(&tx, 1024)?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn raw_rpc_response(
        &self,
        source: &str,
        system: &str,
        method: &str,
        params_json: &str,
    ) -> Result<Option<String>> {
        let parameters: Value = serde_json::from_str(params_json)?;
        if matches!(method, "getblock" | "getblockheader")
            && !parameters[0].as_str().is_some_and(|s| s.len() == 64)
        {
            // Heights are mutable aliases; only explicit bookmark snapshots may reuse them.
            return Ok(None);
        }
        if method == "getrawtransaction"
            && let Some(id) = parameters[0].as_str()
        {
            self.reproject(system, id)?;
        }
        self.connection
            .query_row(
                "select b.payload from raw_rpc_cache r join raw_blobs b on b.hash=r.payload_hash
            where r.source=?1 and r.system_id=?2 and r.method=?3 and r.params_json=?4",
                params![source, system, method, serde_json::to_string(&parameters)?],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    #[cfg(test)]
    pub fn put_currency_response(&self, chain: &str, response: &Value) -> Result<()> {
        self.put_raw_rpc(
            "fixture",
            chain,
            "getcurrency",
            "[]",
            &serde_json::json!({"result":response}).to_string(),
        )
    }

    pub fn currency_definitions(&self, chain: &str) -> Result<Vec<(String, String, Value)>> {
        let mut statement = self.connection.prepare(
            "select d.currency_id,d.name,b.payload from currency_definitions d join raw_blobs b on b.hash=d.definition_hash
             where d.definition_hash=(select o.definition_hash from currency_definition_observations o where o.currency_id=d.currency_id
             order by (o.system_id=?1) desc,o.rowid limit 1) order by d.name",
        )?;
        let rows = statement.query_map([chain], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (currency_id, name, definition_json) = row?;
            Ok((currency_id, name, serde_json::from_str(&definition_json)?))
        })
        .collect()
    }

    pub fn currency_definition(
        &self,
        chain: &str,
        currency_id: &str,
    ) -> Result<Option<(String, String, Value)>> {
        let mut statement = self.connection.prepare(
            "select d.currency_id,d.name,b.payload from currency_definitions d join raw_blobs b on b.hash=d.definition_hash
             join currency_definition_observations o on o.currency_id=d.currency_id and o.definition_hash=d.definition_hash
             where d.currency_id=?2 order by (o.system_id=?1) desc,o.rowid limit 1",
        )?;
        let mut rows = statement.query(params![chain, currency_id])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let currency_id = row.get::<_, String>(0)?;
        let name = row.get::<_, String>(1)?;
        let definition_json = row.get::<_, String>(2)?;
        Ok(Some((
            currency_id,
            name,
            serde_json::from_str(&definition_json)?,
        )))
    }

    pub fn current_currency_state(
        &self,
        source: &str,
        chain: &str,
        currency_id: &str,
        block: &BlockContext,
        ttl_blocks: u64,
    ) -> Result<Option<Value>> {
        self.currency_at(source, chain, currency_id, block, ttl_blocks)
    }

    fn events(&self) -> Result<Vec<SourceEvent>> {
        let mut statement = self.connection.prepare(
            "select event_key,message,observed_at from significant_events
             order by observed_at desc,event_key limit 200",
        )?;
        let mut events = statement
            .query_map([], |row| {
                Ok(SourceEvent {
                    key: row.get(0)?,
                    message: row.get(1)?,
                    observed_at: row.get::<_, i64>(2)?.max(0) as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut sources = self
            .connection
            .prepare("select distinct source_id,endpoint from source_endpoints")?;
        for row in sources.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (id, endpoint) = row?;
            for event in &mut events {
                event.message = event.message.replace(&id, &endpoint);
            }
        }
        Ok(events)
    }

    fn bind_network(&self, network: crate::world::Network) -> Result<()> {
        let network = serde_json::to_string(&network)?;
        let tx = self.connection.unchecked_transaction()?;
        tx.execute(
            "insert or ignore into database_identity(singleton,network) values(1,?1)",
            [&network],
        )?;
        let existing: String = tx.query_row(
            "select network from database_identity where singleton=1",
            [],
            |row| row.get(0),
        )?;
        anyhow::ensure!(network == existing, "database belongs to the other network");
        tx.commit()?;
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        self.connection.execute_batch(
            "begin immediate;
            delete from currency_activity_pages;
            delete from reconciled_chain;
            delete from reconciliation_headers;
            delete from identity_observations;
            delete from identity_evidence_edges;
            delete from identity_evidence_reports;
            delete from defi_lifecycle_edges;
            delete from identity_definitions;
            delete from currency_states;
            delete from currency_definition_observations;
            delete from currency_definitions;
            delete from membership_history;
            delete from mempool_membership;
            delete from source_status;
            delete from observation_heads;
            delete from significant_events;
            delete from block_membership;
            delete from transaction_inclusions;
            delete from relationships;
            delete from object_observations;
            delete from objects;
            delete from raw_rpc_cache;
            delete from raw_blobs;
            delete from systems;
            commit;
            vacuum;
            pragma wal_checkpoint(truncate);",
        )?;
        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        anyhow::ensure!(
            version == 0
                || version == 3
                || version == 4
                || version == 5
                || version == SCHEMA_VERSION,
            "explorer database schema {version} requires a fresh schema-{SCHEMA_VERSION} cache; use --reset-cache to replace this network's cache"
        );
        let legacy: i64 = self.connection.query_row(
            "select count(*) from pragma_table_info('raw_rpc_cache') where name='chain'",
            [],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            legacy == 0,
            "legacy chain cache cannot be used as a network database"
        );
        if version == 0 || version == 3 || version == 4 || version == 5 {
            self.connection.execute_batch(include_str!("schema.sql"))?;
        }
        Ok(())
    }
}

pub fn cache_disk_usage(path: &Path) -> Result<u64> {
    ["", "-wal", "-shm", "-journal"]
        .into_iter()
        .try_fold(0_u64, |total, suffix| {
            let candidate = if suffix.is_empty() {
                path.to_path_buf()
            } else {
                let mut sidecar = path.as_os_str().to_os_string();
                sidecar.push(suffix);
                PathBuf::from(sidecar)
            };
            match fs::metadata(candidate) {
                Ok(metadata) => total
                    .checked_add(metadata.len())
                    .context("cache size exceeded supported range"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(total),
                Err(error) => Err(error.into()),
            }
        })
}

fn currency_definition_projection(response: &Value) -> Value {
    let mut definition = response.as_object().cloned().unwrap_or_default();
    for key in [
        "nodes",
        "bestheight",
        "bestcurrencystate",
        "lastconfirmedheight",
        "lastconfirmedcurrencystate",
        "besttxid",
        "lastconfirmedtxid",
        "definitiontxid",
        "definitiontxout",
        "currencynames",
        "fullyqualifiedname",
    ] {
        definition.remove(key);
    }
    Value::Object(definition)
}

fn currency_current_projection(response: &Value) -> Value {
    let mut current = Map::new();
    for key in [
        "nodes",
        "bestheight",
        "bestcurrencystate",
        "lastconfirmedheight",
        "lastconfirmedcurrencystate",
    ] {
        if let Some(value) = response.get(key) {
            current.insert(key.to_string(), value.clone());
        }
    }
    Value::Object(current)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Cache, StoreConnection, cache_disk_usage};
    use rusqlite::Connection;
    use serde_json::json;

    fn memory_cache() -> StoreConnection {
        let cache = StoreConnection {
            connection: Connection::open_in_memory().expect("open in-memory cache"),
            _ownership: None,
            writes: Default::default(),
        };
        cache.migrate().expect("migrate cache");
        cache
    }

    #[test]
    fn same_transaction_hash_is_independent_on_each_system_and_reorgs_are_source_scoped() {
        let cache = memory_cache();
        for system in ["iFirst", "iSecond"] {
            for source in ["local", "api"] {
                cache.put_raw_rpc(source, system, "getrawtransaction", "[]", &json!({"result":{"txid":"same", "blockhash":"old", "height":9, "confirmations":2}}).to_string()).unwrap();
                cache
                    .put_raw_rpc(
                        source,
                        system,
                        "getblock",
                        "[]",
                        &json!({"result":{"hash":"old", "height":9,"confirmations":2}}).to_string(),
                    )
                    .unwrap();
            }
        }
        cache
            .put_raw_rpc(
                "local",
                "iFirst",
                "getblock",
                "[]",
                &json!({"result":{"hash":"new", "height":9,"confirmations":1}}).to_string(),
            )
            .unwrap();
        let confirmed: i64 = cache
            .connection
            .query_row(
                "select sum(confirmed) from transaction_inclusions",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(confirmed, 3);
        let objects: i64 = cache
            .connection
            .query_row(
                "select count(*) from objects where kind='transaction'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(objects, 2);
        assert!(
            cache
                .events()
                .unwrap()
                .iter()
                .any(|event| event.message.contains("sources disagree"))
        );
    }

    #[test]
    fn identity_content_is_independent_on_each_system_and_locator_anomalies_are_retained() {
        let cache = memory_cache();
        for (system, name, content) in [
            ("iFirst", "vARRR.VRSC@", 1),
            ("iSecond", "vARRR.VRSC@", 2),
            ("iSecond", "vDEX.VRSC@", 3),
        ] {
            cache.put_raw_rpc(system, system, "getidentity", "[]", &json!({"result":{"fullyqualifiedname":name,"blockheight":7,"identity":{"identityaddress":"iCollision","content":content}}}).to_string()).unwrap();
        }
        let observations: i64 = cache
            .connection
            .query_row("select count(*) from identity_observations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(observations, 3);
        let events = cache.events().unwrap();
        assert_eq!(events.len(), 1);
        assert!(
            events[0].message.contains("varrr.vrsc@")
                && events[0].message.contains("vdex.vrsc@")
                && events[0].message.contains("iSecond")
        );
    }

    #[test]
    fn network_binding_survives_clear_and_other_database_remains_intact() {
        use crate::world::Network;
        let root = std::env::temp_dir().join(format!(
            "explorer-network-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let main_path = root.join("mainnet.sqlite3");
        let test_path = root.join("testnet.sqlite3");
        let main = Cache::open(&main_path).unwrap();
        let test = Cache::open(&test_path).unwrap();
        main.bind_network(Network::Mainnet).unwrap();
        test.bind_network(Network::Testnet).unwrap();
        test.put_raw_rpc(
            "local",
            "iTest",
            "getinfo",
            "[]",
            r#"{"result":{"blocks":1}}"#,
        )
        .unwrap();
        Cache::clear_at(&main_path).unwrap();
        assert!(main.bind_network(Network::Testnet).is_err());
        assert!(
            test.raw_rpc_response("local", "iTest", "getinfo", "[]")
                .unwrap()
                .is_some()
        );
        drop(main);
        drop(test);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn currency_definitions_preserve_conflicts_and_state_requires_same_source_block() {
        let cache = memory_cache();
        let first = json!({
            "currencyid": "iPool",
            "fullyqualifiedname": "Pool",
            "currencies": ["iReserve"],
            "bestheight": 100,
            "bestcurrencystate": { "supply": 1 }
        });
        cache
            .put_currency_response("iVRSC", &first)
            .expect("cache first response");
        let changed = json!({
            "currencyid": "iPool",
            "fullyqualifiedname": "Changed",
            "currencies": [],
            "bestheight": 105,
            "bestcurrencystate": { "supply": 2 }
        });
        cache
            .put_currency_response("iVRSC", &changed)
            .expect("cache second response");

        let (_, name, definition) = cache
            .currency_definition("iVRSC", "iPool")
            .expect("read definition")
            .expect("definition exists");
        assert_eq!(name, "Pool");
        assert_eq!(definition["currencies"], json!(["iReserve"]));
        assert!(definition.get("bestcurrencystate").is_none());
        let block = super::BlockContext {
            height: 105,
            hash: "a".repeat(64),
        };
        assert!(
            cache
                .current_currency_state("fixture", "iVRSC", "iPool", &block, 3)
                .unwrap()
                .is_none()
        );
        let order: i64 = cache
            .connection
            .query_row(
                "select request_order from currency_states order by request_order desc limit 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        cache
            .put_raw_rpc(
                "fixture",
                "iVRSC",
                "getblockchaininfo",
                "[]",
                &json!({"result":{"blocks":block.height,"bestblockhash":block.hash}}).to_string(),
            )
            .unwrap();
        cache
            .qualify_currency("fixture", "iVRSC", &changed, &block, order as u64)
            .unwrap();
        assert!(
            cache
                .current_currency_state("fixture", "iVRSC", "iPool", &block, 3)
                .unwrap()
                .is_some()
        );
        assert!(
            cache
                .current_currency_state("api", "iVRSC", "iPool", &block, 3)
                .unwrap()
                .is_none()
        );
        let replacement = super::BlockContext {
            height: 105,
            hash: "b".repeat(64),
        };
        cache
            .put_raw_rpc(
                "fixture",
                "iVRSC",
                "getblockchaininfo",
                "[]",
                &json!({"result":{"blocks":replacement.height,"bestblockhash":replacement.hash}})
                    .to_string(),
            )
            .unwrap();
        assert!(
            cache
                .current_currency_state("fixture", "iVRSC", "iPool", &replacement, 3)
                .unwrap()
                .is_none()
        );
        let definitions: i64 = cache
            .connection
            .query_row("select count(*) from currency_definitions", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(definitions, 2);
    }

    #[test]
    fn raw_rpc_reads_require_an_exact_source_chain_method_and_parameter_match() {
        let cache = memory_cache();
        let parameters = json!(["a".repeat(64), 1]).to_string();
        cache
            .put_raw_rpc(
                "local-daemon",
                "iVRSC",
                "getblock",
                &parameters,
                r#"{"error":null,"result":{"height":42}}"#,
            )
            .expect("cache response");

        assert_eq!(
            cache
                .raw_rpc_response("local-daemon", "iVRSC", "getblock", &parameters)
                .expect("read response")
                .as_deref(),
            Some(r#"{"error":null,"result":{"height":42}}"#)
        );
        assert!(
            cache
                .raw_rpc_response("public-api", "iVRSC", "getblock", &parameters)
                .expect("read different source")
                .is_none()
        );
        assert!(
            cache
                .raw_rpc_response("local-daemon", "iVRSCTEST", "getblock", &parameters)
                .expect("read different chain")
                .is_none()
        );
    }

    #[test]
    fn clear_at_removes_cached_records_and_compacts_database() {
        let path = std::env::temp_dir().join(format!(
            "verus-explorer-cache-clear-{}.sqlite3",
            std::process::id()
        ));
        let cache = Cache::open(&path).expect("open file cache");
        cache
            .put_raw_rpc("daemon", "iVRSC", "method", "[]", r#"{"result":1}"#)
            .expect("write raw response");
        cache
            .put_currency_response(
                "iVRSC",
                &json!({
                    "currencyid": "iPool",
                    "fullyqualifiedname": "Pool"
                }),
            )
            .expect("write currency definition");
        drop(cache);

        assert!(cache_disk_usage(&path).expect("measure cache") > 0);
        let compacted_size = Cache::clear_at(&path).expect("clear cache");
        let cache = Cache::open(&path).expect("reopen cleared cache");

        assert!(
            cache
                .currency_definition("iVRSC", "iPool")
                .expect("query cleared cache")
                .is_none()
        );
        assert_eq!(
            compacted_size,
            cache_disk_usage(&path).expect("measure compacted cache")
        );
        drop(cache);
        let _ = fs::remove_file(path);
    }
}
