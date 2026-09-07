# Robinhood Chain Rust 最小实现单位计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Only use superpowers:subagent-driven-development when parallel agent execution is explicitly selected. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 分步实现只读采集、确定性重放、双腿套利报价、完整模拟及研究报告，每个任务形成可独立验证的交付。

**Architecture:** 一个进程、四个 crate，外部适配与纯计算分离。实时与重放复用同一处理函数，状态维护提供一致视图，研究模块只消费事实与计算结果。

**Tech Stack:** Rust、Tokio、Alloy、Serde、rusqlite/SQLite WAL、tracing、TOML；Feed 客户端按能力核验结果选择。

**Spec:** [架构设计](../../architecture.md)，执行前同时阅读 [README](../../../README.md)。

状态：执行中。任务勾选表示验收完成；各项证据见 `docs/verification/`。

## 全局约束

- 同一代币、两个不同池、同一报价资产的双腿闭环。
- 路线结构使用有序交易腿列表，能够表达多腿；候选生成器只生成双腿路线，不实现通用多跳搜索。
- 不接入私钥、不签名、不广播交易、不建资金池。
- `arb-core` 不访问网络、数据库、文件或系统时钟。
- 金额以最小单位整数表示，使用足够宽的整数和显式舍入规则，检查溢出；浮点数仅用于非权威展示。
- 原始记录及其采集游标在同一存储事务中提交。
- 起始一致性边界使用完整区块处理位置。
- Feed 先记录和用于延迟研究；不实现未经核验的 Feed 推测状态。
- 超时和后端不可用与交易执行失败分开；只读阶段仅报告模拟通过率，不报告真实成交成功率。
- 不增加微服务、Redis、消息队列、动态插件、前端或真实执行模块。
- 不自动清理原始数据；付费服务需要另行确认预算。

## 如何使用这份计划

最小单位是一个有明确输入输出、能独立验收的行为，不是一个文件或一行代码。任务内的测试、实现、文档同步属于同一交付；不为建目录、增加依赖单独创建任务。

每次只领取一个依赖已完成的任务。任务完成需要：目标测试确实执行且通过、无越界修改、下游契约可读。禁止把“命令返回成功但运行了 0 个测试”作为验收。

每项任务采用以下执行循环：

1. 根据指定样例编写具名测试，运行并确认因目标行为缺失而失败；语法或环境错误先单独解决。
2. 实现当前行为，运行同一个测试，再运行受影响 crate 的测试。
3. 检查错误分支和架构边界，勾选任务步骤。
4. 当前目录已是 Git 仓库；每项任务验收后仅提交该任务文件及进度更新。

Rust 测试使用自带测试工具。样例断言中的局部变量由对应步骤给出的输入生成；不要为测试引入通用 fixture 框架。协议测试使用固定、脱敏且附来源的真实样本。任务内存在多个独立失败原因时，可以使用多个小测试，但不为简单访问器增加测试。

文中的类型名沿用架构契约；各类型的最小字段在生产它的任务中定义。接口签名作为模块交接约定，具体第三方 API 在锁定依赖后核验。尚未核验的链 ID、地址、ABI、费用公式和模拟方法不编造：T01、T09、T22 是明确的前置核验门槛，失败时暂停其依赖任务，其他任务仍可进行。

## 文件布局

以下是计划路径，不要求一次性创建空文件。每个文件在对应任务第一次需要时创建，同一模块的后续任务修改原文件。

```text
Cargo.toml / Cargo.lock / rust-toolchain.toml
config/example.toml
crates/
  arb-core/src/
    lib.rs types.rs route.rs state.rs opportunity.rs checkpoint.rs
    protocol/mod.rs protocol/verified.rs
  arb-adapters/src/
    lib.rs store.rs rpc.rs feed.rs discovery.rs decode.rs simulation.rs
    migrations/001.sql
  arb-research/src/
    lib.rs opportunities.rs wallets.rs report.rs
  arb-app/src/
    main.rs lib.rs config.rs ingest.rs pipeline.rs replay.rs recovery.rs runtime.rs
  <crate>/tests/tNN_<behavior>.rs
tests/data/verified/manifest.json
docs/verification/network.md
docs/verification/protocol.md
docs/verification/simulation.md
docs/operations.md
deploy/robinhood-arb.service
```

`protocol/verified.rs` 只承载首个实际核验的池协议，以协议标识标明适用范围；出现第二种协议才增加相应模块。`tests/data/verified/manifest.json` 记录各样本的网络、区块哈希、获取方法和预期事实，不把示意数据标成链上证据。

## 依赖与里程碑

| 阶段 | 任务 | 可交付结果 |
| --- | --- | --- |
| A 基础与落盘 | T01–T08 | 配置可校验，RPC 原始记录可靠保存，可恢复采集 |
| B 协议与状态 | T09–T14 | 真实池初始化、完整区块状态、检查点 |
| C 套利报价 | T15–T18 | 双腿候选、金额扫描、可追溯候选结果 |
| D 重放与恢复 | T19–T21 | 状态重放、观察重放、重组恢复 |
| E 模拟与 Feed | T22–T25 | 经能力核验的完整模拟、Feed 观测记录 |
| F 研究与运行 | T26–T30 | 机会和钱包报告、资源控制、只读上线验收 |

