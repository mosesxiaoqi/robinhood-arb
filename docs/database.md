# 数据库结构与 Rust 模型

当前完整建表 SQL：[schema_v015.sql](../crates/arb-adapters/schema/schema_v015.sql)。共 16 张业务表，字段都有中文注释。完整 SQL 用于空数据库；已有数据库由 `Store::open` 自动升级。

v15 将剩余 10 张表中的通用 `data` 列全部移除。固定字段单独存列，只有原始字节和变长嵌套集合保留为 BLOB / JSON。

| 表 | 独立字段的主要内容 | 保留的复杂内容 |
| --- | --- | --- |
| `raw_records` | 来源、运行、序号、接收时间、请求耗时、记录类型、区块与交易位置、执行和确认状态 | `payload` 保存原始字节 |
| `pool_registry` | 链、池定位方式、合约地址、代币、手续费、精度、初始化位置、验证状态与原因 | `key` 保留原有复合池标识的规范 JSON，维持唯一键和分页顺序；池属性独立存列 |
| `bootstraps` | 版本、区块高度与哈希 | `pools_json`、`evidence_json` |
| `checkpoints` | 运行、版本、配置哈希、算法、注册表版本、原始记录游标、下一区块高度 | `state_json` 保存完整嵌套状态以恢复和重放 |
| `research_runs` | 配置、算法、报价资产、深度及利润门槛、费用资产/金额/依据 | `amounts_json`、`cost_conversion_json` |
| `derived_blocks` | 运行、区块、父区块、视图、规范链标记、排除数量、时间质量 | 池状态、观察、原始记录引用、候选、排除原因和阶段计时列表 |
| `recovery_jobs` | 计划标识、运行、检查点、共同祖先、补采范围、任务状态 | `orphan_hashes_json`、`replay_blocks_json` |
| `simulations` | 任务、队列、候选、视图、预期位置、阶段、结果状态、各时间与耗时、错误、有效性标记 | `request_json`、`result_json`、`error_evidence_json` |
| `wallet_facts` | 链与交易位置、钱包与收发地址、执行状态、事件数量、Gas 费用、证据完整性标记 | 余额变化、转账、估值和证据列表 |
| `runtime_status` | 状态、起止时间、采集与处理计数、区块范围、磁盘占用、最大处理耗时、错误 | 无 |

`source_cursors`、`ingest_gaps`、`feed_cursors`、`feed_seen`、`feed_gaps`、`runtime_checkpoints` 已有明确字段，继续沿用。

金额使用无损十进制文本保存 `U256`，避免浮点精度损失；区块高度、时间和计数使用经过范围检查的 `INTEGER`。部分原有 `u64` 身份与游标字段继续使用十进制文本，保留完整取值范围。`*_json` 是可直接阅读的 JSON 文本，不再用 BLOB 包装整个业务对象。

## 模型和映射

项目已有的 `RawRecord`、`PoolDescriptor`、`Bootstrap`、`Checkpoint`、`RunSpec`、`DerivedBlock`、`SimulationRecord`、`WalletFacts` 等 Rust 结构体承担模型职责。数据库使用现有 `rusqlite`，通过显式参数绑定和行解码还原这些模型，无新增 ORM 依赖。

映射按职责放在：

- [store/raw.rs](../crates/arb-adapters/src/store/raw.rs)：原始记录、采集游标与 Feed 写入。
- [store/catalog.rs](../crates/arb-adapters/src/store/catalog.rs)：池注册表、初始快照、检查点和研究参数。
- [store/research.rs](../crates/arb-adapters/src/store/research.rs)：派生区块、恢复计划和运行汇总。
- [store/simulation.rs](../crates/arb-adapters/src/store/simulation.rs)：模拟任务和钱包事实。
- [store.rs](../crates/arb-adapters/src/store.rs)：数据库打开、事务协调、运行检查点和报告查询。

恢复计划与运行汇总来自上层 `arb-app`，适配层用明确的存储模型接收，避免反向依赖应用层。

## 直接查询示例

```sql
-- 接收时间最新的原始记录
SELECT id, source, kind, block_number, transaction_hash, received_at_ms
FROM raw_records
ORDER BY received_at_ms DESC
LIMIT 20;

-- 已验证池的费率和代币
SELECT chain_id, locator_kind, address, manager, pool_id,
       token, quote_asset, lp_fee, hook_fee_bps, creator_tax_bps
FROM pool_registry
WHERE verification_status = 'supported';

-- 查看运行状态
SELECT run_id, status, collected_blocks, processed_blocks, error
FROM runtime_status;
```

## 旧数据库升级

v15 的 [SQL](../crates/arb-adapters/migrations/schema_v015_split_record_columns.sql) 和 [Rust 转换代码](../crates/arb-adapters/src/store/migration.rs) 配合执行，不能只运行这一个增量 SQL。转换会逐条读取旧 JSON，复用正常写入的模型映射，保留记录编号、自增高水位、规范链状态和检查点关联。任何解码、范围或一致性检查失败，整个事务回滚，旧结构和数据保留。

v12、v13、v14 完整 SQL 作为历史迁移测试输入保留；日常查阅以 v15 为准。
