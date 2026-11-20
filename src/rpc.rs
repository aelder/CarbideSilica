use crate::chain_state::RpcObservation;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
static REQUEST_ORDER: AtomicU64 = AtomicU64::new(1);
pub(crate) fn next_request_order() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(i64::MAX as u128 - 1) as u64;
    let previous = REQUEST_ORDER
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |previous| {
            Some(now.max(previous + 1))
        })
        .unwrap();
    now.max(previous + 1)
}

use anyhow::{Context, Result};
#[derive(Debug)]
struct StorageFailure;
impl std::fmt::Display for StorageFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("storage could not accept this observation; retry after checking cache and source state")
    }
}
impl std::error::Error for StorageFailure {}
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::config::DaemonConfig;
use crate::domain::BlockSummary;

fn requires_identity_index(method: &str) -> bool {
    matches!(
        method,
        "getidentitieswithaddress" | "getidentitieswithrecovery" | "getidentitieswithrevocation"
    )
}

const RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RPC_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_IN_FLIGHT_RPC_CALLS: usize = 4;
static BACKGROUND_PERMITS: Semaphore = Semaphore::const_new(2);
static RPC_PERMITS: Semaphore = Semaphore::const_new(MAX_IN_FLIGHT_RPC_CALLS);

#[derive(Debug, Clone)]
pub struct RpcClient {
    url: String,
    user: Option<String>,
    password: Option<String>,
    http: reqwest::Client,
    observations: Option<UnboundedSender<RpcObservation>>,
    control: std::sync::Arc<crate::sources::Control>,
    api_pool: std::sync::Arc<crate::sources::ApiPool>,
    background: bool,
    interactive: bool,
    probe_identity_index: bool,
    storage: Option<(
        crate::cache::Cache,
        crate::world::Network,
        crate::world::SystemId,
    )>,
}

#[derive(Debug, Clone)]
pub struct RpcCallOutput<T> {
    pub result: T,
    pub response_json: String,
    pub source_id: String,
    pub receipt: Option<crate::cache::Receipt>,
}

#[derive(Debug, Clone)]
pub struct NullableRpcCallOutput<T> {
    pub result: Option<T>,
    pub source_id: String,
}

#[derive(Debug, Serialize)]
struct RpcRequest<'a> {
    jsonrpc: &'static str,
    id: &'static str,
    method: &'a str,
    params: &'a [Value],
}

#[derive(Debug, Deserialize)]
struct RpcResponse<T> {
    result: Option<T>,
    error: Option<Value>,
}

#[derive(Debug)]
pub(crate) struct ApplicationError {
    method: String,
    value: Value,
}

impl std::fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RPC error from {}: {}", self.method, self.value)
    }
}

impl std::error::Error for ApplicationError {}

impl ApplicationError {
    pub(crate) fn is_block_not_found(&self) -> bool {
        self.value["code"].as_i64() == Some(-5)
            && self.value["message"]
                .as_str()
                .is_some_and(|m| m.eq_ignore_ascii_case("Block not found"))
    }
}

impl RpcClient {
    pub fn for_config(config: &crate::config::Config) -> Result<Self> {
        let mut client = Self::new(config.daemon.clone())?;
        client.interactive = config.api_consent && !config.daemon.background;
        let cache = crate::cache::Cache::open(&config.cache_path)?;
        let network = crate::world::Network::from_testnet(config.chain.testnet);
        cache.bind_network(network)?;
        if let Some(system) = config.chain.system_id.get() {
            client.storage = Some((cache, network, system.clone()));
        }
        Ok(client)
    }
    pub fn new(endpoint: DaemonConfig) -> Result<Self> {
        let url = format!("http://{}:{}", endpoint.host, endpoint.port);
        let mut client = Self::with_endpoint(url, endpoint.user, endpoint.password)?;
        client.observations = endpoint.observations;
        client.control = endpoint.control;
        client.api_pool = endpoint.api_pool;
        client.background = endpoint.background;
        client.interactive = !endpoint.background;
        Ok(client)
    }

    fn with_endpoint(url: String, user: Option<String>, password: Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(RPC_CONNECT_TIMEOUT)
            .timeout(RPC_RESPONSE_TIMEOUT)
            .build()?;
        Ok(Self {
            url,
            user,
            password,
            http,
            observations: None,
            control: Default::default(),
            api_pool: Default::default(),
            background: false,
            interactive: true,
            probe_identity_index: false,
            storage: None,
        })
    }

    pub fn noninteractive(mut self) -> Self {
        self.interactive = false;
        self
    }

    /// An explicit historical investigation may establish a source capability.
    pub fn probe_identity_index(mut self) -> Self {
        self.probe_identity_index = true;
        self
    }

    fn identity_index_available(&self, source: &crate::sources::Control) -> bool {
        source.identity_index_enabled()
            || (self.probe_identity_index && source.identity_index_unknown())
    }

