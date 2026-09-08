# Robinhood 链上套利研究

当前阶段：T01–T30 的实现与 macOS 本机工程验收已完成，Linux CI 待执行。已实现 Rust 四 crate 的只读采集、状态与检查点、双腿整数报价、候选管线、链/观察重放、重组恢复、原子模拟、有界队列、Feed、钱包证据和 Markdown/CSV 报告。

已核验真实同币多池与完整原子模拟（含失败回滚）。当前报价仅支持 Pons Hook 的零 LP/协议费、已证明范围内单步计算；其他真实第二池及跨 tick 尚不支持，不能据此宣称已可盈利或可实盘套利。

技术方向：Rust 模块化单体。模块边界、数据契约和运行流程见 [架构设计](docs/architecture.md)。

当前数据库完整结构为 [schema_v015.sql（v15）](crates/arb-adapters/schema/schema_v015.sql)，包含全部表、索引和约束，可用于空数据库初始化。历史升级脚本位于 [migrations](crates/arb-adapters/migrations)，按 `schema_v版本号_变更说明.sql` 命名，文件头注明起止版本和完整结构路径；已有数据库由 `Store::open` 自动按版本升级。 表用途、保留 JSON 的范围及 Rust 模型映射见 [数据库说明](docs/database.md)。

实现顺序、最小任务、依赖和验收标准见 [实现计划](docs/superpowers/plans/2026-09-07-rust-arbitrage.md)。任务勾选记录实际进度。

已确认套利范围：同一代币、两个不同池、同一报价资产的双腿闭环；路线数据结构可表达多腿，暂不实现通用多跳搜索。

## 研究目标

1. 市场容量：查找同一代币有两个以上可交易且有深度的池，统计扣除手续费、税费、价格影响与 Gas 后的闭环价差、持续时间和资金容量。初始候选范围是 Pons V2 毕业币及其其他池。
2. 执行可行性：模拟完整闭环，测量数据接收、解析和模拟延迟。区分模拟结果、估计失败 Gas 与真实广播后的成交成功率；只读阶段不声称获得真实执行结果。
3. 钱包数据：保留原始成交、交易归因和净资产变化，供后续带延迟、费用和滑点的影子跟单与样本外评估。

## 第一阶段边界

- 公开数据只读采集、历史重放与实时模拟；不需要私钥、不广播交易、不建资金池。
- 优先评估免费 RPC 和公开 Feed。购买数据服务前确认必要性、价格与预算。
- 先核验网络可用性、链 ID、部署字节码、工厂版本、Hook、费用与毕业机制。既有聊天中的网络资料是待验证线索，不是已确认事实。
- Feed 的已排序调用不等于成功成交；回执未到标记为未知。
- 多池报价必须基于一致状态，处理重复、断线补采、重组、失败交易、小数位与报价资产差异。

## 功能讨论清单

- 最小模块：币池发现、状态采集、闭环报价与模拟、事件记录、研究报告。
- 定义有效深度、测试金额、净收益口径和机会持续时间。
- 明确观察窗口、运行主机、免费接口限额和存储预算。
- 原始记录保留交易哈希、区块哈希/顺序、本机接收时间、成功状态、事件及资产变化；派生结果可重新计算。
- 跟单归因区分路由器、打包者、转账、赠币、LP、多腿套利与真实方向性交易。
- 以包含失败币和真实延迟的结果决定是否进入执行开发，不根据屏幕价差或源钱包收益作结论。

此目录是本地项目目录；加入 Codex 侧栏项目需要在应用中选择此文件夹。

## 已实现命令

```sh
cargo run -p arb-app -- check-config --config config/example.toml
cargo run -p arb-app -- run --config /path/to/config.toml
cargo run -p arb-app -- report --config /path/to/config.toml --run RUN_ID --out /path/to/new-report
cargo run -p arb-app -- collect --config /path/to/config.toml --from 56819909 --to 56819909
```

示例配置使用无效域名供离线校验；采集前使用已核验 RPC。运行数据写入配置指定的本机 SQLite 文件，数据库无需单独安装。范围起点不得跳过已有采集游标；相同范围重跑只补采未提交区块。当前仅接受批量回执接口可用的 RPC。

验证：`cargo test --workspace` 和 `cargo clippy --workspace --all-targets -- -D warnings`。默认测试不访问公网，但 RPC 测试需要绑定本机回环端口。

链状态重放：`cargo run -p arb-app -- replay --mode chain --config /path/to/config.toml --checkpoint 1 --to 56819920`。检查点须关联已保存研究运行，研究参数及算法版本须匹配。重放只读取本地原始记录；缺失完整区块或分叉不明确时失败，不调用最新 RPC 补出历史状态。

观察过程重放将 `--mode chain` 改成 `--mode observed`。它按 `(本机接收时间, 持久化 ID)` 推进逻辑时钟，等完整区块输入到齐后才发布状态；回执到达前交易结果为未知。当前单次观察窗口限制为 64 MiB / 100000 条输入，超限应拆分窗口。时钟倒退与多运行/来源时钟域会写入派生结果的时间质量标记。


Linux 和 macOS 使用同一套 Rust 源码与前台命令；Linux 另提供 systemd 示例。配置、资源限制、退出/恢复与实际能力边界见 [运行说明](docs/operations.md)，本次测试与真实观察结果见 [验收记录](docs/verification/acceptance.md)。双平台 CI 在提交推送后执行；未执行的 Linux 检查不记为通过。