关键依赖：`T01→T07`，`T09→T10/T11/T15`，`T14+T18→T19→T20/T21`，`T22→T23→T24`。T25 不依赖模拟后端。T26 不以存在盈利机会为验收条件。

各任务未完成前置核验时可以用确定性样例验证通用逻辑，但样例通过不等于真实链接入通过。全部第一阶段完成必须包含真实数据和完整模拟验收；后端不可用时如实标记该部分未完成。

## A. 基础与可靠落盘

### T01 — 核验网络与公开数据能力

**依赖：** 无。**文件：** 新建 `docs/verification/network.md`。

**输入 → 输出：** 官方网络资料、公开端点 → 已核验网络身份、支持能力和探测证据。

- [x] 从架构引用的官方页面核对目标网络，记录 URL、查询日期、链 ID、RPC 和可查证的 Feed 信息；主网与测试网分列，不混用。
- [x] 对选定公开 RPC 执行只读 `eth_chainId`、`eth_getBlockByNumber`、区块日志和历史状态探测，保存脱敏请求与响应片段、区块哈希、错误码。
- [x] 分类记录 HTTP/订阅/历史状态/Feed 是否支持；缺失能力写明“不可用”及影响。无公开 Feed 证据时不猜地址。
- [x] 验收：链 ID 与官方记录一致，至少一个区块可由哈希交叉核对；每项能力都有证据或明确失败原因。

可重复探测模板（执行者将变量赋为本任务已核验值，不把未知网络写成默认值）：

```sh
curl --fail-with-body "$ARB_RPC_URL" -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}'
```

### T02 — 可校验配置的 Rust 程序入口

**依赖：** 无；真实运行值由 T01 提供。

**文件：** 新建根 Cargo 文件、四个 crate 的 `Cargo.toml` 与入口；`arb-app/src/config.rs`、`config/example.toml`、`arb-app/tests/t02_config.rs`。

**接口：** `Config::parse(text: &str) -> Result<Config, ConfigError>`；CLI `check-config --config <path>`。

- [x] 编写 `reject_invalid_config`：空金额列表、零队列容量、非法端点和缺失链 ID 均拒绝；金额以十进制字符串读取，不先转浮点。
- [x] 运行 `cargo test -p arb-app --test t02_config` 确认失败，随后创建最小 workspace，锁定核验过的稳定工具链与依赖版本。
- [x] 实现配置解析与启动校验，字段包含端点、链 ID、报价资产、金额、深度/收益阈值、确认策略、限流/重试、队列、数据库、窗口和磁盘预算；示例配置明确为离线示例，真实采集须 T01 配置。
- [x] 同一测试通过；合法配置返回成功，未知字段和无效组合返回字段级错误。敏感端点不原样输出。

```rust
assert!(Config::parse("queue_capacity = 0").is_err());
```

### T03 — 原始记录与状态位置契约

**依赖：** T02。

**文件：** 新建 `arb-core/src/types.rs`、`arb-core/tests/t03_records.rs`；修改 `arb-core/src/lib.rs`。

**接口：** 定义 `RawRecord`、`ChainPosition`、`ExecutionStatus`、`Confirmation`；`RawRecord::validate(&self) -> Result<(), RecordError>`。

- [x] 编写 `preserve_unknown_and_position`：缺失回执、区块内位置与区块末位置往返序列化不失真；不允许负序号或超长载荷通过入口校验。
- [x] 运行 `cargo test -p arb-core --test t03_records` 确认失败。
- [x] 实现版本、网络、来源、原始字节、来源序号、UTC 接收时间和运行标识；区块/交易信息用可选字段表达。定义 `ExecutionStatus::{Unknown,Succeeded,Reverted}`，确认等级独立存储。
- [x] 同一测试通过，Feed 样例保持未知，不因存在交易哈希转为成功。

```rust
assert_eq!(decoded.execution_status, ExecutionStatus::Unknown);
assert_eq!(decoded, original);
```

### T04 — 路线表达与资产校验

**依赖：** T03。

**文件：** 新建 `arb-core/src/route.rs`、`arb-core/tests/t04_route.rs`；修改 `lib.rs`。

**接口：** 定义 `Leg`、`Route`；`Route::validate(&self) -> Result<(), RouteError>`。池标识包括网络和独立池地址，或网络、PoolManager 地址与 PoolId（适配 V4 singleton）。

- [x] 编写 `validate_closed_route`：合法双腿和合法三腿通过；空路线、断裂资产、跨网络和不闭合路线失败。
- [x] 运行 `cargo test -p arb-core --test t04_route` 确认失败。
- [x] 使用 `Vec<Leg>`，逐腿验证相邻资产及首尾闭合，不在结构层硬编码两腿；两腿限制属于候选生成任务。
- [x] 同一测试通过，三腿结构可表达但不产生多腿搜索实现。

```rust
assert!(closed_three_legs.validate().is_ok());
assert!(broken_assets.validate().is_err());
```

### T05 — 原始数据与游标原子写入

**依赖：** T03。

