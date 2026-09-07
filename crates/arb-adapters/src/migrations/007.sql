ALTER TABLE raw_records ADD COLUMN block_number TEXT;
CREATE INDEX raw_block_lookup ON raw_records(chain_id,block_number,id);
PRAGMA user_version = 7;
