# 完整双腿模拟核验（T22 进行中）

已发现同一真实代币、原生 ETH 报价的四个 V4 池：一个 Pons Hook 池及三个无 Hook 的高费率池。固定区块读取得到四池非零流动性，但后三池不在现有报价支持范围，不能据此宣称存在盈利候选。历史样本区块在公共 RPC 上已裁剪；本地 Anvil 分叉同样无法补出上游缺失状态。最新状态上的 `eth_simulateV1` 只读 `decimals()` 调用可用。

## 新增最小核验子任务

- T22.1：在新获取的固定区块核对两池参数及流动性，保留完整请求。模拟使用真实池状态，不修改池余额、价格或流动性。
- T22.2：编写仅用于模拟的 Solidity 调用封装，通过同一次 `PoolManager.unlock` 回调依次执行两个不同池的 Swap，前腿净输出作为后腿输入，结清余额后返回原生币。用户应用仍使用 Rust；不增加部署、签名或广播功能。
- T22.3：通过 RPC `stateOverrides` 注入该封装的运行时代码，并显式覆盖一个模拟调用者的 ETH 余额。该条件只说明模拟账户已资金充足，不说明任何真实账户资金或授权合格。原生币入口不需要伪造 ERC20 授权。
- T22.4：在同一固定基础状态保存成功请求及第二腿故意使用非法价格限值的失败请求；比较调用者资产与两个池的 Slot0。失败须整体回滚；成功不等于盈利。

每条路线放在一个 EVM 调用中。前后只读检查可以是另外的调用，但不能把两条交易腿拆成两个独立模拟调用。API 返回的是合成子区块，其哈希不是请求的基础状态哈希；必须单独核对基础状态头，不能把返回子区块号冒充历史位置。

## API 和编译依据

使用 [Geth eth_simulateV1 文档](https://geth.ethereum.org/docs/interacting-with-geth/rpc/ns-eth#eth_simulatev1) 定义的 `blockStateCalls/stateOverrides/calls` 结构。每个路线调用内部完成两腿，外围前后读取同一上下文中的余额及两池状态。

模拟封装见 `simulation/AtomicProbe.sol`。可用 `npm ci --prefix simulation --ignore-scripts && npm run --prefix simulation build` 重新编译；运行服务和默认 Rust 测试不依赖 Node。编译器固定 `solc-js 0.8.35+commit.47b9dedd`，优化 200 次、viaIR、EVM Cancun；运行时代码和源文件 SHA-256 写入 `simulation/AtomicProbe.json`。Foundry 原生编译器下载曾发生校验和不匹配，未绕过该检查或使用该下载文件。

## T22 验收结果

固定基础区块 `56881480`，哈希 `0x6543601ac83f33d659d3ed8d816689f4dd46425b4543604d67236d1ab94a6c97`。选择公开 RPC 的 `eth_simulateV1` 加模拟专用封装作为唯一后端；本地分叉不作为备用执行后端。

- 第一池：Pons Hook 池 `0xc4b14f873fd4b8971414c4d5b668c0b692048199fc340c5493854ee6384fde01`。
- 第二池：无 Hook、LP 费率 853875 / 1000000 的真实池 `0x711204cf5ece44dfd8f141d5b828c9e95e36848c3fe7e35c255609e0d123fca0`。两池同为已核验 PoolManager 下的 ETH / `0xfdf57d8fd26beb04275148c4e2c5c24b39ff10a6` 池。
- 成功样本投入 `1000000000` wei，经前腿净输出 `293828362243879529` 个代币最小单位，第二腿返回 `80733325` wei，执行用量 `187167` Gas。该样本本金亏损 `919266675` wei，尚未计 Gas，**不是盈利机会**。
- 第二腿非法价格限值样本执行失败，用量 `117828` Gas；调用者/封装的原生币及代币余额、两池 Slot0 全部恢复到前值，没有保留第一腿变更。

余额覆盖为模拟调用者 `1 ETH`，封装运行时代码注入空地址；两个池、Hook、代币余额和授权均未覆盖。为验证本金回滚，Gas 价格和模拟区块基础费设为零；记录的 Gas 用量不能直接当作实际链上费用或发送条件验证。第二池报价暂未实现，当前候选管线不会把它标记为已支持；模拟证明只覆盖这里的真实路线及已披露条件。

原始请求/响应及独立预期值位于 `tests/data/verified/atomic-simulation.json`、`atomic-expected.json`；能力与历史裁剪证据位于 `simulation-capabilities.json`、`simulation-local-fork.json`。运行 `python3 scripts/verify_simulation_samples.py` 离线核对双腿事件、资产流、基础状态和整体回滚。公共节点历史状态会裁剪，旧固定区块请求日后可能不可重跑；新验收应重新选固定区块，不能静默改为 latest 后沿用旧位置。

## T23 Rust 后端复验

Rust `RpcSource::simulate` 已在新固定区块 `56889840` / `0xb77a557007360d9a2f90bd317602f062391ea092859e64c6f22f6fa8143bff2f` 重复只读验收：成功用量 `187103` Gas，返回 `80068701` wei；第二腿失败用量 `117764` Gas，资产与池状态整体回滚。完整本机复验记录保存在忽略跟踪的 `data/t23-live-simulation.json`；可复用验收命令 `cargo test -p arb-adapters --test t23_simulation verified_live_atomic_route -- --ignored --nocapture`。

默认离线测试消费 T22 实际响应，并拒绝不匹配的基础状态、资金条件、双腿事件或最终价格。超时/传输异常归为未知，缺历史状态或接口不可用归为不可用；仅内层调用的明确失败及回滚证据归为执行失败。错误保留请求与可取得的原始回复，不回退到最新状态。
