use crate::{
    protocol::{Bootstrap, PoolState},
    route::PoolId,
    types::{ChainPosition, Observation, RawRef},
};
use alloy_primitives::B256;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockBatch {
    pub position: ChainPosition,
    pub parent_hash: B256,
    pub covered_pools: Vec<PoolId>,
    pub observations: Vec<Observation>,
    pub raw_refs: Vec<RawRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateView {
    pub position: ChainPosition,
    pub pools: Vec<PoolState>,
    pub raw_refs: Vec<RawRef>,
}
#[derive(Debug, thiserror::Error)]
#[error("invalid state transition: {0}")]
pub struct StateError(pub &'static str);
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    view: StateView,
    last_batch: Option<BlockBatch>,
}
impl State {
    pub fn from_bootstrap(bootstrap: Bootstrap) -> Result<Self, StateError> {
        bootstrap.validate().map_err(|_| StateError("bootstrap"))?;
        Ok(Self {
            view: StateView {
                position: bootstrap.position,
                pools: bootstrap.pools,
                raw_refs: vec![],
            },
            last_batch: None,
        })
    }
    pub fn view(&self) -> StateView {
        self.view.clone()
    }
    pub fn apply_block(&mut self, batch: &BlockBatch) -> Result<StateView, StateError> {
        use crate::types::{ExecutionStatus, Offset, PoolEvent, PoolVerification};
        use alloy_primitives::U256;
        use std::collections::BTreeSet;
        if self.last_batch.as_ref() == Some(batch) {
            return Ok(self.view());
        }
        if batch.position.offset != Offset::BlockEnd
            || self.view.position.block_number.checked_add(1) != Some(batch.position.block_number)
            || batch.parent_hash != self.view.position.block_hash
            || batch.position.block_hash == B256::ZERO
            || batch.raw_refs.is_empty()
        {
            return Err(StateError("incomplete or noncontiguous block"));
        }
        let known = self
            .view
            .pools
            .iter()
            .map(|p| &p.descriptor.id)
            .collect::<BTreeSet<_>>();
        let covered = batch.covered_pools.iter().collect::<BTreeSet<_>>();
        if known != covered || covered.len() != batch.covered_pools.len() {
            return Err(StateError("missing pool coverage"));
        }
        let mut next = self.view.clone();
        let mut previous = None;
        for event in &batch.observations {
            let Offset::Transaction {
                index,
                log_index: Some(log),
            } = event.position.offset
            else {
                return Err(StateError("missing log position"));
            };
            if previous.is_some_and(|(tx, last_log)| index < tx || log <= last_log)
                || event.position.block_number != batch.position.block_number
                || event.position.block_hash != batch.position.block_hash
                || event.execution_status != ExecutionStatus::Succeeded
                || event.transaction_hash == B256::ZERO
                || !batch.raw_refs.contains(&event.raw_ref)
            {
                return Err(StateError("event order, identity or provenance"));
            }
            previous = Some((index, log));
            let pool = next
                .pools
                .iter_mut()
                .find(|p| p.descriptor.id == event.pool)
                .ok_or(StateError("unknown pool"))?;
            match &event.event {
                PoolEvent::ProtocolFeeUpdated { fee } => {
                    if fee & 0xfff > 1000 || fee >> 12 > 1000 {
                        return Err(StateError("protocol fee range"));
                    }
                    pool.protocol_fee = *fee;
                    if *fee != 0 {
                        pool.descriptor.verification =
                            PoolVerification::Unsupported("nonzero protocol fee".into());
                    }
                }
                PoolEvent::Initialized { .. } => return Err(StateError("pool initialized twice")),
                PoolEvent::Swap {
                    sqrt_price_x96,
                    liquidity,
                    tick,
                    lp_fee,
                    ..
                } => {
                    if *sqrt_price_x96 == U256::ZERO
                        || *sqrt_price_x96 >= (U256::from(1) << 160)
                        || !(-887272..=887272).contains(tick)
                        || *lp_fee > 1_000_000
                    {
                        return Err(StateError("invalid swap state"));
                    }
                    pool.sqrt_price_x96 = *sqrt_price_x96;
                    pool.liquidity = *liquidity;
                    pool.tick = *tick;
                    pool.descriptor.lp_fee = *lp_fee;
                    if *lp_fee != 0 {
                        pool.descriptor.verification =
                            PoolVerification::Unsupported("nonzero pool fee".into());
                    }
                    let compressed = tick.div_euclid(pool.descriptor.tick_spacing);
                    if [compressed.div_euclid(256), (compressed + 1).div_euclid(256)]
                        .iter()
                        .any(|i| !pool.tick_bitmap.contains_key(&(*i as i16)))
                    {
                        pool.descriptor.verification = PoolVerification::Pending;
                    }
                }
                PoolEvent::LiquidityChanged {
                    lower,
                    upper,
                    delta,
                } => {
                    let spacing = pool.descriptor.tick_spacing;
                    if lower >= upper
                        || *lower < -887272
                        || *upper > 887272
                        || lower % spacing != 0
                        || upper % spacing != 0
                        || delta.unsigned_abs() > U256::from(u128::MAX)
                    {
                        return Err(StateError("liquidity range or delta"));
                    }
                    if *lower <= pool.tick && pool.tick < *upper {
                        let amount = delta.unsigned_abs().to::<u128>();
                        pool.liquidity = if delta.is_negative() {
                            pool.liquidity.checked_sub(amount)
                        } else {
                            pool.liquidity.checked_add(amount)
                        }
                        .ok_or(StateError("liquidity overflow or underflow"))?;
                    }
                    if !delta.is_zero() {
                        // ponytail: no per-tick gross liquidity cache; bootstrap again to prove bitmap after liquidity edits.
                        pool.tick_bitmap.clear();
                        pool.descriptor.verification = PoolVerification::Pending;
                    }
                }
                PoolEvent::HookFee { currency, .. } => {
                    if *currency != pool.descriptor.currency0
                        && *currency != pool.descriptor.currency1
                    {
                        return Err(StateError("fee currency"));
                    }
                }
            }
        }
        next.position = batch.position.clone();
        next.raw_refs = batch.raw_refs.clone();
        self.view = next;
        self.last_batch = Some(batch.clone());
        Ok(self.view())
    }
}

impl State {
    pub fn validate(&self) -> Result<(), crate::types::RecordError> {
        Bootstrap {
            version: 1,
            position: self.view.position.clone(),
            pools: self.view.pools.clone(),
            evidence: vec![],
        }
        .validate()?;
        if self
            .last_batch
            .as_ref()
            .is_some_and(|b| b.position != self.view.position || b.raw_refs != self.view.raw_refs)
        {
            return Err(crate::types::RecordError(
                "checkpoint state provenance mismatch",
            ));
        }
        Ok(())
    }
}
