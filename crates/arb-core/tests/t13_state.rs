mod support;
use alloy_primitives::{B256, U256};
use arb_core::{state::*, types::*};
#[test]
fn publish_only_complete_block() {
    let mut state = State::from_bootstrap(support::bootstrap()).unwrap();
    let old = state.view();
    let saved = old.clone();
    let id = old.pools[0].descriptor.id.clone();
    let reference = RawRef {
        source: "rpc".into(),
        run_id: "test".into(),
        sequence: 7,
    };
    let mut batch = BlockBatch {
        position: ChainPosition {
            block_number: 2,
            block_hash: B256::repeat_byte(2),
            offset: Offset::BlockEnd,
        },
        parent_hash: old.position.block_hash,
        covered_pools: vec![id.clone()],
        observations: vec![],
        raw_refs: vec![reference.clone()],
    };
    let mut missing = batch.clone();
    missing.covered_pools.clear();
    assert!(state.apply_block(&missing).is_err());
    assert_eq!(state.view(), old);
    let mut bad = batch.clone();
    bad.parent_hash = B256::ZERO;
    assert!(state.apply_block(&bad).is_err());
    assert_eq!(state.view(), old);
    let observation = Observation {
        raw_ref: reference,
        pool: id,
        position: ChainPosition {
            offset: Offset::Transaction {
                index: 0,
                log_index: Some(1),
            },
            ..batch.position.clone()
        },
        transaction_hash: B256::repeat_byte(9),
        execution_status: ExecutionStatus::Succeeded,
        event: PoolEvent::Swap {
            amount0: -1,
            amount1: 1,
            sqrt_price_x96: (U256::from(1) << 96) + U256::from(1),
            liquidity: 100,
            tick: 0,
            lp_fee: 0,
        },
    };
    batch.observations.push(observation.clone());
    let mut bad = batch.clone();
    let mut second = observation;
    second.position.offset = Offset::Transaction {
        index: 0,
        log_index: Some(2),
    };
    second.event = PoolEvent::Swap {
        amount0: 1,
        amount1: -1,
        sqrt_price_x96: U256::ZERO,
        liquidity: 0,
        tick: 0,
        lp_fee: 0,
    };
    bad.observations.push(second);
    assert!(state.apply_block(&bad).is_err());
    assert_eq!(state.view(), old);
    let view = state.apply_block(&batch).unwrap();
    assert_eq!(view.pools[0].liquidity, 100);
    assert_eq!(state.apply_block(&batch).unwrap(), view);
    assert_eq!(old, saved);
    let empty = BlockBatch {
        position: ChainPosition {
            block_number: 3,
            block_hash: B256::repeat_byte(3),
            offset: Offset::BlockEnd,
        },
        parent_hash: view.position.block_hash,
        covered_pools: batch.covered_pools,
        observations: vec![],
        raw_refs: batch.raw_refs,
    };
    assert_eq!(state.apply_block(&empty).unwrap().pools, view.pools);
}
