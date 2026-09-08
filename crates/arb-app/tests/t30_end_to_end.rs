#[path = "../../arb-core/tests/support/mod.rs"]
mod core_support;
#[path = "../../arb-adapters/tests/support/mod.rs"]
#[allow(dead_code)]
mod http;
#[path = "support/replay.rs"]
mod raw;
use alloy_primitives::{B256, U256};
use arb_adapters::{
    rpc::{RpcOptions, RpcSource},
    store::Store,
};
use arb_app::{
    config::Config, ingest::collect, pipeline::Pipeline, replay::replay_chain,
    report::export_report, runtime::*, simulation_queue::*,
};
use arb_core::{simulation::*, types::*};
use serde_json::{Value, json};
use std::sync::Arc;
fn responses(records: &[RawRecord]) -> Vec<(u16, Value)> {
    let values = records
        .iter()
        .map(|r| serde_json::from_slice::<Value>(&r.payload).unwrap()["result"].clone())
        .collect::<Vec<_>>();
    vec![
        json!("0x1237"),
        values[0].clone(),
        values[1].clone(),
        values[2].clone(),
        values[0].clone(),
    ]
    .into_iter()
    .map(|v| (200, v))
    .collect()
}
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
async fn collect_replay_report_roundtrip() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(
        root.join("docs/operations.md").exists(),
        "operations delivery missing"
    );
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("live.db");
    let mut config = Config::parse(include_str!("../../../config/example.toml")).unwrap();
    config.database = database.clone();
    config.run_id = Some("e2e".into());
    config.amounts = vec!["100".into()];
    config.min_depth = "0".into();
    config.min_profit = "1".into();
    config.additional_cost = Some("0".into());
    let mut engine = RuntimeEngine::open(&config).unwrap();
    // Synthetic pool state tests orchestration; actual public simulation evidence is separately verified in T22/T23.
    engine
        .initialize(core_support::profitable_bootstrap())
        .unwrap();
    let a2 = raw::block(2, B256::repeat_byte(1));
    let server = http::serve(responses(&a2)).await;
    let source = RpcSource::new(&server.url, options("e2e-collect")).unwrap();
    assert_eq!(collect(&source, &database, 2, 2, 1).await.unwrap(), 1);
    assert!(engine.replay_committed().unwrap());
    let checkpoint = Store::open(&database)
        .unwrap()
        .runtime_checkpoint("e2e")
        .unwrap()
        .unwrap();
    let block = engine
        .pipeline
        .as_ref()
        .unwrap()
        .store()
        .find_derived("e2e", B256::repeat_byte(2))
        .unwrap()
        .unwrap();
    let candidate = block.candidates[0].clone();
    let pools = block
        .view
        .pools
        .iter()
        .map(|p| SimulationPool {
            currency0: p.descriptor.currency0,
            currency1: p.descriptor.currency1,
            fee: p.descriptor.lp_fee,
            tick_spacing: p.descriptor.tick_spacing,
            hook: p.descriptor.hook,
        })
        .collect::<Vec<_>>();
    let request = SimulationRequest {
        position: candidate.opportunity.position.clone(),
        route: candidate.opportunity.route.clone(),
        pools: pools.try_into().unwrap(),
        amount: candidate.opportunity.amount_in,
        funding_balance: U256::from(100),
        fail_second: false,
    };
    let queue = SimulationQueue::new(
        Arc::new(source.simulation_lane()),
        database.clone(),
        QueueOptions {
            queue_id: "e2e-sim".into(),
            capacity: 1,
            concurrency: 1,
            queue_timeout_ms: 1000,
            call_timeout_ms: 1000,
            disk_budget: Some(engine.budget.clone()),
        },
    )
    .unwrap();
    let job = queue.submit(candidate.clone(), request).await.unwrap();
    queue.shutdown(1500).await.unwrap();
    let result = Store::open(&database)
        .unwrap()
        .load_simulation(job)
        .unwrap();
    assert_eq!(result.candidate_id, candidate.id);
    assert!(result.canonical);
    assert_eq!(result.outcome, SimulationOutcome::Unavailable);
    assert!(!result.validates_original_candidate);
    drop(engine);
    let mut engine = RuntimeEngine::open(&config).unwrap();
    assert_eq!(
        engine.pipeline.as_ref().unwrap().view(),
        checkpoint.state.view()
    );
    assert_eq!(engine.next_collect().unwrap(), Some(3));
    let a3 = raw::block(3, B256::repeat_byte(2));
    engine.commit(&a3, false).unwrap();
    let mut b3 = raw::block(3, B256::repeat_byte(2));
    for record in &mut b3 {
        record.run_id = "fork".into();
        record.position.as_mut().unwrap().block_hash = B256::repeat_byte(33);
        record.payload = String::from_utf8(record.payload.clone())
            .unwrap()
            .replace(
                &B256::repeat_byte(3).to_string(),
                &B256::repeat_byte(33).to_string(),
            )
            .into_bytes();
    }
    let b4 = raw::block(4, B256::repeat_byte(33));
    assert_eq!(engine.commit(&b4, false).unwrap(), RunStatus::DataGapPaused);
    let mut script = responses(&b3);
    script.extend(responses(&a2));
    let server = http::serve(script).await;
    let source = RpcSource::new(&server.url, options("e2e-reorg")).unwrap();
    assert_eq!(engine.recover(&source, b4).await.unwrap(), 2);
    let recovered = engine.pipeline.as_ref().unwrap().view();
    assert_eq!(recovered.position.block_number, 4);
    drop(engine);
    let engine = RuntimeEngine::open(&config).unwrap();
    assert_eq!(engine.next_collect().unwrap(), Some(5));
    assert_eq!(engine.pipeline.as_ref().unwrap().view(), recovered);
    let store = Store::open(&database).unwrap();
    assert!(!store.has_pending_recovery("e2e").unwrap());
    assert!(
        !store
            .find_derived("e2e", B256::repeat_byte(3))
            .unwrap()
            .unwrap()
            .canonical
    );
    let mut clean_store = Store::open(&dir.path().join("clean.db")).unwrap();
    for (n, hash) in [(3, B256::repeat_byte(33)), (4, B256::repeat_byte(4))] {
        let records = store
            .read_raw_block(4663, n, 0)
            .unwrap()
            .into_iter()
            .map(|r| r.record)
            .filter(|r| r.position.as_ref().unwrap().block_hash == hash)
            .collect::<Vec<_>>();
        clean_store
            .append_raw(
                &records,
                &SourceCursor {
                    chain_id: 4663,
                    source: "rpc".into(),
                    next_block: n + 1,
                    last_block_hash: Some(hash),
                },
            )
            .unwrap();
    }
    let mut clean_run = engine.run.clone();
    clean_run.run_id = "clean".into();
    let mut clean = Pipeline::new(clean_store, checkpoint.state, clean_run).unwrap();
    replay_chain(&mut clean, 4).unwrap();
    assert_eq!(clean.view(), recovered);
    let out = dir.path().join("report");
    export_report(&database, "e2e", &out, Some((2, 4))).unwrap();
    let markdown = std::fs::read_to_string(out.join("report.md")).unwrap();
    assert!(markdown.contains("只读模拟，非真实成交"));
    assert!(markdown.contains("Unavailable"));
    assert!(markdown.contains("孤块 1"));
    assert_eq!(store.raw_count().unwrap(), 12);
    drop(engine);
    let mut engine = RuntimeEngine::open(&config).unwrap();
    let mut a4 = raw::block(4, B256::repeat_byte(3));
    for r in &mut a4 {
        r.position.as_mut().unwrap().block_hash = B256::repeat_byte(44);
        r.payload = String::from_utf8(r.payload.clone())
            .unwrap()
            .replace(
                &B256::repeat_byte(4).to_string(),
                &B256::repeat_byte(44).to_string(),
            )
            .into_bytes();
    }
    let a5 = raw::block(5, B256::repeat_byte(44));
    assert_eq!(engine.commit(&a5, false).unwrap(), RunStatus::DataGapPaused);
    let mut script = responses(&a4);
    script.extend(responses(&a3));
    script.extend(responses(&a2));
    let server = http::serve(script).await;
    let source = RpcSource::new(&server.url, options("return-to-a")).unwrap();
    engine.recover(&source, a5).await.unwrap();
    assert!(
        store
            .find_derived("e2e", B256::repeat_byte(3))
            .unwrap()
            .unwrap()
            .canonical
    );
    assert!(
        !store
            .find_derived("e2e", B256::repeat_byte(33))
            .unwrap()
            .unwrap()
            .canonical
    );
    assert!(!store.has_pending_recovery("e2e").unwrap());
    drop(engine);
    assert_eq!(
        RuntimeEngine::open(&config)
            .unwrap()
            .next_collect()
            .unwrap(),
        Some(6)
    );
}