    async fn response(
        &self,
        method: &str,
        params: &[Value],
    ) -> Result<(Value, String, String, Option<crate::cache::Receipt>)> {
        self.response_on(method, params, None).await
    }

    async fn response_on(
        &self,
        method: &str,
        params: &[Value],
        pinned_source: Option<&str>,
    ) -> Result<(Value, String, String, Option<crate::cache::Receipt>)> {
        use crate::sources::{ApiDecision, ConnectionState};

        let local_id = source_id("local-daemon", &self.url);
        loop {
            let local_error = match self
                .request_local(method, params, pinned_source, &local_id)
                .await
            {
                Ok((value, raw, receipt)) => return Ok((value, raw, local_id.clone(), receipt)),
                Err(error)
                    if pinned_source == Some(local_id.as_str())
                        || error.is::<StorageFailure>()
                        || matches!(method, "getrawmempool" | "getpeerinfo") =>
                {
                    return Err(error);
                }
                Err(error) => error,
            };
            let local_failure = format!("{local_error:#}");
            let mut last_error = local_error;
            let endpoints = if pinned_source.is_some() {
                self.api_pool.endpoints.iter().collect()
            } else {
                self.api_pool.candidates()
            };
            let mut disabled = Vec::new();
            for endpoint in endpoints {
                if pinned_source
                    .is_some_and(|source| source != source_id("public-api", &endpoint.url))
                    || !endpoint.control.method_available(method)
                    || (requires_identity_index(method)
                        && !self.identity_index_available(&endpoint.control))
                {
                    continue;
                }
                if endpoint.control.state() == ConnectionState::Disabled {
                    if self.interactive && pinned_source.is_none() {
                        disabled.push(endpoint);
                    }
                    continue;
                }
                let generation = endpoint.control.ticket()?;
                match self
                    .request_endpoint(
                        &endpoint.url,
                        None,
                        None,
                        method,
                        params,
                        Some((&endpoint.control, Some(generation))),
                    )
                    .await
                {
                    Ok((value, raw, receipt)) => {
                        anyhow::ensure!(
                            endpoint.control.generation() == generation,
                            "API permission changed during request"
                        );
                        return Ok((value, raw, source_id("public-api", &endpoint.url), receipt));
                    }
                    Err(error) => {
                        if error.is::<StorageFailure>() {
                            return Err(error);
                        }
                        if pinned_source.is_none()
                            && endpoint.control.method_available(method)
                            && (!requires_identity_index(method)
                                || self.identity_index_available(&endpoint.control))
                        {
                            *endpoint
                                .retry_after
                                .lock()
                                .unwrap_or_else(|e| e.into_inner()) =
                                Some(std::time::Instant::now() + Duration::from_secs(30));
                        }
                        last_error = error;
                    }
                }
            }

            if disabled.is_empty() {
                return Err(last_error);
            }
            disabled.sort_by_key(|endpoint| {
                self.api_pool
                    .endpoints
                    .iter()
                    .position(|configured| std::ptr::eq(configured, *endpoint))
            });
            let failure = if format!("{last_error:#}") == local_failure {
                local_failure
            } else {
                format!(
                    "Local daemon: {local_failure}. Another enabled source also failed: {last_error:#}"
                )
            };
            match self
                .api_pool
                .permission(&disabled, method, &failure)
                .await?
            {
                ApiDecision::Retry => continue,
                ApiDecision::Enable(index) => {
                    let endpoint = disabled
                        .get(index)
                        .context("Selected API is no longer available")?;
                    endpoint.control.set(ConnectionState::Enabled);
                }
                ApiDecision::Cancel => return Err(crate::sources::ApiCancelled.into()),
            }
        }
    }

    async fn request_local(
        &self,
        method: &str,
        params: &[Value],
        pinned_source: Option<&str>,
        local_id: &str,
    ) -> Result<(Value, String, Option<crate::cache::Receipt>)> {
        async {
            anyhow::ensure!(
                pinned_source.is_none_or(|source| source == local_id),
                "Different RPC source selected"
            );
            if self.control.expected.get().is_some() && method != "getinfo" {
                let mut verified = self.control.verified.lock().await;
                if !*verified {
                    self.request_endpoint(
                        &self.url,
                        self.user.as_deref(),
                        self.password.as_deref(),
                        "getinfo",
                        &[],
                        Some((&self.control, None)),
                    )
                    .await?;
                    *verified = true;
                }
            }
            self.request_endpoint(
                &self.url,
                self.user.as_deref(),
                self.password.as_deref(),
                method,
                params,
                Some((&self.control, None)),
            )
            .await
        }
        .await
    }

