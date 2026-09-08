use super::*;
use arb_core::types::{ChainPosition, Confirmation, ExecutionStatus, Offset};

const RAW_COLUMNS: &str = "id,chain_id,source,run_id,sequence,version,received_at_ms,request_elapsed_ns,kind,block_number,block_hash,offset_kind,transaction_index,log_index,transaction_hash,execution_status,confirmation,payload,length(payload)";

fn integer(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Invalid("raw integer exceeds SQLite range"))
}

fn unsigned(row: &rusqlite::Row<'_>, name: &str) -> Result<u64, StoreError> {
    u64::try_from(row.get::<_, i64>(name)?).map_err(|_| StoreError::Invalid("negative raw integer"))
}

fn raw_blob<'a>(
    row: &'a rusqlite::Row<'_>,
    column: &str,
    budget: usize,
) -> Result<&'a [u8], StoreError> {
    let bytes = row
        .get_ref(column)?
        .as_blob()
        .map_err(|_| StoreError::Invalid("raw payload must be a BLOB"))?;
    if bytes.len() > budget {
        return Err(StoreError::Invalid("stored record exceeds read budget"));
    }
    Ok(bytes)
}

fn decode_raw(row: &rusqlite::Row<'_>) -> Result<RawRecord, StoreError> {
    let payload = raw_blob(row, "payload", 64 * 1024 * 1024)?;
    let block: Option<i64> = row.get("block_number")?;
    let hash: Option<String> = row.get("block_hash")?;
    let offset: Option<String> = row.get("offset_kind")?;
    let index: Option<i64> = row.get("transaction_index")?;
    let log: Option<i64> = row.get("log_index")?;
    let position = match (block, hash, offset.as_deref(), index, log) {
        (None, None, None, None, None) => None,
        (Some(block), Some(hash), Some(kind), index, log) => Some(ChainPosition {
            block_number: u64::try_from(block)
                .map_err(|_| StoreError::Invalid("negative raw block"))?,
            block_hash: hash
                .parse()
                .map_err(|_| StoreError::Invalid("invalid raw block hash"))?,
            offset: match (kind, index, log) {
                ("block_end", None, None) => Offset::BlockEnd,
                ("transaction", Some(index), log) => Offset::Transaction {
                    index: u64::try_from(index)
                        .map_err(|_| StoreError::Invalid("negative transaction index"))?,
                    log_index: log
                        .map(|n| {
                            u64::try_from(n).map_err(|_| StoreError::Invalid("negative log index"))
                        })
                        .transpose()?,
                },
                _ => return Err(StoreError::Invalid("invalid raw offset")),
            },
        }),
        _ => return Err(StoreError::Invalid("incomplete raw position")),
    };
    let record = RawRecord {
        version: u32::try_from(unsigned(row, "version")?)
            .map_err(|_| StoreError::Invalid("raw version overflow"))?,
        chain_id: row
            .get::<_, String>("chain_id")?
            .parse()
            .map_err(|_| StoreError::Invalid("invalid raw chain"))?,
        source: row.get("source")?,
        run_id: row.get("run_id")?,
        sequence: row
            .get::<_, String>("sequence")?
            .parse()
            .map_err(|_| StoreError::Invalid("invalid raw sequence"))?,
        received_at_ms: unsigned(row, "received_at_ms")?,
        request_elapsed_ns: row
            .get::<_, Option<i64>>("request_elapsed_ns")?
            .map(|n| u64::try_from(n).map_err(|_| StoreError::Invalid("negative raw duration")))
            .transpose()?,
        kind: row.get("kind")?,
        position,
        transaction_hash: row
            .get::<_, Option<String>>("transaction_hash")?
            .map(|v| {
                v.parse()
                    .map_err(|_| StoreError::Invalid("invalid raw transaction hash"))
            })
            .transpose()?,
        execution_status: match row.get::<_, String>("execution_status")?.as_str() {
            "unknown" => ExecutionStatus::Unknown,
            "succeeded" => ExecutionStatus::Succeeded,
            "reverted" => ExecutionStatus::Reverted,
            _ => return Err(StoreError::Invalid("invalid raw execution status")),
        },
        confirmation: match row.get::<_, String>("confirmation")?.as_str() {
            "unknown" => Confirmation::Unknown,
            "included" => Confirmation::Included,
            "safe" => Confirmation::Safe,
            "finalized" => Confirmation::Finalized,
            _ => return Err(StoreError::Invalid("invalid raw confirmation")),
        },
        payload: payload.to_vec(),
    };
    record.validate()?;
    Ok(record)
}

