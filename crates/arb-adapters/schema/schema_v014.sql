-- 历史数据库结构（迁移测试使用，最新结构见 schema_v015.sql）：schema v14（PRAGMA user_version = 14）。
-- 适用于空数据库；已有数据库由 Store::open 按 ../migrations/schema_vNNN_*.sql 升级。

BEGIN;

-- 原始采集记录，按链、数据源、采集运行和序号去重。
CREATE TABLE raw_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT, -- 本地记录主键，由 SQLite 自动分配
    chain_id TEXT NOT NULL, -- 链 ID，以十进制 u64 文本保存
    source TEXT NOT NULL, -- 数据源标识，如 rpc、feed
    run_id TEXT NOT NULL, -- 采集运行标识，对应 RawRecord.run_id
    sequence TEXT NOT NULL, -- 来源内记录序号（十进制 u64）；Feed 使用本地帧序号
    data BLOB NOT NULL, -- RawRecord 的 JSON 字节，包含原始载荷及采集元数据
    block_number INTEGER CHECK(block_number IS NULL OR (typeof(block_number) = 'integer' AND block_number >= 0)), -- 所属区块高度（非负整数）；无链上位置时为 NULL，如 Feed 观察记录
    UNIQUE(chain_id, source, run_id, sequence)
);

-- 各链、各数据源的已提交采集进度，与原始记录在同一事务中更新。
CREATE TABLE source_cursors (
    chain_id TEXT NOT NULL, -- 链 ID，以十进制 u64 文本保存
    source TEXT NOT NULL, -- 数据源标识，如 rpc、feed
    next_block INTEGER NOT NULL CHECK(typeof(next_block) = 'integer' AND next_block >= 0), -- 下次采集的区块高度
    last_block_hash TEXT CHECK(last_block_hash IS NULL OR (length(last_block_hash) = 66 AND substr(last_block_hash, 1, 2) = '0x' AND substr(last_block_hash, 3) NOT GLOB '*[^0-9a-fA-F]*')), -- 上一个已提交区块的哈希；尚无记录时为空
    PRIMARY KEY(chain_id, source)
);

-- 按区块记录的采集缺口，补采提交后标记为已解决。
CREATE TABLE ingest_gaps (
    chain_id TEXT NOT NULL, -- 链 ID，以十进制 u64 文本保存
    source TEXT NOT NULL, -- 数据源标识，如 rpc、feed
    block_number INTEGER NOT NULL CHECK(typeof(block_number) = 'integer' AND block_number >= 0), -- 非负区块高度，最大值为 i64::MAX
    reason TEXT NOT NULL, -- 缺口原因说明
    resolved INTEGER NOT NULL DEFAULT 0 CHECK(resolved IN (0,1)), -- 是否已补齐：0 未解决，1 已解决
    PRIMARY KEY(chain_id, source, block_number)
);

-- 已发现流动性池的注册信息。
CREATE TABLE pool_registry (
    key TEXT PRIMARY KEY NOT NULL, -- PoolId 序列化后的 JSON 字符串，作为池唯一键
    data BLOB NOT NULL -- PoolDescriptor 的 JSON 字节，保存池描述信息
);

-- 固定区块位置的池初始状态快照及取数证据。
CREATE TABLE bootstraps (
    id INTEGER PRIMARY KEY AUTOINCREMENT, -- 本地记录主键，由 SQLite 自动分配
    data BLOB NOT NULL -- Bootstrap 的 JSON 字节，包含链上位置、池状态和请求响应证据
);

-- 可供恢复和重放使用的处理状态检查点。
CREATE TABLE checkpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT, -- 本地记录主键，由 SQLite 自动分配
    data BLOB NOT NULL -- Checkpoint 的 JSON 字节，包含状态、配置及算法版本和处理游标
);

-- 研究运行参数，同一运行标识不能复用为不同参数。
CREATE TABLE research_runs (
    run_id TEXT PRIMARY KEY NOT NULL, -- 研究运行标识，对应 research_runs.run_id
    data BLOB NOT NULL -- RunSpec 的 JSON 字节，包含配置哈希、版本、报价资产、金额及成本参数
);

-- 由原始记录推导的区块状态及候选机会，保留重组后的历史记录。
CREATE TABLE derived_blocks (
    id INTEGER PRIMARY KEY AUTOINCREMENT, -- 本地记录主键，由 SQLite 自动分配
    run_id TEXT NOT NULL, -- 研究运行标识，对应 research_runs.run_id
    block_hash TEXT NOT NULL, -- 区块哈希（十六进制文本）
    block_number INTEGER NOT NULL CHECK(typeof(block_number) = 'integer' AND block_number >= 0), -- 非负区块高度，最大值为 i64::MAX
    canonical INTEGER NOT NULL DEFAULT 1, -- 是否属于当前规范链：1 是，0 否；链重组时更新
    data BLOB NOT NULL, -- DerivedBlock 的 JSON 字节，包含区块批次、状态视图、候选及原始记录引用
    UNIQUE(run_id,block_hash)
);

CREATE INDEX raw_block_lookup ON raw_records(chain_id,block_number,id);