    async fn request_endpoint(
        &self,
        url: &str,
        user: Option<&str>,
        password: Option<&str>,
        method: &str,
        params: &[Value],
        control: Option<(&std::sync::Arc<crate::sources::Control>, Option<u64>)>,
    ) -> Result<(Value, String, Option<crate::cache::Receipt>)> {
        let identity_index_method = requires_identity_index(method);
        if let Some((source, _)) = control {
            anyhow::ensure!(
                source.method_available(method),
                "RPC method is unavailable from this source"
            );
        }
        if identity_index_method && let Some((source, _)) = control {
            anyhow::ensure!(
                self.identity_index_available(source),
                "identity reverse index is not available for automatic lookup"
            );
        }
        let ticket = control
            .map(|(control, once)| once.map(Ok).unwrap_or_else(|| control.ticket()))
            .transpose()?;
        let _background = if self.background {
            Some(
                BACKGROUND_PERMITS
                    .acquire()
                    .await
                    .context("background limiter closed")?,
            )
        } else {
            None
        };
        let _source = if let Some((control, _)) = control {
            Some(
                control
                    .permits
                    .acquire()
                    .await
                    .context("source limiter closed")?,
            )
        } else {
            None
        };
        let _permit = RPC_PERMITS
            .acquire()
            .await
            .context("RPC request limiter closed")?;
        if let (Some((control, once)), Some(ticket)) = (control, ticket) {
            anyhow::ensure!(
                if once.is_some() {
                    control.generation() == ticket
                } else {
                    control.accepts(ticket)
                },
                "connection changed while queued"
            );
        }
        if identity_index_method && let Some((source, _)) = control {
            anyhow::ensure!(
                self.identity_index_available(source),
                "identity reverse index is not available for automatic lookup"
            );
        }
        if let Some((source, _)) = control {
            anyhow::ensure!(
                source.method_available(method),
                "RPC method is unavailable from this source"
            );
        }
        let order = next_request_order();
        let branch_epoch = control
            .map(|(c, _)| c.branch_epoch.load(Ordering::Acquire))
            .unwrap_or(0);
        let epoch = self
            .storage
            .as_ref()
            .map(|(cache, _, _)| cache.epoch())
            .unwrap_or(0);
        let body = RpcRequest {
            jsonrpc: "1.0",
            id: "verus-explorer-tui",
            method,
            params,
        };
        let mut request = self.http.post(url).json(&body);
        if let Some(user) = user {
            request = request.basic_auth(user, password);
        }
        // Track only requests that pass all authorization/capability gates and
        // actually reach the transport. Cache hits and skipped methods are idle.
        let mut activity = control
            .zip(ticket)
            .map(|((source, _), generation)| source.begin_request(generation, order));
        let response = request.send().await.map_err(reqwest::Error::without_url)?;
        let status = response.status();
        let raw = response.text().await.map_err(reqwest::Error::without_url)?;
        if let (Some((control, once)), Some(ticket)) = (control, ticket) {
            anyhow::ensure!(
                if once.is_some() {
                    control.generation() == ticket
                } else {
                    control.accepts(ticket)
                },
                "connection changed during request"
            );
        }
        let response: RpcResponse<Value> = serde_json::from_str(&raw)
            .with_context(|| format!("HTTP {status} invalid JSON-RPC response for {method}"))?;
        if let Some(error) = response.error {
            // A normal JSON-RPC application error (not found, missing optional
            // index, etc.) still proves that the endpoint is communicating.
            if status != reqwest::StatusCode::UNAUTHORIZED
                && status != reqwest::StatusCode::FORBIDDEN
                && let Some(activity) = &mut activity
            {
                activity.responding(false);
            }

            if error["code"].as_i64() == Some(-32601)
                && let Some((source, _)) = control
            {
                source.unsupported_method(method);
            }
            if identity_index_method && let Some((source, _)) = control {
                let message = error["message"]
                    .as_str()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if message.contains("requires -idindex") {
                    source.set_identity_index(false);
                }
            }
            return Err(ApplicationError {
                method: method.into(),
                value: error,
            }
            .into());
        }
        if identity_index_method
            && status.is_success()
            && response.result.as_ref().is_some_and(Value::is_array)
            && let Some((source, _)) = control
        {
            source.set_identity_index(true);
        }
        anyhow::ensure!(status.is_success(), "HTTP {status} from {method}");
        if method == "getinfo"
            && let Some((gate, once)) = control
            && let Some((system, network)) = gate.expected.get()
        {
            let info = response.result.as_ref().context("getinfo omitted result")?;
            if once.is_none() || info.get("chainid").is_some() {
                anyhow::ensure!(
                    info.get("chainid").and_then(Value::as_str) == Some(system.as_ref()),
                    "source system identity does not match configured chain"
                );
            }
            if let Some(testnet) = info.get("testnet").and_then(Value::as_bool) {
                anyhow::ensure!(
                    crate::world::Network::from_testnet(testnet) == *network,
                    "source belongs to the other network"
                );
            }
        }
        if let Some(activity) = &mut activity {
            activity.responding(true);
        }
        // Finish network health before persistence: a cache error is not a
        // transport failure and must not turn a responding source red.
        drop(activity);
        let receipt =
            control
                .zip(ticket)
                .map(|((control, once), generation)| crate::cache::Receipt {
                    branch_epoch,
                    control: control.clone(),
                    generation,
                    order,
                    epoch,
                    once: once.is_some(),
                });
        if let (Some((cache, network, system)), Some(receipt)) = (&self.storage, &receipt) {
            let source = source_id(
                if url == self.url {
                    "local-daemon"
                } else {
                    "public-api"
                },
                url,
            );
            anyhow::ensure!(
                cache
                    .observe(
                        *network,
                        receipt.clone(),
                        source,
                        system.to_string(),
                        method.into(),
                        serde_json::to_string(params)?,
                        raw.clone()
                    )
                    .await
                    .context(StorageFailure)?,
                StorageFailure
            );
        }
        if control.is_some_and(|(_, once)| once.is_none()) {
            self.observe(order, method, params, &raw, ticket, branch_epoch);
        }
        Ok((response.result.unwrap_or(Value::Null), raw, receipt))
    }

