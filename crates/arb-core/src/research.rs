use crate::{
    opportunity::{CostEstimate, Exclusion, Opportunity},
    route::Route,
    state::{BlockBatch, StateView},
    types::{RawRef, RecordError},
};
use alloy_primitives::{Address, B256, U256};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSpec {
    pub run_id: String,
    pub config_hash: B256,
    pub algorithm_version: String,
    pub registry_version: u64,
    pub quote_asset: Address,
    pub amounts: Vec<U256>,
    pub min_depth: U256,
    pub min_profit: U256,
    pub costs: CostEstimate,
}
impl RunSpec {
    pub fn validate(&self) -> Result<(), RecordError> {
        if self.run_id.is_empty()
            || self.run_id.len() > 128
            || self.run_id.chars().any(char::is_control)
            || self.algorithm_version.is_empty()
            || self.algorithm_version.len() > 128
            || self.config_hash == B256::ZERO
            || self.registry_version == 0
            || self.amounts.is_empty()
            || self.amounts.len() > 1000
            || self.amounts.iter().any(U256::is_zero)
            || self
                .amounts
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.amounts.len()
        {
            return Err(RecordError("invalid research run"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SimulationStatus {
    NotRun,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub id: B256,
    pub run_id: String,
    pub view_id: B256,
    pub config_hash: B256,
    pub algorithm_version: String,
    pub raw_refs: Vec<RawRef>,
    pub detected_at_ms: u64,
    pub opportunity: Opportunity,
    pub simulation_status: SimulationStatus,
    pub canonical: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteExclusions {
    pub route: Route,
    pub entries: Vec<Exclusion>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedBlock {
    pub run_id: String,
    pub view_id: B256,
    pub batch: BlockBatch,
    pub view: StateView,
    pub candidates: Vec<Candidate>,
    pub exclusions: Vec<RouteExclusions>,
    pub excluded_pools: usize,
}
