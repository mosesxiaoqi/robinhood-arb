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
        match version {
            0 => {
                let count: i64 = connection.query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table'",
                    [],
                    |r| r.get(0),
                )?;
                if count != 0 {
                    return Err(StoreError::Invalid("unversioned nonempty database"));
                }
                let tx = connection.transaction()?;
                tx.execute_batch(include_str!("migrations/001.sql"))?;
                tx.commit()?;
            }
            1 => {}
            _ => return Err(StoreError::Invalid("unsupported database version")),
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
