#[path = "../../arb-core/tests/support/mod.rs"]
mod core_support;
#[path = "support/replay.rs"]
mod support;
use alloy_primitives::{Address, B256, U256};
use arb_adapters::{assemble::assemble_block, store::Store};
use arb_app::{pipeline::Pipeline, recovery::RecoveryPlan};
use arb_core::{
    checkpoint::*, opportunity::CostEstimate, research::RunSpec, state::State, types::*,
};
#[test]
fn reorg_matches_clean_branch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fork.db");
    let mut store = Store::open(&path).unwrap();
    let state = State::from_bootstrap(core_support::profitable_bootstrap()).unwrap();
    let pools = state
        .view()
        .pools
        .iter()
        .map(|p| p.descriptor.clone())
        .collect::<Vec<_>>();
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
    let checkpoint = Checkpoint {
        version: 1,
        research_run_id: Some(run.run_id.clone()),
        state: state.clone(),
        registry_version: 1,
        config_hash: run.config_hash,
        algorithm_version: run.algorithm_version.clone(),
        processing_cursor: ProcessingCursor {
            last_raw_id: 0,
            next_block: 2,
        },
    };
    store.save_checkpoint(&checkpoint).unwrap();
    let a2 = support::block(2, B256::repeat_byte(1));
    store
        .append_raw(
            &a2,
            &SourceCursor {
                chain_id: 4663,
                source: "rpc".into(),
                next_block: 3,
                last_block_hash: Some(B256::repeat_byte(2)),
            },
        )
        .unwrap();
    let mut live = Pipeline::new(store, state.clone(), run.clone()).unwrap();
    live.process_records(&a2, 202).unwrap();
    let mut b2 = support::block(2, B256::repeat_byte(1));
    for raw in &mut b2 {
        raw.run_id = "fork".into();
        raw.position.as_mut().unwrap().block_hash = B256::repeat_byte(22);
        raw.payload = String::from_utf8(raw.payload.clone())
            .unwrap()
            .replace(
                &B256::repeat_byte(2).to_string(),
                &B256::repeat_byte(22).to_string(),
            )
            .into_bytes();
    }
    let mut b3 = support::block(3, B256::repeat_byte(22));
    for raw in &mut b3 {
        raw.run_id = "fork".into();
    }
    let mut writer = Store::open(&path).unwrap();
    for records in [&b2, &b3] {
        let at = records[0].position.as_ref().unwrap();
        writer
            .append_raw(
                records,
                &SourceCursor {
                    chain_id: 4663,
                    source: "rpc".into(),
                    next_block: at.block_number + 1,
                    last_block_hash: Some(at.block_hash),
                },
            )
            .unwrap();
    }
    let replacement = vec![
        assemble_block(&b2, &pools).unwrap(),
        assemble_block(&b3, &pools).unwrap(),
    ];
    let plan = RecoveryPlan::build(&live, replacement.clone()).unwrap();
    assert_eq!(plan.common_ancestor.block_number, 1);
    assert_eq!(plan.orphan_hashes, vec![B256::repeat_byte(2)]);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("CREATE TRIGGER interrupt_recovery BEFORE INSERT ON derived_blocks WHEN NEW.block_number='3' BEGIN SELECT RAISE(ABORT,'interrupted recovery'); END;").unwrap();
    assert!(
        plan.execute(&mut live)
            .unwrap_err()
            .to_string()
            .contains("interrupted recovery")
    );
    assert!(live.is_paused());
    assert!(live.store().has_pending_recovery("live").unwrap());
    drop(live);
    let mut live = Pipeline::new(Store::open(&path).unwrap(), state.clone(), run.clone()).unwrap();
    assert!(live.is_paused());
    assert!(live.process_records(&b3, 300).is_err());
    let resumed = RecoveryPlan::load(&live, plan.id).unwrap();
    assert_eq!(resumed, plan);
    sql.execute_batch("DROP TRIGGER interrupt_recovery")
        .unwrap();
    let recovered = resumed.execute(&mut live).unwrap();
    assert!(!live.is_paused());
    let mut clean_run = run;
    clean_run.run_id = "clean".into();
    let mut clean = Pipeline::new(Store::open(&path).unwrap(), state, clean_run).unwrap();
    for batch in replacement {
        clean.process(batch, 300).unwrap();
    }
    assert_eq!(recovered, clean.view());
    assert_eq!(plan.execute(&mut live).unwrap(), recovered);
    let orphan = live
        .store()
        .find_derived("live", B256::repeat_byte(2))
        .unwrap()
        .unwrap();
    assert!(!orphan.candidates[0].canonical);
    assert!(!orphan.canonical);
}
