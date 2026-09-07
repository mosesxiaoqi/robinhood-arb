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
