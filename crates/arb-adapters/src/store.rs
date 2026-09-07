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
            let tx = connection.transaction()?;
            for migration in &migrations[version as usize..] {
                tx.execute_batch(migration)?;
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
        let tx = self.connection.transaction()?;
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
            tx.execute("INSERT INTO raw_records(chain_id,source,run_id,sequence,data) VALUES(?1,?2,?3,?4,?5)",params![raw.chain_id.to_string(),raw.source,raw.run_id,raw.sequence.to_string(),data])?;
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
        let tx = self.connection.transaction()?;
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