    pub async fn call_with_raw<T: DeserializeOwned>(
        &self,
        method: &str,
        params: &[Value],
    ) -> Result<RpcCallOutput<T>> {
        let (value, response_json, source_id, receipt) = self.response(method, params).await?;
        anyhow::ensure!(!value.is_null(), "RPC response for {method} had no result");
        Ok(RpcCallOutput {
            result: serde_json::from_value(value)?,
            response_json,
            source_id,
            receipt,
        })
    }

    pub async fn call_nullable_with_raw<T: DeserializeOwned>(
        &self,
        method: &str,
        params: &[Value],
    ) -> Result<NullableRpcCallOutput<T>> {
        let (value, _, source_id, _) = self.response(method, params).await?;
        let result = if value.is_null() {
            None
        } else {
            Some(serde_json::from_value(value)?)
        };
        Ok(NullableRpcCallOutput { result, source_id })
    }

    pub(crate) async fn call_on(
        &self,
        source: &str,
        method: &str,
        params: &[Value],
    ) -> Result<Value> {
        let (value, _, _, _) = self.response_on(method, params, Some(source)).await?;
        Ok(value)
    }

    fn observe(
        &self,
        order: u64,
        method: &str,
        params: &[Value],
        raw: &str,
        ticket: Option<u64>,
        branch_epoch: u64,
    ) {
        if !matches!(
            method,
            "getinfo"
                | "getblockchaininfo"
                | "getblockcount"
                | "getbestblockhash"
                | "getblock"
                | "getblockheader"
                | "getblockhash"
                | "gettxout"
                | "getrawtransaction"
                | "getrawmempool"
                | "getcurrency"
        ) {
            return;
        }
        if let Some(sender) = &self.observations
            && let Ok(mut response) = serde_json::from_str::<Value>(raw)
            && let Some(result) = response.get_mut("result")
        {
            let _ = sender.send(RpcObservation {
                authorization: ticket
                    .map(|generation| (self.control.clone(), generation, branch_epoch)),
                order,
                method: method.into(),
                params: params.to_vec(),
                result: result.take(),
            });
        }
    }

    pub async fn get_block_count(&self) -> Result<RpcCallOutput<u64>> {
        self.call_with_raw("getblockcount", &[]).await
    }

    pub async fn get_info(&self) -> Result<RpcCallOutput<Value>> {
        self.call_with_raw("getinfo", &[]).await
    }

    pub(crate) async fn call_local(&self, method: &str, params: &[Value]) -> Result<Value> {
        let local_id = source_id("local-daemon", &self.url);
        let (result, _, _) = self
            .request_local(method, params, Some(&local_id), &local_id)
            .await?;
        Ok(result)
    }

    pub async fn get_raw_mempool_verbose(
        &self,
        full: bool,
    ) -> Result<RpcCallOutput<HashMap<String, Value>>> {
        if full {
            let result = self
                .call_with_raw(
                    "getrawmempool",
                    &[Value::Bool(true), serde_json::json!({"fulltxes": true})],
                )
                .await;
            match &result {
                Err(error)
                    if error
                        .downcast_ref::<ApplicationError>()
                        .is_some_and(|error| {
                            matches!(error.value["code"].as_i64(), Some(-1 | -8 | -32602))
                        }) =>
                {
                    // Older daemons can accept verbose but reject the qualifiers argument.
                }
                _ => return result,
            }
        }
        self.call_with_raw("getrawmempool", &[Value::from(true)])
            .await
    }

    pub async fn get_offers(
        &self,
        currency_or_identity: &str,
        is_currency: bool,
    ) -> Result<RpcCallOutput<Value>> {
        self.call_with_raw(
            "getoffers",
            &[
                Value::from(currency_or_identity),
                Value::from(is_currency),
                Value::from(false),
            ],
        )
        .await
    }

