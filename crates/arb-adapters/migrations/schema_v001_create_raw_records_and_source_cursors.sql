-- schema: v000 -> v001
-- 当前完整结构：../schema/schema_v015.sql（v15）

CREATE TABLE raw_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    chain_id TEXT NOT NULL,
    source TEXT NOT NULL,
    run_id TEXT NOT NULL,
    sequence TEXT NOT NULL,
    data BLOB NOT NULL,
    UNIQUE(chain_id, source, run_id, sequence)
);
CREATE TABLE source_cursors (
    chain_id TEXT NOT NULL,
    source TEXT NOT NULL,
    data BLOB NOT NULL,
    PRIMARY KEY(chain_id, source)
);
PRAGMA user_version = 1;
