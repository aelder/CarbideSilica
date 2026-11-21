//! One serial branch walker per local daemon. RPC observations never commit its mapping.
use crate::{
    cache::{Cache, Receipt},
    config::Config,
    rpc::RpcClient,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub hash: String,
    pub height: u64,
    pub parent: Option<String>,
    pub time: u64,
}
impl Header {
    fn parse(raw: &Value) -> Result<Self> {
        let header = Self {
            hash: raw["hash"].as_str().context("header hash missing")?.into(),
            height: raw["height"].as_u64().context("header height missing")?,
            parent: raw["previousblockhash"].as_str().map(str::to_owned),
            time: raw["time"].as_u64().context("header time missing")?,
        };
        ensure!(
            header.height == 0 || header.parent.is_some(),
            "header parent missing"
        );
        Ok(header)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    pub revision: u64,
    #[serde(default)]
    pub detached: BTreeMap<String, Header>,
    pub canonical: BTreeMap<u64, Header>,
}
impl State {
    pub fn tip(&self) -> Option<&Header> {
        self.canonical.last_key_value().map(|(_, h)| h)
    }
}

#[derive(Debug, Clone)]
pub struct Change {
    pub state: State,
    pub detached: Vec<Header>,
    pub attached: Vec<Header>,
    pub ancestor: Option<Header>,
}

/// Pure branch planning, shared by production and fake-header tests.
pub async fn plan<F, Fut>(old: &State, tip: Header, mut fetch: F) -> Result<Change>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<Header>>,
{
    if old.tip() == Some(&tip) {
        return Ok(Change {
            state: old.clone(),
            detached: vec![],
            attached: vec![],
            ancestor: Some(tip),
        });
    }
    let mut attached = Vec::new();
    let mut detached = Vec::new();
    let mut new = tip;
    let mut previous = old.tip().cloned();
    let ancestor = loop {
        if previous.as_ref().is_some_and(|h| h.hash == new.hash) {
            break Some(new);
        }
        // Bootstrap is bounded; a detected mismatch is never truncated by depth.
        if previous.is_none() && (attached.len() == 99 || new.height == 0) {
            attached.push(new);
            break None;
        }
        let descend_old = previous.as_ref().is_some_and(|h| h.height >= new.height);
        let descend_new = previous.as_ref().is_none_or(|h| new.height >= h.height);
        if descend_old {
            let h = previous.take().unwrap();
            ensure!(h.height > 0, "branches disagree at genesis");
            let parent = h.parent.clone().context("old branch parent missing")?;
            let p = match old
                .canonical
                .get(&(h.height - 1))
                .filter(|p| p.hash == parent)
            {
                Some(p) => p.clone(),
                None => fetch(parent.clone()).await?,
            };
            ensure!(
                p.hash == parent && p.height + 1 == h.height,
                "invalid old parent link"
            );
            detached.push(h);
            previous = Some(p);
        }
        if descend_new {
            ensure!(new.height > 0, "branches disagree at genesis");
            let hash = new.parent.clone().context("new branch parent missing")?;
            let parent = fetch(hash.clone()).await?;
            ensure!(
                parent.hash == hash && parent.height + 1 == new.height,
                "invalid new parent link"
            );
            attached.push(new);
            new = parent;
        }
        if (attached.len() + detached.len()).is_multiple_of(32) {
            tokio::task::yield_now().await;
        }
    };
    let mut state = old.clone();
    if let Some(ancestor) = &ancestor {
        state
            .canonical
            .retain(|height, _| *height <= ancestor.height);
        state.canonical.insert(ancestor.height, ancestor.clone());
    }
    for h in &attached {
        state.canonical.insert(h.height, h.clone());
        state.detached.remove(&h.hash);
    }
    for h in &detached {
        state.detached.insert(h.hash.clone(), h.clone());
    }
    state.revision += 1;
    Ok(Change {
        state,
        attached,
        detached,
        ancestor,
    })
}

pub struct Reconciler {
    config: Config,
    client: RpcClient,
    cache: Cache,
    pub state: State,
    source: String,
    epoch: u64,
}

#[cfg(test)]
#[path = "reconciliation_tests.rs"]
mod tests;
impl Reconciler {
    pub async fn new(mut config: Config) -> Result<Self> {
        config.daemon.api_pool = Default::default();
        config.daemon.observations = None;
        let client = RpcClient::for_config(&config)?.noninteractive();
        let cache = Cache::open(&config.cache_path)?;
        let source = config.daemon_cache_key();
        let state = cache
            .reconciliation_state(&config.chain_cache_key(), &source)
            .await?;
        let epoch = cache.epoch();
        Ok(Self {
            config,
            client,
            cache,
            state,
            source,
            epoch,
        })
    }