**文件：** 新建 `arb-adapters/src/store.rs`、`migrations/001.sql`、`arb-adapters/tests/t05_atomic_store.rs`；修改 `lib.rs`。

**接口：** `Store::open(path: &Path) -> Result<Store, StoreError>`；`append_raw(&mut self, records: &[RawRecord], cursor: &SourceCursor) -> Result<(), StoreError>`；`SourceCursor` 在 `types.rs` 定义为网络、来源、可恢复位置。

- [x] 编写 `rollback_record_and_cursor`：用 SQLite 约束失败中断批写，验证该批记录及游标都未提交；重复批次不重复产生领域输入。
- [x] 运行 `cargo test -p arb-adapters --test t05_atomic_store` 确认失败。
- [x] 创建迁移版本、原始记录、观察时间和来源游标表，开启 WAL，事务内完成批写；分别保留同一链事实的多来源观测，不能去重掉接收时间证据。
- [x] 同一测试通过；重开数据库后游标与记录仍一致，未知迁移版本拒绝打开写入。

```rust
assert!(write_result.is_err());
assert_eq!(cursor_after, cursor_before);
assert_eq!(count_after, count_before);
```

### T06 — 原始记录的稳定读取

**依赖：** T05。

**文件：** 修改 `store.rs`；新建 `arb-adapters/tests/t06_read_raw.rs`。

**接口：** `read_raw_after(&self, id: u64, limit: usize) -> Result<Vec<StoredRaw>, StoreError>`；`StoredRaw` 在 store 模块定义为数据库 ID 与 `RawRecord`。

- [x] 编写 `paginate_without_skip`：保存三条记录、每页两条，遍历恰好得到三条；重开数据库后顺序不变。
- [x] 运行 `cargo test -p arb-adapters --test t06_read_raw` 确认失败。
- [x] 按持久化 ID 进行游标分页，限制单页数量；同时提供按网络和来源过滤，禁止整库无界加载。
- [x] 同一测试通过；读取损坏版本返回显式错误，不跳过坏记录继续宣称完整。

```rust
assert_eq!(ids, vec![1, 2, 3]);
```

### T07 — RPC 区块只读采集

**依赖：** T01、T02、T03。

**文件：** 新建 `arb-adapters/src/rpc.rs`、`arb-adapters/tests/t07_rpc.rs`；修改 `lib.rs`。

**接口：** `RpcSource::fetch_block(&self, number: u64) -> Result<Vec<RawRecord>, SourceError>`，异步；输出原始头、所需交易/回执/日志，保留获取范围证据。

- [x] 编写 `reject_wrong_chain_and_partial_block`：本地有限响应服务返回错误链 ID 或缺失回执时不能报告完整区块；限流响应最多重试配置次数。
- [x] 运行 `cargo test -p arb-adapters --test t07_rpc` 确认失败。
- [x] 用 Alloy 已支持的只读接口采集；请求前确认网络，限制并发/响应大小/超时，区分可重试错误与永久错误。
- [x] 同一测试通过；另对 T01 区块做一次显式线上验收，按哈希核对返回事实，线上检查不进入默认测试。

```rust
assert!(wrong_chain_result.is_err());
assert!(request_count <= retry_limit + 1);
```

### T08 — 采集落盘与断点恢复闭环

**依赖：** T05、T06、T07。

**文件：** 新建 `arb-app/src/ingest.rs`、`arb-app/tests/t08_ingest.rs`；修改 app 入口。

**接口：** CLI `collect --config <path> --from <block> --to <block>`；编排 `RpcSource` 和专用线程上的 `Store`。

- [ ] 编写 `resume_after_write_failure`：采集两块，第二块落盘失败，重启后从未提交位置补采；不跳过第二块。
- [ ] 运行 `cargo test -p arb-app --test t08_ingest` 确认失败。
- [ ] 用有界队列把采集结果交给单写线程，收到提交确认后才推进采集进度；记录断线缺口，补采成功再清除。
- [ ] 同一测试通过；运行有限区块范围得到可重开的数据库，写入失败明确退出或暂停，不继续推进游标。

```rust
assert_eq!(resumed_start, failed_block);
assert_eq!(persisted_last_block, expected_last_block);
```

## B. 协议与状态

### T09 — 核验首个池协议并固定真实样本

**依赖：** T01、T07。

**文件：** 新建 `docs/verification/protocol.md`、`tests/data/verified/manifest.json` 及其列出的原始 JSON 样本。

**输入 → 输出：** 官方部署信息、字节码、交易与池状态 → 首个受支持协议的 ABI、状态字段、费用/舍入规则、毕业规则和独立预期输出。

- [ ] 核对 Pons V2 工厂、毕业流程、池版本、代理实现和 Hook；不能仅凭项目名中的 V2 选择恒定乘积公式。
- [ ] 固定创建/毕业、一笔成功交易、一笔失败交易及前后状态样本，记录区块哈希和数据来源；记录税费、特殊代币支持边界。
- [ ] 根据实际协议写出状态转换和报价公式以及至少一组可独立核对的输入/输出。无法确认时标为不支持，并暂停 T10/T11/T15 的协议实现。
- [ ] 验收：字节码/ABI 与样本事件对应，公式预期与实际交易或可信协议参考实现一致；不能用将要实现的公式生成自己的唯一预期值。

