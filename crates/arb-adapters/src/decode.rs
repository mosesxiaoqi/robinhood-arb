use crate::discovery::{DiscoveryError, abi::Initialize, receipt_logs};
use alloy_primitives::U256;
use alloy_sol_types::{SolEvent, sol};
use arb_core::{
    route::PoolLocator,
    types::{ExecutionStatus, Observation, PoolDescriptor, PoolEvent, RawRecord, RawRef},
};
sol! {
 event Swap(bytes32 indexed id,address indexed sender,int128 amount0,int128 amount1,uint160 sqrtPriceX96,uint128 liquidity,int24 tick,uint24 fee);
 event ModifyLiquidity(bytes32 indexed id,address indexed sender,int24 tickLower,int24 tickUpper,int256 liquidityDelta,bytes32 salt);
 event HookFeeCollected(bytes32 indexed poolId,address currency,uint256 feeAmount,uint256 taxAmount);
}
pub fn decode(raw: &RawRecord, pool: &PoolDescriptor) -> Result<Vec<Observation>, DiscoveryError> {
    if raw.chain_id != pool.id.chain_id {
        return Err(DiscoveryError("network mismatch"));
    }
    let PoolLocator::Singleton { manager, pool_id } = pool.id.locator else {
        return Err(DiscoveryError("unsupported pool protocol"));
    };
    let mut result = Vec::new();
    for log in receipt_logs(raw)? {
        if log.topics.get(1) != Some(&pool_id) {
            continue;
        }
        let topic = log.topics.first().copied();
        let event = if log.address == manager && topic == Some(Initialize::SIGNATURE_HASH) {
            let event: Initialize = log.event()?;
            PoolEvent::Initialized {
                sqrt_price_x96: U256::from(event.sqrtPriceX96),
                tick: event.tick.as_i32(),
            }
        } else if log.address == manager && topic == Some(Swap::SIGNATURE_HASH) {
            let event: Swap = log.event()?;
            PoolEvent::Swap {
                amount0: event.amount0,
                amount1: event.amount1,
                sqrt_price_x96: U256::from(event.sqrtPriceX96),
                liquidity: event.liquidity,
                tick: event.tick.as_i32(),
                lp_fee: event.fee.to(),
            }
        } else if log.address == manager && topic == Some(ModifyLiquidity::SIGNATURE_HASH) {
            let event: ModifyLiquidity = log.event()?;
            PoolEvent::LiquidityChanged {
                lower: event.tickLower.as_i32(),
                upper: event.tickUpper.as_i32(),
                delta: event.liquidityDelta,
            }
        } else if log.address == pool.hook && topic == Some(HookFeeCollected::SIGNATURE_HASH) {
            let event: HookFeeCollected = log.event()?;
            PoolEvent::HookFee {
                currency: event.currency,
                fee: event.feeAmount,
                tax: event.taxAmount,
            }
        } else {
            continue;
        };
        result.push(Observation {
            raw_ref: RawRef {
                source: raw.source.clone(),
                run_id: raw.run_id.clone(),
                sequence: raw.sequence,
            },
            pool: pool.id.clone(),
            position: log.position(),
            transaction_hash: log.transaction_hash,
            execution_status: ExecutionStatus::Succeeded,
            event,
        });
    }
    Ok(result)
}
