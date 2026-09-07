use alloy_primitives::B256;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Offset {
    BlockEnd,
    Transaction { index: u64, log_index: Option<u64> },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainPosition {
    pub block_number: u64,
    pub block_hash: B256,
    pub offset: Offset,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionStatus {
    Unknown,
    Succeeded,
    Reverted,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confirmation {
    Unknown,
    Included,
    Safe,
    Finalized,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRecord {
    pub version: u32,
    pub chain_id: u64,
    pub source: String,
    pub run_id: String,
    pub sequence: u64,
    pub received_at_ms: u64,
    /// Monotonic duration of this successful RPC request, scoped to run_id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_elapsed_ns: Option<u64>,
    pub kind: String,
    pub position: Option<ChainPosition>,
    pub transaction_hash: Option<B256>,
    pub execution_status: ExecutionStatus,
    pub confirmation: Confirmation,
    pub payload: Vec<u8>,
}
#[derive(Debug, thiserror::Error)]
#[error("invalid raw record: {0}")]
pub struct RecordError(pub &'static str);
impl RawRecord {
    pub fn validate(&self) -> Result<(), RecordError> {
        if self.version != 1 {
            return Err(RecordError("unsupported version"));
        }
        if self.chain_id == 0 {
            return Err(RecordError("zero chain id"));
        }
        for value in [&self.source, &self.run_id, &self.kind] {
            if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
                return Err(RecordError("invalid metadata label"));
            }
        }
        if self.payload.len() > 64 * 1024 * 1024 {
            return Err(RecordError("payload exceeds 64 MiB"));
        }
        if self.execution_status != ExecutionStatus::Unknown && self.transaction_hash.is_none() {
            return Err(RecordError("execution status requires transaction hash"));
        }
        if self.confirmation != Confirmation::Unknown && self.position.is_none() {
            return Err(RecordError("confirmation requires chain position"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCursor {
    pub chain_id: u64,
    pub source: String,
    pub next_block: u64,
    pub last_block_hash: Option<B256>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolVerification {
    Pending,
    Supported,
    Unsupported(String),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PoolDescriptor {
    pub id: crate::route::PoolId,
    pub protocol: String,
    pub token: alloy_primitives::Address,
    pub quote_asset: alloy_primitives::Address,
    pub currency0: alloy_primitives::Address,
    pub currency1: alloy_primitives::Address,
    pub hook: alloy_primitives::Address,
    pub lp_fee: u32,
    pub tick_spacing: i32,
    pub hook_fee_bps: Option<u16>,
    pub creator_tax_bps: Option<u16>,
    pub token_decimals: Option<u8>,
    pub quote_decimals: Option<u8>,
    pub initialized_at: ChainPosition,
    pub verification: PoolVerification,
}
impl PoolDescriptor {
    pub fn is_quoteable(&self) -> bool {
        self.verification == PoolVerification::Supported
            && self.token_decimals.is_some()
            && self.quote_decimals.is_some()
            && self.hook_fee_bps.is_some()
            && self.creator_tax_bps.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawRef {
    pub source: String,
    pub run_id: String,
    pub sequence: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolEvent {
    ProtocolFeeUpdated {
        fee: u32,
    },
    Initialized {
        sqrt_price_x96: alloy_primitives::U256,
        tick: i32,
    },
    Swap {
        amount0: i128,
        amount1: i128,
        sqrt_price_x96: alloy_primitives::U256,
        liquidity: u128,
        tick: i32,
        lp_fee: u32,
    },
    LiquidityChanged {
        lower: i32,
        upper: i32,
        delta: alloy_primitives::I256,
    },
    HookFee {
        currency: alloy_primitives::Address,
        fee: alloy_primitives::U256,
        tax: alloy_primitives::U256,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub raw_ref: RawRef,
    pub pool: crate::route::PoolId,
    pub position: ChainPosition,
    pub transaction_hash: B256,
    pub execution_status: ExecutionStatus,
    pub event: PoolEvent,
}
