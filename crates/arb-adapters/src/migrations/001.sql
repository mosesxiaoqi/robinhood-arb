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
