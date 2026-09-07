#[path = "../../arb-core/tests/support/mod.rs"]
mod support;
use alloy_primitives::{Address, B256, U256};
use arb_adapters::store::Store;
use arb_app::pipeline::Pipeline;
use arb_core::{opportunity::CostEstimate, research::*, state::*, types::*};
#[test]
fn persist_candidate_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipeline.db");
    let mut store = Store::open(&path).unwrap();
    let state = State::from_bootstrap(support::profitable_bootstrap()).unwrap();
    let before = state.view();
    let position = ChainPosition {
        block_number: 2,
        block_hash: B256::repeat_byte(2),
        offset: Offset::BlockEnd,
    };
    let raw = RawRecord {
        version: 1,
        chain_id: 4663,
        source: "rpc".into(),
        run_id: "raw-run".into(),
        sequence: 1,
        received_at_ms: 10,
        kind: "block".into(),
        position: Some(position.clone()),
        transaction_hash: None,
        execution_status: ExecutionStatus::Unknown,
        confirmation: Confirmation::Included,
        payload: b"{}".to_vec(),
    };
    store
        .append_raw(
            &[raw],
            &SourceCursor {
                chain_id: 4663,
                source: "rpc".into(),
                next_block: 3,
                last_block_hash: Some(position.block_hash),
            },
        )
        .unwrap();
    let run = RunSpec {
        run_id: "research-1".into(),
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
            basis: "test estimate".into(),
        },
    };
    let batch = BlockBatch {
        position,
        parent_hash: before.position.block_hash,
        covered_pools: before
            .pools
            .iter()
            .map(|p| p.descriptor.id.clone())
            .collect(),
        observations: vec![],
        raw_refs: vec![RawRef {
            source: "rpc".into(),
            run_id: "raw-run".into(),
            sequence: 1,
        }],
    };
    let mut pipeline = Pipeline::new(store, state, run.clone()).unwrap();
    let mut invalid = batch.clone();
    invalid.covered_pools.clear();
    assert!(pipeline.process(invalid, 10).is_err());
    assert_eq!(pipeline.view(), before);
    let result = pipeline.process(batch.clone(), 10).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(pipeline.process(batch.clone(), 20).unwrap(), result);
    let loaded = pipeline
        .store()
        .find_derived(&run.run_id, batch.position.block_hash)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.candidates[0].view_id, loaded.view_id);
    assert_eq!(loaded.candidates[0].raw_refs, batch.raw_refs);
    assert_eq!(loaded.candidates[0].detected_at_ms, 10);
    assert_eq!(loaded.candidates[0].config_hash, run.config_hash);
    assert_eq!(
        loaded.candidates[0].simulation_status,
        SimulationStatus::NotRun
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("CREATE TRIGGER reject_derived BEFORE INSERT ON derived_blocks BEGIN SELECT RAISE(ABORT,'failure'); END;").unwrap();
    let saved = pipeline.view();
    let mut next = batch;
    next.parent_hash = saved.position.block_hash;
    next.position.block_number = 3;
    next.position.block_hash = B256::repeat_byte(3);
    let mut raw = pipeline
        .store()
        .read_raw_after(0, 1)
        .unwrap()
        .remove(0)
        .record;
    raw.position = Some(next.position.clone());
    raw.sequence = 2;
    Store::open(&path)
        .unwrap()
        .append_raw(
            &[raw],
            &SourceCursor {
                chain_id: 4663,
                source: "rpc".into(),
                next_block: 4,
                last_block_hash: Some(next.position.block_hash),
            },
        )
        .unwrap();
    next.raw_refs[0].sequence = 2;
    assert!(
        pipeline
            .process(next, 30)
            .unwrap_err()
            .to_string()
            .contains("failure")
    );
    let mut collision = run;
    collision.min_profit = U256::from(999);
    assert!(
        Pipeline::new(
            Store::open(&path).unwrap(),
            State::from_bootstrap(support::profitable_bootstrap()).unwrap(),
            collision
        )
        .is_err()
    );
    assert_eq!(pipeline.view(), saved);
}
