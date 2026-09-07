use crate::types::{ChainPosition, PoolDescriptor};
use alloy_primitives::U256;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolState {
    pub descriptor: PoolDescriptor,
    pub sqrt_price_x96: U256,
    pub tick: i32,
    pub liquidity: u128,
    pub protocol_fee: u32,
    pub tick_bitmap: BTreeMap<i16, U256>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bootstrap {
    pub version: u32,
    pub position: ChainPosition,
    pub pools: Vec<PoolState>,
    /// Exact replies and request parameters, including fixed block tags.
    pub evidence: Vec<Vec<u8>>,
}
impl Bootstrap {
    pub fn validate(&self) -> Result<(), crate::types::RecordError> {
        use crate::types::{Offset, RecordError};
        use std::collections::BTreeSet;
        if self.version != 1
            || self.position.offset != Offset::BlockEnd
            || self.position.block_hash == alloy_primitives::B256::ZERO
            || self.pools.is_empty()
            || self.pools.len() > 128
        {
            return Err(RecordError("invalid bootstrap version or position"));
        }
        let mut ids = BTreeSet::new();
        for state in &self.pools {
            let p = &state.descriptor;
            if p.id.chain_id != self.pools[0].descriptor.id.chain_id
                || !ids.insert(&p.id)
                || p.id.chain_id == 0
                || p.tick_spacing <= 0
                || p.tick_spacing > 32767
                || state.sqrt_price_x96 == U256::ZERO
                || !(-887272..=887272).contains(&state.tick)
                || p.initialized_at.block_number > self.position.block_number
            {
                return Err(RecordError("invalid bootstrap pool"));
            }
            let compressed = state.tick.div_euclid(p.tick_spacing);
            for word in [compressed.div_euclid(256), (compressed + 1).div_euclid(256)] {
                if p.is_quoteable() && !state.tick_bitmap.contains_key(&(word as i16)) {
                    return Err(RecordError("missing bootstrap bitmap"));
                }
            }
        }
        Ok(())
    }
}
pub mod verified;
