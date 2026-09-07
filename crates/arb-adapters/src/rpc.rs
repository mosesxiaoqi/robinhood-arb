use alloy_json_rpc::{Id, Request, Response, ResponsePayload};
use alloy_primitives::B256;
use arb_core::types::{ChainPosition, Confirmation, ExecutionStatus, Offset, RawRecord};
use reqwest::Client;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;
use tokio::{
    sync::{Mutex, Semaphore},
    time::{Instant, sleep, sleep_until},
};

#[derive(Clone)]
pub struct RpcOptions {
    pub chain_id: u64,
    pub source: String,
    pub run_id: String,
    pub requests_per_second: u32,
    pub max_concurrency: usize,
    pub retry_limit: u32,
    pub timeout_ms: u64,
    pub max_response_bytes: usize,
}
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("RPC transport failed (endpoint redacted)")]
    Transport,
    #[error("RPC HTTP status {0}")]
    Http(u16),
    #[error("RPC error code {0}")]
    Rpc(i64),
    #[error("RPC response exceeds byte limit")]
    TooLarge,
    #[error("invalid RPC data: {0}")]
    Invalid(&'static str),
}
struct Reply {
    value: Value,
    bytes: Vec<u8>,
    id: u64,
    received_at_ms: u64,
}
pub struct RpcSource {
    client: Client,
    endpoint: reqwest::Url,
    options: RpcOptions,
    next_request: Mutex<Instant>,
    pub(crate) gate: Semaphore,
    sequence: AtomicU64,
}
impl RpcSource {
    pub fn new(endpoint: &str, options: RpcOptions) -> Result<Self, SourceError> {
        let endpoint =
            reqwest::Url::parse(endpoint).map_err(|_| SourceError::Invalid("endpoint"))?;
        if !matches!(endpoint.scheme(), "http" | "https")
            || endpoint.host_str().is_none()
            || endpoint.fragment().is_some()
        {
            return Err(SourceError::Invalid("HTTP(S) endpoint required"));
        }
        if options.chain_id == 0
            || options.requests_per_second == 0
            || options.max_concurrency == 0
            || options.max_concurrency > 1024
            || options.retry_limit > 10
            || options.timeout_ms == 0
            || options.timeout_ms > 300_000
            || options.max_response_bytes == 0
            || options.max_response_bytes > 64 * 1024 * 1024
        {
            return Err(SourceError::Invalid("request limits"));
        }
        for label in [&options.source, &options.run_id] {
            if label.is_empty()
                || label.len() > 128
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            {
                return Err(SourceError::Invalid("source/run label"));
            }
        }
        let mut builder = Client::builder()
            .timeout(Duration::from_millis(options.timeout_ms))
            .redirect(reqwest::redirect::Policy::none());
        if matches!(
            endpoint.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]")
        ) {
            builder = builder.no_proxy();
        }
        Ok(Self {
            client: builder.build().map_err(|_| SourceError::Transport)?,
            endpoint,
            gate: Semaphore::new(options.max_concurrency),
            options,
            next_request: Mutex::new(Instant::now()),
            sequence: AtomicU64::new(1),
        })
    }

    async fn request(&self, method: &'static str, params: Value) -> Result<Reply, SourceError> {
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let request = Request::new(method, Id::Number(id), params);
        for attempt in 0..=self.options.retry_limit {
            {
                let mut next = self.next_request.lock().await;
                sleep_until(*next).await;
                *next = Instant::now()
                    + Duration::from_secs_f64(1.0 / f64::from(self.options.requests_per_second));
            }
            let result = self.send(&request, id).await;
            let retry = matches!(
                &result,
                Err(SourceError::Transport)
                    | Err(SourceError::Http(429 | 500 | 502 | 503 | 504))
                    | Err(SourceError::Rpc(-32005 | 429))
            );
            if !retry || attempt == self.options.retry_limit {
                return result;
            }
            sleep(Duration::from_millis(100_u64 << attempt)).await;
        }
        unreachable!("inclusive retry loop returns")
    }

    async fn send(&self, request: &Request<Value>, id: u64) -> Result<Reply, SourceError> {
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .json(request)
            .send()
            .await
            .map_err(|_| SourceError::Transport)?;
        if !response.status().is_success() {
            return Err(SourceError::Http(response.status().as_u16()));
        }
        if response
            .content_length()
            .is_some_and(|n| n > self.options.max_response_bytes as u64)
        {
            return Err(SourceError::TooLarge);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| SourceError::Transport)? {
            if bytes.len() + chunk.len() > self.options.max_response_bytes {
                return Err(SourceError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        let received_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SourceError::Invalid("system clock"))?
            .as_millis()
            .try_into()
            .map_err(|_| SourceError::Invalid("timestamp overflow"))?;
        let response: Response<Value, Value> = serde_json::from_slice(&bytes)
            .map_err(|_| SourceError::Invalid("JSON-RPC envelope"))?;
        if response.id != Id::Number(id) {
            return Err(SourceError::Invalid("response id mismatch"));
        }
        match response.payload {
            ResponsePayload::Success(value) => Ok(Reply {
                value,
                bytes,
                id,
                received_at_ms,
            }),
            ResponsePayload::Failure(error) => Err(SourceError::Rpc(error.code)),
        }
    }

    pub async fn fetch_block(&self, number: u64) -> Result<Vec<RawRecord>, SourceError> {
        let _permit = self
            .gate
            .acquire()
            .await
            .map_err(|_| SourceError::Invalid("source closed"))?;
        let chain = self.request("eth_chainId", json!([])).await?;
        if quantity(&chain.value)? != self.options.chain_id {
            return Err(SourceError::Invalid("chain id mismatch"));
        }
        let tag = format!("0x{number:x}");
        let block = self
            .request("eth_getBlockByNumber", json!([tag, false]))
            .await?;
        if quantity(&block.value["number"])? != number {
            return Err(SourceError::Invalid("block number mismatch"));
        }
        let block_hash = hash(&block.value["hash"])?;
        hash(&block.value["parentHash"])?;
        let transactions = block.value["transactions"]
            .as_array()
            .ok_or(SourceError::Invalid("transaction list"))?;
        let mut hashes = BTreeSet::new();
        for tx in transactions {
            if !hashes.insert(hash(tx)?) {
                return Err(SourceError::Invalid("duplicate transaction"));
            }
        }
        let receipts = self.request("eth_getBlockReceipts", json!([tag])).await?;
        let items = receipts
            .value
            .as_array()
            .ok_or(SourceError::Invalid("receipt list"))?;
        if items.len() != transactions.len() {
            return Err(SourceError::Invalid("incomplete receipts"));
        }
        let mut seen = BTreeSet::new();
        let mut receipt_logs = Vec::new();
        for receipt in items {
            if hash(&receipt["blockHash"])? != block_hash
                || quantity(&receipt["blockNumber"])? != number
            {
                return Err(SourceError::Invalid("receipt block mismatch"));
            }
            let tx_hash = hash(&receipt["transactionHash"])?;
            let index = usize::try_from(quantity(&receipt["transactionIndex"])?)
                .map_err(|_| SourceError::Invalid("transaction index"))?;
            if !seen.insert(tx_hash)
                || transactions.get(index).and_then(Value::as_str)
                    != receipt["transactionHash"].as_str()
            {
                return Err(SourceError::Invalid("receipt transaction mismatch"));
            }
            let status = quantity(&receipt["status"])?;
            if status > 1 {
                return Err(SourceError::Invalid("receipt status"));
            }
            let logs = receipt["logs"]
                .as_array()
                .ok_or(SourceError::Invalid("receipt logs"))?;
            if status == 0 && !logs.is_empty() {
                return Err(SourceError::Invalid("reverted receipt contains logs"));
            }
            for log in logs {
                if hash(&log["blockHash"])? != block_hash
                    || hash(&log["transactionHash"])? != tx_hash
                    || quantity(&log["transactionIndex"])? != index as u64
                    || log["removed"] != false
                {
                    return Err(SourceError::Invalid("log identity"));
                }
                receipt_logs.push(log.to_string());
            }
        }
        let logs = self
            .request("eth_getLogs", json!([{"blockHash":block_hash}]))
            .await?;
        let mut queried_logs = logs
            .value
            .as_array()
            .ok_or(SourceError::Invalid("log list"))?
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>();
        queried_logs.sort();
        receipt_logs.sort();
        if queried_logs != receipt_logs {
            return Err(SourceError::Invalid("logs and receipts disagree"));
        }
        let end = self
            .request("eth_getBlockByNumber", json!([tag, false]))
            .await?;
        if hash(&end.value["hash"])? != block_hash {
            return Err(SourceError::Invalid("block changed during collection"));
        }
        let position = ChainPosition {
            block_number: number,
            block_hash,
            offset: Offset::BlockEnd,
        };
        let mut result = Vec::new();
        let mut bytes = 0;
        for (kind, reply) in [("block", block), ("receipts", receipts), ("logs", logs)] {
            bytes += reply.bytes.len();
            if bytes > self.options.max_response_bytes {
                return Err(SourceError::TooLarge);
            }
            result.push(RawRecord {
                version: 1,
                chain_id: self.options.chain_id,
                source: self.options.source.clone(),
                run_id: self.options.run_id.clone(),
                sequence: reply.id,
                received_at_ms: reply.received_at_ms,
                kind: kind.into(),
                position: Some(position.clone()),
                transaction_hash: None,
                execution_status: ExecutionStatus::Unknown,
                confirmation: Confirmation::Included,
                payload: reply.bytes,
            });
        }
        Ok(result)
    }
}
fn quantity(value: &Value) -> Result<u64, SourceError> {
    let text = value
        .as_str()
        .and_then(|v| v.strip_prefix("0x"))
        .ok_or(SourceError::Invalid("hex quantity"))?;
    u64::from_str_radix(text, 16).map_err(|_| SourceError::Invalid("hex quantity"))
}
fn hash(value: &Value) -> Result<B256, SourceError> {
    B256::from_str(value.as_str().ok_or(SourceError::Invalid("hash"))?)
        .map_err(|_| SourceError::Invalid("hash"))
}

impl RpcSource {
    pub fn chain_id(&self) -> u64 {
        self.options.chain_id
    }
    pub fn source_label(&self) -> &str {
        &self.options.source
    }
}

impl RpcSource {
    pub(crate) async fn bootstrap_request(
        &self,
        method: &'static str,
        params: Value,
        evidence: &mut Vec<Vec<u8>>,
    ) -> Result<Value, SourceError> {
        let reply = self.request(method, params.clone()).await?;
        let entry = serde_json::to_vec(
            &json!({"method":method,"params":params,"response_bytes":reply.bytes}),
        )
        .map_err(|_| SourceError::Invalid("evidence JSON"))?;
        if evidence.iter().map(Vec::len).sum::<usize>() + entry.len() > 64 * 1024 * 1024 {
            return Err(SourceError::TooLarge);
        }
        evidence.push(entry);
        Ok(reply.value)
    }
}