    pub async fn get_block(
        &self,
        block: BlockId,
        verbosity: u8,
    ) -> Result<RpcCallOutput<BlockSummary>> {
        let selector = match block {
            BlockId::Height(height) => Value::from(height.to_string()),
            BlockId::Hash(hash) => Value::from(hash),
        };
        self.call_with_raw("getblock", &[selector, Value::from(verbosity)])
            .await
    }

    pub async fn get_raw_transaction(&self, txid: &str) -> Result<RpcCallOutput<Value>> {
        self.call_with_raw("getrawtransaction", &[Value::from(txid), Value::from(1)])
            .await
    }

    pub async fn get_address_txids(&self, address: &str) -> Result<RpcCallOutput<Vec<String>>> {
        self.call_with_raw(
            "getaddresstxids",
            &[serde_json::json!({ "addresses": [address] })],
        )
        .await
    }

    pub async fn get_address_txids_range(
        &self,
        address: &str,
        start: u64,
        end: u64,
    ) -> Result<RpcCallOutput<Vec<String>>> {
        self.call_with_raw(
            "getaddresstxids",
            &[serde_json::json!({
                "addresses": [address],
                "start": start,
                "end": end
            })],
        )
        .await
    }

    pub async fn get_address_utxos(&self, address: &str) -> Result<RpcCallOutput<Vec<Value>>> {
        self.call_with_raw(
            "getaddressutxos",
            &[serde_json::json!({ "addresses": [address] })],
        )
        .await
    }

    pub async fn get_address_balance(&self, address: &str) -> Result<RpcCallOutput<Value>> {
        self.call_with_raw(
            "getaddressbalance",
            &[serde_json::json!({
                "addresses": [address],
                "friendlynames": true
            })],
        )
        .await
    }

    pub async fn get_tx_out(&self, txid: &str, index: u64) -> Result<NullableRpcCallOutput<Value>> {
        self.call_nullable_with_raw(
            "gettxout",
            &[Value::from(txid), Value::from(index), Value::from(true)],
        )
        .await
    }

    pub async fn get_identity(
        &self,
        identity: &str,
        height: Option<u64>,
    ) -> Result<RpcCallOutput<Value>> {
        let mut params = vec![Value::from(identity)];
        if let Some(height) = height {
            params.push(Value::from(height));
        }
        self.call_with_raw("getidentity", &params).await
    }

    pub async fn get_identity_history(&self, identity: &str) -> Result<RpcCallOutput<Value>> {
        self.call_with_raw("getidentityhistory", &[Value::from(identity)])
            .await
    }

    pub async fn get_currency(&self, currency: &str) -> Result<RpcCallOutput<Value>> {
        let context_client = self.clone().noninteractive();
        let before = context_client
            .call_with_raw::<Value>("getblockchaininfo", &[])
            .await
            .ok();
        let output: RpcCallOutput<Value> = self
            .call_with_raw("getcurrency", &[Value::from(currency)])
            .await?;
        if let (Some(before), Some(receipt), Some((cache, _, system))) =
            (before, &output.receipt, &self.storage)
            && before.source_id == output.source_id
            && let Ok(after) = context_client
                .call_with_raw::<Value>("getblockchaininfo", &[])
                .await
            && after.source_id == output.source_id
            && before.result["bestblockhash"] == after.result["bestblockhash"]
            && before.result["blocks"] == after.result["blocks"]
            && let (Some(height), Some(hash)) = (
                after.result["blocks"].as_u64(),
                after.result["bestblockhash"].as_str(),
            )
        {
            cache
                .qualify_currency(
                    output.source_id.clone(),
                    system.to_string(),
                    output.result.clone(),
                    receipt.clone(),
                    crate::cache::BlockContext {
                        height,
                        hash: hash.into(),
                    },
                )
                .await?;
        }
        Ok(output)
    }

    pub async fn get_currency_state(
        &self,
        currency: &str,
        height: u64,
    ) -> Result<RpcCallOutput<Vec<Value>>> {
        self.call_with_raw(
            "getcurrencystate",
            &[Value::from(currency), Value::from(height)],
        )
        .await
    }

    pub async fn get_blockchain_info(&self) -> Result<RpcCallOutput<Value>> {
        self.call_with_raw("getblockchaininfo", &[]).await
    }

    pub async fn get_identities_with_address(
        &self,
        address: &str,
    ) -> Result<RpcCallOutput<Vec<Value>>> {
        self.call_with_raw(
            "getidentitieswithaddress",
            &[serde_json::json!({ "address": address, "unspent": true })],
        )
        .await
    }

