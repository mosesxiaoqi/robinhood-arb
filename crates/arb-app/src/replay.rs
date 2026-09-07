use crate::pipeline::{Pipeline, PipelineError};
use arb_adapters::assemble::assemble_block;
use arb_core::{opportunity::Opportunity, types::RawRecord};
use std::collections::BTreeMap;
pub fn replay_chain(pipeline: &mut Pipeline, to: u64) -> Result<Vec<Opportunity>, PipelineError> {
    let from = pipeline
        .view()
        .position
        .block_number
        .checked_add(1)
        .ok_or(PipelineError::Invalid("block overflow"))?;
    if to < from - 1 {
        return Err(PipelineError::Invalid(
            "replay endpoint precedes checkpoint",
        ));
    }
    let mut output = vec![];
    for number in from..=to {
        let view = pipeline.view();
        let chain = view.pools[0].descriptor.id.chain_id;
        let pools = view
            .pools
            .iter()
            .map(|p| p.descriptor.clone())
            .collect::<Vec<_>>();
        let mut groups = BTreeMap::<_, Vec<RawRecord>>::new();
        let mut cursor = 0;
        let mut bytes = 0;
        loop {
            let page = pipeline.store().read_raw_block(chain, number, cursor)?;
            if page.is_empty() {
                break;
            }
            for raw in page {
                cursor = raw.id;
                let raw = raw.record;
                if !matches!(raw.kind.as_str(), "block" | "receipts" | "logs") {
                    continue;
                }
                bytes += raw.payload.len();
                if bytes > 64 * 1024 * 1024 {
                    return Err(PipelineError::Invalid(
                        "block observations exceed replay budget",
                    ));
                }
                let at = raw
                    .position
                    .as_ref()
                    .ok_or(PipelineError::Invalid("raw position"))?;
                groups
                    .entry((at.block_hash, raw.source.clone(), raw.run_id.clone()))
                    .or_default()
                    .push(raw);
            }
        }
        let mut selected = None;
        for observations in groups.values() {
            let mut by_kind = BTreeMap::new();
            for raw in observations {
                by_kind
                    .entry(raw.kind.as_str())
                    .or_insert_with(|| raw.clone());
            }
            let records = by_kind.into_values().collect::<Vec<_>>();
            let Ok(batch) = assemble_block(&records, &pools) else {
                continue;
            };
            if batch.parent_hash != view.position.block_hash {
                continue;
            }
            if selected
                .as_ref()
                .is_some_and(|(prior, _): &(arb_core::state::BlockBatch, u64)| {
                    prior.position.block_hash != batch.position.block_hash
                })
            {
                return Err(PipelineError::Invalid(
                    "ambiguous fork requires recovery selection",
                ));
            }
            if selected.is_none() {
                selected = Some((
                    batch,
                    records.iter().map(|r| r.received_at_ms).max().unwrap_or(0),
                ));
            }
        }
        let (batch, time) = selected.ok_or(PipelineError::Invalid(
            "data gap: no complete successor block",
        ))?;
        output.extend(pipeline.process(batch, time)?);
    }
    Ok(output)
}
pub const ALGORITHM_VERSION: &str = "pons-v2-v4-step1-v1";
pub fn replay_checkpoint(
    config: crate::config::Config,
    id: u64,
    to: u64,
) -> Result<usize, PipelineError> {
    use arb_adapters::store::Store;
    let store = Store::open(&config.database)?;
    let checkpoint = store.load_checkpoint(id)?;
    let run_id = checkpoint
        .research_run_id
        .as_deref()
        .ok_or(PipelineError::Invalid(
            "checkpoint lacks research run reference",
        ))?;
    let mut run = store.load_run(run_id)?;
    if checkpoint.config_hash != config.research_hash()
        || run.config_hash != checkpoint.config_hash
        || run.algorithm_version != ALGORITHM_VERSION
        || run.algorithm_version != checkpoint.algorithm_version
        || run.registry_version != checkpoint.registry_version
        || checkpoint
            .state
            .view()
            .pools
            .iter()
            .any(|p| p.descriptor.id.chain_id != config.chain_id)
    {
        return Err(PipelineError::Invalid(
            "checkpoint configuration or algorithm mismatch",
        ));
    }
    run.run_id = format!("chain-replay-{id}-{to}-{}", config.research_hash());
    let mut pipeline = Pipeline::new(store, checkpoint.state, run)?;
    Ok(replay_chain(&mut pipeline, to)?.len())
}