    async fn header(&self, hash: String) -> Result<Header> {
        ensure!(
            self.config.daemon.control.polling(),
            "reconciliation paused"
        );
        let params = json!([hash]).to_string();
        let raw = self
            .cache
            .raw_rpc_response_async(
                &self.source,
                &self.config.chain_cache_key(),
                "getblockheader",
                &params,
            )
            .await?;
        let result = if let Some(raw) = raw {
            serde_json::from_str::<Value>(&raw)?["result"].clone()
        } else {
            self.client
                .call_with_raw::<Value>("getblockheader", &[json!(hash)])
                .await?
                .result
        };
        let header = Header::parse(&result)?;
        ensure!(header.hash == hash, "header does not match requested hash");
        Ok(header)
    }

    pub async fn reconcile(&mut self, started: impl FnOnce()) -> Result<Change> {
        if self.epoch != self.cache.epoch() {
            self.state = self
                .cache
                .reconciliation_state(&self.config.chain_cache_key(), &self.source)
                .await?;
            self.epoch = self.cache.epoch();
        }
        let before = self
            .client
            .call_with_raw::<Value>("getblockchaininfo", &[])
            .await?;
        let receipt: Receipt = before
            .receipt
            .context("reconciliation requires qualified source")?;
        let hash = before.result["bestblockhash"]
            .as_str()
            .context("tip hash missing")?;
        if self.state.tip().is_none_or(|h| h.hash != hash) {
            started();
        }
        let tip = self.header(hash.into()).await?;
        ensure!(
            before.result["blocks"].as_u64() == Some(tip.height),
            "tip height mismatch"
        );
        let mut change = plan(&self.state, tip, |hash| self.header(hash)).await?;
        let requested: Vec<_> = self
            .config
            .daemon
            .control
            .reconciliation_heights
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .take(16)
            .copied()
            .collect();
        for height in &requested {
            if change.state.canonical.contains_key(height)
                || *height > change.state.tip().map(|h| h.height).unwrap_or(0)
            {
                continue;
            }
            // Certify a requested historical height against this same daemon/tip.
            // This avoids walking millions of ancestors just to open an old object.
            let output = self
                .client
                .call_with_raw::<Value>("getblockhash", &[json!(height)])
                .await?;
            let header = self
                .header(
                    output
                        .result
                        .as_str()
                        .context("historical block hash missing")?
                        .into(),
                )
                .await?;
            ensure!(header.height == *height, "historical height mismatch");
            change.state.canonical.insert(*height, header.clone());
            change.attached.push(header);
            change.state.revision = self.state.revision + 1;
        }
        let after = self
            .client
            .call_with_raw::<Value>("getblockchaininfo", &[])
            .await?;
        ensure!(
            after.result["bestblockhash"] == before.result["bestblockhash"]
                && after.result["blocks"] == before.result["blocks"],
            "tip changed during reconciliation; retrying"
        );
        self.cache
            .commit_reconciliation(
                &self.config.chain_cache_key(),
                &self.source,
                receipt,
                self.state.revision,
                change.clone(),
            )
            .await?;
        self.state = change.state.clone();
        let mut pending = self
            .config
            .daemon
            .control
            .reconciliation_heights
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        for height in requested {
            pending.remove(&height);
        }
        Ok(change)
    }
}
