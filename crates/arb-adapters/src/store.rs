mod catalog;
mod migration;
mod raw;
mod research;
mod simulation;

use research::{derived_from_row, load_derived};

use arb_core::types::{RawRecord, RecordError, SourceCursor};
use rusqlite::{Connection, OptionalExtension, params};
use std::{path::Path, time::Duration};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLite operation failed: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("stored record cannot be decoded")]
    Decode(#[from] serde_json::Error),
    #[error(transparent)]
    Record(#[from] RecordError),
    #[error("invalid storage operation: {0}")]
    Invalid(&'static str),
}

fn sql_block_number(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value)
        .map_err(|_| StoreError::Invalid("block number exceeds SQLite INTEGER range"))
}

fn save_source_cursor(connection: &Connection, cursor: &SourceCursor) -> Result<(), StoreError> {
    connection.execute(
        "INSERT INTO source_cursors(chain_id,source,next_block,last_block_hash) VALUES(?1,?2,?3,?4) ON CONFLICT(chain_id,source) DO UPDATE SET next_block=excluded.next_block,last_block_hash=excluded.last_block_hash",
        params![cursor.chain_id.to_string(), cursor.source, sql_block_number(cursor.next_block)?, cursor.last_block_hash.map(|hash| hash.to_string())],
    )?;
    Ok(())
}

fn check_row_budget(row: &rusqlite::Row<'_>, limit: usize) -> Result<(), StoreError> {
    let mut bytes = 0usize;
    for index in 0..row.as_ref().column_count() {
        if let rusqlite::types::ValueRef::Text(value) | rusqlite::types::ValueRef::Blob(value) =
            row.get_ref(index)?
        {
            bytes = bytes.saturating_add(value.len());
        }
    }
    if bytes > limit {
        return Err(StoreError::Invalid("record read budget"));
    }
    Ok(())
}

pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let migrations = [
            include_str!("../migrations/schema_v001_create_raw_records_and_source_cursors.sql"),
            include_str!("../migrations/schema_v002_create_ingest_gaps.sql"),
            include_str!("../migrations/schema_v003_create_pool_registry.sql"),
            include_str!("../migrations/schema_v004_create_bootstraps.sql"),
            include_str!("../migrations/schema_v005_create_checkpoints.sql"),
            include_str!("../migrations/schema_v006_create_research_runs_and_derived_blocks.sql"),
            include_str!("../migrations/schema_v007_add_raw_records_block_number.sql"),
            include_str!("../migrations/schema_v008_create_recovery_jobs.sql"),
            include_str!("../migrations/schema_v009_create_simulations.sql"),
            include_str!("../migrations/schema_v010_create_feed_tracking.sql"),
            include_str!("../migrations/schema_v011_create_wallet_facts.sql"),
            include_str!("../migrations/schema_v012_create_runtime_checkpoints_and_status.sql"),
            include_str!("../migrations/schema_v013_use_integer_block_numbers.sql"),
            include_str!("../migrations/schema_v014_split_source_cursor_columns.sql"),
            include_str!("../migrations/schema_v015_split_record_columns.sql"),
        ];
        if version as usize > migrations.len() {
            return Err(StoreError::Invalid("unsupported database version"));
        }
        if version == 0 {
            let count: i64 = connection.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            if count != 0 {
                return Err(StoreError::Invalid("unversioned nonempty database"));
            }
        }
        if (version as usize) < migrations.len() {
            let tx =
                connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            for (index, migration) in migrations.iter().enumerate().skip(version as usize) {
                if index == 14 && version < 7 {
                    let mut statement = tx.prepare("SELECT id,data FROM raw_records")?;
                    let mut rows = statement.query([])?;
                    while let Some(row) = rows.next()? {
                        let id: i64 = row.get(0)?;
                        let data: Vec<u8> = row.get(1)?;
                        let raw: RawRecord = serde_json::from_slice(&data)?;
                        if let Some(position) = raw.position {
                            tx.execute(
                                "UPDATE raw_records SET block_number=?1 WHERE id=?2",
                                params![sql_block_number(position.block_number)?, id],
                            )?;
                        }
                    }
                }
                tx.execute_batch(migration)?;
                if index == 14 {
                    migration::migrate_v15(&tx)?;
                }
            }
            tx.commit()?;
        }
        let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if mode != "wal" {
            return Err(StoreError::Invalid("WAL requires a local file database"));
        }
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self { connection })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct StoredRaw {
    pub id: u64,
    pub record: RawRecord,
}