```sh
python3 -m json.tool tests/data/verified/manifest.json
```

### T10 — 币池发现与核验登记

**依赖：** T09、T05。

**文件：** 新建 `arb-adapters/src/discovery.rs`、`arb-adapters/tests/t10_discovery.rs`；修改 `types.rs`、`store.rs` 和迁移。

**接口：** 定义 `PoolDescriptor`；`discover(records: &[RawRecord]) -> Result<Vec<PoolDescriptor>, DiscoveryError>`。

- [ ] 编写 `register_verified_pool_once`：T09 毕业样本关联到正确代币和池；重复输入不重复登记；同名伪工厂事件不能登记为已核验。
- [ ] 运行 `cargo test -p arb-adapters --test t10_discovery` 确认失败。
- [ ] 限定已核验工厂与实现，登记资产、小数位、协议标识、费用规则和支持状态；保存登记依据及版本。
- [ ] 同一测试通过；缺少字节码/元数据时保留未核验记录，不进入可报价集合。

```rust
assert_eq!(verified_pools.len(), 1);
assert!(!unverified_pool.is_quoteable());
```

### T11 — 协议事件解码

**依赖：** T03、T09、T10。

**文件：** 新建 `arb-adapters/src/decode.rs`、`arb-adapters/tests/t11_decode.rs`；修改 core `types.rs`。

**接口：** 定义 `Observation` 与协议领域事件；`decode(raw: &RawRecord, pool: &PoolDescriptor) -> Result<Vec<Observation>, DecodeError>`。

- [ ] 编写 `decode_verified_event`：核对 T09 事件字段、原始引用与交易状态；错误 topic、截断 ABI、错误池地址不能转成有效状态更新。
- [ ] 运行 `cargo test -p arb-adapters --test t11_decode` 确认失败。
- [ ] 以已核验 ABI 解码，不在此处修改状态；Feed 没有回执时保持未知，失败交易不输出已生效的池更新。
- [ ] 同一测试通过；未知事件可保留记录，但不伪造解码成功。

```rust
assert_eq!(observation.raw_id, expected_raw_id);
assert_eq!(failed_transaction_updates.len(), 0);
```

### T12 — 固定区块初始化池状态

**依赖：** T07、T10、T11。

**文件：** 新建 `arb-core/src/protocol/mod.rs`、`protocol/verified.rs`；修改 `rpc.rs`、`store.rs`；新建 `arb-adapters/tests/t12_bootstrap.rs`。

**接口：** 定义 `PoolState`、`Bootstrap`；`bootstrap(pools: &[PoolDescriptor], at: ChainPosition) -> Result<Bootstrap, SourceError>`，异步。

- [ ] 编写 `bootstrap_at_one_hash`：两池读取必须绑定同一区块哈希；其中一池缺状态时整体不能形成完整起始视图。
- [ ] 运行 `cargo test -p arb-adapters --test t12_bootstrap` 确认失败。
- [ ] 按 T09 协议读取所有报价所需状态和参数，不只读取最新储备；节点仅支持区块号时前后核对区块哈希，变化则拒绝结果。
- [ ] 同一测试通过；初始化状态、元数据和参数持久化，可离线加载。在线中途发现池时沿用该初始化规则。

```rust
assert_eq!(bootstrap.position.block_hash, requested_hash);
assert!(partial_bootstrap.is_err());
```

### T13 — 完整区块状态转换与一致视图

**依赖：** T11、T12。

**文件：** 新建 `arb-core/src/state.rs`、`arb-core/tests/t13_state.rs`。

**接口：** 定义 `BlockBatch`、`StateView`、`StateError`；`State::apply_block(&mut self, batch: &BlockBatch) -> Result<StateView, StateError>`；`State::from_bootstrap(bootstrap: Bootstrap) -> State`。

- [ ] 编写 `publish_only_complete_block`：完整连续块发布视图；缺日志范围、父哈希错误或中途事件错误时不部分提交；重复同块无重复更新。
- [ ] 运行 `cargo test -p arb-core --test t13_state` 确认失败。
- [ ] `BlockBatch` 携带头、覆盖范围和有序事件，app 在所有请求成功后才创建完整批；先在候选状态应用并校验，再原子替换内存状态。
- [ ] 同一测试通过；无事件但已证明覆盖完整的池沿用状态，旧视图在新块到来后保持不变。

```rust
assert_eq!(old_view, saved_old_view);
assert_eq!(state_after_failed_batch, state_before_failed_batch);
```

### T14 — 检查点保存与恢复

**依赖：** T05、T10、T13。

**文件：** 新建 `arb-core/src/checkpoint.rs`、`arb-adapters/tests/t14_checkpoint.rs`；修改 `store.rs`。

**接口：** 定义 `Checkpoint`；`save_checkpoint(&mut self, checkpoint: &Checkpoint) -> Result<(), StoreError>`；`load_checkpoint(&self, id: u64) -> Result<Checkpoint, StoreError>`。

