#[path = "../../arb-core/tests/support/mod.rs"]
mod core_support;
#[path = "../../arb-adapters/tests/support/simulation.rs"]
mod fixture;
#[path = "support/replay.rs"]
mod raw_support;
use alloy_primitives::{Address, B256, U256};
use arb_adapters::{
    rpc::{RpcOptions, RpcSource},
    store::Store,
};
use arb_app::{pipeline::Pipeline, simulation_queue::*};
use arb_core::{opportunity::*, research::*, simulation::*, state::State, types::*};
use std::sync::Arc;
fn candidate(id: u8) -> Candidate {
    let request = fixture::request();
    Candidate {
        id: B256::repeat_byte(id),
        run_id: "simulation-fixture".into(),
        view_id: B256::repeat_byte(8),
        config_hash: B256::repeat_byte(7),
        algorithm_version: "test".into(),
        raw_refs: vec![],
        detected_at_ms: 1,
        canonical: true,
        simulation_status: SimulationStatus::NotRun,
        opportunity: Opportunity {
            route: request.route,
            position: request.position,
            amount_in: request.amount,
            amount_out: U256::from(80733325),
            quotes: vec![],
            gross_profit: SignedAmount {
                negative: true,
                magnitude: U256::from(919266675),
            },
            net_profit: None,
            costs: CostEstimate {
                asset: Address::ZERO,
                amount: None,
                conversion: None,
                basis: "queue identity fixture, no economic assertion".into(),
            },
        },
    }
}
#[tokio::test]
async fn timeout_does_not_block_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("queue.db");
    let mut store = Store::open(&path).unwrap();
    let records = raw_support::block(2, B256::repeat_byte(1));
    store
        .append_raw(
            &records,
            &SourceCursor {
                chain_id: 4663,
                source: "rpc".into(),
                next_block: 3,
                last_block_hash: Some(B256::repeat_byte(2)),
            },
        )
        .unwrap();
    let run = RunSpec {
        run_id: "live".into(),
        config_hash: B256::repeat_byte(7),
        algorithm_version: "v1".into(),
        registry_version: 1,
        quote_asset: Address::ZERO,
        amounts: vec![U256::from(100)],
        min_depth: U256::ZERO,
        min_profit: U256::from(1),
        costs: CostEstimate {
            asset: Address::ZERO,
            amount: Some(U256::from(3)),
            conversion: None,
            basis: "test".into(),
        },
    };
    let mut pipeline = Pipeline::new(
        store,
        State::from_bootstrap(core_support::profitable_bootstrap()).unwrap(),
        run,
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (entered, waiting) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        let _ = entered.send(());
        std::future::pending::<()>().await;
    });
    let source = Arc::new(
        RpcSource::new(
            &url,
            RpcOptions {
                chain_id: 4663,
                source: "sim".into(),
                run_id: "test".into(),
                requests_per_second: 1000,
                max_concurrency: 1,
                retry_limit: 0,
                timeout_ms: 5000,
                max_response_bytes: 1024 * 1024,
            },
        )
        .unwrap(),
    );
    let queue = SimulationQueue::new(
        source,
        path.clone(),
        QueueOptions {
            queue_id: "queue-test".into(),
            capacity: 1,
            concurrency: 1,
            queue_timeout_ms: 200,
            call_timeout_ms: 1000,
        },
    )
    .unwrap();
    let first = queue
        .submit(candidate(1), fixture::request())
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), waiting)
        .await
        .unwrap()
        .unwrap();
    let second = queue
        .submit(candidate(2), fixture::request())
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    let full = queue
        .submit(candidate(3), fixture::request())
        .await
        .unwrap();
    pipeline.process_records(&records, 202).unwrap();
    assert_eq!(pipeline.view().position.block_number, 2);
    queue.shutdown(2000).await.unwrap();
    server.abort();
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.load_simulation(first).unwrap().outcome,
        SimulationOutcome::Unknown
    );
    assert_eq!(
        store.load_simulation(second).unwrap().phase,
        SimulationPhase::QueueExpired
    );
    assert_eq!(
        store.load_simulation(full).unwrap().phase,
        SimulationPhase::QueueFull
    );
    // Storage association fixture: exercise canonicality independently of RPC parsing.
    let block = store
        .find_derived("live", B256::repeat_byte(2))
        .unwrap()
        .unwrap();
    let c = &block.candidates[0];
    let mut record = store.load_simulation(first).unwrap();
    record.id = B256::repeat_byte(99);
    record.run_id = c.run_id.clone();
    record.candidate_id = c.id;
    record.view_id = c.view_id;
    record.expected_position = c.opportunity.position.clone();
    record.request.position = c.opportunity.position.clone();
    record.request.route = c.opportunity.route.clone();
    record.request.amount = c.opportunity.amount_in;
    record.phase = SimulationPhase::Finished;
    record.outcome = SimulationOutcome::Succeeded;
    record.result = Some(SimulationResult {
        request: record.request.clone(),
        actual_position: c.opportunity.position.clone(),
        simulated_block_hash: B256::repeat_byte(10),
        outcome: SimulationOutcome::Succeeded,
        middle_amount: None,
        amount_out: Some(c.opportunity.amount_out),
        gas_used: 1,
        asset_changes: vec![],
        pool_slots_before: [B256::ZERO; 2],
        pool_slots_after: [B256::ZERO; 2],
        state_rolled_back: false,
        error: None,
        evidence: vec![],
    });
    let mut store = store;
    store.insert_simulation(&record).unwrap();
    store.save_simulation(&mut record).unwrap();
    assert!(
        store
            .load_simulation(record.id)
            .unwrap()
            .validates_original_candidate
    );
    store
        .begin_recovery(
            B256::repeat_byte(90),
            "live",
            b"test",
            &[B256::repeat_byte(2)],
        )
        .unwrap();
    assert!(
        !store
            .load_simulation(record.id)
            .unwrap()
            .validates_original_candidate
    );
    store.finish_recovery(B256::repeat_byte(90)).unwrap();
    store.save_simulation(&mut record).unwrap();
    let late = store.load_simulation(record.id).unwrap();
    assert!(late.result.is_some());
    assert!(!late.canonical);
    assert!(!late.validates_original_candidate);
}
#[test]
fn mismatched_state_does_not_validate_candidate() {
    let raw: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/data/verified/atomic-simulation.json"
    ))
    .unwrap();
    let response = &raw["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["request"]["method"] == "eth_simulateV1")
        .unwrap()["response"]["result"];
    let result =
        arb_adapters::simulation::parse_result(&fixture::request(), response, vec![]).unwrap();
    let mut c = candidate(1);
    assert!(validates_candidate(&c, &result));
    c.opportunity.amount_out += U256::from(1);
    assert!(!validates_candidate(&c, &result));
    c = candidate(1);
    c.opportunity.position.block_hash = B256::ZERO;
    assert!(!validates_candidate(&c, &result));
    c = candidate(1);
    c.canonical = false;
    assert!(!validates_candidate(&c, &result));
}