#[derive(Clone, Copy)]
pub enum ReportTable {
    Blocks,
    Simulations,
    Wallets,
}
impl Store {
    /// Read-only transaction pins one WAL snapshot for every section of a report.
    pub fn open_report(path: &Path) -> Result<Self, StoreError> {
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("BEGIN DEFERRED")?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != 15 {
            return Err(StoreError::Invalid("report requires current schema"));
        }
        Ok(Self { connection })
    }
    pub fn report_bounds(&self, run: &str) -> Result<Option<(u64, u64)>, StoreError> {
        let mut bounds = vec![];
        for sql in [
            "SELECT block_number FROM derived_blocks WHERE run_id=?1 ORDER BY block_number LIMIT 1",
            "SELECT block_number FROM derived_blocks WHERE run_id=?1 ORDER BY block_number DESC LIMIT 1",
        ] {
            let value: Option<i64> = self
                .connection
                .query_row(sql, [run], |r| r.get(0))
                .optional()?;
            let Some(value) = value else { return Ok(None) };
            bounds.push(
                u64::try_from(value).map_err(|_| StoreError::Invalid("report block number"))?,
            );
        }
        Ok(Some((bounds[0], bounds[1])))
    }
    pub fn read_report_page(
        &self,
        table: ReportTable,
        run: &str,
        after: u64,
        from: u64,
        to: u64,
    ) -> Result<Vec<(u64, serde_json::Value)>, StoreError> {
        let table_name = match table {
            ReportTable::Blocks => "derived_blocks",
            ReportTable::Simulations => "simulations",
            ReportTable::Wallets => "wallet_facts",
        };
        let mut statement = self.connection.prepare(&format!("SELECT * FROM {table_name} WHERE run_id=?1 AND id>?2 AND block_number BETWEEN ?3 AND ?4 ORDER BY id LIMIT 100"))?;
        let mut rows = statement.query(params![
            run,
            sql_block_number(after)?,
            sql_block_number(from)?,
            sql_block_number(to)?
        ])?;
        let mut output = vec![];
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let value = match table {
                ReportTable::Blocks => serde_json::to_value(derived_from_row(row)?)?,
                ReportTable::Simulations => {
                    serde_json::to_value(simulation::simulation_from_row(row)?)?
                }
                ReportTable::Wallets => serde_json::to_value(simulation::wallet_from_row(row)?)?,
            };
            let size = serde_json::to_vec(&value)?.len();
            if size > 67108864 {
                return Err(StoreError::Invalid("report record budget"));
            }
            if bytes + size > 67108864 {
                break;
            }
            bytes += size;
            output.push((
                u64::try_from(row.get::<_, i64>("id")?)
                    .map_err(|_| StoreError::Invalid("report id"))?,
                value,
            ));
        }
        Ok(output)
    }
    pub fn report_gap_counts(
        &self,
        chain: u64,
        from: u64,
        to: u64,
    ) -> Result<(u64, u64), StoreError> {
        let from = i64::try_from(from).map_err(|_| StoreError::Invalid("report range"))?;
        let to = i64::try_from(to).map_err(|_| StoreError::Invalid("report range"))?;
        let rpc:i64=self.connection.query_row("SELECT count(*) FROM ingest_gaps WHERE chain_id=?1 AND resolved=0 AND block_number BETWEEN ?2 AND ?3",params![chain.to_string(),from,to],|r|r.get(0))?;
        let feed: i64 = self.connection.query_row(
            "SELECT count(*) FROM feed_gaps WHERE chain_id=?1",
            [chain.to_string()],
            |r| r.get(0),
        )?;
        Ok((rpc as u64, feed as u64))
    }
}

