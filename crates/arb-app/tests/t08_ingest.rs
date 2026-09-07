#[path = "../../arb-adapters/tests/support/mod.rs"]
mod support;
use arb_adapters::{
    rpc::{RpcOptions, RpcSource},
    store::Store,
};
use arb_app::ingest::collect;
use rusqlite::Connection;
use serde_json::json;
fn options(run: &str) -> RpcOptions {
    RpcOptions {
        chain_id: 4663,
        source: "rpc".into(),
        run_id: run.into(),
        requests_per_second: 1000,
        max_concurrency: 1,
        retry_limit: 0,
        timeout_ms: 1000,
        max_response_bytes: 1024 * 1024,
    }
}
#[tokio::test]
async fn resume_after_write_failure() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ingest.db");
    drop(Store::open(&path).unwrap());
    let sql = Connection::open(&path).unwrap();
    sql.execute_batch("CREATE TRIGGER stop_second BEFORE INSERT ON raw_records WHEN NEW.sequence='7' BEGIN SELECT RAISE(ABORT,'injected disk failure'); END;").unwrap();
    let e = support::evidence();
    let a = e["requests"][1]["response"]["result"].clone();
    let n =
        u64::from_str_radix(a["number"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
    let mut b = a.clone();
    b["number"] = json!(format!("0x{:x}", n + 1));
    b["hash"] = json!(format!("0x{}", "bb".repeat(32)));
    b["parentHash"] = a["hash"].clone();
    b["transactions"] = json!([]);
    let mut script = vec![
        (200, json!("0x1237")),
        (200, a.clone()),
        (200, e["requests"][4]["response"]["result"].clone()),
        (200, e["requests"][3]["response"]["result"].clone()),
        (200, a),
    ];
    let second = vec![
        (200, json!("0x1237")),
        (200, b.clone()),
        (200, json!([])),
        (200, json!([])),
        (200, b),
    ];
    script.extend(second.clone());
    let server = support::serve(script).await;
    let source = RpcSource::new(&server.url, options("first")).unwrap();
    assert!(collect(&source, &path, n, n + 1, 1).await.is_err());
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.cursor(4663, "rpc").unwrap().unwrap().next_block,
        n + 1
    );
    assert_eq!(store.raw_count().unwrap(), 3);
    drop(store);
    sql.execute_batch("DROP TRIGGER stop_second").unwrap();
    let server = support::serve(second).await;
    let source = RpcSource::new(&server.url, options("restart")).unwrap();
    assert_eq!(collect(&source, &path, n, n + 1, 1).await.unwrap(), 1);
    assert_eq!(server.calls.load(std::sync::atomic::Ordering::SeqCst), 5);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.cursor(4663, "rpc").unwrap().unwrap().next_block,
        n + 2
    );
    assert_eq!(store.raw_count().unwrap(), 6);
}
