use alloy_primitives::{Address, B256, U256};
use arb_core::{
    protocol::{Bootstrap, PoolState},
    route::{PoolId, PoolLocator},
    types::*,
};
use std::collections::BTreeMap;
pub fn bootstrap() -> Bootstrap {
    Bootstrap {
        version: 1,
        position: ChainPosition {
            block_number: 1,
            block_hash: B256::repeat_byte(1),
            offset: Offset::BlockEnd,
        },
        evidence: vec![],
        pools: vec![PoolState {
            descriptor: PoolDescriptor {
                id: PoolId {
                    chain_id: 4663,
                    locator: PoolLocator::Singleton {
                        manager: Address::repeat_byte(2),
                        pool_id: B256::repeat_byte(3),
                    },
                },
                protocol: "pons-v2".into(),
                token: Address::repeat_byte(4),
                quote_asset: Address::ZERO,
                currency0: Address::ZERO,
                currency1: Address::repeat_byte(4),
                hook: Address::repeat_byte(5),
                lp_fee: 0,
                tick_spacing: 200,
                hook_fee_bps: Some(100),
                creator_tax_bps: Some(200),
                token_decimals: Some(18),
                quote_decimals: Some(18),
                initialized_at: ChainPosition {
                    block_number: 1,
                    block_hash: B256::repeat_byte(1),
                    offset: Offset::BlockEnd,
                },
                verification: PoolVerification::Supported,
            },
            sqrt_price_x96: U256::from(1) << 96,
            tick: 0,
            liquidity: 1000000,
            protocol_fee: 0,
            tick_bitmap: BTreeMap::from([(0, U256::ZERO)]),
        }],
    }
}
#[allow(dead_code)]
pub fn profitable_bootstrap() -> Bootstrap {
    use alloy_primitives::address;
    use arb_core::protocol::verified::sqrt_at_tick;
    let mut b = bootstrap();
    let p = &mut b.pools[0];
    p.descriptor.protocol = "pons-v2-v4".into();
    p.descriptor.hook = address!("E5e702641Ea86F4ae6cC3cDaeD2B886f976Be044");
    p.descriptor.id.locator = PoolLocator::Singleton {
        manager: address!("8366a39cc670b4001a1121b8f6a443a643e40951"),
        pool_id: B256::repeat_byte(1),
    };
    p.tick = 4000;
    p.sqrt_price_x96 = sqrt_at_tick(p.tick).unwrap();
    let mut second = p.clone();
    second.tick = 1000;
    second.sqrt_price_x96 = sqrt_at_tick(second.tick).unwrap();
    second.descriptor.id.locator = PoolLocator::Singleton {
        manager: address!("8366a39cc670b4001a1121b8f6a443a643e40951"),
        pool_id: B256::repeat_byte(2),
    };
    b.pools.push(second);
    b
}