- [ ] 编写 `restore_checkpoint_exactly`：保存后重开数据库，状态、登记版本、配置引用和处理游标一致；格式版本不支持时明确失败。
- [ ] 运行 `cargo test -p arb-adapters --test t14_checkpoint` 确认失败。
- [ ] 一致保存恢复所需内容与处理游标，采集游标和处理游标分别命名；半写入不能成为可恢复检查点。
- [ ] 同一测试通过；检查点后的输入重放不重新应用检查点已包含区块。

```rust
assert_eq!(restored, saved);
```

## C. 双腿套利报价

### T15 — 首个真实协议的整数报价

**依赖：** T09、T12。

**文件：** 修改 `protocol/verified.rs`；新建 `arb-core/tests/t15_quote.rs`。

**接口：** 定义 `Quote`、`QuoteError`；`quote(pool: &PoolState, asset_in: Address, amount_in: U256) -> Result<Quote, QuoteError>`；整数与地址复用通用原语。

- [ ] 编写 `match_verified_quote`：T09 独立预期值准确一致；零输入、空流动性、溢出和不支持税费/Hook 均有明确结果。
- [ ] 运行 `cargo test -p arb-core --test t15_quote` 确认失败。
- [ ] 只实现 T09 已核验公式与舍入，乘除使用足够宽的中间值；报价输出说明已包含的池手续费，避免下游重复扣除。
- [ ] 同一测试通过；对两个方向分别核对真实样本，不引入通用 AMM 框架。

```rust
assert_eq!(quote_result.amount_out, independently_verified_amount_out);
```

### T16 — 仅生成同币同报价资产双腿路线

**依赖：** T04、T10。

**文件：** 修改 `route.rs`；新建 `arb-core/tests/t16_candidates.rs`。

**接口：** `two_leg_routes(pools: &[PoolDescriptor], quote_asset: Address) -> Vec<Route>`。

- [ ] 编写 `pair_two_pools_both_directions`：两个有效池产生两个方向；第三个不同目标币或报价资产的池不能串入；同池不能配对。
- [ ] 运行 `cargo test -p arb-core --test t16_candidates` 确认失败。
- [ ] 按网络、目标币、报价资产分组，生成不同池的有向组合，排序保证重放结果稳定；忽略未核验或不支持池，并保留排除计数。
- [ ] 同一测试通过；三腿合法结构依然可用，但候选生成输出全是两腿。

```rust
assert_eq!(routes.len(), 2);
assert!(routes.iter().all(|route| route.legs.len() == 2));
```

### T17 — 金额扫描与净收益口径

**依赖：** T13、T15、T16。

**文件：** 新建 `arb-core/src/opportunity.rs`、`arb-core/tests/t17_evaluate.rs`。

**接口：** 定义 `CostEstimate`、`Opportunity`；`evaluate(view: &StateView, route: &Route, amount: U256, costs: &CostEstimate) -> Result<Opportunity, EvaluateError>`。

- [ ] 编写 `subtract_cost_once`：输入 100，闭环输出 110，额外同资产成本 3，净收益为 7；额外成本未知时净收益为未知；输出低于投入不能无符号下溢。
- [ ] 运行 `cargo test -p arb-core --test t17_evaluate` 确认失败。
- [ ] 将前腿输出作为后腿输入，复用同一视图；收益使用符号加绝对值或已检查有符号表示。Gas 不是报价资产时须显式换算依据，否则收益未知。
- [ ] 同一测试通过；遍历配置金额，记录深度阈值和排除原因；费用已包含项不再次扣除，估计值保留估计标签。

```rust
assert_eq!(net_profit, Some(expected_positive_seven));
assert_eq!(unknown_cost_net_profit, None);
```

### T18 — 共用处理管线与候选持久化

**依赖：** T08、T11、T13、T17。

**文件：** 新建 `arb-app/src/pipeline.rs`、`arb-app/tests/t18_pipeline.rs`；修改 `store.rs`。

**接口：** `Pipeline::process(&mut self, batch: BlockBatch, observed_at: u64) -> Result<Vec<Opportunity>, PipelineError>`；逻辑时间由参数提供。

- [ ] 编写 `persist_candidate_provenance`：处理一个完整区块，候选能追溯到原始记录、视图、配置/算法版本和检测时间；坏区块不能产出候选。
- [ ] 运行 `cargo test -p arb-app --test t18_pipeline` 确认失败。
- [ ] 组合状态转换、路线和金额扫描；SQLite 增加派生记录与运行版本，幂等键包含运行语义，重放新参数不会覆盖旧研究结果。
- [ ] 同一测试通过；保存候选不以模拟成功为前提，模拟状态初始明确为未运行。

```rust
assert_eq!(loaded_opportunity.view_id, computed_opportunity.view_id);
assert_eq!(invalid_block_opportunities.len(), 0);
```

## D. 重放与恢复

### T19 — 链状态重放

**依赖：** T06、T14、T18。

**文件：** 新建 `arb-app/src/replay.rs`、`arb-app/tests/t19_replay.rs`；修改 CLI。

**接口：** CLI `replay --mode chain --config <path> --checkpoint <id> --to <block>`；重放调用 T18 `Pipeline::process`。

