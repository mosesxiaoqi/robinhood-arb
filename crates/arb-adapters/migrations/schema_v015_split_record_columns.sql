-- schema: v014 -> v015
-- 当前完整结构：../schema/schema_v015.sql（v15）
-- 必须由 Store::open 执行：本 SQL 建表，store/migration.rs 使用强类型模型转换旧 JSON、清理旧表并设置版本；全程同一事务。

DROP INDEX raw_block_lookup;
DROP INDEX recovery_by_run;
DROP INDEX simulations_by_run;
DROP INDEX simulations_by_queue;
DROP INDEX wallet_run;

ALTER TABLE runtime_checkpoints RENAME TO runtime_checkpoints_v014;
ALTER TABLE raw_records RENAME TO raw_records_v014;
ALTER TABLE pool_registry RENAME TO pool_registry_v014;
ALTER TABLE bootstraps RENAME TO bootstraps_v014;
ALTER TABLE checkpoints RENAME TO checkpoints_v014;
ALTER TABLE research_runs RENAME TO research_runs_v014;
ALTER TABLE derived_blocks RENAME TO derived_blocks_v014;
ALTER TABLE recovery_jobs RENAME TO recovery_jobs_v014;
ALTER TABLE runtime_status RENAME TO runtime_status_v014;
ALTER TABLE simulations RENAME TO simulations_v014;
ALTER TABLE wallet_facts RENAME TO wallet_facts_v014;

-- 原始观测：元数据分列保存，网络原始字节保留为 BLOB。
CREATE TABLE raw_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK(id > 0), -- 本地递增记录编号。
    chain_id TEXT NOT NULL CHECK(length(chain_id) BETWEEN 1 AND 20 AND chain_id NOT GLOB '*[^0-9]*' AND substr(chain_id,1,1) != '0' AND (length(chain_id)<20 OR chain_id<='18446744073709551615')), -- 链编号，十进制文本保留完整 u64。
    source TEXT NOT NULL CHECK(length(source) BETWEEN 1 AND 128), -- 采集源名称。
    run_id TEXT NOT NULL CHECK(length(run_id) BETWEEN 1 AND 128), -- 采集进程运行编号。
    sequence TEXT NOT NULL CHECK(length(sequence) BETWEEN 1 AND 20 AND sequence NOT GLOB '*[^0-9]*' AND (sequence='0' OR substr(sequence,1,1)!='0') AND (length(sequence)<20 OR sequence<='18446744073709551615')), -- 源内序号，十进制文本保留完整 u64。
    version INTEGER NOT NULL CHECK(version=1), -- 原始记录格式版本。
    received_at_ms INTEGER NOT NULL CHECK(received_at_ms>=0), -- 接收时间，毫秒。
    request_elapsed_ns INTEGER CHECK(request_elapsed_ns>=0), -- RPC 请求单调耗时，纳秒，可空。
    kind TEXT NOT NULL CHECK(length(kind) BETWEEN 1 AND 128), -- 原始观测类型。
    block_number INTEGER CHECK(block_number>=0), -- 区块高度，无链位置时为空。
    block_hash TEXT CHECK(length(block_hash)=66 AND substr(block_hash,1,2)='0x' AND substr(block_hash,3) NOT GLOB '*[^0-9a-f]*'), -- 区块哈希。
    offset_kind TEXT CHECK(offset_kind IN ('block_end','transaction')), -- 区块末尾或交易位置。
    transaction_index INTEGER CHECK(transaction_index>=0), -- 区块内交易索引。
    log_index INTEGER CHECK(log_index>=0), -- 交易日志索引，可空。
    transaction_hash TEXT CHECK(length(transaction_hash)=66 AND substr(transaction_hash,1,2)='0x' AND substr(transaction_hash,3) NOT GLOB '*[^0-9a-f]*'), -- 交易哈希，可空。
    execution_status TEXT NOT NULL CHECK(execution_status IN ('unknown','succeeded','reverted')), -- 交易执行状态。
    confirmation TEXT NOT NULL CHECK(confirmation IN ('unknown','included','safe','finalized')), -- 链上确认状态。
    payload BLOB NOT NULL CHECK(typeof(payload)='blob' AND length(payload)<=67108864), -- 原始网络载荷，不作 JSON 重编码。
    UNIQUE(chain_id,source,run_id,sequence),
    CHECK((block_number IS NULL AND block_hash IS NULL AND offset_kind IS NULL AND transaction_index IS NULL AND log_index IS NULL)
       OR (block_number IS NOT NULL AND block_hash IS NOT NULL AND offset_kind IS NOT NULL AND
           ((offset_kind='block_end' AND transaction_index IS NULL AND log_index IS NULL)
            OR (offset_kind='transaction' AND transaction_index IS NOT NULL)))),
    CHECK(execution_status='unknown' OR transaction_hash IS NOT NULL),
    CHECK(confirmation='unknown' OR block_number IS NOT NULL)
);
-- 按链、区块和游标读取原始观测。
CREATE INDEX raw_block_lookup ON raw_records(chain_id,block_number,id);


