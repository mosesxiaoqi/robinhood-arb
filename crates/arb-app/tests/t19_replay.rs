#[path = "../../arb-core/tests/support/mod.rs"]
mod core_support;
#[path = "support/replay.rs"]
mod support;
use alloy_primitives::{Address, B256, U256};
use arb_adapters::{assemble::assemble_block, store::Store};
use arb_app::{pipeline::Pipeline, replay::replay_chain};
use arb_core::{opportunity::CostEstimate, research::RunSpec, state::State, types::SourceCursor};
#[test]
fn replay_matches_live_core_results() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replay.db");
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
            basis: "test estimate".into(),
        },
    };
    for number in 2..=4 {
        store
            .append_raw(
                &support::block(number, B256::repeat_byte((number - 1) as u8)),
                &SourceCursor {
                    chain_id: 4663,
                    source: "rpc".into(),
                    next_block: number + 1,
                    last_block_hash: Some(B256::repeat_byte(number as u8)),
                },
            )
            .unwrap();
    }
    let mut live = Pipeline::new(store, state.clone(), run.clone()).unwrap();
    let mut expected = vec![];
    for number in 2..=4 {
        let batch = assemble_block(
            &support::block(number, B256::repeat_byte((number - 1) as u8)),
            &pools,
        )
        .unwrap();
        expected.extend(live.process(batch, number * 100 + 2).unwrap());
    }
    let checkpoint = arb_core::checkpoint::Checkpoint {
        research_run_id: Some(run.run_id.clone()),
        version: 1,
        state: state.clone(),
        registry_version: run.registry_version,
        config_hash: run.config_hash,
        algorithm_version: run.algorithm_version.clone(),
        processing_cursor: arb_core::checkpoint::ProcessingCursor {
            last_raw_id: 0,
            next_block: 2,
        },
    };
    let mut reopened = Store::open(&path).unwrap();
    let checkpoint_id = reopened.save_checkpoint(&checkpoint).unwrap();
    let restored = reopened.load_checkpoint(checkpoint_id).unwrap();
    let mut replay_run = run;
    replay_run.run_id = "replay".into();
    let mut replay =
        Pipeline::new(Store::open(&path).unwrap(), restored.state, replay_run).unwrap();
    let actual = replay_chain(&mut replay, 4).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(replay.view(), live.view());
    assert!(replay_chain(&mut replay, 5).is_err());
    let mut partial = support::block(5, B256::repeat_byte(4));
    partial.pop();
    assert!(assemble_block(&partial, &pools).is_err());
}