- [ ] 编写 `replay_matches_live_core_results`：同一初始化和三块固定输入，实时模拟入口与存储重放的池状态、路线、金额、报价完全相同。
- [ ] 运行 `cargo test -p arb-app --test t19_replay` 确认失败。
- [ ] 从检查点分页读取、按规范链位置组装完整批，使用固定版本和参数；结果比较排除运行 ID/墙钟耗时等非业务字段。
- [ ] 同一测试通过；缺少初始化或历史区间返回数据缺口，不回退使用最新 RPC 状态。

```rust
assert_eq!(replayed_core_results, live_core_results);
```

### T20 — 观察过程重放与延迟记录

**依赖：** T03、T19。

**文件：** 修改 `replay.rs`、`pipeline.rs`；新建 `arb-app/tests/t20_observed_replay.rs`。

**接口：** CLI `replay --mode observed ...`；输入按 `(received_at, persisted_id)` 提供，处理管线继续使用链位置协调状态。

- [ ] 编写 `late_receipt_stays_unknown`：10 时收到调用，20 时收到回执；逻辑时间 15 时结果未知，20 时才可更新执行状态。
- [ ] 运行 `cargo test -p arb-app --test t20_observed_replay` 确认失败。
- [ ] 逻辑时间显式推进，无真实 sleep；实时采集/解析/报价/模拟耗时使用本进程单调时钟，持久化 UTC 和运行标识。不跨运行计算单调时间差。
- [ ] 同一测试通过；本机历史时钟倒退或来源时间不可比较时标记时间质量问题，不制造负延迟统计。

```rust
assert_eq!(status_at_15, ExecutionStatus::Unknown);
assert_eq!(status_at_20, ExecutionStatus::Succeeded);
```

### T21 — 重组恢复与结果失效

**依赖：** T14、T18、T19。

**文件：** 新建 `arb-app/src/recovery.rs`、`arb-app/tests/t21_reorg.rs`；修改 `store.rs`。

**接口：** `RecoveryPlan` 在 recovery 定义，包含恢复检查点、需补采区间与失效分支；app 执行恢复后复用 pipeline。

- [ ] 编写 `reorg_matches_clean_branch`：原分支 A1→A2 被 A1→B2→B3 替换，恢复结果与从 A1 全新处理 B 分支一致。
- [ ] 运行 `cargo test -p arb-app --test t21_reorg` 确认失败。
- [ ] 查共同祖先与可用检查点；暂停发布受影响视图，事务标记旧候选/模拟失效，保留原始记录；无检查点则重新初始化。
- [ ] 同一测试通过；恢复中再次退出能幂等继续，报告不能读取失效分支收益。

```rust
assert_eq!(recovered_state, clean_branch_state);
assert!(!orphan_opportunity.canonical);
```

## E. 完整模拟与 Feed

### T22 — 验证完整模拟路径

**依赖：** T01、T09、T12、T17。

**文件：** 新建 `docs/verification/simulation.md`，新增真实探测样本并登记到 manifest。

**输入 → 输出：** 真实双腿路线、历史位置、公开节点能力 → 一个选定模拟后端的可重复请求、执行语义和限制。

- [ ] 先验证节点能否在指定状态执行承接状态变化的完整调用，检查调用者余额、授权、路由器和 Gas 语义。
- [ ] 若节点不支持，验证本地分叉是否能加载目标历史状态并执行闭环。只选择一个可行后端，不同时开发两个。
- [ ] 保存一个完整成功样本和一个第二腿失败样本；核对资产变化与原子回滚。模拟专用余额/代码覆盖必须披露，不能记作真实账户条件通过。
- [ ] 验收：请求明确状态位置、两腿共享执行上下文且失败会回滚。只有分离报价或两笔非原子模拟时不通过；无法实现则记录阻塞，T23/T24 不标完成。

本任务产出实际调用封装与 API 后，再将其请求字段和样本加入 T23 的执行记录。若需要 Solidity 辅助合约，先补充最小模拟专用设计；不悄悄把本项目扩大为链上部署任务。

将实际探测得到的语义整理为后续测试断言；以下变量分别来自请求位置、返回位置、初始余额和失败回滚后的余额：

```rust
assert_eq!(actual_position, requested_position);
assert_eq!(balance_after_reverted_route, balance_before_route);
```

余额断言针对交易腿涉及的代币本金，Gas 支出单独核对；不能把包含 Gas 扣费的原生币余额也要求原样恢复。

### T23 — 首个完整闭环模拟后端

**依赖：** T22。

**文件：** 新建 `arb-adapters/src/simulation.rs`、`arb-adapters/tests/t23_simulation.rs`；修改 core `types.rs`。

**接口：** 定义 `SimulationRequest`、`SimulationResult`；`simulate(&self, request: &SimulationRequest) -> Result<SimulationResult, SimulationTransportError>`，异步；请求含路线、金额、账户条件和状态位置。

- [ ] 编写 `simulate_atomic_route`：消费 T22 成功与第二腿失败样本，断言最终资产变化、Gas、回滚和实际状态位置；不使用自造“成功”布尔响应替代语义验证。
- [ ] 运行 `cargo test -p arb-adapters --test t23_simulation` 确认失败。
- [ ] 按 T22 唯一选定方法实现请求构造、结果解析和受限超时；明确成功、执行失败、不可用、未知与错误证据。
- [ ] 同一测试通过；对实际后端重复只读验收。超时不计为执行失败，缺历史状态不自动使用最新块。

