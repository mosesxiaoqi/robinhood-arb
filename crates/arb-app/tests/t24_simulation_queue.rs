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
            disk_budget: None,
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
    let recovery_bytes = |id, orphan_hashes| {
        serde_json::to_vec(&arb_app::recovery::RecoveryPlan {
            id,
            run_id: "live".into(),
            checkpoint_id: 0,
            common_ancestor: arb_core::types::ChainPosition {
                block_number: 1,
                block_hash: B256::repeat_byte(1),
                offset: arb_core::types::Offset::BlockEnd,
            },
            required_fetch: None,
            orphan_hashes,
            replay_blocks: vec![],
        })
        .unwrap()
    };
    store
        .begin_recovery(
            B256::repeat_byte(90),
            "live",
            &recovery_bytes(B256::repeat_byte(90), vec![B256::repeat_byte(2)]),
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
    // A later reorg can restore this exact block and its original simulation.
    let recovery = B256::repeat_byte(91);
    store
        .begin_recovery(recovery, "live", &recovery_bytes(recovery, vec![]), &[])
        .unwrap();
    store
        .reactivate_derived("live", B256::repeat_byte(2))
        .unwrap();
    assert!(!store.load_simulation(record.id).unwrap().canonical);
    store.finish_recovery(recovery).unwrap();
    let restored = store.load_simulation(record.id).unwrap();
    assert!(restored.canonical);
    assert!(restored.validates_original_candidate);
    assert_eq!(restored.result, late.result);
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

#[tokio::test]
async fn bounded_shutdown_interrupts_slow_simulation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stop.db");
    drop(Store::open(&path).unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let source = Arc::new(
        RpcSource::new(
            &format!("http://{}", listener.local_addr().unwrap()),
            RpcOptions {
                chain_id: 4663,
                source: "slow".into(),
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
            queue_id: "stop".into(),
            capacity: 1,
            concurrency: 1,
            queue_timeout_ms: 5000,
            call_timeout_ms: 5000,
            disk_budget: None,
        },
    )
    .unwrap();
    let id = queue
        .submit(candidate(9), fixture::request())
        .await
        .unwrap();
    let started = std::time::Instant::now();
    queue.shutdown(10).await.unwrap();
    assert!(started.elapsed() < std::time::Duration::from_millis(500));
    let record = Store::open(&path).unwrap().load_simulation(id).unwrap();
    assert_eq!(record.phase, SimulationPhase::Interrupted);
    assert_eq!(record.outcome, SimulationOutcome::Unknown);
}
