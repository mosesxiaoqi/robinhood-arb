-- schema: v006 -> v007
-- 当前完整结构：../schema/schema_v015.sql（v15）

ALTER TABLE raw_records ADD COLUMN block_number TEXT;
CREATE INDEX raw_block_lookup ON raw_records(chain_id,block_number,id);
PRAGMA user_version = 7;
