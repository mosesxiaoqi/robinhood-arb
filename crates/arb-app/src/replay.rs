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
    replay_checkpoint_mode(config, id, to, false)
}
pub fn replay_checkpoint_mode(
    config: crate::config::Config,
    id: u64,
    to: u64,
    observed: bool,
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
    let mode = if observed { "observed" } else { "chain" };
    run.run_id = format!("{mode}-replay-{id}-{to}-{}", config.research_hash());
    let mut pipeline = Pipeline::new(store, checkpoint.state, run)?;
    if observed {
        let report = replay_observed(&mut pipeline, checkpoint.processing_cursor.last_raw_id, to)?;
        println!(
            "time quality: {} clock regressions; multiple domains: {}",
            report.clock_regressions, report.multiple_clock_domains
        );
        Ok(report.opportunities.len())
    } else {
        Ok(replay_chain(&mut pipeline, to)?.len())
    }
}

pub struct ObservedReplay {
    pending: std::collections::VecDeque<arb_adapters::store::StoredRaw>,
    statuses: BTreeMap<alloy_primitives::B256, arb_core::types::ExecutionStatus>,
    logical_time: u64,
    regressions: usize,
    pub multiple_clock_domains: bool,
}
impl ObservedReplay {
    pub fn new(mut records: Vec<arb_adapters::store::StoredRaw>) -> Result<Self, PipelineError> {
        if records
            .iter()
            .map(|r| r.record.payload.len())
            .sum::<usize>()
            > 64 * 1024 * 1024
            || records.len() > 100000
        {
            return Err(PipelineError::Invalid(
                "observed replay window exceeds budget",
            ));
        }
        records.sort_by_key(|r| r.id);
        let chain = records.first().map(|r| r.record.chain_id);
        let mut clocks = BTreeMap::new();
        let mut regressions = 0;
        let mut ids = std::collections::BTreeSet::new();
        for r in &records {
            if Some(r.record.chain_id) != chain {
                return Err(PipelineError::Invalid("mixed observed chains"));
            }
            r.record
                .validate()
                .map_err(|_| PipelineError::Invalid("raw record"))?;
            if !ids.insert(r.id) {
                return Err(PipelineError::Invalid("duplicate persisted id"));
            }
            if clocks
                .insert(
                    (r.record.source.clone(), r.record.run_id.clone()),
                    r.record.received_at_ms,
                )
                .is_some_and(|before| before > r.record.received_at_ms)
            {
                regressions += 1;
            }
        }
        records.sort_by_key(|r| (r.record.received_at_ms, r.id));
        Ok(Self {
            pending: records.into(),
            statuses: BTreeMap::new(),
            logical_time: 0,
            regressions,
            multiple_clock_domains: clocks.len() > 1,
        })
    }
    pub fn next_time(&self) -> Option<u64> {
        self.pending.front().map(|r| r.record.received_at_ms)
    }
    pub fn advance(
        &mut self,
        time: u64,
    ) -> Result<Vec<arb_adapters::store::StoredRaw>, PipelineError> {
        use arb_core::types::ExecutionStatus;
        if time < self.logical_time {
            return Err(PipelineError::Invalid("logical clock regression"));
        }
        let count = self
            .pending
            .iter()
            .take_while(|r| r.record.received_at_ms <= time)
            .count();
        let mut updates = BTreeMap::new();
        for r in self.pending.iter().take(count) {
            if let Some(hash) = r.record.transaction_hash {
                updates.entry(hash).or_insert_with(|| self.status(hash));
            }
            if r.record.kind == "receipts" {
                let envelope: serde_json::Value = serde_json::from_slice(&r.record.payload)?;
                let receipts = envelope["result"]
                    .as_array()
                    .ok_or(PipelineError::Invalid("observed receipt list"))?;
                for receipt in receipts {
                    let position = r
                        .record
                        .position
                        .as_ref()
                        .ok_or(PipelineError::Invalid("receipt observation position"))?;
                    if receipt["blockHash"] != serde_json::json!(position.block_hash)
                        || receipt["blockNumber"]
                            != serde_json::json!(format!("0x{:x}", position.block_number))
                    {
                        return Err(PipelineError::Invalid("receipt observation block mismatch"));
                    }
                    let hash = serde_json::from_value(receipt["transactionHash"].clone())?;
                    let status = match receipt["status"].as_str() {
                        Some("0x1") => ExecutionStatus::Succeeded,
                        Some("0x0") => ExecutionStatus::Reverted,
                        _ => ExecutionStatus::Unknown,
                    };
                    let prior = updates.entry(hash).or_insert_with(|| self.status(hash));
                    if *prior == ExecutionStatus::Unknown {
                        *prior = status;
                    } else if status != ExecutionStatus::Unknown && status != *prior {
                        return Err(PipelineError::Invalid(
                            "conflicting observed execution outcomes",
                        ));
                    }
                }
            }
        }
        self.statuses.extend(updates);
        let available = self.pending.drain(..count).collect();
        self.logical_time = time;
        Ok(available)
    }
    pub fn status(&self, hash: alloy_primitives::B256) -> arb_core::types::ExecutionStatus {
        self.statuses
            .get(&hash)
            .cloned()
            .unwrap_or(arb_core::types::ExecutionStatus::Unknown)
    }
    pub fn clock_regressions(&self) -> usize {
        self.regressions
    }
}
#[derive(Debug)]
pub struct ObservedReport {
    pub opportunities: Vec<Opportunity>,
    pub clock_regressions: usize,
    pub multiple_clock_domains: bool,
}
pub fn replay_observed(
    pipeline: &mut Pipeline,
    after_raw_id: u64,
    to: u64,
) -> Result<ObservedReport, PipelineError> {
    let initial = pipeline.view();
    let chain = initial.pools[0].descriptor.id.chain_id;
    let mut records = vec![];
    let mut cursor = after_raw_id;
    let mut bytes = 0;
    loop {
        let page = pipeline
            .store()
            .read_raw_filtered_after(cursor, 100, Some(chain), None)?;
        if page.is_empty() {
            break;
        }
        for record in page {
            cursor = record.id;
            if record.record.position.as_ref().is_some_and(|p| {
                p.block_number <= initial.position.block_number || p.block_number > to
            }) {
                continue;
            }
            bytes += record.record.payload.len();
            if bytes > 64 * 1024 * 1024 || records.len() >= 100000 {
                return Err(PipelineError::Invalid(
                    "observed replay window exceeds budget",
                ));
            }
            records.push(record);
        }
    }
    let mut timeline = ObservedReplay::new(records)?;
    pipeline.set_time_quality(arb_core::research::TimeQuality {
        clock_regressions: timeline.clock_regressions(),
        multiple_clock_domains: timeline.multiple_clock_domains,
    });
    let mut pending = BTreeMap::<_, BTreeMap<String, RawRecord>>::new();
    let mut report = ObservedReport {
        opportunities: vec![],
        clock_regressions: timeline.clock_regressions(),
        multiple_clock_domains: timeline.multiple_clock_domains,
    };
    let mut applied =
        BTreeMap::from([(initial.position.block_number, initial.position.block_hash)]);
    while let Some(time) = timeline.next_time() {
        for record in timeline.advance(time)? {
            let raw = record.record;
            if !matches!(raw.kind.as_str(), "block" | "receipts" | "logs") {
                continue;
            }
            let position = raw
                .position
                .as_ref()
                .ok_or(PipelineError::Invalid("observed block position"))?;
            if let Some(hash) = applied.get(&position.block_number) {
                if *hash != position.block_hash {
                    return Err(PipelineError::Invalid("observed fork requires recovery"));
                }
                continue;
            }
            pending
                .entry((
                    position.block_number,
                    position.block_hash,
                    raw.source.clone(),
                    raw.run_id.clone(),
                ))
                .or_default()
                .entry(raw.kind.clone())
                .or_insert(raw);
        }
        loop {
            let view = pipeline.view();
            if view.position.block_number >= to {
                break;
            }
            let next = view.position.block_number + 1;
            let pools = view
                .pools
                .iter()
                .map(|p| p.descriptor.clone())
                .collect::<Vec<_>>();
            let mut ready = None;
            for (key, records) in pending.iter().filter(|(k, _)| k.0 == next) {
                let bundle = records.values().cloned().collect::<Vec<_>>();
                if let Ok(batch) = assemble_block(&bundle, &pools)
                    && batch.parent_hash == view.position.block_hash
                {
                    if ready.as_ref().is_some_and(
                        |(_, prior): &(_, arb_core::state::BlockBatch)| {
                            prior.position.block_hash != batch.position.block_hash
                        },
                    ) {
                        return Err(PipelineError::Invalid(
                            "ambiguous simultaneous observed fork",
                        ));
                    }
                    if ready.is_none() {
                        ready = Some((key.clone(), batch));
                    }
                }
            }
            let Some((key, batch)) = ready else {
                break;
            };
            report.opportunities.extend(pipeline.process(batch, time)?);
            applied.insert(next, key.1);
            pending.remove(&key);
            pending.retain(|(number, hash, _, _), _| *number != next || *hash != key.1);
        }
    }
    if pipeline.view().position.block_number != to {
        return Err(PipelineError::Invalid("observed replay data gap"));
    }
    Ok(report)
}
