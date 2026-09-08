-- schema: v002 -> v003
-- 当前完整结构：../schema/schema_v015.sql（v15）

CREATE TABLE pool_registry (key TEXT PRIMARY KEY NOT NULL, data BLOB NOT NULL);
PRAGMA user_version = 3;
