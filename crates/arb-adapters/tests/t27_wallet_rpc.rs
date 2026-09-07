#[allow(dead_code)]
mod support;
#[path = "support/verified.rs"]
mod verified;
use alloy_primitives::{Address, B256};
use arb_adapters::{discovery::discover, rpc::*};
use arb_core::wallet::BalanceScope;
use serde_json::{Value, json};
#[tokio::test]
async fn block_balances_are_not_transaction_profit() {
    let raw = verified::receipt();
    let receipt: Value = serde_json::from_slice(&raw.payload).unwrap();
    let receipt = receipt["result"][0].clone();
    let pools = discover(std::slice::from_ref(&raw)).unwrap();
    let at = raw.position.unwrap();
    let previous = B256::repeat_byte(9);
    let parent = json!({"hash":previous,"number":format!("0x{:x}",at.block_number-1)});
    let header = json!({"hash":at.block_hash,"number":format!("0x{:x}",at.block_number),"parentHash":previous});
    let wallet: Address = receipt["from"].as_str().unwrap().parse().unwrap();
    let tx = json!({"hash":receipt["transactionHash"],"blockHash":receipt["blockHash"],"blockNumber":receipt["blockNumber"],"transactionIndex":receipt["transactionIndex"],"from":receipt["from"],"to":receipt["to"]});
    let script = vec![
        json!("0x1237"),
        tx,
        receipt.clone(),
        header.clone(),
        parent.clone(),
        json!("0x100"),
        json!("0x80"),
        json!("0x0"),
        json!("0x1000"),
        json!("0x"),
        parent,
        header,
    ];
    let server = support::serve(script.into_iter().map(|v| (200, v)).collect()).await;
    let source = RpcSource::new(
        &server.url,
        RpcOptions {
            chain_id: 4663,
            source: "wallet".into(),
            run_id: "test".into(),
            requests_per_second: 1000,
            max_concurrency: 1,
            retry_limit: 0,
            timeout_ms: 1000,
            max_response_bytes: 1024 * 1024,
        },
    )
    .unwrap();
    let facts = source
        .wallet_facts(
            receipt["transactionHash"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
            wallet,
            &[pools[0].token],
            &pools,
        )
        .await
        .unwrap();
    assert_eq!(facts.balance_scope, BalanceScope::BlockBoundary);
    assert!(!facts.transaction_balances_complete);
    assert!(facts.valuation.is_none());
    assert!(facts.swap_count > 0);
    assert!(!facts.transfers.is_empty());
    assert_eq!(facts.changes.len(), 2);
    assert_eq!(facts.evidence.len(), 12);
}
