-- schema: v012 -> v013
-- 当前完整结构：../schema/schema_v015.sql（v15）
-- INTEGER 亲和性转换旧文本；CHECK 拒绝负数、非整数及溢出值，失败由外层事务回滚。

ALTER TABLE raw_records RENAME TO raw_records_v012;
CREATE TABLE raw_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chain_id TEXT NOT NULL,
    source TEXT NOT NULL,
    run_id TEXT NOT NULL,
    sequence TEXT NOT NULL,
    data BLOB NOT NULL,
    block_number INTEGER CHECK(block_number IS NULL OR (typeof(block_number) = 'integer' AND block_number >= 0)),
    UNIQUE(chain_id, source, run_id, sequence)
);
INSERT INTO raw_records SELECT * FROM raw_records_v012;
UPDATE sqlite_sequence SET seq = max(seq, coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'raw_records_v012'), 0)) WHERE name = 'raw_records';
DROP TABLE raw_records_v012;

ALTER TABLE ingest_gaps RENAME TO ingest_gaps_v012;
CREATE TABLE ingest_gaps (
    chain_id TEXT NOT NULL,
    source TEXT NOT NULL,
    block_number INTEGER NOT NULL CHECK(typeof(block_number) = 'integer' AND block_number >= 0),
    reason TEXT NOT NULL,
    resolved INTEGER NOT NULL DEFAULT 0 CHECK(resolved IN (0,1)),
    PRIMARY KEY(chain_id, source, block_number)
);
INSERT INTO ingest_gaps SELECT * FROM ingest_gaps_v012;
DROP TABLE ingest_gaps_v012;

ALTER TABLE derived_blocks RENAME TO derived_blocks_v012;
CREATE TABLE derived_blocks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL,
    block_hash TEXT NOT NULL,
    block_number INTEGER NOT NULL CHECK(typeof(block_number) = 'integer' AND block_number >= 0),
    canonical INTEGER NOT NULL DEFAULT 1,
    data BLOB NOT NULL,
    UNIQUE(run_id,block_hash)
);
INSERT INTO derived_blocks SELECT * FROM derived_blocks_v012;
UPDATE sqlite_sequence SET seq = max(seq, coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'derived_blocks_v012'), 0)) WHERE name = 'derived_blocks';
DROP TABLE derived_blocks_v012;

CREATE INDEX raw_block_lookup ON raw_records(chain_id,block_number,id);

PRAGMA user_version = 13;
