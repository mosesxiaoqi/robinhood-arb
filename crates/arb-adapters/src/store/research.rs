use super::*;

impl Store {
    pub fn save_derived(
        &mut self,
        block: &arb_core::research::DerivedBlock,
    ) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(block)?;
        if bytes.len() > 64 * 1024 * 1024
            || block.batch.position != block.view.position
            || block.view.pools.is_empty()
            || block.batch.raw_refs.is_empty()
        {
            return Err(StoreError::Invalid("derived block scope or size"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let _: String = tx.query_row(
            "SELECT run_id FROM research_runs WHERE run_id=?1",
            [&block.run_id],
            |r| r.get(0),
        )?;
        for reference in &block.batch.raw_refs {
            let raw = raw::find_raw(
                &tx,
                block.view.pools[0].descriptor.id.chain_id,
                &reference.source,
                &reference.run_id,
                reference.sequence,
            )?
            .ok_or(StoreError::Invalid("raw provenance missing"))?;
            if raw.position.as_ref().is_none_or(|p| {
                p.block_hash != block.view.position.block_hash
                    || p.block_number != block.view.position.block_number
            }) {
                return Err(StoreError::Invalid("raw provenance block mismatch"));
            }
        }
        insert_derived(&tx, None, block)?;
        tx.commit()?;
        Ok(())
    }
    pub fn find_derived(
        &self,
        run: &str,
        hash: alloy_primitives::B256,
    ) -> Result<Option<arb_core::research::DerivedBlock>, StoreError> {
        load_derived(&self.connection, run, hash)
    }
    pub fn has_pending_recovery(&self, run: &str) -> Result<bool, StoreError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM recovery_jobs WHERE run_id=?1 AND status='pending')",
            [run],
            |r| r.get(0),
        )?)
    }
    pub fn begin_recovery(
        &mut self,
        id: alloy_primitives::B256,
        run: &str,
        data: &[u8],
        orphans: &[alloy_primitives::B256],
    ) -> Result<(), StoreError> {
        if data.len() > 64 * 1024 * 1024 || orphans.len() > 4096 {
            return Err(StoreError::Invalid("recovery budget"));
        }
        let plan: RecoveryData = serde_json::from_slice(data)?;
        if plan.id != id || plan.run_id != run || plan.orphan_hashes != orphans {
            return Err(StoreError::Invalid("recovery plan identity mismatch"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let prior = load_recovery(&tx, id)?;
        if let Some((existing, status)) = prior {
            if existing != plan {
                return Err(StoreError::Invalid("recovery identity collision"));
            }
            if status == "complete" {
                return Ok(());
            }
        } else {
            let pending: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM recovery_jobs WHERE run_id=?1 AND status='pending')",
                [run],
                |r| r.get(0),
            )?;
            if pending {
                return Err(StoreError::Invalid(
                    "resume existing recovery before starting another",
                ));
            }
            insert_recovery(&tx, &plan, "pending")?;
        }
        for hash in orphans {
            tx.execute("UPDATE simulations SET canonical=0,validates_original=0 WHERE run_id=?1 AND block_hash=?2",params![run,hash.to_string()])?;
            tx.execute(
                "UPDATE derived_blocks SET canonical=0 WHERE run_id=?1 AND block_hash=?2",
                params![run, hash.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn finish_recovery(&mut self, id: alloy_primitives::B256) -> Result<(), StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let run: String = tx.query_row(
            "SELECT run_id FROM recovery_jobs WHERE id=?1",
            [id.to_string()],
            |r| r.get(0),
        )?;
        refresh_canonical_simulations(&tx, &run)?;
        tx.execute(
            "UPDATE recovery_jobs SET status='complete' WHERE id=?1",
            [id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn recovery_data(&self, id: alloy_primitives::B256) -> Result<Vec<u8>, StoreError> {
        let (plan, _) =
            load_recovery(&self.connection, id)?.ok_or(StoreError::Invalid("recovery missing"))?;
        Ok(serde_json::to_vec(&plan)?)
    }
    pub fn save_runtime_status(
        &mut self,
        run: &str,
        value: &serde_json::Value,
    ) -> Result<(), StoreError> {
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > 65536 {
            return Err(StoreError::Invalid("runtime status budget"));
        }
        let status: RuntimeStatusRow = serde_json::from_value(value.clone())?;
        if status.run_id != run {
            return Err(StoreError::Invalid("runtime status run mismatch"));
        }
        insert_runtime_status(&self.connection, &status)
    }
    pub fn runtime_status(&self, run: &str) -> Result<Option<serde_json::Value>, StoreError> {
        let mut statement = self
            .connection
            .prepare("SELECT * FROM runtime_status WHERE run_id=?1")?;
        let mut rows = statement.query([run])?;
        rows.next()?
            .map(|row| Ok(serde_json::to_value(runtime_from_row(row)?)?))
            .transpose()
    }
}

use alloy_primitives::B256;
use arb_core::{
    research::{DerivedBlock, TimeQuality},
    state::{BlockBatch, StateView},
    types::{ChainPosition, Offset},
};
use serde::{Deserialize, Serialize};

const BUDGET: usize = 64 * 1024 * 1024;

fn json<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    name: &str,
) -> Result<T, StoreError> {
    let text = row
        .get_ref(name)?
        .as_str()
        .map_err(|_| StoreError::Invalid("expected JSON text"))?;
    if text.len() > BUDGET {
        return Err(StoreError::Invalid("JSON read budget"));
    }
    Ok(serde_json::from_str(text)?)
}

fn number(row: &rusqlite::Row<'_>, name: &str) -> Result<u64, StoreError> {
    u64::try_from(row.get::<_, i64>(name)?)
        .map_err(|_| StoreError::Invalid("negative stored integer"))
}

fn optional_number(row: &rusqlite::Row<'_>, name: &str) -> Result<Option<u64>, StoreError> {
    row.get::<_, Option<i64>>(name)?
        .map(|n| u64::try_from(n).map_err(|_| StoreError::Invalid("negative stored integer")))
        .transpose()
}

fn hash(row: &rusqlite::Row<'_>, name: &str) -> Result<B256, StoreError> {
    row.get::<_, String>(name)?
        .parse()
        .map_err(|_| StoreError::Invalid("invalid stored hash"))
}

pub(super) fn derived_from_row(row: &rusqlite::Row<'_>) -> Result<DerivedBlock, StoreError> {
    check_row_budget(row, BUDGET)?;
    let position = ChainPosition {
        block_number: number(row, "block_number")?,
        block_hash: hash(row, "block_hash")?,
        offset: Offset::BlockEnd,
    };
    let time_quality = match (
        optional_number(row, "clock_regressions")?,
        row.get::<_, Option<bool>>("multiple_clock_domains")?,
    ) {
        (Some(n), Some(multiple_clock_domains)) => Some(TimeQuality {
            clock_regressions: usize::try_from(n)
                .map_err(|_| StoreError::Invalid("clock regressions"))?,
            multiple_clock_domains,
        }),
        (None, None) => None,
        _ => return Err(StoreError::Invalid("incomplete time quality")),
    };
    let mut block = DerivedBlock {
        canonical: row.get("canonical")?,
        time_quality,
        timings: json(row, "timings_json")?,
        run_id: row.get("run_id")?,
        view_id: hash(row, "view_id")?,
        batch: BlockBatch {
            position: position.clone(),
            parent_hash: hash(row, "parent_hash")?,
            covered_pools: json(row, "covered_pools_json")?,
            observations: json(row, "observations_json")?,
            raw_refs: json(row, "raw_refs_json")?,
        },
        view: StateView {
            position,
            pools: json(row, "pools_json")?,
            raw_refs: json(row, "view_raw_refs_json")?,
        },
        candidates: json(row, "candidates_json")?,
        exclusions: json(row, "exclusions_json")?,
        excluded_pools: usize::try_from(number(row, "excluded_pools")?)
            .map_err(|_| StoreError::Invalid("excluded pools"))?,
    };
    for candidate in &mut block.candidates {
        candidate.canonical = block.canonical;
    }
    Ok(block)
}

pub(super) fn load_derived(
    connection: &Connection,
    run: &str,
    block_hash: B256,
) -> Result<Option<DerivedBlock>, StoreError> {
    let mut statement =
        connection.prepare("SELECT * FROM derived_blocks WHERE run_id=?1 AND block_hash=?2")?;
    let mut rows = statement.query(params![run, block_hash.to_string()])?;
    rows.next()?.map(derived_from_row).transpose()
}

fn insert_derived(
    connection: &Connection,
    id: Option<i64>,
    block: &DerivedBlock,
) -> Result<(), StoreError> {
    if block.batch.position != block.view.position
        || block.view.position.offset != Offset::BlockEnd
        || serde_json::to_vec(block)?.len() > BUDGET
    {
        return Err(StoreError::Invalid("derived block position or budget"));
    }
    connection.execute(
        "INSERT INTO derived_blocks(id,run_id,block_hash,block_number,canonical,view_id,parent_hash,excluded_pools,clock_regressions,multiple_clock_domains,covered_pools_json,observations_json,raw_refs_json,pools_json,view_raw_refs_json,candidates_json,exclusions_json,timings_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
        params![id,block.run_id,block.view.position.block_hash.to_string(),sql_block_number(block.view.position.block_number)?,block.canonical,block.view_id.to_string(),block.batch.parent_hash.to_string(),i64::try_from(block.excluded_pools).map_err(|_|StoreError::Invalid("excluded pools"))?,block.time_quality.as_ref().map(|q|i64::try_from(q.clock_regressions).map_err(|_|StoreError::Invalid("clock regressions"))).transpose()?,block.time_quality.as_ref().map(|q|q.multiple_clock_domains),serde_json::to_string(&block.batch.covered_pools)?,serde_json::to_string(&block.batch.observations)?,serde_json::to_string(&block.batch.raw_refs)?,serde_json::to_string(&block.view.pools)?,serde_json::to_string(&block.view.raw_refs)?,serde_json::to_string(&block.candidates)?,serde_json::to_string(&block.exclusions)?,serde_json::to_string(&block.timings)?],
    )?;
    Ok(())
}

// Storage model for arb-app's recovery plan; the adapter cannot depend on arb-app.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryData {
    id: B256,
    run_id: String,
    checkpoint_id: u64,
    common_ancestor: ChainPosition,
    required_fetch: Option<(u64, u64)>,
    orphan_hashes: Vec<B256>,
    replay_blocks: Vec<BlockBatch>,
}

fn recovery_from_row(row: &rusqlite::Row<'_>) -> Result<RecoveryData, StoreError> {
    check_row_budget(row, BUDGET)?;
    let required_fetch = match (
        optional_number(row, "fetch_from")?,
        optional_number(row, "fetch_to")?,
    ) {
        (None, None) => None,
        (Some(from), Some(to)) => Some((from, to)),
        _ => return Err(StoreError::Invalid("incomplete recovery range")),
    };
    Ok(RecoveryData {
        id: hash(row, "id")?,
        run_id: row.get("run_id")?,
        checkpoint_id: number(row, "checkpoint_id")?,
        common_ancestor: ChainPosition {
            block_number: number(row, "ancestor_block_number")?,
            block_hash: hash(row, "ancestor_block_hash")?,
            offset: Offset::BlockEnd,
        },
        required_fetch,
        orphan_hashes: json(row, "orphan_hashes_json")?,
        replay_blocks: json(row, "replay_blocks_json")?,
    })
}

fn load_recovery(
    connection: &Connection,
    id: B256,
) -> Result<Option<(RecoveryData, String)>, StoreError> {
    let mut statement = connection.prepare("SELECT * FROM recovery_jobs WHERE id=?1")?;
    let mut rows = statement.query([id.to_string()])?;
    rows.next()?
        .map(|r| Ok((recovery_from_row(r)?, r.get("status")?)))
        .transpose()
}

fn insert_recovery(
    connection: &Connection,
    plan: &RecoveryData,
    status: &str,
) -> Result<(), StoreError> {
    if plan.common_ancestor.offset != Offset::BlockEnd || serde_json::to_vec(plan)?.len() > BUDGET {
        return Err(StoreError::Invalid("recovery position or budget"));
    }
    connection.execute("INSERT INTO recovery_jobs(id,run_id,checkpoint_id,ancestor_block_number,ancestor_block_hash,fetch_from,fetch_to,orphan_hashes_json,replay_blocks_json,status) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![plan.id.to_string(),plan.run_id,sql_block_number(plan.checkpoint_id)?,sql_block_number(plan.common_ancestor.block_number)?,plan.common_ancestor.block_hash.to_string(),plan.required_fetch.map(|r|sql_block_number(r.0)).transpose()?,plan.required_fetch.map(|r|sql_block_number(r.1)).transpose()?,serde_json::to_string(&plan.orphan_hashes)?,serde_json::to_string(&plan.replay_blocks)?,status])?;
    Ok(())
}

// Explicit database model for the runtime summary sent by arb-app.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeStatusRow {
    run_id: String,
    status: String,
    started_at_ms: u64,
    ended_at_ms: u64,
    collected_blocks: u64,
    processed_blocks: u64,
    first_collected: Option<u64>,
    last_collected: Option<u64>,
    disk_start: u64,
    disk_end: u64,
    max_processing_ns: u64,
    error: Option<String>,
}

fn runtime_from_row(row: &rusqlite::Row<'_>) -> Result<RuntimeStatusRow, StoreError> {
    check_row_budget(row, 65536)?;
    Ok(RuntimeStatusRow {
        run_id: row.get("run_id")?,
        status: row.get("status")?,
        started_at_ms: number(row, "started_at_ms")?,
        ended_at_ms: number(row, "ended_at_ms")?,
        collected_blocks: number(row, "collected_blocks")?,
        processed_blocks: number(row, "processed_blocks")?,
        first_collected: optional_number(row, "first_collected")?,
        last_collected: optional_number(row, "last_collected")?,
        disk_start: number(row, "disk_start")?,
        disk_end: number(row, "disk_end")?,
        max_processing_ns: number(row, "max_processing_ns")?,
        error: row.get("error")?,
    })
}

fn insert_runtime_status(
    connection: &Connection,
    status: &RuntimeStatusRow,
) -> Result<(), StoreError> {
    if serde_json::to_vec(status)?.len() > 65536 {
        return Err(StoreError::Invalid("runtime status budget"));
    }
    connection.execute("INSERT INTO runtime_status(run_id,status,started_at_ms,ended_at_ms,collected_blocks,processed_blocks,first_collected,last_collected,disk_start,disk_end,max_processing_ns,error) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(run_id) DO UPDATE SET status=excluded.status,started_at_ms=excluded.started_at_ms,ended_at_ms=excluded.ended_at_ms,collected_blocks=excluded.collected_blocks,processed_blocks=excluded.processed_blocks,first_collected=excluded.first_collected,last_collected=excluded.last_collected,disk_start=excluded.disk_start,disk_end=excluded.disk_end,max_processing_ns=excluded.max_processing_ns,error=excluded.error",params![
        status.run_id,
        status.status,
        sql_block_number(status.started_at_ms)?,
        sql_block_number(status.ended_at_ms)?,
        sql_block_number(status.collected_blocks)?,
        sql_block_number(status.processed_blocks)?,
        status.first_collected.map(sql_block_number).transpose()?,
        status.last_collected.map(sql_block_number).transpose()?,
        sql_block_number(status.disk_start)?,
        sql_block_number(status.disk_end)?,
        sql_block_number(status.max_processing_ns)?,
        status.error,
    ])?;
    Ok(())
}

pub(super) fn migrate(tx: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
    let mut statement = tx.prepare("SELECT * FROM derived_blocks_v014")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let bytes = row
            .get_ref("data")?
            .as_blob()
            .map_err(|_| StoreError::Invalid("legacy derived data"))?;
        if bytes.len() > BUDGET {
            return Err(StoreError::Invalid("legacy derived budget"));
        }
        let mut block: DerivedBlock = serde_json::from_slice(bytes)?;
        if block.run_id != row.get::<_, String>("run_id")?
            || block.view.position.block_hash.to_string() != row.get::<_, String>("block_hash")?
            || block.view.position.block_number != number(row, "block_number")?
        {
            return Err(StoreError::Invalid("legacy derived identity mismatch"));
        }
        block.canonical = row.get("canonical")?;
        insert_derived(tx, Some(row.get("id")?), &block)?;
    }
    let mut statement = tx.prepare("SELECT * FROM recovery_jobs_v014")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let bytes = row
            .get_ref("data")?
            .as_blob()
            .map_err(|_| StoreError::Invalid("legacy recovery data"))?;
        if bytes.len() > BUDGET {
            return Err(StoreError::Invalid("legacy recovery budget"));
        }
        let plan: RecoveryData = serde_json::from_slice(bytes)?;
        if plan.id.to_string() != row.get::<_, String>("id")?
            || plan.run_id != row.get::<_, String>("run_id")?
        {
            return Err(StoreError::Invalid("legacy recovery identity mismatch"));
        }
        insert_recovery(tx, &plan, &row.get::<_, String>("status")?)?;
    }
    let mut statement = tx.prepare("SELECT * FROM runtime_status_v014")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let bytes = row
            .get_ref("data")?
            .as_blob()
            .map_err(|_| StoreError::Invalid("legacy runtime data"))?;
        if bytes.len() > 65536 {
            return Err(StoreError::Invalid("legacy runtime budget"));
        }
        let status: RuntimeStatusRow = serde_json::from_slice(bytes)?;
        if status.run_id != row.get::<_, String>("run_id")? {
            return Err(StoreError::Invalid("legacy runtime identity mismatch"));
        }
        insert_runtime_status(tx, &status)?;
    }
    Ok(())
}
