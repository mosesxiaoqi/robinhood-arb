CREATE TABLE recovery_jobs (id TEXT PRIMARY KEY NOT NULL,run_id TEXT NOT NULL,data BLOB NOT NULL,status TEXT NOT NULL CHECK(status IN ('pending','complete')));
CREATE INDEX recovery_by_run ON recovery_jobs(run_id,status);
PRAGMA user_version = 8;
