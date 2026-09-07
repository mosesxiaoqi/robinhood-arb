use crate::{route::Route, types::ChainPosition};
use alloy_primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationPool {
    pub currency0: Address,
    pub currency1: Address,
    pub fee: u32,
    pub tick_spacing: i32,
    pub hook: Address,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationRequest {
    pub position: ChainPosition,
    pub route: Route,
    pub pools: [SimulationPool; 2],
    pub amount: U256,
    pub funding_balance: U256,
    pub fail_second: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimulationOutcome {
    Succeeded,
    Reverted,
    Unavailable,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetChange {
    pub account: Address,
    pub asset: Address,
    pub before: U256,
    pub after: U256,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationResult {
    pub request: SimulationRequest,
    pub actual_position: ChainPosition,
    pub simulated_block_hash: B256,
    pub outcome: SimulationOutcome,
    pub middle_amount: Option<U256>,
    pub amount_out: Option<U256>,
    pub gas_used: u64,
    pub asset_changes: Vec<AssetChange>,
    pub pool_slots_before: [B256; 2],
    pub pool_slots_after: [B256; 2],
    pub state_rolled_back: bool,
    pub error: Option<String>,
    pub evidence: Vec<Vec<u8>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimulationPhase {
    Queued,
    Running,
    Finished,
    QueueFull,
    QueueExpired,
    Interrupted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationRecord {
    pub id: B256,
    pub queue_id: String,
    pub candidate_id: B256,
    pub run_id: String,
    pub view_id: B256,
    pub expected_position: ChainPosition,
    pub request: SimulationRequest,
    pub phase: SimulationPhase,
    pub outcome: SimulationOutcome,
    pub submitted_at_ms: u64,
    pub queued_at_ms: Option<u64>,
    pub started_at_ms: Option<u64>,
    pub ended_at_ms: Option<u64>,
    pub queue_wait_ns: Option<u64>,
    pub elapsed_ns: Option<u64>,
    pub result: Option<SimulationResult>,
    pub error: Option<String>,
    pub error_evidence: Vec<Vec<u8>>,
    pub canonical: bool,
    pub validates_original_candidate: bool,
}
pub fn validates_candidate(
    candidate: &crate::research::Candidate,
    result: &SimulationResult,
) -> bool {
    candidate.canonical
        && result.outcome == SimulationOutcome::Succeeded
        && !result.request.fail_second
        && result.actual_position == candidate.opportunity.position
        && result.request.position == candidate.opportunity.position
        && result.request.route == candidate.opportunity.route
        && result.request.amount == candidate.opportunity.amount_in
        && result.amount_out == Some(candidate.opportunity.amount_out)
}
