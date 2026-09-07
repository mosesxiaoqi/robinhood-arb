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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteCandidates {
    pub routes: Vec<Route>,
    pub excluded_pools: usize,
}
pub fn two_leg_routes(pools: &[crate::types::PoolDescriptor], quote_asset: Address) -> Vec<Route> {
    route_candidates(pools, quote_asset).routes
}
pub fn route_candidates(
    pools: &[crate::types::PoolDescriptor],
    quote_asset: Address,
) -> RouteCandidates {
    use std::collections::{BTreeMap, BTreeSet};
    let mut unique = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    let mut excluded_pools = 0;
    for p in pools {
        if let Some(previous) = unique.insert(&p.id, p)
            && previous != p
        {
            conflicts.insert(&p.id);
        }
    }
    let mut groups = BTreeMap::<(u64, Address), Vec<&crate::types::PoolDescriptor>>::new();
    for (id, p) in unique {
        let address = match id.locator {
            PoolLocator::Contract(a) => a,
            PoolLocator::Singleton { manager, .. } => manager,
        };
        if conflicts.contains(id)
            || !p.is_quoteable()
            || p.id.chain_id == 0
            || address == Address::ZERO
            || p.protocol.is_empty()
            || p.quote_asset != quote_asset
            || p.token == quote_asset
            || !((p.currency0 == p.token && p.currency1 == quote_asset)
                || (p.currency1 == p.token && p.currency0 == quote_asset))
        {
            excluded_pools += 1;
            continue;
        }
        groups.entry((id.chain_id, p.token)).or_default().push(p);
    }
    let mut routes = vec![];
    // ponytail: O(n²) per token group; first phase bounds the registry, add indexing if that ceiling grows.
    for group in groups.values() {
        for first in group {
            for second in group {
                if first.id == second.id {
                    continue;
                }
                routes.push(Route {
                    legs: vec![
                        Leg {
                            pool: first.id.clone(),
                            protocol: first.protocol.clone(),
                            asset_in: quote_asset,
                            asset_out: first.token,
                        },
                        Leg {
                            pool: second.id.clone(),
                            protocol: second.protocol.clone(),
                            asset_in: second.token,
                            asset_out: quote_asset,
                        },
                    ],
                });
            }
        }
    }
    RouteCandidates {
        routes,
        excluded_pools,
    }
}