-- 链重组恢复任务及其持久化状态。
CREATE TABLE recovery_jobs (
    id TEXT PRIMARY KEY NOT NULL, -- 恢复任务标识（B256 的十六进制文本）
    run_id TEXT NOT NULL, -- 研究运行标识，对应 research_runs.run_id
    data BLOB NOT NULL, -- 调用方传入的恢复计划字节，当前业务使用 RecoveryPlan 的 JSON
    status TEXT NOT NULL CHECK(status IN ('pending','complete')) -- 恢复状态：pending 待完成，complete 已完成
);

CREATE INDEX recovery_by_run ON recovery_jobs(run_id,status);

-- 候选机会的模拟任务和结果，按任务标识去重。
CREATE TABLE simulations (
    id INTEGER PRIMARY KEY AUTOINCREMENT, -- 本地记录主键，由 SQLite 自动分配
    job_id TEXT NOT NULL UNIQUE, -- 模拟任务唯一标识（B256 的十六进制文本）
    queue_id TEXT NOT NULL, -- 任务所属模拟队列标识
    run_id TEXT NOT NULL, -- 研究运行标识，对应 research_runs.run_id
    block_hash TEXT NOT NULL, -- 模拟预期执行位置的区块哈希（十六进制文本）
    phase TEXT NOT NULL, -- SimulationPhase 枚举的调试名称，如 Queued、Running、Interrupted
    canonical INTEGER NOT NULL, -- 关联候选当前是否有效地属于规范链：1 是，0 否；恢复期间可为 0
    validates_original INTEGER NOT NULL, -- 模拟结果是否验证原始候选：1 是，0 否；由业务校验逻辑计算
    data BLOB NOT NULL -- SimulationRecord 的 JSON 字节，包含候选引用、预期位置及模拟结果
);

CREATE INDEX simulations_by_run ON simulations(run_id,id);

CREATE INDEX simulations_by_queue ON simulations(queue_id,phase);

-- 每条链的 Feed 本地帧进度和消息序号进度。
CREATE TABLE feed_cursors (
    chain_id TEXT PRIMARY KEY, -- 链 ID，以十进制 u64 文本保存
    next_frame TEXT NOT NULL, -- 下一条待保存的本地帧序号（十进制 u64），写入 raw_records.sequence
    next_sequence TEXT -- 下一条期望收到的 Feed 消息序号（十进制 u64）；尚未知时为 NULL
);

-- 已接收 Feed 消息的摘要，用于去重和检测同序号内容冲突。
CREATE TABLE feed_seen (
    chain_id TEXT NOT NULL, -- 链 ID，以十进制 u64 文本保存
    sequence TEXT NOT NULL, -- Feed 消息序号（十进制 u64），区别于本地帧序号
    digest BLOB NOT NULL, -- 消息编码内容的 Keccak-256 摘要（32 字节）
    PRIMARY KEY(chain_id,sequence)
);

-- Feed 消息序号缺口历史，当前不记录是否已补齐。
CREATE TABLE feed_gaps (
    id INTEGER PRIMARY KEY, -- 本地记录主键，由 SQLite 自动分配
    chain_id TEXT NOT NULL, -- 链 ID，以十进制 u64 文本保存
    first_sequence TEXT NOT NULL, -- 缺失消息的起始序号（十进制 u64，包含此序号）
    last_sequence TEXT NOT NULL, -- 缺失消息的结束序号（十进制 u64，包含此序号）
    reason TEXT NOT NULL -- 缺口原因及恢复验证情况说明
);

-- 特定交易、钱包和区块位置上的钱包事实与归因证据。
CREATE TABLE wallet_facts (
    id INTEGER PRIMARY KEY, -- 本地记录主键，由 SQLite 自动分配
    run_id TEXT NOT NULL, -- 研究运行标识，对应 research_runs.run_id
    transaction_hash TEXT NOT NULL, -- 交易哈希（十六进制文本）
    wallet TEXT NOT NULL, -- 钱包地址（十六进制文本）
    block_hash TEXT NOT NULL, -- 区块哈希（十六进制文本）
    data BLOB NOT NULL, -- WalletFacts 的 JSON 字节，包含链上位置和钱包证据
    UNIQUE(run_id,transaction_hash,wallet,block_hash)
);

CREATE INDEX wallet_run ON wallet_facts(run_id,id);

-- 每次研究运行当前用于恢复的检查点关联。
CREATE TABLE runtime_checkpoints (
    run_id TEXT PRIMARY KEY, -- 研究运行标识，对应 research_runs.run_id
    checkpoint_id INTEGER NOT NULL REFERENCES checkpoints(id) -- 关联 checkpoints.id 的检查点编号
);

-- 每次研究运行最近保存的运行状态。
CREATE TABLE runtime_status (
    run_id TEXT PRIMARY KEY, -- 研究运行标识，对应 research_runs.run_id
    data BLOB NOT NULL -- 运行状态的 JSON 字节，由调用方提供 serde_json::Value
);

PRAGMA user_version = 14;

COMMIT;
