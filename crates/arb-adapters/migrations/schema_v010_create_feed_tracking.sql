-- schema: v009 -> v010
-- 当前完整结构：../schema/schema_v015.sql（v15）

CREATE TABLE feed_cursors(chain_id TEXT PRIMARY KEY, next_frame TEXT NOT NULL, next_sequence TEXT);
CREATE TABLE feed_seen(chain_id TEXT NOT NULL, sequence TEXT NOT NULL, digest BLOB NOT NULL, PRIMARY KEY(chain_id,sequence));
CREATE TABLE feed_gaps(id INTEGER PRIMARY KEY, chain_id TEXT NOT NULL, first_sequence TEXT NOT NULL, last_sequence TEXT NOT NULL, reason TEXT NOT NULL);
PRAGMA user_version=10;
