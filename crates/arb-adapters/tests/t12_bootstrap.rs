#[allow(dead_code)]
mod support;
#[path = "support/verified.rs"]
mod verified;
use alloy_primitives::U256;
use arb_adapters::{
    discovery::discover,
    rpc::{RpcOptions, RpcSource},
};
use serde_json::{Value, json};
fn word(n: U256) -> Value {
    json!(format!("0x{n:064x}"))
}
fn options() -> RpcOptions {
    RpcOptions {
        chain_id: 4663,
        source: "rpc".into(),
        run_id: "bootstrap".into(),
        requests_per_second: 1000,
        max_concurrency: 1,
        retry_limit: 0,
        timeout_ms: 1000,
        max_response_bytes: 1024 * 1024,
    }
}
#[tokio::test]
async fn bootstrap_at_one_hash() {
    let raw = verified::receipt();
    let pools = discover(std::slice::from_ref(&raw)).unwrap();
    let at = raw.position.unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/data/verified/pons-observations.json"
    ))
    .unwrap();
    let codes: Value =
        serde_json::from_str(include_str!("../../../tests/data/verified/contracts.json")).unwrap();
    let result = |i: usize| fixture["requests"][i]["response"]["result"].clone();
    let header = json!({"number":format!("0x{:x}",at.block_number),"hash":at.block_hash});
    let slot = U256::from_str_radix("547066765876760436649075174264278", 10).unwrap()
        | (U256::from(176808u64) << 160);
    let mut script = vec![
        (200, json!("0x1237")),
        (200, header.clone()),
        (200, codes[0]["runtime_code"].clone()),
        (200, result(7)),
        (200, result(8)),
        (200, result(10)),
        (200, result(11)),
        (200, result(12)),
        (200, word(slot)),
        (200, word(U256::from(29277002188455995792899u128))),
        (200, word(U256::ZERO)),
        (200, header.clone()),
    ];
    let server = support::serve(script.clone()).await;
    let snapshot = RpcSource::new(&server.url, options())
        .unwrap()
        .bootstrap(&pools, at.clone())
        .await
        .unwrap();
    assert_eq!(snapshot.position, at);
    assert_eq!(snapshot.pools.len(), 1);
    assert!(snapshot.pools[0].descriptor.is_quoteable());
    assert_eq!(snapshot.pools[0].tick, 176808);
    for entry in &snapshot.evidence {
        let entry: Value = serde_json::from_slice(entry).unwrap();
        if matches!(entry["method"].as_str(), Some("eth_call" | "eth_getCode")) {
            assert_eq!(
                entry["params"][1],
                json!(format!("0x{:x}", at.block_number))
            );
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snapshot.db");
    let mut store = arb_adapters::store::Store::open(&path).unwrap();
    let id = store.save_bootstrap(&snapshot).unwrap();
    drop(store);
    assert_eq!(
        arb_adapters::store::Store::open(&path)
            .unwrap()
            .load_bootstrap(id)
            .unwrap(),
        snapshot
    );
    // A changed end hash must never publish a coherent initial view.
    script.last_mut().unwrap().1["hash"] = json!(alloy_primitives::B256::ZERO);
    let server = support::serve(script).await;
    assert!(
        RpcSource::new(&server.url, options())
            .unwrap()
            .bootstrap(&pools, at.clone())
            .await
            .is_err()
    );
    // A second pool with unavailable state makes the whole request fail.
    let mut two = pools.clone();
    let mut second = pools[0].clone();
    second.tick_spacing = 400;
    second.id.locator = arb_core::route::PoolLocator::Singleton {
        manager: arb_adapters::discovery::MANAGER,
        pool_id: alloy_primitives::keccak256(alloy_sol_types::SolValue::abi_encode(&(
            second.currency0,
            second.currency1,
            U256::ZERO,
            U256::from(400),
            second.hook,
        ))),
    };
    two.push(second);
    let mut second_launch = result(10).as_str().unwrap().to_owned();
    second_launch.replace_range(2 + 7 * 64..2 + 8 * 64, &format!("{:064x}", 400));
    let mut pair_script = vec![
        (200, json!("0x1237")),
        (200, header.clone()),
        (200, codes[0]["runtime_code"].clone()),
        (200, result(7)),
        (200, result(8)),
        (200, result(10)),
        (200, result(11)),
        (200, result(12)),
        (200, word(slot)),
        (200, word(U256::from(1))),
        (200, word(U256::ZERO)),
        (200, json!(second_launch)),
        (200, result(11)),
        (200, result(12)),
        (200, word(slot)),
        (200, word(U256::from(2))),
        (200, word(U256::ZERO)),
        (200, header),
    ];
    let server = support::serve(pair_script.clone()).await;
    let pair = RpcSource::new(&server.url, options())
        .unwrap()
        .bootstrap(&two, at.clone())
        .await
        .unwrap();
    assert_eq!(pair.pools.len(), 2);
    assert_eq!(pair.position, at);
    pair_script[15].1 = json!("0x");
    let server = support::serve(pair_script).await;
    assert!(
        RpcSource::new(&server.url, options())
            .unwrap()
            .bootstrap(&two, at)
            .await
            .is_err()
    );
}