-- 池注册表：标量字段为权威数据，key 保留原有复合身份的规范 JSON 排序以兼容分页。
CREATE TABLE pool_registry (
    key TEXT PRIMARY KEY NOT NULL, -- 原有池身份分页键
    chain_id TEXT NOT NULL, -- 链 ID，无符号十进制
    locator_kind TEXT NOT NULL CHECK(locator_kind IN ('contract','singleton')), -- 定位类型
    address TEXT, -- 独立池合约地址
    manager TEXT, -- 单例管理合约地址
    pool_id TEXT, -- 单例池哈希
    protocol TEXT NOT NULL, -- 协议标识
    token TEXT NOT NULL, -- 交易代币地址
    quote_asset TEXT NOT NULL, -- 计价资产地址
    currency0 TEXT NOT NULL, -- 第一币种
    currency1 TEXT NOT NULL, -- 第二币种
    hook TEXT NOT NULL, -- 钩子合约
    lp_fee INTEGER NOT NULL CHECK(lp_fee BETWEEN 0 AND 4294967295), -- 流动性提供者费率
    tick_spacing INTEGER NOT NULL CHECK(tick_spacing BETWEEN -2147483648 AND 2147483647), -- 刻度间距
    hook_fee_bps INTEGER CHECK(hook_fee_bps BETWEEN 0 AND 65535), -- 钩子费率，基点
    creator_tax_bps INTEGER CHECK(creator_tax_bps BETWEEN 0 AND 65535), -- 创建者税率，基点
    token_decimals INTEGER CHECK(token_decimals BETWEEN 0 AND 255), -- 代币精度
    quote_decimals INTEGER CHECK(quote_decimals BETWEEN 0 AND 255), -- 计价币精度
    initialized_block_number INTEGER NOT NULL CHECK(initialized_block_number>=0), -- 初始化区块高度
    initialized_block_hash TEXT NOT NULL, -- 初始化区块哈希
    initialized_offset_kind TEXT NOT NULL CHECK(initialized_offset_kind IN ('block_end','transaction')), -- 初始化位置类型
    initialized_transaction_index TEXT, -- 初始化交易序号，无符号十进制
    initialized_log_index TEXT, -- 初始化日志序号，无符号十进制
    verification_status TEXT NOT NULL CHECK(verification_status IN ('pending','supported','unsupported')), -- 验证状态
    verification_reason TEXT, -- 不支持原因
    CHECK((locator_kind='contract' AND address IS NOT NULL AND manager IS NULL AND pool_id IS NULL) OR (locator_kind='singleton' AND address IS NULL AND manager IS NOT NULL AND pool_id IS NOT NULL)),
    CHECK((initialized_offset_kind='block_end' AND initialized_transaction_index IS NULL AND initialized_log_index IS NULL) OR (initialized_offset_kind='transaction' AND initialized_transaction_index IS NOT NULL)),
    CHECK((verification_status='unsupported' AND verification_reason IS NOT NULL) OR (verification_status!='unsupported' AND verification_reason IS NULL))
);
-- 启动快照：区块末尾位置独立存储，池状态与原始证据保留结构化集合。
CREATE TABLE bootstraps (
    id INTEGER PRIMARY KEY AUTOINCREMENT, -- 快照 ID
    version INTEGER NOT NULL CHECK(version=1), -- 快照格式版本
    block_number INTEGER NOT NULL CHECK(block_number>=0), -- 区块末尾高度
    block_hash TEXT NOT NULL, -- 区块哈希
    pools_json TEXT NOT NULL CHECK(json_valid(pools_json)), -- 池状态列表 JSON
    evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json)) -- 请求及回复证据列表 JSON
);
-- 检查点：恢复元数据与处理游标可直接查询，嵌套状态单独保存。
CREATE TABLE checkpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT, -- 检查点 ID
    research_run_id TEXT, -- 所属研究运行
    version INTEGER NOT NULL CHECK(version=1), -- 检查点格式版本
    registry_version TEXT NOT NULL, -- 注册表版本，无符号十进制
    config_hash TEXT NOT NULL, -- 配置哈希
    algorithm_version TEXT NOT NULL, -- 算法版本
    last_raw_id TEXT NOT NULL, -- 最后处理的原始记录 ID，无符号十进制
    next_block INTEGER NOT NULL CHECK(next_block>=0), -- 下一个待处理区块
    state_json TEXT NOT NULL CHECK(json_valid(state_json)) -- 嵌套池状态 JSON
);
-- 研究运行参数：数额以十进制文本保存，避免 256 位整数精度丢失。
CREATE TABLE research_runs (
    run_id TEXT PRIMARY KEY NOT NULL, -- 运行 ID
    config_hash TEXT NOT NULL, -- 配置哈希
    algorithm_version TEXT NOT NULL, -- 算法版本
    registry_version TEXT NOT NULL, -- 注册表版本，无符号十进制
    quote_asset TEXT NOT NULL, -- 计价资产地址
    amounts_json TEXT NOT NULL CHECK(json_valid(amounts_json)), -- 输入数额列表 JSON
    min_depth TEXT NOT NULL, -- 最小深度，256 位无符号十进制
    min_profit TEXT NOT NULL, -- 最小利润，256 位无符号十进制
    cost_asset TEXT NOT NULL, -- 成本资产地址
    cost_amount TEXT, -- 成本数额，256 位无符号十进制
    cost_basis TEXT NOT NULL, -- 成本估算依据
    cost_conversion_json TEXT NOT NULL CHECK(json_valid(cost_conversion_json)) -- 可选兑换报价及链上位置 JSON
);


