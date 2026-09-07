use alloy_primitives::{Address, U256, address};
use arb_core::{route::*, simulation::*, types::*};
use serde_json::Value;
pub fn request() -> SimulationRequest {
    let expected: Value = serde_json::from_str(include_str!(
        "../../../../tests/data/verified/atomic-expected.json"
    ))
    .unwrap();
    let token = address!("fdf57d8fd26beb04275148c4e2c5c24b39ff10a6");
    let ids = expected["pool_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|id| PoolId {
            chain_id: 4663,
            locator: PoolLocator::Singleton {
                manager: arb_adapters::discovery::MANAGER,
                pool_id: id.as_str().unwrap().parse().unwrap(),
            },
        })
        .collect::<Vec<_>>();
    SimulationRequest {
        position: ChainPosition {
            block_number: expected["base_position"]["number"].as_u64().unwrap(),
            block_hash: expected["base_position"]["hash"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
            offset: Offset::BlockEnd,
        },
        route: Route {
            legs: vec![
                Leg {
                    pool: ids[0].clone(),
                    protocol: "pons-v2-v4".into(),
                    asset_in: Address::ZERO,
                    asset_out: token,
                },
                Leg {
                    pool: ids[1].clone(),
                    protocol: "uniswap-v4".into(),
                    asset_in: token,
                    asset_out: Address::ZERO,
                },
            ],
        },
        pools: [
            SimulationPool {
                currency0: Address::ZERO,
                currency1: token,
                fee: 0,
                tick_spacing: 200,
                hook: arb_adapters::discovery::HOOK,
            },
            SimulationPool {
                currency0: Address::ZERO,
                currency1: token,
                fee: 853875,
                tick_spacing: 1,
                hook: Address::ZERO,
            },
        ],
        amount: U256::from(1000000000),
        funding_balance: U256::from(1000000000000000000u64),
        fail_second: false,
    }
}
