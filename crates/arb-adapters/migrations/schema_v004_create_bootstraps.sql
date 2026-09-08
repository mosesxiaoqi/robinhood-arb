-- schema: v003 -> v004
-- 当前完整结构：../schema/schema_v015.sql（v15）

CREATE TABLE bootstraps (id INTEGER PRIMARY KEY AUTOINCREMENT, data BLOB NOT NULL);
PRAGMA user_version = 4;