    pub async fn get_identities_with_authority(
        &self,
        authority: &str,
        recovery: bool,
    ) -> Result<RpcCallOutput<Vec<Value>>> {
        let method = if recovery {
            "getidentitieswithrecovery"
        } else {
            "getidentitieswithrevocation"
        };
        self.call_with_raw(
            method,
            &[serde_json::json!({ "identityid": authority, "unspent": true })],
        )
        .await
    }

    pub async fn get_historical_identities_with_authority(
        &self,
        authority: &str,
        recovery: bool,
        from_height: u64,
        to_height: u64,
    ) -> Result<RpcCallOutput<Vec<Value>>> {
        let method = if recovery {
            "getidentitieswithrecovery"
        } else {
            "getidentitieswithrevocation"
        };
        self.call_with_raw(
            method,
            &[serde_json::json!({
                "identityid": authority, "fromheight": from_height,
                "toheight": to_height, "unspent": false,
            })],
        )
        .await
    }

    pub async fn get_historical_identities_with_address(
        &self,
        address: &str,
        from_height: u64,
        to_height: u64,
    ) -> Result<RpcCallOutput<Vec<Value>>> {
        self.call_with_raw(
            "getidentitieswithaddress",
            &[serde_json::json!({
                "address": address, "fromheight": from_height,
                "toheight": to_height, "unspent": false,
            })],
        )
        .await
    }
}

#[derive(Debug, Clone)]
pub enum BlockId {
    Height(u64),
    Hash(String),
}

