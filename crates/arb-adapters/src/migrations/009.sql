CREATE TABLE simulations (id INTEGER PRIMARY KEY AUTOINCREMENT,job_id TEXT NOT NULL UNIQUE,queue_id TEXT NOT NULL,run_id TEXT NOT NULL,block_hash TEXT NOT NULL,phase TEXT NOT NULL,canonical INTEGER NOT NULL,validates_original INTEGER NOT NULL,data BLOB NOT NULL);
CREATE INDEX simulations_by_run ON simulations(run_id,id);
CREATE INDEX simulations_by_queue ON simulations(queue_id,phase);
PRAGMA user_version = 9;