impl Store {
    pub fn runtime_checkpoint(
        &self,
        run: &str,
    ) -> Result<Option<arb_core::checkpoint::Checkpoint>, StoreError> {
        let id: Option<i64> = self
            .connection
            .query_row(
                "SELECT checkpoint_id FROM runtime_checkpoints WHERE run_id=?1",
                [run],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| self.load_checkpoint(id as u64)).transpose()
    }
    pub fn save_runtime_checkpoint(
        &mut self,
        checkpoint: &arb_core::checkpoint::Checkpoint,
    ) -> Result<(), StoreError> {
        self.write_runtime_checkpoint(checkpoint, None)
    }
    pub fn finish_runtime_recovery(
        &mut self,
        checkpoint: &arb_core::checkpoint::Checkpoint,
        id: alloy_primitives::B256,
    ) -> Result<(), StoreError> {
        self.write_runtime_checkpoint(checkpoint, Some(id))
    }
    fn write_runtime_checkpoint(
        &mut self,
        checkpoint: &arb_core::checkpoint::Checkpoint,
        recovery: Option<alloy_primitives::B256>,
    ) -> Result<(), StoreError> {
        checkpoint.validate()?;
        let run = checkpoint
            .research_run_id
            .as_ref()
            .ok_or(StoreError::Invalid("runtime run identity"))?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let id = catalog::insert_checkpoint(&tx, None, checkpoint)?;
        tx.execute("INSERT INTO runtime_checkpoints(run_id,checkpoint_id) VALUES(?1,?2) ON CONFLICT(run_id) DO UPDATE SET checkpoint_id=excluded.checkpoint_id",params![run,id])?;
        if let Some(recovery) = recovery {
            let view = checkpoint.state.view();
            let cursor = SourceCursor {
                chain_id: view.pools[0].descriptor.id.chain_id,
                source: "rpc".into(),
                next_block: checkpoint.processing_cursor.next_block,
                last_block_hash: Some(view.position.block_hash),
            };
            save_source_cursor(&tx, &cursor)?;
            tx.execute("UPDATE ingest_gaps SET resolved=1 WHERE chain_id=?1 AND source='rpc' AND block_number=?2",params![cursor.chain_id.to_string(),sql_block_number(view.position.block_number)?])?;
            refresh_canonical_simulations(&tx, run)?;
            tx.execute(
                "UPDATE recovery_jobs SET status='complete' WHERE id=?1",
                [recovery.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn pending_recovery(
        &self,
        run: &str,
    ) -> Result<Option<alloy_primitives::B256>, StoreError> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM recovery_jobs WHERE run_id=?1 AND status='pending'",
                [run],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|v| v.parse().map_err(|_| StoreError::Invalid("recovery id")))
            .transpose()
    }
    pub fn last_raw_id(&self) -> Result<u64, StoreError> {
        Ok(self
            .connection
            .query_row("SELECT COALESCE(max(id),0) FROM raw_records", [], |r| {
                r.get::<_, i64>(0)
            })? as u64)
    }
}

impl Store {
    pub fn reactivate_derived(
        &mut self,
        run: &str,
        hash: alloy_primitives::B256,
    ) -> Result<(), StoreError> {
        if !self.has_pending_recovery(run)? {
            return Err(StoreError::Invalid("reactivation requires recovery"));
        }
        if self.connection.execute(
            "UPDATE derived_blocks SET canonical=1 WHERE run_id=?1 AND block_hash=?2",
            params![run, hash.to_string()],
        )? != 1
        {
            return Err(StoreError::Invalid("reactivation block missing"));
        }
        Ok(())
    }
}
fn refresh_canonical_simulations(
    tx: &rusqlite::Transaction<'_>,
    run: &str,
) -> Result<(), StoreError> {
    let mut after = 0i64;
    loop {
        let row: Option<(i64,String,String)> = tx.query_row(
            "SELECT s.id,s.job_id,s.block_hash FROM simulations s JOIN derived_blocks d ON s.run_id=d.run_id AND s.block_hash=d.block_hash WHERE s.run_id=?1 AND s.id>?2 AND s.canonical=0 AND d.canonical=1 ORDER BY s.id LIMIT 1",
            params![run,after], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
        ).optional()?;
        let Some((id, job, hash)) = row else {
            break;
        };
        after = id;
        let record = simulation::load_simulation(
            tx,
            job.parse()
                .map_err(|_| StoreError::Invalid("simulation id"))?,
        )?;
        let block = load_derived(
            tx,
            run,
            hash.parse()
                .map_err(|_| StoreError::Invalid("derived hash"))?,
        )?
        .ok_or(StoreError::Invalid("derived block missing"))?;
        if let Some(mut candidate) = block
            .candidates
            .into_iter()
            .find(|c| c.id == record.candidate_id && c.view_id == record.view_id)
        {
            candidate.canonical = true;
            let validates = record
                .result
                .as_ref()
                .is_some_and(|r| arb_core::simulation::validates_candidate(&candidate, r));
            tx.execute(
                "UPDATE simulations SET canonical=1,validates_original=?1 WHERE id=?2",
                params![validates, id],
            )?;
        }
    }
    Ok(())
}
