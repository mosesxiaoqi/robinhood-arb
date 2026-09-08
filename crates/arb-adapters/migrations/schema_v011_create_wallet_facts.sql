-- schema: v010 -> v011
-- 当前完整结构：../schema/schema_v015.sql（v15）

CREATE TABLE wallet_facts(id INTEGER PRIMARY KEY,run_id TEXT NOT NULL,transaction_hash TEXT NOT NULL,wallet TEXT NOT NULL,block_hash TEXT NOT NULL,data BLOB NOT NULL,UNIQUE(run_id,transaction_hash,wallet,block_hash));
CREATE INDEX wallet_run ON wallet_facts(run_id,id);
PRAGMA user_version=11;
