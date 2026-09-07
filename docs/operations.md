# Linux / macOS 运行说明

本项目为只读研究程序。Rust 运行时不需要 Node.js、独立 SQLite 服务、私钥或链上部署；SQLite 与 WAL 由程序创建。`simulation/` 的 Node 工具仅用于显式重编译核验辅助合约，不参与日常运行。

## 构建与离线检查

需要 Rustup 和本机 C 工具链（Linux GCC/Clang；macOS Xcode Command Line Tools）。仓库固定 Rust 1.93.1。

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release -p arb-app --locked
python3 scripts/verify_protocol_samples.py
python3 scripts/verify_simulation_samples.py
```

默认测试使用本地样本，可绑定回环端口，不访问公网。`.github/workflows/ci.yml` 分别在 Linux 和 macOS 执行上述检查，Linux 额外校验替换本机路径后的 systemd unit。CI 未运行时不能称 Linux 验收通过；本次实际本机与公网检查见 [验收记录](verification/acceptance.md)。

## 配置与有限运行

复制 `config/example.toml` 到本机配置，填入已核验 RPC `https://rpc.mainnet.chain.robinhood.com`，设定报价资产、金额、阈值、数据库路径和观察时长。示例故意使用无效 RPC 域名。配置不接受未知字段，不包含签名或广播选项。

```sh
./target/release/arb-app check-config --config /absolute/path/config.toml
./target/release/arb-app run --config /absolute/path/config.toml
```

两平台均可前台运行，Ctrl-C 或 SIGTERM 受控退出；窗口到期正常结束。标准输出给出 run_id、真实原始采集范围、状态处理数、磁盘起止与最大处理耗时；详细日志包含运行/来源/块/候选和队列占用。数据库也保存最近运行摘要。

- 新库省略 `start_block` 时从确认头开始；已有采集游标优先，不跳过未采集区间。
- `run_id` 可选，默认由研究配置摘要确定。相同 ID/参数与数据库恢复检查点；改变研究参数请使用新 ID。采集来源共享一个持久化 RPC 游标。
- 无池时状态为 AwaitingPools，只采集原始块与发现毕业事件，不生成伪造的空池快照。已有注册池在固定区块核验代码/参数后初始化。
- 每次 run 的池集合固定在初始化时。后续新池留在注册表，使用新 run_id 才纳入；局部 bitmap 失效的已有池会在已核验位置刷新。最多 128 池、每块 10,000 个路线×金额组合。
- `additional_cost` 是报价资产原生单位的**人工总额外成本估计**，不是实际 Gas。省略时成本未知，不产生“净收益达标”候选；不得为了得到机会随意写零。
- 同一数据库只允许一个 `run` 进程。独占锁随进程退出释放，锁文件本身不需要删除。报告使用独立只读快照，可同时导出；不要并发运行手动采集命令修改同库。

## 预算、停止与恢复

`run` 检查数据库、`-wal`、`-shm` 的实际大小及所在文件系统可用空间，按即将写入数据预留 SQLite 放大和停止元数据空间，预算检查与状态/模拟写入串行；预算是保守提前暂停阈值，不是达到限制后删数据。请求响应、队列、模拟并发与排队/调用时间都有上限。模拟独立并发槽与采集共享总 RPC 限速。

模拟排空最多 5 秒，超期标 Interrupted/Unknown；SQLite 等待锁最多 5 秒。已经开始的短同步事务先结束，未提交原始记录不推进游标；重启先处理已落盘但尚未进入检查点的块，再拉新数据。原始输入与池注册同事务，恢复结束状态、运行检查点与游标同事务。磁盘不足时大模拟结果不写入，并留下结果未持久化的未知记录。

重组最多回查 128 块/64 MiB，验证共同祖先和替代分支后恢复；失败中的恢复计划重启可继续。越过初始快照、缺共同检查点或缺历史状态时 DataGapPaused，不使用最新状态填补过去。此时保留原库，核对数据源/历史覆盖后在新的数据库和新 run_id 从明确位置重新观察。RPC 失败、预算暂停和数据缺口退出非零，运行摘要保留原因，不自动无限重试。

手动 `collect` / `collect-feed` 是有限采样工具，不是长期守护进程；各自有区块/帧/消息/队列上限，长期运行的统一磁盘与信号控制入口是 `run`。Feed 不影响确认状态；其消息序号、观测缺口与执行 Unknown 语义见 [网络核验](verification/network.md)。

## 重放与报告

```sh
./target/release/arb-app collect --config /path/config.toml --from 56830269 --to 56830269
./target/release/arb-app collect-feed --config /path/config.toml --frames 3
./target/release/arb-app replay --mode chain --config /path/config.toml --checkpoint 1 --to 56830275
./target/release/arb-app replay --mode observed --config /path/config.toml --checkpoint 1 --to 56830275
./target/release/arb-app report --config /path/config.toml --run RESEARCH_RUN_ID --out /path/new-report
```

检查点 ID 是数据库 checkpoints.id，须匹配研究配置及算法版本。链重放遇到多个同父分支会拒绝猜测，应先执行恢复或使用已明确分支的原始数据集；观察重放按接收顺序推进，受 64 MiB/100,000 条输入限制，不伪造最初接收时间。

报告目录必须不存在；Markdown 和 CSV 均成功后才原子发布。可加 `--from N --to M` 选择最多 10,000 块窗口。窗口内分页读取最多 100,000 行，选中序列化记录最多 64 MiB，超限用更小窗口。分析覆盖只计算完整视图；原始采集覆盖另外显示，未初始化不等于 RPC 未采集。重组未完成时拒绝导出；孤块候选/模拟不计有效容量。钱包区块边界余额不能隔离单笔交易，缺 trace/估值/入出金或成本基础时利润未知。

## Linux systemd 示例

`deploy/robinhood-arb.service` 使用 `/usr/local/bin/robinhood-arb`、`/etc/robinhood-arb/config.toml` 和 `/var/lib/robinhood-arb`。部署前由管理员创建对应普通服务账户、安装二进制与配置，确认数据目录所有权、配置内数据库路径和磁盘预算，然后执行 `systemd-analyze verify`。示例自身不安装、不启动服务，不含凭据。

服务收到 SIGTERM 走相同退出路径，TimeoutStopSec=30；只对异常信号等情况自动重启。正常窗口到期、主动停止、预算暂停及普通非零错误不自动开始无限新窗口。手动再次启动会恢复同库进度。资源/日志配置依据 [systemd.service](https://www.freedesktop.org/software/systemd/man/systemd.service.html)；macOS 无需 systemd，使用相同二进制前台命令即可。

## 当前协议限制

真实主网已核验 Pons Hook 池和同币其他 V4 池、完整原子双腿模拟及第二腿失败回滚。但报价适配器只覆盖已核验 Pons Hook、零 LP/协议费、已证明 bitmap 内的单步计算。其他真实池的非零 LP/协议费、跨 tick 报价和通用多跳搜索尚不支持。因此实际同币双池套利搜索仍可能因第二池不支持而无法形成候选，不能把工程验收当作策略已经可盈利或可实盘使用。

实际原子模拟样本是亏损样本；调用用临时余额/代码覆盖、base fee 与 gas price 归零隔离本金变化，无真实资金转移。余额/Gas/价格信息与不可用边界见 [模拟核验](verification/simulation.md)。真实广播成功率不在本项目只读阶段验收范围。
