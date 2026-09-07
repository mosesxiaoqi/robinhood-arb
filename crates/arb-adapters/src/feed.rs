use alloy_primitives::{B256, keccak256};
use arb_core::types::*;
use futures_util::StreamExt;
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{client::IntoClientRequest, protocol::WebSocketConfig},
};

#[derive(Debug, thiserror::Error)]
#[error("feed: {0}")]
pub struct FeedError(pub &'static str);
pub struct FeedPacket {
    pub raw: RawRecord,
    pub messages: Vec<(u64, B256)>,
}
#[derive(Debug, Default)]
pub struct FeedCommit {
    pub new_messages: usize,
    pub duplicates: usize,
    pub gaps: Vec<(u64, u64)>,
}
pub struct FeedSource {
    url: String,
    chain: u64,
    limit: usize,
    timeout: Duration,
    retries: u32,
}
impl FeedSource {
    pub fn retry_limit(&self) -> u32 {
        self.retries
    }

    pub fn new(
        url: &str,
        chain: u64,
        limit: usize,
        timeout_ms: u64,
        retries: u32,
    ) -> Result<Self, FeedError> {
        let parsed = url::Url::parse(url).map_err(|_| FeedError("invalid URL"))?;
        if !matches!(parsed.scheme(), "wss" | "ws")
            || parsed.host_str().is_none()
            || parsed.fragment().is_some()
            || chain != 4663
            || !(1..=67108864).contains(&limit)
            || !(1..=300000).contains(&timeout_ms)
            || retries > 10
        {
            return Err(FeedError("unsupported source or bounds"));
        }
        Ok(Self {
            url: url.into(),
            chain,
            limit,
            timeout: Duration::from_millis(timeout_ms),
            retries,
        })
    }
    pub fn decode(&self, bytes: &[u8], run: &str, received: u64) -> Result<FeedPacket, FeedError> {
        if bytes.len() > self.limit {
            return Err(FeedError("message size limit"));
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|_| FeedError("invalid JSON"))?;
        if value["version"].as_u64() != Some(1) {
            return Err(FeedError("unsupported version"));
        }
        let mut messages = vec![];
        if let Some(array) = value.get("messages").and_then(Value::as_array) {
            if array.len() > 10000 {
                return Err(FeedError("message count limit"));
            }
            for message in array {
                let sequence = message["sequenceNumber"]
                    .as_u64()
                    .filter(|v| *v < u64::MAX)
                    .ok_or(FeedError("missing sequence"))?;
                if !message["message"]["message"]["l2Msg"].is_string()
                    || !message["message"]["message"]["header"].is_object()
                {
                    return Err(FeedError("invalid message envelope"));
                }
                // Preserve opaque L2 payload; no decompression, signature validation or execution claim.
                let encoded = serde_json::to_vec(&message["message"])
                    .map_err(|_| FeedError("message encoding"))?;
                messages.push((sequence, keccak256(encoded)));
            }
        } else if value["confirmedSequenceNumberMessage"]["sequenceNumber"]
            .as_u64()
            .is_none()
        {
            return Err(FeedError("unknown message variant"));
        }
        let raw = RawRecord {
            version: 1,
            chain_id: self.chain,
            source: "feed".into(),
            run_id: run.into(),
            sequence: 0,
            received_at_ms: received,
            request_elapsed_ns: None,
            kind: "feed-frame-v1".into(),
            position: None,
            transaction_hash: None,
            execution_status: ExecutionStatus::Unknown,
            confirmation: Confirmation::Unknown,
            payload: bytes.to_vec(),
        };
        raw.validate().map_err(|_| FeedError("invalid record"))?;
        Ok(FeedPacket { raw, messages })
    }
    /// A session keeps its WebSocket open; reconnect uses only the durable cursor.
    pub async fn connect(&self, next: Option<u64>) -> Result<FeedSession<'_>, FeedError> {
        let mut last = FeedError("connection unavailable");
        for attempt in 0..=self.retries {
            let result = tokio::time::timeout(self.timeout, async {
                let mut request = self
                    .url
                    .as_str()
                    .into_client_request()
                    .map_err(|_| FeedError("invalid request"))?;
                request
                    .headers_mut()
                    .insert("Arbitrum-Feed-Client-Version", "2".parse().unwrap());
                if let Some(next) = next {
                    request.headers_mut().insert(
                        "Arbitrum-Requested-Sequence-Number",
                        next.to_string()
                            .parse()
                            .map_err(|_| FeedError("sequence header"))?,
                    );
                }
                let config = WebSocketConfig::default()
                    .max_message_size(Some(self.limit))
                    .max_frame_size(Some(self.limit));
                let (socket, response) = connect_async_with_config(request, Some(config), false)
                    .await
                    .map_err(|_| FeedError("WebSocket connection failed"))?;
                if response.headers().contains_key("sec-websocket-extensions") {
                    return Err(FeedError("unexpected compression extension"));
                }
                if response
                    .headers()
                    .get("Arbitrum-Chain-Id")
                    .is_some_and(|v| {
                        v.to_str().ok().and_then(|s| s.parse::<u64>().ok()) != Some(self.chain)
                    })
                {
                    return Err(FeedError("chain header mismatch"));
                }
                if response
                    .headers()
                    .get("Arbitrum-Feed-Server-Version")
                    .is_some_and(|v| v.to_str().ok() != Some("2"))
                {
                    return Err(FeedError("server version mismatch"));
                }
                Ok(FeedSession {
                    source: self,
                    socket,
                })
            })
            .await;
            match result {
                Ok(Ok(session)) => return Ok(session),
                Ok(Err(e)) => last = e,
                Err(_) => last = FeedError("connect timeout"),
            }
            if attempt < self.retries {
                tokio::time::sleep(Duration::from_millis(250 * (1 << attempt))).await;
            }
        }
        Err(last)
    }
}
pub struct FeedSession<'a> {
    source: &'a FeedSource,
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}
impl FeedSession<'_> {
    pub async fn receive(&mut self, run: &str) -> Result<FeedPacket, FeedError> {
        tokio::time::timeout(self.source.timeout, async {
            while let Some(message) = self.socket.next().await {
                let message = message.map_err(|_| FeedError("WebSocket read failed"))?;
                if message.is_text() || message.is_binary() {
                    let time = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| FeedError("system clock"))?
                        .as_millis()
                        .try_into()
                        .map_err(|_| FeedError("system clock overflow"))?;
                    return self.source.decode(&message.into_data(), run, time);
                }
                if message.is_close() {
                    break;
                }
            }
            Err(FeedError("connection closed before message"))
        })
        .await
        .map_err(|_| FeedError("receive timeout"))?
    }
}
