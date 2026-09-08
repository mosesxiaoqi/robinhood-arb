-- schema: v013 -> v014
-- 当前完整结构：../schema/schema_v015.sql（v15）
-- 旧 JSON 的高度必须是非负整数且不超过 i64::MAX，否则整个升级回滚。

ALTER TABLE source_cursors RENAME TO source_cursors_v013;
CREATE TABLE source_cursors (
    chain_id TEXT NOT NULL,
    source TEXT NOT NULL,
    next_block INTEGER NOT NULL CHECK(typeof(next_block) = 'integer' AND next_block >= 0),
    last_block_hash TEXT CHECK(last_block_hash IS NULL OR (length(last_block_hash) = 66 AND substr(last_block_hash, 1, 2) = '0x' AND substr(last_block_hash, 3) NOT GLOB '*[^0-9a-fA-F]*')),
    PRIMARY KEY(chain_id, source)
);
INSERT INTO source_cursors(chain_id, source, next_block, last_block_hash)
SELECT chain_id, source,
       CASE WHEN json_type(CAST(data AS TEXT), '$.next_block') = 'integer'
            THEN json_extract(CAST(data AS TEXT), '$.next_block') END,
       json_extract(CAST(data AS TEXT), '$.last_block_hash')
FROM source_cursors_v013;
DROP TABLE source_cursors_v013;

PRAGMA user_version = 14;
