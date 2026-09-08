-- schema: v005 -> v006
-- 当前完整结构：../schema/schema_v015.sql（v15）

CREATE TABLE research_runs (run_id TEXT PRIMARY KEY NOT NULL, data BLOB NOT NULL);
CREATE TABLE derived_blocks (id INTEGER PRIMARY KEY AUTOINCREMENT,run_id TEXT NOT NULL,block_hash TEXT NOT NULL,block_number TEXT NOT NULL,canonical INTEGER NOT NULL DEFAULT 1,data BLOB NOT NULL,UNIQUE(run_id,block_hash));
PRAGMA user_version = 6;
