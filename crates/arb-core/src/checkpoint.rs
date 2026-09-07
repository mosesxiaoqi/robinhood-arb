use crate::{state::State, types::RecordError};
use alloy_primitives::B256;
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessingCursor {
    pub last_raw_id: u64,
    pub next_block: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    #[serde(default)]
    pub research_run_id: Option<String>,
    pub version: u32,
    pub state: State,
    pub registry_version: u64,
    pub config_hash: B256,
    pub algorithm_version: String,
    pub processing_cursor: ProcessingCursor,
}
impl Checkpoint {
    pub fn validate(&self) -> Result<(), RecordError> {
        if self.version != 1
            || self.registry_version == 0
            || self.config_hash == B256::ZERO
            || self.algorithm_version.is_empty()
            || self.algorithm_version.len() > 128
            || self.algorithm_version.chars().any(char::is_control)
            || self.state.view().position.block_number.checked_add(1)
                != Some(self.processing_cursor.next_block)
        {
            return Err(RecordError(
                "invalid checkpoint metadata or processing cursor",
            ));
        }
        self.state.validate()
    }
}