-- 派生区块及研究结果；固定元数据独立列，变长池状态、证据和候选集合保留 JSON。
CREATE TABLE derived_blocks (
    id INTEGER PRIMARY KEY AUTOINCREMENT, -- 本地记录编号
    run_id TEXT NOT NULL, -- 所属研究运行
    block_hash TEXT NOT NULL, -- 区块哈希
    block_number INTEGER NOT NULL CHECK(typeof(block_number) = 'integer' AND block_number >= 0), -- 区块高度
    canonical INTEGER NOT NULL DEFAULT 1 CHECK(canonical IN (0,1)), -- 是否位于当前规范链
    view_id TEXT NOT NULL, -- 状态视图标识
    parent_hash TEXT NOT NULL, -- 父区块哈希；位置固定为区块结束
    excluded_pools INTEGER NOT NULL CHECK(excluded_pools >= 0), -- 被排除的池数量
    clock_regressions INTEGER CHECK(clock_regressions >= 0), -- 检出的时钟回退次数；无时间质量信息时为空
    multiple_clock_domains INTEGER CHECK(multiple_clock_domains IN (0,1)), -- 是否跨时钟域；无时间质量信息时为空
    covered_pools_json TEXT NOT NULL CHECK(json_valid(covered_pools_json)), -- 区块覆盖的池标识列表
    observations_json TEXT NOT NULL CHECK(json_valid(observations_json)), -- 区块观察事件列表
    raw_refs_json TEXT NOT NULL CHECK(json_valid(raw_refs_json)), -- 区块批次引用的原始记录
    pools_json TEXT NOT NULL CHECK(json_valid(pools_json)), -- 状态视图中的池状态集合
    view_raw_refs_json TEXT NOT NULL CHECK(json_valid(view_raw_refs_json)), -- 状态视图引用的原始记录
    candidates_json TEXT NOT NULL CHECK(json_valid(candidates_json)), -- 候选机会集合；读取时以 canonical 列更新其有效性
    exclusions_json TEXT NOT NULL CHECK(json_valid(exclusions_json)), -- 路由排除原因集合
    timings_json TEXT NOT NULL CHECK(json_valid(timings_json)), -- 各阶段计时列表
    CHECK((clock_regressions IS NULL) = (multiple_clock_domains IS NULL)),
    UNIQUE(run_id,block_hash)
);

