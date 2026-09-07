CREATE TABLE ingest_gaps (
    chain_id TEXT NOT NULL,
    source TEXT NOT NULL,
    block_number TEXT NOT NULL,
    reason TEXT NOT NULL,
    resolved INTEGER NOT NULL DEFAULT 0 CHECK(resolved IN (0,1)),
    PRIMARY KEY(chain_id, source, block_number)
);
PRAGMA user_version = 2;
