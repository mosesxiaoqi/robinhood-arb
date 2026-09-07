#[path = "../../arb-core/tests/support/mod.rs"]
mod support;
use alloy_primitives::{Address, B256, U256};
use arb_core::{opportunity::CostEstimate, research::*, route::two_leg_routes, state::*, types::*};
use arb_research::opportunities::summarize_opportunities;
fn block(n: u64) -> DerivedBlock {
    let mut view = State::from_bootstrap(support::profitable_bootstrap())
        .unwrap()
        .view();
    view.position = ChainPosition {
        block_number: n,
        block_hash: B256::repeat_byte(n as u8),
        offset: Offset::BlockEnd,
    };
    let route = two_leg_routes(
        &view
            .pools
            .iter()
            .map(|p| p.descriptor.clone())
            .collect::<Vec<_>>(),
        Address::ZERO,
    )
    .remove(0);
    let costs = CostEstimate {
        asset: Address::ZERO,
        amount: Some(U256::ZERO),
        conversion: None,
        basis: "fixture".into(),
    };
    let opportunity =
        arb_core::opportunity::evaluate(&view, &route, U256::from(100), &costs).unwrap();
    assert!(!opportunity.net_profit.as_ref().unwrap().negative);
    let c = Candidate {
        id: B256::repeat_byte(n as u8),
        run_id: "test".into(),
        view_id: view.position.block_hash,
        config_hash: B256::repeat_byte(7),
        algorithm_version: "v1".into(),
        raw_refs: vec![],
        detected_at_ms: n,
        opportunity,
        simulation_status: SimulationStatus::NotRun,
        canonical: true,
    };
    DerivedBlock {
        canonical: true,
        time_quality: None,
        timings: vec![],
        run_id: "test".into(),
        view_id: view.position.block_hash,
        batch: BlockBatch {
            position: view.position.clone(),
            parent_hash: B256::repeat_byte((n - 1) as u8),
            covered_pools: view.pools.iter().map(|p| p.descriptor.id.clone()).collect(),
            observations: vec![],
            raw_refs: vec![],
        },
        view,
        candidates: vec![c],
        exclusions: vec![],
        excluded_pools: 1,
    }
}
#[test]
fn gap_splits_opportunity_window() {
    let a = block(2);
    let b = block(3);
    let c = block(5);
    let result = summarize_opportunities(&[a.clone(), b.clone(), c.clone()], &[], 2, 5).unwrap();
    assert_eq!(result.windows.len(), 2);
    assert_eq!(result.windows[0].blocks, 2);
    assert_eq!(result.windows[0].sampled_capacity, U256::from(100));
    assert_eq!(result.windows[0].simulated_capacity, None);
    assert_eq!(result.complete_blocks, 3);
    assert_eq!(result.gap_blocks, 1);
    assert_eq!(result.token_block_samples, 3);
    assert_eq!(result.excluded_pool_samples, 3);
    let mut unknown = b.clone();
    unknown.candidates[0].opportunity.net_profit = None;
    let r = summarize_opportunities(&[a.clone(), unknown, block(4)], &[], 2, 4).unwrap();
    assert_eq!(r.windows.len(), 2);
    let mut orphan = b;
    orphan.canonical = false;
    let r = summarize_opportunities(&[a, orphan, block(4)], &[], 2, 4).unwrap();
    assert_eq!(r.windows.len(), 2);
    assert_eq!(r.orphan_blocks, 1);
    let mut zero = c;
    zero.candidates.clear();
    let r = summarize_opportunities(&[zero], &[], 5, 5).unwrap();
    assert!(r.windows.is_empty());
    assert_eq!(r.token_block_samples, 1);
}

#[test]
fn quote_assets_and_simulation_evidence_remain_separate() {
    use arb_core::simulation::*;
    let mut a = block(2);
    let mut other = a.candidates[0].clone();
    other.id = B256::repeat_byte(77);
    other.opportunity.route.legs[0].asset_in = Address::repeat_byte(9);
    other.opportunity.route.legs[1].asset_out = Address::repeat_byte(9);
    a.candidates.push(other);
    let c = &a.candidates[0];
    let pool = SimulationPool {
        currency0: Address::ZERO,
        currency1: Address::repeat_byte(4),
        fee: 0,
        tick_spacing: 200,
        hook: Address::repeat_byte(5),
    };
    let request = SimulationRequest {
        position: c.opportunity.position.clone(),
        route: c.opportunity.route.clone(),
        pools: [pool.clone(), pool],
        amount: c.opportunity.amount_in,
        funding_balance: U256::from(1000),
        fail_second: false,
    };
    // Association fixture only; real RPC success/rollback is verified in T23/T24.
    let result = SimulationResult {
        request: request.clone(),
        actual_position: request.position.clone(),
        simulated_block_hash: B256::repeat_byte(8),
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
    };
    let mut simulation = SimulationRecord {
        id: B256::repeat_byte(10),
        queue_id: "test".into(),
        candidate_id: c.id,
        run_id: c.run_id.clone(),
        view_id: c.view_id,
        expected_position: c.opportunity.position.clone(),
        request,
        phase: SimulationPhase::Finished,
        outcome: SimulationOutcome::Succeeded,
        submitted_at_ms: 1,
        queued_at_ms: Some(1),
        started_at_ms: Some(1),
        ended_at_ms: Some(2),
        queue_wait_ns: Some(1),
        elapsed_ns: Some(1),
        result: Some(result),
        error: None,
        error_evidence: vec![],
        canonical: true,
        validates_original_candidate: true,
    };
    let r = summarize_opportunities(&[a.clone()], &[simulation.clone()], 2, 2).unwrap();
    assert_eq!(r.windows.len(), 2);
    assert_eq!(
        r.windows
            .iter()
            .filter(|w| w.simulated_capacity == Some(U256::from(100)))
            .count(),
        1
    );
    simulation.canonical = false;
    let r = summarize_opportunities(&[a], &[simulation], 2, 2).unwrap();
    assert!(r.windows.iter().all(|w| w.simulated_capacity.is_none()));
}
