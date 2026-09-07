#[path = "support/simulation.rs"]
mod fixture;
#[allow(dead_code)]
mod support;
use arb_adapters::rpc::{RpcOptions, RpcSource};
use arb_core::simulation::SimulationOutcome;
use serde_json::{Value, json};
fn options() -> RpcOptions {
    RpcOptions {
        chain_id: 4663,
        source: "simulation".into(),
        run_id: "test".into(),
        requests_per_second: 1000,
        max_concurrency: 1,
        retry_limit: 0,
        timeout_ms: 1000,
        max_response_bytes: 1024 * 1024,
    }
}
#[tokio::test]
async fn simulate_atomic_route() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../tests/data/verified/atomic-simulation.json"
    ))
    .unwrap();
    let old: Value = serde_json::from_str(include_str!(
        "../../../tests/data/verified/pons-observations.json"
    ))
    .unwrap();
    let expected: Value = serde_json::from_str(include_str!(
        "../../../tests/data/verified/atomic-expected.json"
    ))
    .unwrap();
    let simulations = evidence["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["request"]["method"] == "eth_simulateV1")
        .collect::<Vec<_>>();
    let header = evidence["requests"][0]["response"]["result"].clone();
    for (index, case) in simulations.iter().enumerate() {
        let server = support::serve(vec![
            (200, json!("0x1237")),
            (200, header.clone()),
            (200, old["requests"][8]["response"]["result"].clone()),
            (200, old["requests"][7]["response"]["result"].clone()),
            (200, json!("0x")),
            (200, json!("0x")),
            (200, case["response"]["result"].clone()),
            (200, header.clone()),
        ])
        .await;
        let mut request = fixture::request();
        request.fail_second = index == 1;
        let result = RpcSource::new(&server.url, options())
            .unwrap()
            .simulate(&request)
            .await
            .unwrap();
        assert_eq!(result.actual_position, request.position);
        if index == 0 {
            assert_eq!(result.outcome, SimulationOutcome::Succeeded);
            assert_eq!(
                result.amount_out.unwrap().to_string(),
                expected["amount_out"].as_str().unwrap()
            );
        } else {
            assert_eq!(result.outcome, SimulationOutcome::Reverted);
            assert!(result.state_rolled_back);
            assert!(result.asset_changes.iter().all(|a| a.before == a.after));
        }
    }
}

#[tokio::test]
#[ignore = "explicit read-only public RPC acceptance; uses temporary simulation funding only"]
async fn verified_live_atomic_route() {
    let mut options = options();
    options.requests_per_second = 2;
    options.timeout_ms = 20000;
    options.retry_limit = 1;
    let source = RpcSource::new("https://rpc.mainnet.chain.robinhood.com", options).unwrap();
    let mut request = fixture::request();
    request.position = source.latest_position().await.unwrap();
    let success = source.simulate(&request).await.unwrap();
    assert_eq!(success.outcome, SimulationOutcome::Succeeded);
    request.fail_second = true;
    let failure = source.simulate(&request).await.unwrap();
    assert_eq!(failure.outcome, SimulationOutcome::Reverted);
    assert!(failure.state_rolled_back);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
    std::fs::create_dir_all(&path).unwrap();
    std::fs::write(
        path.join("t23-live-simulation.json"),
        serde_json::to_vec_pretty(&vec![success, failure]).unwrap(),
    )
    .unwrap();
}
#[test]
fn reject_mismatched_simulated_state() {
    let evidence: Value = serde_json::from_str(include_str!(
        "../../../tests/data/verified/atomic-simulation.json"
    ))
    .unwrap();
    let mut response = evidence["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["request"]["method"] == "eth_simulateV1")
        .unwrap()["response"]["result"]
        .clone();
    response[0]["parentHash"] = json!(alloy_primitives::B256::ZERO);
    assert!(
        arb_adapters::simulation::parse_result(&fixture::request(), &response, vec![]).is_err()
    );
}
