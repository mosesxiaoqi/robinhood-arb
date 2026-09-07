use alloy_primitives::{Address, B256};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PoolLocator {
    Contract(Address),
    Singleton { manager: Address, pool_id: B256 },
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PoolId {
    pub chain_id: u64,
    pub locator: PoolLocator,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leg {
    pub pool: PoolId,
    pub protocol: String,
    pub asset_in: Address,
    pub asset_out: Address,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub legs: Vec<Leg>,
}
#[derive(Debug, thiserror::Error)]
#[error("invalid route: {0}")]
pub struct RouteError(pub &'static str);
impl Route {
    pub fn validate(&self) -> Result<(), RouteError> {
        if self.legs.len() < 2 {
            return Err(RouteError("at least two legs required"));
        }
        let chain = self.legs[0].pool.chain_id;
        for (i, leg) in self.legs.iter().enumerate() {
            if chain == 0 || leg.pool.chain_id != chain {
                return Err(RouteError("network mismatch"));
            }
            if leg.protocol.is_empty() || leg.asset_in == leg.asset_out {
                return Err(RouteError("invalid leg"));
            }
            let address = match leg.pool.locator {
                PoolLocator::Contract(a) => a,
                PoolLocator::Singleton { manager, .. } => manager,
            };
            if address == Address::ZERO {
                return Err(RouteError("zero pool contract"));
            }
            if leg.asset_out != self.legs[(i + 1) % self.legs.len()].asset_in {
                return Err(RouteError("assets do not form closed route"));
            }
        }
        Ok(())
    }
}