-- 持久化链重组恢复计划，保留变长重放批次和孤块列表。
CREATE TABLE recovery_jobs (
    id TEXT PRIMARY KEY NOT NULL, -- 恢复计划哈希标识
    run_id TEXT NOT NULL, -- 所属研究运行
    checkpoint_id INTEGER NOT NULL CHECK(checkpoint_id >= 0), -- 恢复起点的检查点编号
    ancestor_block_number INTEGER NOT NULL CHECK(ancestor_block_number >= 0), -- 共同祖先区块高度
    ancestor_block_hash TEXT NOT NULL, -- 共同祖先区块哈希；位置固定为区块结束
    fetch_from INTEGER CHECK(fetch_from >= 0), -- 需要补采的起始高度，可空
    fetch_to INTEGER CHECK(fetch_to >= 0), -- 需要补采的结束高度，可空
    orphan_hashes_json TEXT NOT NULL CHECK(json_valid(orphan_hashes_json)), -- 被移出规范链的区块哈希列表
    replay_blocks_json TEXT NOT NULL CHECK(json_valid(replay_blocks_json)), -- 恢复时重放的区块批次
    status TEXT NOT NULL CHECK(status IN ('pending','complete')), -- 待完成或已完成
    CHECK((fetch_from IS NULL AND fetch_to IS NULL) OR (fetch_from IS NOT NULL AND fetch_to IS NOT NULL AND fetch_from <= fetch_to))
);
CREATE INDEX recovery_by_run ON recovery_jobs(run_id,status);

-- 每次研究运行最近一次保存的汇总状态。
CREATE TABLE runtime_status (
    run_id TEXT PRIMARY KEY, -- 研究运行标识
    status TEXT NOT NULL CHECK(status IN ('Running','AwaitingPools','DataGapPaused','DiskPaused','Stopped','WindowEnded','Failed')), -- 当前运行状态
    started_at_ms INTEGER NOT NULL CHECK(started_at_ms >= 0), -- 开始时间，Unix 毫秒
    ended_at_ms INTEGER NOT NULL CHECK(ended_at_ms >= 0), -- 结束时间，Unix 毫秒
    collected_blocks INTEGER NOT NULL CHECK(collected_blocks >= 0), -- 已采集区块数
    processed_blocks INTEGER NOT NULL CHECK(processed_blocks >= 0), -- 已处理区块数
    first_collected INTEGER CHECK(first_collected >= 0), -- 本次首个采集区块高度，可空
    last_collected INTEGER CHECK(last_collected >= 0), -- 本次最后采集区块高度，可空
    disk_start INTEGER NOT NULL CHECK(disk_start >= 0), -- 开始时数据库及关联文件占用，字节
    disk_end INTEGER NOT NULL CHECK(disk_end >= 0), -- 结束时数据库及关联文件占用，字节
    max_processing_ns INTEGER NOT NULL CHECK(max_processing_ns >= 0), -- 最大单区块处理耗时，纳秒
    error TEXT -- 错误说明，成功时为空
);