```rust
assert_eq!(result.actual_position, request.position);
assert_eq!(reverted_result.asset_changes, expected_rolled_back_changes);
```

### T24 — 模拟任务调度与结果关联

**依赖：** T18、T23。

**文件：** 修改 `pipeline.rs`、`store.rs`；新建 `arb-app/tests/t24_simulation_queue.rs`。

**接口：** 有界候选队列输入 `Opportunity`，输出按候选 ID 关联的 `SimulationResult`；模拟不占用状态写任务。

- [ ] 编写 `timeout_does_not_block_state`：模拟任务超时期间仍处理下一个完整块；状态不匹配结果不能成为原候选通过证据。
- [ ] 运行 `cargo test -p arb-app --test t24_simulation_queue` 确认失败。
- [ ] 限制并发与排队时间；保存排队、开始、结束时刻，队列超限显式记录未模拟，不能无界积压或静默丢弃。
- [ ] 同一测试通过；候选被重组失效后，晚到模拟仍存档但不计入有效统计。

```rust
assert_eq!(last_processed_block, next_block);
assert!(!mismatched_result.validates_original_candidate);
```

### T25 — Feed 原始观测与缺口记录

**依赖：** T01、T03、T08、T20。

**文件：** 新建 `arb-adapters/src/feed.rs`、`arb-adapters/tests/t25_feed.rs`；修改采集编排。

**接口：** `FeedSource` 产生 `RawRecord`，复用落盘入口；不向已确认状态管线发送推测池更新。

- [ ] 编写 `feed_gap_is_explicit`：可恢复序号 10 后收到 12，记录缺口；断线重连的重复输入不重复改变交易事实；无回执保持未知。
- [ ] 运行 `cargo test -p arb-adapters --test t25_feed` 确认失败。
- [ ] 使用 T01 实际协议，限制消息大小与解压大小、超时和重试；支持补采则补采，不支持则保留不可恢复缺口。日志不暴露认证 URL。
- [ ] 同一测试通过；公开 Feed 不可用时交付显式禁用状态及证据，不能称 Feed 接入已完成，RPC 主流程仍可运行。

```rust
assert_eq!(feed_execution_status, ExecutionStatus::Unknown);
assert_eq!(missing_sequence, Some(11));
```

## F. 研究分析与运行交付

### T26 — 机会持续时间与采样容量

**依赖：** T17、T18、T21；模拟统计接入 T24 后启用。

**文件：** 新建 `arb-research/src/opportunities.rs`、`arb-research/tests/t26_opportunities.rs`。

**接口：** 定义 `OpportunitySummary`；`summarize_opportunities(records: &[Opportunity]) -> Vec<OpportunitySummary>`，按网络、路线和报价资产分组。

- [ ] 编写 `gap_splits_opportunity_window`：连续有效两块合并；中间缺口/未知/孤块切断；不同报价资产不相加。
- [ ] 运行 `cargo test -p arb-research --test t26_opportunities` 确认失败。
- [ ] 输出区块粒度持续区间、数据覆盖、排除原因和最大已测试有效金额；分别给出报价候选与模拟验证容量，不把模拟未知的金额称已验证。
- [ ] 同一测试通过；全程没有盈利时仍返回覆盖及零机会报告，失败币不能从分母消失。

```rust
assert_eq!(windows.len(), 2);
assert_eq!(sampled_capacity, largest_eligible_tested_amount);
```

### T27 — 钱包资产变化与证据归因

**依赖：** T07、T11、T18。

**文件：** 新建 `arb-research/src/wallets.rs`、`arb-research/tests/t27_wallets.rs`；修改 types、store 和 RPC 读取路径。

**接口：** 定义 `WalletFacts`、`WalletAttribution`；`attribute(facts: &WalletFacts) -> WalletAttribution`；事实保留哈希、资产变化、费用与证据完整性。

- [ ] 编写 `do_not_treat_transfer_as_profit`：赠币、转账、LP、路由器、多腿套利、方向成交各有样例；证据不足必须未知。
- [ ] 运行 `cargo test -p arb-research --test t27_wallets` 确认失败。
- [ ] 从交易/回执和所需余额差提取事实，内部原生币变化缺 trace 等证据时标记不完整；按证据标记参与角色，不能简单把发送者或收款路由器当作交易者。
- [ ] 同一测试通过；资产数量变化与统一计价损益分开，没有估值与入出金依据时不输出确定净利润。

```rust
assert_eq!(gift_attribution.realized_profit, None);
assert!(!incomplete_facts_attribution.complete);
```

### T28 — 可导出的研究报告

**依赖：** T06、T26、T27；完整模拟章节依赖 T24。

**文件：** 新建 `arb-research/src/report.rs`、`arb-app/tests/t28_report.rs`；修改 CLI。

**接口：** CLI `report --config <path> --run <id> --out <directory>`；research 向 `std::io::Write` 输出 Markdown 与 CSV，app 负责文件路径和读取。

