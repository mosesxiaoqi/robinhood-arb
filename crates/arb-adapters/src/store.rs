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

pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let version: u32 = connection.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let migrations = [
            include_str!("migrations/001.sql"),
            include_str!("migrations/002.sql"),
            include_str!("migrations/003.sql"),
            include_str!("migrations/004.sql"),
            include_str!("migrations/005.sql"),
            include_str!("migrations/006.sql"),
            include_str!("migrations/007.sql"),
            include_str!("migrations/008.sql"),
            include_str!("migrations/009.sql"),
            include_str!("migrations/010.sql"),
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
            for migration in &migrations[version as usize..] {
                tx.execute_batch(migration)?;
            }
            if version < 7 {
                let mut statement = tx.prepare("SELECT id,data FROM raw_records")?;
                let mut rows = statement.query([])?;
                while let Some(row) = rows.next()? {
                    let id: i64 = row.get(0)?;
                    let data: Vec<u8> = row.get(1)?;
                    let raw: RawRecord = serde_json::from_slice(&data)?;
                    if let Some(position) = raw.position {
                        tx.execute(
                            "UPDATE raw_records SET block_number=?1 WHERE id=?2",
                            params![position.block_number.to_string(), id],
                        )?;
                    }
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

    pub fn append_raw(
        &mut self,
        records: &[RawRecord],
        cursor: &SourceCursor,
    ) -> Result<(), StoreError> {
        if records.is_empty() || cursor.chain_id == 0 || cursor.source.is_empty() {
            return Err(StoreError::Invalid("empty batch or invalid cursor"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<Vec<u8>> = tx
            .query_row(
                "SELECT data FROM source_cursors WHERE chain_id=?1 AND source=?2",
                params![cursor.chain_id.to_string(), cursor.source],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(data) = previous {
            let previous: SourceCursor = serde_json::from_slice(&data)?;
            if previous.next_block > cursor.next_block {
                return Err(StoreError::Invalid("cursor regression"));
            }
        }
        for raw in records {
            raw.validate()?;
            if raw.chain_id != cursor.chain_id || raw.source != cursor.source {
                return Err(StoreError::Invalid("batch and cursor source mismatch"));
            }
            let data = serde_json::to_vec(raw)?;
            let existing:Option<Vec<u8>>=tx.query_row("SELECT data FROM raw_records WHERE chain_id=?1 AND source=?2 AND run_id=?3 AND sequence=?4",params![raw.chain_id.to_string(),raw.source,raw.run_id,raw.sequence.to_string()],|r|r.get(0)).optional()?;
            if let Some(existing) = existing {
                if existing != data {
                    return Err(StoreError::Invalid("observation identity collision"));
                }
                continue;
            }
            tx.execute("INSERT INTO raw_records(chain_id,source,run_id,sequence,data,block_number) VALUES(?1,?2,?3,?4,?5,?6)",params![raw.chain_id.to_string(),raw.source,raw.run_id,raw.sequence.to_string(),data,raw.position.as_ref().map(|p|p.block_number.to_string())])?;
        }
        tx.execute("INSERT INTO source_cursors(chain_id,source,data) VALUES(?1,?2,?3) ON CONFLICT(chain_id,source) DO UPDATE SET data=excluded.data",params![cursor.chain_id.to_string(),cursor.source,serde_json::to_vec(cursor)?])?;
        if let Some(block) = cursor.next_block.checked_sub(1) {
            tx.execute("UPDATE ingest_gaps SET resolved=1 WHERE chain_id=?1 AND source=?2 AND block_number=?3",params![cursor.chain_id.to_string(),cursor.source,block.to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn cursor(&self, chain_id: u64, source: &str) -> Result<Option<SourceCursor>, StoreError> {
        let bytes: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT data FROM source_cursors WHERE chain_id=?1 AND source=?2",
                params![chain_id.to_string(), source],
                |r| r.get(0),
            )
            .optional()?;
        bytes
            .map(|b| serde_json::from_slice(&b).map_err(StoreError::from))
            .transpose()
    }

    pub fn raw_count(&self) -> Result<i64, StoreError> {
        Ok(self
            .connection
            .query_row("SELECT count(*) FROM raw_records", [], |r| r.get(0))?)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct StoredRaw {
    pub id: u64,
    pub record: RawRecord,
}
impl Store {
    pub fn read_raw_after(&self, id: u64, limit: usize) -> Result<Vec<StoredRaw>, StoreError> {
        self.read_raw_filtered_after(id, limit, None, None)
    }
    pub fn read_raw_filtered_after(
        &self,
        id: u64,
        limit: usize,
        chain: Option<u64>,
        source: Option<&str>,
    ) -> Result<Vec<StoredRaw>, StoreError> {
        if !(1..=1000).contains(&limit) {
            return Err(StoreError::Invalid("page limit must be 1..1000"));
        }
        let id =
            i64::try_from(id).map_err(|_| StoreError::Invalid("record id exceeds SQLite range"))?;
        let mut statement=self.connection.prepare("SELECT id,data,length(data) FROM raw_records WHERE id>?1 AND (?2 IS NULL OR chain_id=?2) AND (?3 IS NULL OR source=?3) ORDER BY id LIMIT ?4")?;
        let mut rows = statement.query(params![
            id,
            chain.map(|n| n.to_string()),
            source,
            limit as i64
        ])?;
        let mut result = Vec::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let size: i64 = row.get(2)?;
            if size > 64 * 1024 * 1024 {
                return Err(StoreError::Invalid("stored record exceeds read budget"));
            }
            if bytes + size > 64 * 1024 * 1024 {
                break;
            }
            let data: Vec<u8> = row.get(1)?;
            let record: RawRecord = serde_json::from_slice(&data)?;
            record.validate()?;
            let id: i64 = row.get(0)?;
            let id = u64::try_from(id).map_err(|_| StoreError::Invalid("negative record id"))?;
            result.push(StoredRaw { id, record });
            bytes += size;
        }
        Ok(result)
    }
}

impl Store {
    pub fn record_gap(
        &mut self,
        chain: u64,
        source: &str,
        block: u64,
        reason: &str,
    ) -> Result<(), StoreError> {
        self.connection.execute("INSERT INTO ingest_gaps(chain_id,source,block_number,reason) VALUES(?1,?2,?3,?4) ON CONFLICT(chain_id,source,block_number) DO UPDATE SET reason=excluded.reason,resolved=0",params![chain.to_string(),source,block.to_string(),reason])?;
        Ok(())
    }
}

impl Store {
    pub fn register_pools(
        &mut self,
        pools: &[arb_core::types::PoolDescriptor],
    ) -> Result<(), StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for pool in pools {
            tx.execute(
                "INSERT INTO pool_registry(key,data) VALUES(?1,?2) ON CONFLICT(key) DO NOTHING",
                params![serde_json::to_string(&pool.id)?, serde_json::to_vec(pool)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn read_pools(
        &self,
        after: Option<&arb_core::route::PoolId>,
        limit: usize,
    ) -> Result<Vec<arb_core::types::PoolDescriptor>, StoreError> {
        if !(1..=1000).contains(&limit) {
            return Err(StoreError::Invalid("page limit must be 1..1000"));
        }
        let key = after
            .map(serde_json::to_string)
            .transpose()?
            .unwrap_or_default();
        let mut statement = self
            .connection
            .prepare("SELECT data FROM pool_registry WHERE key>?1 ORDER BY key LIMIT ?2")?;
        let rows = statement.query_map(params![key, limit as i64], |r| r.get::<_, Vec<u8>>(0))?;
        rows.map(|row| serde_json::from_slice(&row?).map_err(StoreError::from))
            .collect()
    }
}

impl Store {
    pub fn save_bootstrap(
        &mut self,
        snapshot: &arb_core::protocol::Bootstrap,
    ) -> Result<u64, StoreError> {
        snapshot.validate()?;
        let bytes = serde_json::to_vec(snapshot)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(StoreError::Invalid("bootstrap exceeds storage budget"));
        }
        self.connection
            .execute("INSERT INTO bootstraps(data) VALUES(?1)", params![bytes])?;
        Ok(self.connection.last_insert_rowid() as u64)
    }
    pub fn load_bootstrap(&self, id: u64) -> Result<arb_core::protocol::Bootstrap, StoreError> {
        let id = i64::try_from(id).map_err(|_| StoreError::Invalid("bootstrap id"))?;
        let bytes: Vec<u8> = self.connection.query_row(
            "SELECT data FROM bootstraps WHERE id=?1 AND length(data)<=67108864",
            [id],
            |r| r.get(0),
        )?;
        let snapshot: arb_core::protocol::Bootstrap = serde_json::from_slice(&bytes)?;
        snapshot.validate()?;
        Ok(snapshot)
    }
}

impl Store {
    pub fn save_checkpoint(
        &mut self,
        checkpoint: &arb_core::checkpoint::Checkpoint,
    ) -> Result<u64, StoreError> {
        checkpoint.validate()?;
        let bytes = serde_json::to_vec(checkpoint)?;
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(StoreError::Invalid("checkpoint exceeds storage budget"));
        }
        self.connection
            .execute("INSERT INTO checkpoints(data) VALUES(?1)", params![bytes])?;
        Ok(self.connection.last_insert_rowid() as u64)
    }
    pub fn load_checkpoint(&self, id: u64) -> Result<arb_core::checkpoint::Checkpoint, StoreError> {
        let id = i64::try_from(id).map_err(|_| StoreError::Invalid("checkpoint id"))?;
        let bytes: Vec<u8> = self.connection.query_row(
            "SELECT data FROM checkpoints WHERE id=?1 AND length(data)<=67108864",
            [id],
            |r| r.get(0),
        )?;
        let checkpoint: arb_core::checkpoint::Checkpoint = serde_json::from_slice(&bytes)?;
        checkpoint.validate()?;
        Ok(checkpoint)
    }
}

impl Store {
    pub fn register_run(&mut self, run: &arb_core::research::RunSpec) -> Result<(), StoreError> {
        run.validate()?;
        let bytes = serde_json::to_vec(run)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Option<Vec<u8>> = tx
            .query_row(
                "SELECT data FROM research_runs WHERE run_id=?1",
                [&run.run_id],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing != bytes {
                return Err(StoreError::Invalid(
                    "run id reused with different parameters",
                ));
            }
        } else {
            tx.execute(
                "INSERT INTO research_runs(run_id,data) VALUES(?1,?2)",
                params![run.run_id, bytes],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
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
        let _: Vec<u8> = tx.query_row(
            "SELECT data FROM research_runs WHERE run_id=?1",
            [&block.run_id],
            |r| r.get(0),
        )?;
        for reference in &block.batch.raw_refs {
            let bytes:Vec<u8>=tx.query_row("SELECT data FROM raw_records WHERE chain_id=?1 AND source=?2 AND run_id=?3 AND sequence=?4",params![block.view.pools[0].descriptor.id.chain_id.to_string(),reference.source,reference.run_id,reference.sequence.to_string()],|r|r.get(0))?;
            let raw: RawRecord = serde_json::from_slice(&bytes)?;
            if raw.position.as_ref().is_none_or(|p| {
                p.block_hash != block.view.position.block_hash
                    || p.block_number != block.view.position.block_number
            }) {
                return Err(StoreError::Invalid("raw provenance block mismatch"));
            }
        }
        tx.execute(
            "INSERT INTO derived_blocks(run_id,block_hash,block_number,data) VALUES(?1,?2,?3,?4)",
            params![
                block.run_id,
                block.view.position.block_hash.to_string(),
                block.view.position.block_number.to_string(),
                bytes
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn find_derived(
        &self,
        run: &str,
        hash: alloy_primitives::B256,
    ) -> Result<Option<arb_core::research::DerivedBlock>, StoreError> {
        let row:Option<(Vec<u8>,bool)>=self.connection.query_row("SELECT data,canonical FROM derived_blocks WHERE run_id=?1 AND block_hash=?2 AND length(data)<=67108864",params![run,hash.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        row.map(|(bytes, canonical)| {
            let mut block: arb_core::research::DerivedBlock = serde_json::from_slice(&bytes)?;
            block.canonical = canonical;
            for candidate in &mut block.candidates {
                candidate.canonical = canonical;
            }
            Ok(block)
        })
        .transpose()
    }
}

impl Store {
    pub fn read_raw_block(
        &self,
        chain: u64,
        number: u64,
        after: u64,
    ) -> Result<Vec<StoredRaw>, StoreError> {
        let after = i64::try_from(after).map_err(|_| StoreError::Invalid("raw cursor"))?;
        let mut statement=self.connection.prepare("SELECT id,data,length(data) FROM raw_records WHERE chain_id=?1 AND block_number=?2 AND id>?3 ORDER BY id LIMIT 100")?;
        let mut rows = statement.query(params![chain.to_string(), number.to_string(), after])?;
        let mut result = vec![];
        let mut total = 0;
        while let Some(row) = rows.next()? {
            let size = usize::try_from(row.get::<_, i64>(2)?)
                .map_err(|_| StoreError::Invalid("negative raw size"))?;
            if size > 64 * 1024 * 1024 {
                return Err(StoreError::Invalid("raw block read budget"));
            }
            if total + size > 64 * 1024 * 1024 {
                break;
            }
            total += size;
            let data: Vec<u8> = row.get(1)?;
            let record: RawRecord = serde_json::from_slice(&data)?;
            record.validate()?;
            result.push(StoredRaw {
                id: row.get::<_, i64>(0)? as u64,
                record,
            });
        }
        Ok(result)
    }
}
impl Store {
    pub fn load_run(&self, id: &str) -> Result<arb_core::research::RunSpec, StoreError> {
        let bytes: Vec<u8> = self.connection.query_row(
            "SELECT data FROM research_runs WHERE run_id=?1",
            [id],
            |r| r.get(0),
        )?;
        let run: arb_core::research::RunSpec = serde_json::from_slice(&bytes)?;
        run.validate()?;
        Ok(run)
    }
}

impl Store {
    pub fn read_checkpoints_before(
        &self,
        before: u64,
    ) -> Result<Vec<(u64, arb_core::checkpoint::Checkpoint)>, StoreError> {
        let before = i64::try_from(before).map_err(|_| StoreError::Invalid("checkpoint cursor"))?;
        let mut statement = self.connection.prepare(
            "SELECT id,data,length(data) FROM checkpoints WHERE id<?1 ORDER BY id DESC LIMIT 100",
        )?;
        let mut rows = statement.query([before])?;
        let mut output = vec![];
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let size: i64 = row.get(2)?;
            if size > 67108864 {
                return Err(StoreError::Invalid("checkpoint budget"));
            }
            if bytes + size > 67108864 {
                break;
            }
            bytes += size;
            let data: Vec<u8> = row.get(1)?;
            let checkpoint: arb_core::checkpoint::Checkpoint = serde_json::from_slice(&data)?;
            checkpoint.validate()?;
            output.push((row.get::<_, i64>(0)? as u64, checkpoint));
        }
        Ok(output)
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
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let prior: Option<(Vec<u8>, String)> = tx
            .query_row(
                "SELECT data,status FROM recovery_jobs WHERE id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((existing, status)) = prior {
            if existing != data {
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
            tx.execute(
                "INSERT INTO recovery_jobs(id,run_id,data,status) VALUES(?1,?2,?3,'pending')",
                params![id.to_string(), run, data],
            )?;
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
        if self.connection.execute(
            "UPDATE recovery_jobs SET status='complete' WHERE id=?1",
            [id.to_string()],
        )? != 1
        {
            return Err(StoreError::Invalid("missing recovery job"));
        }
        Ok(())
    }
    pub fn recovery_data(&self, id: alloy_primitives::B256) -> Result<Vec<u8>, StoreError> {
        Ok(self.connection.query_row(
            "SELECT data FROM recovery_jobs WHERE id=?1 AND length(data)<=67108864",
            [id.to_string()],
            |r| r.get(0),
        )?)
    }
}

impl Store {
    pub fn insert_simulation(
        &mut self,
        record: &arb_core::simulation::SimulationRecord,
    ) -> Result<bool, StoreError> {
        let data = serde_json::to_vec(record)?;
        if data.len() > 67108864 {
            return Err(StoreError::Invalid("simulation record budget"));
        }
        Ok(self.connection.execute("INSERT INTO simulations(job_id,queue_id,run_id,block_hash,phase,canonical,validates_original,data) VALUES(?1,?2,?3,?4,?5,0,0,?6) ON CONFLICT(job_id) DO NOTHING",params![record.id.to_string(),record.queue_id,record.run_id,record.expected_position.block_hash.to_string(),format!("{:?}",record.phase),data])?==1)
    }
    pub fn save_simulation(
        &mut self,
        record: &mut arb_core::simulation::SimulationRecord,
    ) -> Result<(), StoreError> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let phase: String = tx.query_row(
            "SELECT phase FROM simulations WHERE job_id=?1",
            [record.id.to_string()],
            |r| r.get(0),
        )?;
        if phase == "Interrupted" {
            return Ok(());
        }
        let block: Option<(Vec<u8>, bool)> = tx
            .query_row(
                "SELECT data,canonical FROM derived_blocks WHERE run_id=?1 AND block_hash=?2",
                params![
                    record.run_id,
                    record.expected_position.block_hash.to_string()
                ],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        record.canonical = false;
        record.validates_original_candidate = false;
        if let Some((bytes, canonical)) = block {
            let block: arb_core::research::DerivedBlock = serde_json::from_slice(&bytes)?;
            let pending: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM recovery_jobs WHERE run_id=?1 AND status='pending')",
                [&record.run_id],
                |r| r.get(0),
            )?;
            if let Some(mut candidate) = block
                .candidates
                .into_iter()
                .find(|c| c.id == record.candidate_id && c.view_id == record.view_id)
            {
                candidate.canonical = canonical && !pending;
                record.canonical = candidate.canonical;
                record.validates_original_candidate =
                    record.result.as_ref().is_some_and(|result| {
                        arb_core::simulation::validates_candidate(&candidate, result)
                    });
            }
        }
        let data = serde_json::to_vec(record)?;
        if data.len() > 67108864 {
            return Err(StoreError::Invalid("simulation record budget"));
        }
        tx.execute("UPDATE simulations SET phase=?1,canonical=?2,validates_original=?3,data=?4 WHERE job_id=?5",params![format!("{:?}",record.phase),record.canonical,record.validates_original_candidate,data,record.id.to_string()])?;
        tx.commit()?;
        Ok(())
    }
    pub fn load_simulation(
        &self,
        id: alloy_primitives::B256,
    ) -> Result<arb_core::simulation::SimulationRecord, StoreError> {
        let (data,canonical,validates):(Vec<u8>,bool,bool)=self.connection.query_row("SELECT data,canonical,validates_original FROM simulations WHERE job_id=?1 AND length(data)<=67108864",[id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let mut record: arb_core::simulation::SimulationRecord = serde_json::from_slice(&data)?;
        record.canonical = canonical;
        record.validates_original_candidate = validates;
        Ok(record)
    }
    pub fn interrupt_simulations(&mut self, queue: &str, ended: u64) -> Result<(), StoreError> {
        loop {
            let records = {
                let mut statement=self.connection.prepare("SELECT data FROM simulations WHERE queue_id=?1 AND phase IN ('Queued','Running') LIMIT 100")?;
                statement
                    .query_map([queue], |r| r.get::<_, Vec<u8>>(0))?
                    .collect::<Result<Vec<_>, _>>()?
            };
            if records.is_empty() {
                break;
            }
            for bytes in records {
                let mut record: arb_core::simulation::SimulationRecord =
                    serde_json::from_slice(&bytes)?;
                record.phase = arb_core::simulation::SimulationPhase::Interrupted;
                record.outcome = arb_core::simulation::SimulationOutcome::Unknown;
                record.ended_at_ms = Some(ended);
                record.error = Some("queue stopped before completion".into());
                record.validates_original_candidate = false;
                self.connection.execute("UPDATE simulations SET phase='Interrupted',validates_original=0,data=?1 WHERE job_id=?2 AND phase IN ('Queued','Running')",params![serde_json::to_vec(&record)?,record.id.to_string()])?;
            }
        }
        Ok(())
    }
}

impl Store {
    pub fn feed_next_sequence(&self, chain: u64) -> Result<Option<u64>, StoreError> {
        let value: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT next_sequence FROM feed_cursors WHERE chain_id=?1",
                [chain.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        value
            .flatten()
            .map(|s| s.parse().map_err(|_| StoreError::Invalid("feed cursor")))
            .transpose()
    }
    pub fn append_feed(
        &mut self,
        packet: &crate::feed::FeedPacket,
    ) -> Result<crate::feed::FeedCommit, StoreError> {
        packet.raw.validate()?;
        if packet.raw.source != "feed"
            || packet.raw.position.is_some()
            || packet.raw.execution_status != arb_core::types::ExecutionStatus::Unknown
        {
            return Err(StoreError::Invalid("feed observation scope"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let chain = packet.raw.chain_id.to_string();
        let cursor: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT next_frame,next_sequence FROM feed_cursors WHERE chain_id=?1",
                [&chain],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (frame, mut next) = match cursor {
            Some((f, n)) => (
                f.parse::<u64>()
                    .map_err(|_| StoreError::Invalid("feed frame cursor"))?,
                n.map(|v| {
                    v.parse::<u64>()
                        .map_err(|_| StoreError::Invalid("feed sequence cursor"))
                })
                .transpose()?,
            ),
            None => (0, None),
        };
        let mut result = crate::feed::FeedCommit::default();
        for (sequence, digest) in &packet.messages {
            let prior: Option<Vec<u8>> = tx
                .query_row(
                    "SELECT digest FROM feed_seen WHERE chain_id=?1 AND sequence=?2",
                    params![chain, sequence.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(prior) = prior {
                if prior != digest.as_slice() {
                    return Err(StoreError::Invalid("feed sequence content conflict"));
                }
                result.duplicates += 1;
                continue;
            }
            if let Some(expected) = next
                && *sequence > expected
            {
                result.gaps.push((expected, sequence - 1));
                tx.execute("INSERT INTO feed_gaps(chain_id,first_sequence,last_sequence,reason) VALUES(?1,?2,?3,'relay did not supply requested sequence; recovery unverified')",params![chain,expected.to_string(),(sequence-1).to_string()])?;
            }
            next = Some(
                next.unwrap_or(0).max(
                    sequence
                        .checked_add(1)
                        .ok_or(StoreError::Invalid("feed sequence overflow"))?,
                ),
            );
            tx.execute(
                "INSERT INTO feed_seen(chain_id,sequence,digest) VALUES(?1,?2,?3)",
                params![chain, sequence.to_string(), digest.as_slice()],
            )?;
            result.new_messages += 1;
        }
        let mut raw = packet.raw.clone();
        raw.sequence = frame;
        tx.execute("INSERT INTO raw_records(chain_id,source,run_id,sequence,data) VALUES(?1,'feed',?2,?3,?4)",params![chain,raw.run_id,frame.to_string(),serde_json::to_vec(&raw)?])?;
        tx.execute("INSERT INTO feed_cursors(chain_id,next_frame,next_sequence) VALUES(?1,?2,?3) ON CONFLICT(chain_id) DO UPDATE SET next_frame=excluded.next_frame,next_sequence=excluded.next_sequence",params![chain,frame.checked_add(1).ok_or(StoreError::Invalid("feed frame overflow"))?.to_string(),next.map(|n|n.to_string())])?;
        tx.commit()?;
        Ok(result)
    }
}
