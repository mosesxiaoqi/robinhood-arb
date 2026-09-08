-- schema: v004 -> v005
-- 当前完整结构：../schema/schema_v015.sql（v15）

CREATE TABLE checkpoints (id INTEGER PRIMARY KEY AUTOINCREMENT, data BLOB NOT NULL);
PRAGMA user_version = 5;
