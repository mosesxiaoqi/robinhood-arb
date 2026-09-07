use crate::{
    decode::decode,
    rpc::{SourceError, hash, validate_block},
};
use arb_core::{
    state::BlockBatch,
    types::{Offset, PoolDescriptor, RawRecord, RawRef},
};
use serde_json::Value;
pub fn assemble_block(
    records: &[RawRecord],
    pools: &[PoolDescriptor],
) -> Result<BlockBatch, SourceError> {
    let invalid = || SourceError::Invalid("incomplete or inconsistent raw block bundle");
    if records.len() != 3 || pools.is_empty() {
        return Err(invalid());
    }
    let first = &records[0];
    let at = first.position.as_ref().ok_or_else(invalid)?;
    if at.offset != Offset::BlockEnd || pools.iter().any(|p| p.id.chain_id != first.chain_id) {
        return Err(invalid());
    }
    let mut values = std::collections::BTreeMap::new();
    let mut refs = vec![];
    for raw in records {
        raw.validate().map_err(|_| invalid())?;
        if raw.position.as_ref() != Some(at)
            || raw.chain_id != first.chain_id
            || raw.source != first.source
            || raw.run_id != first.run_id
            || !matches!(raw.kind.as_str(), "block" | "receipts" | "logs")
        {
            return Err(invalid());
        }
        let envelope: Value = serde_json::from_slice(&raw.payload).map_err(|_| invalid())?;
        if envelope.get("error").is_some()
            || values
                .insert(
                    raw.kind.as_str(),
                    envelope.get("result").ok_or_else(invalid)?.clone(),
                )
                .is_some()
        {
            return Err(invalid());
        }
        refs.push(RawRef {
            source: raw.source.clone(),
            run_id: raw.run_id.clone(),
            sequence: raw.sequence,
        });
    }
    if validate_block(
        &values["block"],
        &values["receipts"],
        &values["logs"],
        at.block_number,
    )? != at.block_hash
    {
        return Err(invalid());
    }
    let receipt = records
        .iter()
        .find(|r| r.kind == "receipts")
        .ok_or_else(invalid)?;
    let mut observations = vec![];
    for pool in pools {
        observations.extend(decode(receipt, pool).map_err(|_| invalid())?);
    }
    observations.sort_by_key(|o| match o.position.offset {
        Offset::Transaction { index, log_index } => (index, log_index),
        Offset::BlockEnd => (u64::MAX, None),
    });
    refs.sort_by_key(|r| r.sequence);
    Ok(BlockBatch {
        position: at.clone(),
        parent_hash: hash(&values["block"]["parentHash"])?,
        covered_pools: pools.iter().map(|p| p.id.clone()).collect(),
        observations,
        raw_refs: refs,
    })
}