- [ ] 编写 `report_preserves_unknown_and_coverage`：零机会、模拟不可用、数据缺口都可导出；包含逗号/引号的文本符合 CSV 转义。
- [ ] 运行 `cargo test -p arb-app --test t28_report` 确认失败。
- [ ] 输出覆盖区间、采样参数、版本、机会/容量、模拟状态分布、延迟和钱包证据；分页或按窗口处理，避免无界读全库，分析使用独立读连接。
- [ ] 同一测试通过；写入失败不留下冒充完成的报告，采用临时文件成功后替换；报告显著标明“只读模拟，非真实成交”。

```rust
assert!(markdown.contains("只读模拟，非真实成交"));
assert!(markdown.contains("数据缺口"));
```

### T29 — 资源上限、日志与受控退出

**依赖：** T08、T18、T21、T24。

**文件：** 新建 `arb-app/src/runtime.rs`、`arb-app/tests/t29_runtime.rs`；修改配置与入口。

**接口：** 统一退出信号；运行状态区分运行、数据缺口暂停、磁盘暂停与失败。CLI `run --config <path>` 组装已完成模块。

- [ ] 编写 `stop_without_advancing_unwritten_cursor`：收到退出或磁盘预算不足时停止新采集，已确认写入保留，未提交游标不推进；模拟慢任务不阻止无限退出。
- [ ] 运行 `cargo test -p arb-app --test t29_runtime` 确认失败。
- [ ] 检查数据库与 WAL 等实际磁盘占用，配置安全余量并在触及预算前暂停；不删除数据。日志携带运行/来源/位置/候选标识，记录队列占用与阶段耗时，脱敏错误。
- [ ] 同一测试通过；设置有限排空期限，超期工作标记未完成并在重启恢复；观察窗口到期受控结束。

```rust
assert_eq!(durable_cursor_after_shutdown, last_committed_cursor);
assert_eq!(deleted_raw_records, 0);
```

### T30 — Linux 部署与端到端只读验收

**依赖：** T01–T29 全部适用任务；任何不可用项必须在验收结论列明，不以跳过冒充完成。

**文件：** 新建 `deploy/robinhood-arb.service`、`docs/operations.md`、`arb-app/tests/t30_end_to_end.rs`；修改 README。

**接口：** 一个 release 二进制，由 systemd 使用显式配置和工作目录运行；不要求开发者桌面具备 systemd。

- [ ] 编写 `collect_replay_report_roundtrip`：有限输入完成采集落盘、状态初始化、候选、模拟样本关联、重放和报告；注入一次重启和重组后核心结果仍一致。
- [ ] 运行 `cargo test -p arb-app --test t30_end_to_end` 确认目标缺口，再补齐部署和串联问题，不重写各模块。
- [ ] 在目标 Linux 主机检查 unit 语法、可写数据目录、预算和重启行为；样例 unit 不含私钥或付费凭据，不自动安装或启动到未选定主机。
- [ ] 完成下列全局检查，记录有限真实观察窗口的处理延迟、数据覆盖、磁盘增长与实际模拟能力。没有盈利机会也能通过工程验收；真实执行成功率始终不在验收项中。

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p arb-app
systemd-analyze verify deploy/robinhood-arb.service
```

最后一条仅在 Linux 环境执行。默认 Rust 测试使用本地样本，不依赖公网；真实 RPC、Feed 与模拟验收显式运行并附证据。

## 全局完成标准与覆盖核对

| 架构要求 | 对应任务 | 必须留下的证据 |
| --- | --- | --- |
| 四 crate 单向依赖、只读边界 | T02、T30 | manifests 与 workspace 检查；无签名/广播功能 |
| 网络、工厂、费用、毕业机制核验 | T01、T09 | 官方来源、实际响应、字节码与样本 |
| 原始数据、游标、背压与恢复 | T03、T05–T08、T29 | 原子回滚、重复与重启测试 |
| 初始状态、完整视图、检查点 | T10–T14 | 缺失拒绝、旧视图稳定与恢复一致测试 |
| 双腿、整数、费用、金额扫描 | T04、T15–T18 | 独立报价样本与成本边界测试 |
| 两种重放、延迟、重组 | T19–T21 | 无未来信息、核心结果一致、孤块失效 |
| 完整原子模拟、状态匹配 | T22–T24 | 实际后端成功/回滚证据与错误分类 |
| Feed 不等于成功、缺口可见 | T25 | 序号缺口、未知状态与接入能力证据 |
| 机会容量、持续时间、失败样本 | T26、T28 | 采样口径、覆盖及未知项报告 |
| 钱包归因与净资产变化证据 | T27、T28 | 入出金/LP/赠币/路由/多腿样例，未知保留 |
| 预算、退出、部署 | T29、T30 | 停止不丢游标、Linux 验收与资源实测 |

每个里程碑可以单独交付，但不得把部分交付命名为整个项目完成。真实协议、Feed 或模拟后端不可用时，记录受影响任务及可继续部分；不通过放宽正确性条件消除阻塞。

执行中如果 T09 揭示复杂池数学、T22 要求新的模拟调用封装，应在对应门槛处补充以真实资料为依据的子任务，再继续其依赖项；不先写猜测实现。本计划不估计固定分钟数，核验耗时和协议复杂度必须以实际结果为准。
