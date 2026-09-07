mod support;
use arb_adapters::rpc::{RpcOptions, RpcSource};
use serde_json::json;
use std::sync::atomic::Ordering;
fn options() -> RpcOptions {
    RpcOptions {
        chain_id: 4663,
        source: "rpc".into(),
        run_id: "test".into(),
        requests_per_second: 1000,
        max_concurrency: 1,
        retry_limit: 1,
        timeout_ms: 1000,
        max_response_bytes: 1024 * 1024,
    }
}
#[tokio::test]
async fn reject_wrong_chain_and_partial_block() {
    let wrong = support::serve(vec![(200, json!("0x1"))]).await;
    assert!(
        RpcSource::new(&wrong.url, options())
            .unwrap()
            .fetch_block(1)
            .await
            .is_err()
    );
    let e = support::evidence();
    let block = e["requests"][1]["response"]["result"].clone();
    let number = u64::from_str_radix(
        block["number"].as_str().unwrap().trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let partial = support::serve(vec![
        (200, json!("0x1237")),
        (200, block.clone()),
        (200, json!([])),
    ])
    .await;
    assert!(
        RpcSource::new(&partial.url, options())
            .unwrap()
            .fetch_block(number)
            .await
            .is_err()
    );
    let limited = support::serve(vec![(429, json!(null)), (429, json!(null))]).await;
    assert!(
        RpcSource::new(&limited.url, options())
            .unwrap()
            .fetch_block(number)
            .await
            .is_err()
    );
    assert_eq!(limited.calls.load(Ordering::SeqCst), 2);
    let huge = support::serve(vec![(200, json!("x".repeat(2000)))]).await;
    let mut small = options();
    small.max_response_bytes = 100;
    assert!(
        RpcSource::new(&huge.url, small)
            .unwrap()
            .fetch_block(number)
            .await
            .is_err()
    );
    let mut queried = e["requests"][3]["response"]["result"].clone();
    for log in queried.as_array_mut().unwrap() {
        log["blockTimestamp"] = json!("0x0");
    }
    let success = support::serve(vec![
        (200, json!("0x1237")),
        (200, block.clone()),
        (200, e["requests"][4]["response"]["result"].clone()),
        (200, queried),
        (200, block),
    ])
    .await;
    let records = RpcSource::new(&success.url, options())
        .unwrap()
        .fetch_block(number)
        .await
        .unwrap();
    assert_eq!(records.len(), 3);
    assert!(records.iter().all(|r| r.request_elapsed_ns.is_some()));
    assert_eq!(records[0].position.as_ref().unwrap().block_number, number);
}

#[tokio::test]
#[ignore = "explicit public RPC acceptance; does not run in offline suite"]
async fn verified_mainnet_block() {
    let e = support::evidence();
    let expected = &e["requests"][1]["response"]["result"];
    let number = u64::from_str_radix(
        expected["number"]
            .as_str()
            .unwrap()
            .trim_start_matches("0x"),
        16,
    )
    .unwrap();
    let mut settings = options();
    settings.requests_per_second = 2;
    settings.timeout_ms = 20000;
    settings.max_response_bytes = 16 * 1024 * 1024;
    let source = RpcSource::new("https://rpc.mainnet.chain.robinhood.com", settings).unwrap();
    let records = source.fetch_block(number).await.unwrap();
    assert_eq!(
        format!("{:#x}", records[0].position.as_ref().unwrap().block_hash),
        expected["hash"].as_str().unwrap()
    );
    assert_eq!(records.len(), 3);
    assert!(records.iter().all(|r| r.request_elapsed_ns.is_some()));
}