-- 模拟任务、生命周期与执行结果。
CREATE TABLE simulations (
    id INTEGER PRIMARY KEY AUTOINCREMENT, -- 记录序号
    job_id TEXT NOT NULL UNIQUE, -- 任务标识
    queue_id TEXT NOT NULL, -- 队列标识
    candidate_id TEXT NOT NULL, -- 候选标识
    run_id TEXT NOT NULL, -- 运行标识
    view_id TEXT NOT NULL, -- 视图标识
    block_number INTEGER NOT NULL CHECK(block_number>=0), -- 预期区块高度
    block_hash TEXT NOT NULL, -- 预期区块哈希
    offset_kind TEXT NOT NULL CHECK(offset_kind IN ('BlockEnd','Transaction')), -- 预期位置类型
    transaction_index INTEGER CHECK(transaction_index>=0), -- 预期交易索引
    log_index INTEGER CHECK(log_index>=0), -- 预期日志索引
    request_json TEXT NOT NULL CHECK(json_valid(request_json)), -- 嵌套模拟请求
    phase TEXT NOT NULL CHECK(phase IN ('Queued','Running','Finished','QueueFull','QueueExpired','Interrupted')), -- 任务阶段
    outcome TEXT NOT NULL CHECK(outcome IN ('Succeeded','Reverted','Unavailable','Unknown')), -- 模拟结果状态
    submitted_at_ms INTEGER NOT NULL CHECK(submitted_at_ms>=0), -- 提交时间毫秒
    queued_at_ms INTEGER CHECK(queued_at_ms>=0), -- 入队时间毫秒
    started_at_ms INTEGER CHECK(started_at_ms>=0), -- 开始时间毫秒
    ended_at_ms INTEGER CHECK(ended_at_ms>=0), -- 结束时间毫秒
    queue_wait_ns INTEGER CHECK(queue_wait_ns>=0), -- 排队耗时纳秒
    elapsed_ns INTEGER CHECK(elapsed_ns>=0), -- 执行耗时纳秒
    result_json TEXT CHECK(result_json IS NULL OR json_valid(result_json)), -- 嵌套模拟执行结果
    error TEXT, -- 错误说明
    error_evidence_json TEXT NOT NULL CHECK(json_valid(error_evidence_json)), -- 错误证据列表
    canonical INTEGER NOT NULL CHECK(canonical IN (0,1)), -- 当前规范链状态
    validates_original INTEGER NOT NULL CHECK(validates_original IN (0,1)), -- 是否验证原始候选
    CHECK((offset_kind='BlockEnd' AND transaction_index IS NULL AND log_index IS NULL) OR (offset_kind='Transaction' AND transaction_index IS NOT NULL))
);
CREATE INDEX simulations_by_run ON simulations(run_id,id);
CREATE INDEX simulations_by_queue ON simulations(queue_id,phase);
-- 钱包交易事实与证据。
CREATE TABLE wallet_facts (
    id INTEGER PRIMARY KEY, -- 记录序号
    run_id TEXT NOT NULL, -- 运行标识
    chain_id TEXT NOT NULL CHECK(length(chain_id) BETWEEN 1 AND 20 AND chain_id NOT GLOB '*[^0-9]*' AND substr(chain_id,1,1) != '0' AND (length(chain_id)<20 OR chain_id<='18446744073709551615')), -- 链标识，十进制文本保留完整 u64
    block_number INTEGER NOT NULL CHECK(block_number>=0), -- 实际区块高度
    block_hash TEXT NOT NULL, -- 实际区块哈希
    offset_kind TEXT NOT NULL CHECK(offset_kind IN ('BlockEnd','Transaction')), -- 实际位置类型
    transaction_index INTEGER CHECK(transaction_index>=0), -- 实际交易索引
    log_index INTEGER CHECK(log_index>=0), -- 实际日志索引
    transaction_hash TEXT NOT NULL, -- 交易哈希
    wallet TEXT NOT NULL, -- 钱包地址
    sender TEXT NOT NULL, -- 发送地址
    recipient TEXT, -- 接收地址
    wallet_is_contract INTEGER NOT NULL CHECK(wallet_is_contract IN (0,1)), -- 钱包是否合约
    execution_status TEXT NOT NULL CHECK(execution_status IN ('Unknown','Succeeded','Reverted')), -- 交易执行状态
    changes_json TEXT NOT NULL CHECK(json_valid(changes_json)), -- 余额变化列表
    transfers_json TEXT NOT NULL CHECK(json_valid(transfers_json)), -- 转账列表
    swap_count INTEGER NOT NULL CHECK(swap_count>=0), -- 兑换次数
    liquidity_count INTEGER NOT NULL CHECK(liquidity_count>=0), -- 流动性事件次数
    gas_cost TEXT, -- 燃料费用无损十进制值
    balance_scope TEXT NOT NULL CHECK(balance_scope IN ('BlockBoundary','Transaction')), -- 余额统计范围
    receipt_complete INTEGER NOT NULL CHECK(receipt_complete IN (0,1)), -- 收据是否完整
    transaction_balances_complete INTEGER NOT NULL CHECK(transaction_balances_complete IN (0,1)), -- 交易余额是否完整
    valuation_json TEXT CHECK(valuation_json IS NULL OR json_valid(valuation_json)), -- 嵌套估值记录
    evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json)), -- 证据列表
    CHECK((offset_kind='BlockEnd' AND transaction_index IS NULL AND log_index IS NULL) OR (offset_kind='Transaction' AND transaction_index IS NOT NULL)),
    UNIQUE(run_id,transaction_hash,wallet,block_hash)
);
CREATE INDEX wallet_run ON wallet_facts(run_id,id);


CREATE TABLE runtime_checkpoints (
    run_id TEXT PRIMARY KEY, -- 研究运行标识，对应 research_runs.run_id
    checkpoint_id INTEGER NOT NULL REFERENCES checkpoints(id) -- 关联 checkpoints.id 的检查点编号
);