#[test]
fn shallow_recovery_uses_recent_checkpoint_in_long_history() {
    use arb_core::{checkpoint::*, research::DerivedBlock, state::BlockBatch};
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::parse(include_str!("../../../config/example.toml")).unwrap();
    config.database = dir.path().join("long.db");
    config.run_id = Some("long".into());
    let engine = RuntimeEngine::open(&config).unwrap();
    let run = engine.run.clone();
    drop(engine);
    let mut state =
        arb_core::state::State::from_bootstrap(core_support::profitable_bootstrap()).unwrap();
    let mut sql = rusqlite::Connection::open(&config.database).unwrap();
    let tx = sql.transaction().unwrap();
    let mut recent = None;
    for n in 2..=4104u64 {
        let before = state.view();
        let hash = B256::from(U256::from(n).to_be_bytes::<32>());
        let batch = BlockBatch {
            position: ChainPosition {
                block_number: n,
                block_hash: hash,
                offset: Offset::BlockEnd,
            },
            parent_hash: before.position.block_hash,
            covered_pools: before
                .pools
                .iter()
                .map(|p| p.descriptor.id.clone())
                .collect(),
            observations: vec![],
            raw_refs: vec![RawRef {
                source: "fixture".into(),
                run_id: "long".into(),
                sequence: n,
            }],
        };
        let view = state.apply_block(&batch).unwrap();
        let block = DerivedBlock {
            canonical: true,
            time_quality: None,
            timings: vec![],
            run_id: run.run_id.clone(),
            view_id: hash,
            batch,
            view,
            candidates: vec![],
            exclusions: vec![],
            excluded_pools: 0,
        };
        // Synthetic metadata history isolates checkpoint selection; no live-chain evidence claim.
        tx.execute(
            "INSERT INTO derived_blocks(run_id,block_hash,block_number,view_id,parent_hash,excluded_pools,covered_pools_json,observations_json,raw_refs_json,pools_json,view_raw_refs_json,candidates_json,exclusions_json,timings_json) VALUES(?1,?2,?3,?4,?5,0,?6,?7,?8,?9,?10,'[]','[]','[]')",
            rusqlite::params![
                run.run_id,
                hash.to_string(),
                n as i64,
                block.view_id.to_string(),
                block.batch.parent_hash.to_string(),
                serde_json::to_string(&block.batch.covered_pools).unwrap(),
                serde_json::to_string(&block.batch.observations).unwrap(),
                serde_json::to_string(&block.batch.raw_refs).unwrap(),
                serde_json::to_string(&block.view.pools).unwrap(),
                serde_json::to_string(&block.view.raw_refs).unwrap()
            ],
        )
        .unwrap();
        if n == 4103 {
            recent = Some(state.clone());
        }
    }
    tx.commit().unwrap();
    let mut store = Store::open(&config.database).unwrap();
    store
        .save_checkpoint(&Checkpoint {
            version: 1,
            research_run_id: Some(run.run_id.clone()),
            state: recent.unwrap(),
            registry_version: run.registry_version,
            config_hash: run.config_hash,
            algorithm_version: run.algorithm_version.clone(),
            processing_cursor: ProcessingCursor {
                last_raw_id: 0,
                next_block: 4104,
            },
        })
        .unwrap();
    let pipeline = Pipeline::new(store, state, run).unwrap();
    let view = pipeline.view();
    let batch = BlockBatch {
        position: ChainPosition {
            block_number: 4104,
            block_hash: B256::repeat_byte(99),
            offset: Offset::BlockEnd,
        },
        parent_hash: B256::from(U256::from(4103).to_be_bytes::<32>()),
        covered_pools: view.pools.iter().map(|p| p.descriptor.id.clone()).collect(),
        observations: vec![],
        raw_refs: vec![RawRef {
            source: "fixture".into(),
            run_id: "replacement".into(),
            sequence: 1,
        }],
    };
    let plan = arb_app::recovery::RecoveryPlan::build(&pipeline, vec![batch]).unwrap();
    assert_eq!(plan.common_ancestor.block_number, 4103);
    assert_eq!(plan.replay_blocks.len(), 1);
}
