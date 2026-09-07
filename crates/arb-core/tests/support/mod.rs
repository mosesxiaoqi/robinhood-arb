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