pub fn source_id(kind: &str, endpoint: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{kind}:{:x}", Sha256::digest(endpoint.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::{ApiEndpoint, ApiPool, ConnectionState, Control};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize},
        },
    };

    struct Server {
        url: String,
        hits: Arc<AtomicUsize>,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Server {
        fn start(value: Value) -> Self {
            Self::start_response(
                serde_json::json!({"result":value,"error":null,"id":"verus-explorer-tui"}),
            )
        }
        fn start_response(response: Value) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let hits = Arc::new(AtomicUsize::new(0));
            let stop = Arc::new(AtomicBool::new(false));
            let (count, stopping) = (hits.clone(), stop.clone());
            let thread = std::thread::spawn(move || {
                while !stopping.load(Ordering::Acquire) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                        Err(error) => panic!("{error}"),
                    };
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut buffer = vec![];
                    let mut chunk = [0u8; 4096];
                    loop {
                        let n = stream.read(&mut chunk).unwrap();
                        if n == 0 {
                            break;
                        }
                        buffer.extend_from_slice(&chunk[..n]);
                        if let Some(end) = buffer.windows(4).position(|s| s == b"\r\n\r\n") {
                            let header = String::from_utf8_lossy(&buffer[..end]);
                            let length = header
                                .lines()
                                .find_map(|l| {
                                    l.to_lowercase()
                                        .strip_prefix("content-length:")
                                        .and_then(|s| s.trim().parse::<usize>().ok())
                                })
                                .unwrap_or(0);
                            if buffer.len() >= end + 4 + length {
                                break;
                            }
                        }
                    }
                    count.fetch_add(1, Ordering::AcqRel);
                    let body = response.to_string();
                    write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
                }
            });
            Self {
                url,
                hits,
                stop,
                thread: Some(thread),
            }
        }
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            self.thread.take().unwrap().join().unwrap();
        }
    }
    fn endpoint(url: &str, state: ConnectionState) -> ApiEndpoint {
        let control = Arc::new(Control::default());
        control.set(state);
        ApiEndpoint {
            url: url.into(),
            control,
            retry_after: Default::default(),
        }
    }
    fn client(pool: ApiPool) -> RpcClient {
        let mut client = RpcClient::with_endpoint("http://127.0.0.1:1".into(), None, None).unwrap();
        client.control.set(ConnectionState::Disabled);
        client.api_pool = Arc::new(pool);
        client
    }

    #[tokio::test]
    async fn unsupported_api_method_does_not_disable_other_api_methods() {
        let api = Server::start_response(serde_json::json!({"result":null,
            "error":{"code":-32601,"message":"Method not found"},"id":"verus-explorer-tui"}));
        let client = client(ApiPool::new(
            "chain".into(),
            vec![endpoint(&api.url, ConnectionState::Enabled)],
            None,
        ))
        .noninteractive();
        client.api_pool.endpoints[0]
            .control
            .set_identity_index(true);
        assert!(
            client
                .get_identities_with_authority("seed", false)
                .await
                .is_err()
        );
        assert!(
            !client.api_pool.endpoints[0]
                .control
                .method_available("getidentitieswithrevocation")
        );
        assert!(
            client.api_pool.endpoints[0]
                .retry_after
                .lock()
                .unwrap()
                .is_none()
        );
        assert!(client.get_raw_transaction("target").await.is_err());
        assert_eq!(api.hits.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn reverse_indexes_are_not_probed_automatically_and_missing_support_is_remembered() {
        let local = Server::start_response(serde_json::json!({"result":null,"error":{
            "code":-32602,"message":"getidentitieswithrevocation requires -idindex=1 when starting the daemon"},"id":"verus-explorer-tui"}));
        let api = Server::start(serde_json::json!([]));
        let mut client = client(ApiPool::new(
            "chain".into(),
            vec![endpoint(&api.url, ConnectionState::Enabled)],
            None,
        ))
        .noninteractive();
        client.url = local.url.clone();
        client.control.set(ConnectionState::Enabled);
        assert!(
            client
                .get_identities_with_authority("seed", false)
                .await
                .is_err()
        );
        assert_eq!(local.hits.load(Ordering::Acquire), 0);
        assert_eq!(api.hits.load(Ordering::Acquire), 0);
        // An explicit investigation establishes capabilities for each source.
        assert!(
            client
                .clone()
                .probe_identity_index()
                .get_identities_with_authority("seed", false)
                .await
                .is_ok()
        );
        assert_eq!(local.hits.load(Ordering::Acquire), 1);
        assert_eq!(api.hits.load(Ordering::Acquire), 1);
        assert!(!client.control.identity_index_enabled());
        assert!(
            client.api_pool.endpoints[0]
                .control
                .identity_index_enabled()
        );
        assert!(
            client
                .get_identities_with_authority("seed", true)
                .await
                .is_ok()
        );
        assert_eq!(local.hits.load(Ordering::Acquire), 1);
        assert_eq!(api.hits.load(Ordering::Acquire), 2);
    }

    #[tokio::test]
    async fn rpc_application_errors_are_responses_not_transport_failures() {
        let server = Server::start_response(
            serde_json::json!({"result":null,"error":{"code":-5,"message":"Identity not found"}}),
        );
        let client = RpcClient::with_endpoint(server.url.clone(), None, None).unwrap();
        assert!(client.get_identity("missing", None).await.is_err());
        let status = client.control.communication();
        assert_eq!(
            status.outcome,
            Some(crate::sources::CommunicationOutcome::Responding)
        );
        assert!(status.last_contact.is_some());
        assert!(status.last_success.is_none());
    }

    #[tokio::test]
    async fn optional_requests_prefer_local_daemon_even_with_enabled_api() {
        let local = Server::start(serde_json::json!({"source":"local"}));
        let api = Server::start(serde_json::json!({"source":"api"}));
        let mut client = client(ApiPool::new(
            "chain".into(),
            vec![endpoint(&api.url, ConnectionState::Enabled)],
            None,
        ))
        .noninteractive();
        client.url = local.url.clone();
        client.control.set(ConnectionState::Enabled);
        for method in ["getcurrency", "getoffers", "getaddressbalance"] {
            assert_eq!(
                client
                    .call_with_raw::<Value>(method, &[])
                    .await
                    .unwrap()
                    .result["source"],
                "local"
            );
        }
        assert_eq!(local.hits.load(Ordering::Acquire), 3);
        assert_eq!(api.hits.load(Ordering::Acquire), 0);
        assert!(client.control.communication().last_success.is_some());
        assert!(
            client.api_pool.endpoints[0]
                .control
                .communication()
                .outcome
                .is_none()
        );
    }

    #[tokio::test]
    async fn noninteractive_requests_skip_disabled_apis_without_prompting() {
        let disabled = Server::start(serde_json::json!({"txid":"disabled"}));
        let enabled = Server::start(serde_json::json!({"txid":"enabled"}));
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let client = client(ApiPool::new(
            "chain".into(),
            vec![endpoint(&disabled.url, ConnectionState::Disabled)],
            Some(sender.clone()),
        ))
        .noninteractive();
        assert!(client.get_raw_transaction("target").await.is_err());
        assert!(receiver.try_recv().is_err());
        assert_eq!(disabled.hits.load(Ordering::Acquire), 0);
        let mut allowed = client.clone();
        allowed.api_pool = Arc::new(ApiPool::new(
            "chain".into(),
            vec![endpoint(&enabled.url, ConnectionState::Enabled)],
            Some(sender),
        ));
        assert_eq!(
            allowed.get_raw_transaction("target").await.unwrap().result["txid"],
            "enabled"
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(enabled.hits.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn api_cancellation_sends_no_network_request() {
        let server = Server::start(serde_json::json!({"txid":"test"}));
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        let client = client(ApiPool::new(
            "chain".into(),
            vec![endpoint(&server.url, ConnectionState::Disabled)],
            Some(sender),
        ));
        let task = tokio::spawn(async move {
            client
                .call_with_raw::<Value>("getrawtransaction", &[])
                .await
        });
        let request = receiver.recv().await.unwrap();
        assert!(
            request
                .local_failure
                .contains("connection disabled; enable it in Sources (F2)")
        );
        assert_eq!(server.hits.load(Ordering::Acquire), 0);
        request
            .reply
            .send(crate::sources::ApiDecision::Cancel)
            .unwrap();
        assert!(task.await.unwrap().is_err());
        assert_eq!(server.hits.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn authorized_api_pool_rotates_and_preserves_endpoint_provenance() {
        let first = Server::start(serde_json::json!({"txid":"one"}));
        let second = Server::start(serde_json::json!({"txid":"two"}));
        let client = client(ApiPool::new(
            "chain".into(),
            vec![
                endpoint(&first.url, ConnectionState::Enabled),
                endpoint(&second.url, ConnectionState::Enabled),
            ],
            None,
        ));
        let one = client
            .call_with_raw::<Value>("getrawtransaction", &[])
            .await
            .unwrap();
        let two = client
            .call_with_raw::<Value>("getrawtransaction", &[])
            .await
            .unwrap();
        assert_eq!(one.result["txid"], "one");
        assert_eq!(two.result["txid"], "two");
        assert_eq!(one.source_id, source_id("public-api", &first.url));
        assert_eq!(two.source_id, source_id("public-api", &second.url));
        assert_eq!(first.hits.load(Ordering::Acquire), 1);
        assert_eq!(second.hits.load(Ordering::Acquire), 1);
    }

    #[tokio::test]
    async fn revoking_api_permission_cancels_queued_request_before_network() {
        let server = Server::start(serde_json::json!({"txid":"test"}));
        let endpoint = endpoint(&server.url, ConnectionState::Enabled);
        let control = endpoint.control.clone();
        let permits = control.permits.acquire_many(2).await.unwrap();
        let client = client(ApiPool::new("chain".into(), vec![endpoint], None));
        let request = client.call_with_raw::<Value>("getrawtransaction", &[]);
        tokio::pin!(request);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut request)
                .await
                .is_err()
        );
        control.set(ConnectionState::Disabled);
        drop(permits);
        assert!(request.await.is_err());
        assert_eq!(server.hits.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn disabled_daemon_never_sends_even_metadata() {
        let server = Server::start(serde_json::json!({"chainid":"iWrong"}));
        let client = RpcClient::with_endpoint(server.url.clone(), None, None).unwrap();
        client.control.set(ConnectionState::Disabled);
        assert!(client.get_info().await.is_err());
        assert_eq!(server.hits.load(Ordering::Acquire), 0);
        assert!(client.control.communication().outcome.is_none());
    }

    #[tokio::test]
    async fn rpc_persists_before_publishing_and_storage_failure_does_not_fallback() {
        use crate::{
            cache::Cache,
            world::{Network, SystemId},
        };
        let server = Server::start(serde_json::json!({"txid":"target"}));
        let api = Server::start(serde_json::json!({"txid":"target"}));
        let root = std::env::temp_dir().join(format!(
            "rpc-storage-{}-{}",
            std::process::id(),
            next_request_order()
        ));
        let cache = Cache::open(&root.join("mainnet.sqlite3")).unwrap();
        let mut client = RpcClient::with_endpoint(server.url.clone(), None, None).unwrap();
        client.storage = Some((
            cache.clone(),
            Network::Mainnet,
            SystemId::try_from("iSystem".to_string()).unwrap(),
        ));
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        client.observations = Some(sender);
        client
            .call_with_raw::<Value>(
                "getrawtransaction",
                &[Value::String("target".into()), Value::from(1)],
            )
            .await
            .unwrap();
        assert!(receiver.try_recv().is_ok());
        assert!(
            cache
                .raw_rpc_response(
                    &source_id("local-daemon", &server.url),
                    "iSystem",
                    "getrawtransaction",
                    "[\"target\",1]"
                )
                .unwrap()
                .is_some()
        );
        client.storage.as_mut().unwrap().1 = Network::Testnet;
        client.api_pool = ApiPool::new(
            "chain".into(),
            vec![endpoint(&api.url, ConnectionState::Enabled)],
            None,
        )
        .into();
        assert!(
            client
                .call_with_raw::<Value>("getrawtransaction", &[])
                .await
                .is_err()
        );
        assert!(receiver.try_recv().is_err());
        assert_eq!(api.hits.load(Ordering::Acquire), 0);
        drop(client);
        drop(cache);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn wrong_system_handshake_stops_object_fetch_and_observations() {
        let server = Server::start(serde_json::json!({"chainid":"iWrong","testnet":false}));
        let mut client = RpcClient::with_endpoint(server.url.clone(), None, None).unwrap();
        client
            .control
            .expected
            .set((
                crate::world::SystemId::try_from("iExpected".to_string()).unwrap(),
                crate::world::Network::Mainnet,
            ))
            .unwrap();
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        client.observations = Some(sender);
        assert!(client.get_raw_transaction("target").await.is_err());
        assert_eq!(server.hits.load(Ordering::Acquire), 1);
        assert!(receiver.try_recv().is_err());
    }
}
