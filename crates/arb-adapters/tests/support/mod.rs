use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
pub struct Server {
    pub url: String,
    pub calls: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
pub async fn serve(script: Vec<(u16, Value)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let task = tokio::spawn(async move {
        for (status, result) in script {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let request = loop {
                let mut chunk = [0u8; 4096];
                let n = stream.read(&mut chunk).await.unwrap();
                if n == 0 {
                    return;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]);
                    let len: usize = headers
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    if bytes.len() >= end + 4 + len {
                        break serde_json::from_slice::<Value>(&bytes[end + 4..end + 4 + len])
                            .unwrap();
                    }
                }
                assert!(bytes.len() < 65536);
            };
            count.fetch_add(1, Ordering::SeqCst);
            let envelope = if result.get("error").is_some() {
                json!({"jsonrpc":"2.0","id":request["id"],"error":result["error"]})
            } else {
                json!({"jsonrpc":"2.0","id":request["id"],"result":result})
            };
            let body = serde_json::to_vec(&envelope).unwrap();
            let head = format!(
                "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            if stream.write_all(head.as_bytes()).await.is_err() {
                continue;
            }
            let _ = stream.write_all(&body).await;
        }
    });
    Server { url, calls, task }
}
pub fn evidence() -> Value {
    serde_json::from_str(include_str!(
        "../../../../docs/verification/network-probe.json"
    ))
    .unwrap()
}
