use crate::pipeline::{Pipeline, PipelineError};
use alloy_primitives::{B256, keccak256};
use arb_core::{
    state::{BlockBatch, StateView},
    types::{ChainPosition, Offset},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryPlan {
    pub id: B256,
    pub run_id: String,
    pub checkpoint_id: u64,
    pub common_ancestor: ChainPosition,
    pub required_fetch: Option<(u64, u64)>,
    pub orphan_hashes: Vec<B256>,
    pub replay_blocks: Vec<BlockBatch>,
}
impl RecoveryPlan {
    pub fn build(pipeline: &Pipeline, replacement: Vec<BlockBatch>) -> Result<Self, PipelineError> {
        if replacement.is_empty() || replacement.len() > 4096 {
            return Err(PipelineError::Invalid("replacement branch size"));
        }
        let first = &replacement[0];
        let ancestor = ChainPosition {
            block_number: first
                .position
                .block_number
                .checked_sub(1)
                .ok_or(PipelineError::Invalid("genesis replacement"))?,
            block_hash: first.parent_hash,
            offset: Offset::BlockEnd,
        };
        let mut path = BTreeMap::new();
        let mut at = pipeline.view().position;
        let mut positions = BTreeMap::from([(at.block_number, at.block_hash)]);
        while let Some(block) = pipeline
            .store
            .find_derived(&pipeline.run.run_id, at.block_hash)?
        {
            if path.len() >= 4096 {
                return Err(PipelineError::Invalid(
                    "recovery lookback exceeds 4096 blocks",
                ));
            }
            let parent = ChainPosition {
                block_number: at
                    .block_number
                    .checked_sub(1)
                    .ok_or(PipelineError::Invalid("derived genesis"))?,
                block_hash: block.batch.parent_hash,
                offset: Offset::BlockEnd,
            };
            if block.view.position != at {
                return Err(PipelineError::Invalid("stored branch identity"));
            }
            path.insert(at.block_number, block.batch);
            at = parent;
            positions.insert(at.block_number, at.block_hash);
        }
        if positions.get(&ancestor.block_number) != Some(&ancestor.block_hash) {
            return Err(PipelineError::Invalid(
                "common ancestor not present; more history required",
            ));
        }
        let mut cursor = i64::MAX as u64;
        let mut selected = None;
        loop {
            let page = pipeline.store.read_checkpoints_before(cursor)?;
            if page.is_empty() {
                break;
            }
            for (id, checkpoint) in page {
                cursor = id;
                let p = checkpoint.state.view().position;
                if p.block_number <= ancestor.block_number
                    && positions.get(&p.block_number) == Some(&p.block_hash)
                    && checkpoint.config_hash == pipeline.run.config_hash
                    && checkpoint.algorithm_version == pipeline.run.algorithm_version
                    && checkpoint.registry_version == pipeline.run.registry_version
                    && checkpoint.research_run_id.as_deref() == Some(&pipeline.run.run_id)
                    && selected.as_ref().is_none_or(
                        |(_, prior): &(u64, arb_core::checkpoint::Checkpoint)| {
                            prior.state.view().position.block_number < p.block_number
                        },
                    )
                {
                    selected = Some((id, checkpoint));
                }
            }
        }
        let (checkpoint_id, checkpoint) = selected.ok_or(PipelineError::Invalid(
            "no common checkpoint; reinitialize at a verified block",
        ))?;
        let base = checkpoint.state.view().position.block_number;
        let mut replay_blocks = path
            .iter()
            .filter(|(number, _)| **number > base && **number <= ancestor.block_number)
            .map(|(_, b)| b.clone())
            .collect::<Vec<_>>();
        let orphan_hashes = path
            .range((ancestor.block_number + 1)..)
            .map(|(_, b)| b.position.block_hash)
            .collect();
        let mut required_fetch = None;
        let mut previous = ancestor.clone();
        for block in &replacement {
            if block.position.block_number <= previous.block_number {
                return Err(PipelineError::Invalid("replacement order"));
            }
            if block.position.block_number != previous.block_number + 1 {
                required_fetch = Some((previous.block_number + 1, block.position.block_number - 1));
                break;
            }
            if block.parent_hash != previous.block_hash {
                return Err(PipelineError::Invalid("replacement parent"));
            }
            previous = block.position.clone();
        }
        replay_blocks.extend(replacement);
        let mut plan = Self {
            id: B256::ZERO,
            run_id: pipeline.run.run_id.clone(),
            checkpoint_id,
            common_ancestor: ancestor,
            required_fetch,
            orphan_hashes,
            replay_blocks,
        };
        plan.id = keccak256(serde_json::to_vec(&plan)?);
        Ok(plan)
    }
    pub fn load(pipeline: &Pipeline, id: B256) -> Result<Self, PipelineError> {
        Ok(serde_json::from_slice(&pipeline.store.recovery_data(id)?)?)
    }
    pub fn execute(&self, pipeline: &mut Pipeline) -> Result<StateView, PipelineError> {
        if self.run_id != pipeline.run.run_id {
            return Err(PipelineError::Invalid("recovery run mismatch"));
        }
        pipeline.paused = true;
        if self.required_fetch.is_some() {
            return Err(PipelineError::Invalid(
                "recovery branch has a missing block range",
            ));
        }
        let mut unsigned = self.clone();
        unsigned.id = B256::ZERO;
        if keccak256(serde_json::to_vec(&unsigned)?) != self.id {
            return Err(PipelineError::Invalid("recovery plan hash"));
        }
        let checkpoint = pipeline.store.load_checkpoint(self.checkpoint_id)?;
        if checkpoint.config_hash != pipeline.run.config_hash
            || checkpoint.algorithm_version != pipeline.run.algorithm_version
            || checkpoint.registry_version != pipeline.run.registry_version
        {
            return Err(PipelineError::Invalid("recovery checkpoint parameters"));
        }
        let mut probe = checkpoint.state.clone();
        for batch in &self.replay_blocks {
            probe.apply_block(batch)?;
        }
        pipeline.store.begin_recovery(
            self.id,
            &self.run_id,
            &serde_json::to_vec(self)?,
            &self.orphan_hashes,
        )?;
        pipeline.state = checkpoint.state;
        for batch in &self.replay_blocks {
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| PipelineError::Invalid("recovery UTC clock"))?
                .as_millis()
                .try_into()
                .map_err(|_| PipelineError::Invalid("recovery time overflow"))?;
            pipeline.process_timed(batch.clone(), time, vec![])?;
        }
        pipeline.store.finish_recovery(self.id)?;
        pipeline.paused = false;
        Ok(pipeline.view())
    }
}
