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
