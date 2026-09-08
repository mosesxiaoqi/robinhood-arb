-- schema: v011 -> v012
-- 当前完整结构：../schema/schema_v015.sql（v15）

CREATE TABLE runtime_checkpoints(run_id TEXT PRIMARY KEY,checkpoint_id INTEGER NOT NULL REFERENCES checkpoints(id));
CREATE TABLE runtime_status(run_id TEXT PRIMARY KEY,data BLOB NOT NULL);
PRAGMA user_version=12;