pub(super) fn insert_raw(
    connection: &Connection,
    id: Option<i64>,
    raw: &RawRecord,
) -> Result<(), StoreError> {
    raw.validate()?;
    let (block, hash, offset, index, log) = match &raw.position {
        None => (None, None, None, None, None),
        Some(p) => {
            let (kind, index, log) = match p.offset {
                Offset::BlockEnd => ("block_end", None, None),
                Offset::Transaction { index, log_index } => (
                    "transaction",
                    Some(integer(index)?),
                    log_index.map(integer).transpose()?,
                ),
            };
            (
                Some(sql_block_number(p.block_number)?),
                Some(p.block_hash.to_string()),
                Some(kind),
                index,
                log,
            )
        }
    };
    connection.execute("INSERT INTO raw_records(id,chain_id,source,run_id,sequence,version,received_at_ms,request_elapsed_ns,kind,block_number,block_hash,offset_kind,transaction_index,log_index,transaction_hash,execution_status,confirmation,payload) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)", params![
        id, raw.chain_id.to_string(), raw.source, raw.run_id, raw.sequence.to_string(), raw.version,
        integer(raw.received_at_ms)?, raw.request_elapsed_ns.map(integer).transpose()?, raw.kind,
        block, hash, offset, index, log, raw.transaction_hash.map(|h| h.to_string()),
        match raw.execution_status { ExecutionStatus::Unknown => "unknown", ExecutionStatus::Succeeded => "succeeded", ExecutionStatus::Reverted => "reverted" },
        match raw.confirmation { Confirmation::Unknown => "unknown", Confirmation::Included => "included", Confirmation::Safe => "safe", Confirmation::Finalized => "finalized" }, raw.payload,
    ])?;
    Ok(())
}

pub(super) fn find_raw(
    connection: &Connection,
    chain: u64,
    source: &str,
    run: &str,
    sequence: u64,
) -> Result<Option<RawRecord>, StoreError> {
    let mut statement = connection.prepare(&format!("SELECT {RAW_COLUMNS} FROM raw_records WHERE chain_id=?1 AND source=?2 AND run_id=?3 AND sequence=?4"))?;
    let mut rows = statement.query(params![
        chain.to_string(),
        source,
        run,
        sequence.to_string()
    ])?;
    rows.next()?.map(decode_raw).transpose()
}

pub(super) fn migrate(tx: &rusqlite::Transaction<'_>) -> Result<(), StoreError> {
    let mut statement = tx.prepare("SELECT id,chain_id,source,run_id,sequence,block_number,data FROM raw_records_v014 ORDER BY id")?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        // Legacy JSON encodes each payload byte as up to four bytes ("255,").
        let raw: RawRecord =
            serde_json::from_slice(raw_blob(row, "data", 4 * 64 * 1024 * 1024 + 4096)?)?;
        raw.validate()?;
        if row.get::<_, String>("chain_id")? != raw.chain_id.to_string()
            || row.get::<_, String>("source")? != raw.source
            || row.get::<_, String>("run_id")? != raw.run_id
            || row.get::<_, String>("sequence")? != raw.sequence.to_string()
            || row.get::<_, Option<i64>>("block_number")?
                != raw
                    .position
                    .as_ref()
                    .map(|p| sql_block_number(p.block_number))
                    .transpose()?
        {
            return Err(StoreError::Invalid("raw migration identity mismatch"));
        }
        insert_raw(tx, Some(row.get("id")?), &raw)?;
    }
    Ok(())
}

