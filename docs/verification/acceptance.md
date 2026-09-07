# T30 只读工程验收

日期：2026-09-07。按用户确认交付 Linux/macOS 支持，不安装服务、不启动目标主机部署。T01–T30 的代码与本机工程验收完成；Linux CI 与目标 Linux 主机运行尚未执行，不能标记为已通过。

## 可复现检查

本机：macOS 26.6.2 arm64，Rust 1.93.1。最终代码执行以下命令通过：

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release -p arb-app --locked
python3 scripts/verify_protocol_samples.py
python3 scripts/verify_simulation_samples.py
```

工作区测试结果为 35 passed、0 failed、3 ignored。三个默认忽略项是显式公网 RPC、原子模拟、Feed 验收，曾在 T07/T23/T25 单独执行；本次全量离线测试不冒充重新调用公网。其网络、固定状态和模拟响应证据见 [网络核验](network.md)、[协议核验](protocol.md) 和 [模拟核验](simulation.md)。补充旧分支模拟关联恢复断言后，单独重跑 T24 三个测试和 fmt/clippy。

T30 串联测试覆盖采集持久化、初始化、候选、模拟不可用记录的精确关联、重启、重组、独立干净分支重放与报告。状态与 HTTP 输入是明确的合成测试数据，不能代替链上协议验证。最终审查另外复现并修复：

- 已处理 4,104 块后浅重组：从最近共同检查点恢复，回查上限约束恢复深度。
- 链从 A 切到 B 再回 A：保留原始证据、视图与候选标识，恢复完成事务内重新核验模拟有效性。
- 历史存在 100,001 条窗口外钱包记录时，小窗口报告仍可导出；行数预算作用于选中窗口。

## 有限真实观察

实际执行主网只读 `run`，chain ID 4663，RPC 为 `https://rpc.mainnet.chain.robinhood.com`。参数及每条原始记录的元数据、长度与 SHA-256 见 [观察证据](runtime-acceptance.json)。完整响应保留在本机被 Git 忽略的 `data/runtime-acceptance.db`；JSON 元数据本身不是可重放输入。

| 项目 | 实测 |
| --- | --- |
| 观察窗口 | 20 秒，2 请求/秒，采集并发 1 |
| 原始采集覆盖 | 56,924,060–56,924,065，共 6 块 |
| 持久化原始记录 | 18 条：区块、回执、日志各 6 条 |
| 持久化下一块游标 | 56,924,066 |
| 完整分析视图 | 0：该窗口未发现可初始化池 |
| 最大单块处理耗时 | 87.648250 ms；不含网络采集等待 |
| DB/WAL/SHM 起止占用 | 196,688 → 8,754,048 字节，增长 8,557,360 字节 |
| 连接关闭后 DB 大小 | 5,550,080 字节，WAL 归并后口径不同 |
| 停止状态 | WindowEnded，error = null |
| 报告 | release CLI 成功导出 Markdown 与 CSV |

该窗口验证有限采集、持久化游标、时间/磁盘测量、正常停止和零分析覆盖报告，不代表全链无机会，也不包含候选模拟性能。处理延迟来自这个短窗口，不能外推长期吞吐。

## 实际模拟能力与尚未覆盖项

T22/T23 已核验真实同币两池完整原子模拟、固定区块身份、余额变化和第二腿失败后的原子回滚。已保存的成功执行样本输入 1,000,000,000 wei、输出 80,733,325 wei，本金亏损 919,266,675 wei，尚未扣 Gas。临时代码/余额覆盖与零 base fee/gas price 的口径在模拟核验文档明确记录；没有签名、广播或真实资金转移。

报价仅支持已核验 Pons Hook 的零 LP/协议费、局部 bitmap 内单步计算。发现的真实第二池有非零费用，当前尚不能报价；跨 tick 和通用多跳搜索亦未实现。因此已完成的是本阶段已核验范围内的只读工程管线，真实同币双池策略搜索能力仍有限，不能声明已可盈利或可实盘运行。

[双平台 CI](../../.github/workflows/ci.yml) 提供 Linux/macOS 同套构建和测试，Linux 额外验证 systemd unit；本次仅运行 macOS。未推送执行的 CI、Linux 主机重启/目录权限和真实广播成功率均不记为已验收。前台命令及 Linux 服务示例见 [运行说明](../operations.md)。