impl Store {
    pub fn append_raw(
        &mut self,
        records: &[RawRecord],
        cursor: &SourceCursor,
    ) -> Result<(), StoreError> {
        self.append_raw_with_pools(records, cursor, &[])
    }
    pub fn append_raw_with_pools(
        &mut self,
        records: &[RawRecord],
        cursor: &SourceCursor,
        pools: &[arb_core::types::PoolDescriptor],
    ) -> Result<(), StoreError> {
        if records.is_empty() || cursor.chain_id == 0 || cursor.source.is_empty() {
            return Err(StoreError::Invalid("empty batch or invalid cursor"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let previous: Option<i64> = tx
            .query_row(
                "SELECT next_block FROM source_cursors WHERE chain_id=?1 AND source=?2",
                params![cursor.chain_id.to_string(), cursor.source],
                |r| r.get(0),
            )
            .optional()?;
        let next_block = sql_block_number(cursor.next_block)?;
        if previous.is_some_and(|previous| previous > next_block) {
            return Err(StoreError::Invalid("cursor regression"));
        }
        for raw in records {
            raw.validate()?;
            if raw.chain_id != cursor.chain_id || raw.source != cursor.source {
                return Err(StoreError::Invalid("batch and cursor source mismatch"));
            }
            if let Some(existing) =
                find_raw(&tx, raw.chain_id, &raw.source, &raw.run_id, raw.sequence)?
            {
                if existing != *raw {
                    return Err(StoreError::Invalid("observation identity collision"));
                }
                continue;
            }
            insert_raw(&tx, None, raw)?;
        }
        for pool in pools {
            if pool.id.chain_id != cursor.chain_id {
                return Err(StoreError::Invalid("pool registry network"));
            }
            super::catalog::insert_pool(&tx, pool)?;
        }
        save_source_cursor(&tx, cursor)?;
        if let Some(block) = cursor.next_block.checked_sub(1) {
            tx.execute("UPDATE ingest_gaps SET resolved=1 WHERE chain_id=?1 AND source=?2 AND block_number=?3",params![cursor.chain_id.to_string(),cursor.source,sql_block_number(block)?])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn cursor(&self, chain_id: u64, source: &str) -> Result<Option<SourceCursor>, StoreError> {
        let row: Option<(i64, Option<String>)> = self
            .connection
            .query_row(
                "SELECT next_block,last_block_hash FROM source_cursors WHERE chain_id=?1 AND source=?2",
                params![chain_id.to_string(), source],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(next_block, hash)| {
            Ok(SourceCursor {
                chain_id,
                source: source.into(),
                next_block: u64::try_from(next_block)
                    .map_err(|_| StoreError::Invalid("negative cursor block"))?,
                last_block_hash: hash
                    .map(|hash| {
                        hash.parse()
                            .map_err(|_| StoreError::Invalid("invalid cursor block hash"))
                    })
                    .transpose()?,
            })
        })
        .transpose()
    }
    pub fn raw_count(&self) -> Result<i64, StoreError> {
        Ok(self
            .connection
            .query_row("SELECT count(*) FROM raw_records", [], |r| r.get(0))?)
    }
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
        let mut statement=self.connection.prepare(&format!("SELECT {RAW_COLUMNS} FROM raw_records WHERE id>?1 AND (?2 IS NULL OR chain_id=?2) AND (?3 IS NULL OR source=?3) ORDER BY id LIMIT ?4"))?;
        let mut rows = statement.query(params![
            id,
            chain.map(|n| n.to_string()),
            source,
            limit as i64
        ])?;
        let mut result = Vec::new();
        let mut bytes = 0;
        while let Some(row) = rows.next()? {
            let size: i64 = row.get("length(payload)")?;
            if size > 64 * 1024 * 1024 {
                return Err(StoreError::Invalid("stored record exceeds read budget"));
            }
            if bytes + size > 64 * 1024 * 1024 {
                break;
            }
            let record = decode_raw(row)?;
            let id: i64 = row.get(0)?;
            let id = u64::try_from(id).map_err(|_| StoreError::Invalid("negative record id"))?;
            result.push(StoredRaw { id, record });
            bytes += size;
        }
        Ok(result)
    }
    pub fn record_gap(
        &mut self,
        chain: u64,
        source: &str,
        block: u64,
        reason: &str,
    ) -> Result<(), StoreError> {
        self.connection.execute("INSERT INTO ingest_gaps(chain_id,source,block_number,reason) VALUES(?1,?2,?3,?4) ON CONFLICT(chain_id,source,block_number) DO UPDATE SET reason=excluded.reason,resolved=0",params![chain.to_string(),source,sql_block_number(block)?,reason])?;
        Ok(())
    }
    pub fn read_raw_block(
        &self,
        chain: u64,
        number: u64,
        after: u64,
    ) -> Result<Vec<StoredRaw>, StoreError> {
        let after = i64::try_from(after).map_err(|_| StoreError::Invalid("raw cursor"))?;
        let mut statement=self.connection.prepare(&format!("SELECT {RAW_COLUMNS} FROM raw_records WHERE chain_id=?1 AND block_number=?2 AND id>?3 ORDER BY id LIMIT 100"))?;
        let mut rows =
            statement.query(params![chain.to_string(), sql_block_number(number)?, after])?;
        let mut result = vec![];
        let mut total = 0;
        while let Some(row) = rows.next()? {
            let size = usize::try_from(row.get::<_, i64>("length(payload)")?)
                .map_err(|_| StoreError::Invalid("negative raw size"))?;
            if size > 64 * 1024 * 1024 {
                return Err(StoreError::Invalid("raw block read budget"));
            }
            if total + size > 64 * 1024 * 1024 {
                break;
            }
            total += size;
            let record = decode_raw(row)?;
            result.push(StoredRaw {
                id: unsigned(row, "id")?,
                record,
            });
        }
        Ok(result)
    }
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
        insert_raw(&tx, None, &raw)?;
        tx.execute("INSERT INTO feed_cursors(chain_id,next_frame,next_sequence) VALUES(?1,?2,?3) ON CONFLICT(chain_id) DO UPDATE SET next_frame=excluded.next_frame,next_sequence=excluded.next_sequence",params![chain,frame.checked_add(1).ok_or(StoreError::Invalid("feed frame overflow"))?.to_string(),next.map(|n|n.to_string())])?;
        tx.commit()?;
        Ok(result)
    }
}
