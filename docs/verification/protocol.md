# T09 首个协议核验

核验对象：Robinhood Chain 主网 Pons V2 当前工厂及其毕业后的 V4 Hook 池。源记录见 [样本索引](../../tests/data/verified/manifest.json)。

## 部署依据

| 合约 | 地址 | 核验结果 |
| --- | --- | --- |
| PonsV2LaunchFactory | `0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e` | 主网运行时代码逐字节等于 Sourcify 记录；创建与运行时均 exact_match |
| PonsV2MemeHook | `0xE5e702641Ea86F4ae6cC3cDaeD2B886f976Be044` | 工厂 getter 指向此地址；主网代码逐字节等于 Sourcify 链上代码；Sourcify 源码匹配级别为 match，不能称元数据 exact_match |
| PoolManager | `0x8366a39cc670b4001a1121b8f6a443a643e40951` | 从工厂 getter 读取且存在代码；其独立 Sourcify 记录为空，不声称已独立复编译验证整个 Manager |

[工厂验证 API](https://sourcify.dev/server/v2/contract/4663/0x7eD598BcEf8bd9Edd8C97A195C6d13f40801EC7e?fields=all)与 [Hook 验证 API](https://sourcify.dev/server/v2/contract/4663/0xE5e702641Ea86F4ae6cC3cDaeD2B886f976Be044?fields=all)提供实际部署对应的源码、ABI 和编译信息。固定副本记录了编译参数、源文件 SHA-256、运行时代码及 Keccak 指纹。

工厂编译器为 Solidity 0.8.35，viaIR，optimizer runs 200，EVM cancun。工厂为直接部署；关联组件通过 getter 固定读取，不按网站上一个地址推断所有历史 launch 都属于同一版本。

[官方公开源码仓库](https://github.com/ponsdotdev/ponsfamily/tree/8b9bf371030279133017b5c1b713823f5889c5d2)作为阅读线索；实现依据部署验证记录中的源码，不将仓库最新版本自动等同链上部署。

## 生命周期与池标识

固定样本包含同一代币的 TokenLaunched 与 PoolGraduated 事件。毕业交易同时包含 PoolManager.Initialize、ModifyLiquidity、Hook.PoolRegistered 和随后一笔成功 Swap。仅看到 LaunchSwept 不能判定池已建立。

V4 使用 `(currency0, currency1, fee, tickSpacing, hooks)` 的 ABI 编码 Keccak 作为 PoolId，必须与 Initialize 的索引值核对。采集登记需要管理器地址和 PoolId，不能用管理器地址单独作为池主键。

## 报价独立样本

[quote-expected.json](../../tests/data/verified/quote-expected.json)来自区块 56830269 的真实毕业交易。前状态由同一回执中更早的初始化与流动性事件提供，不使用今天的状态推算历史。

样本输入为 40000000000000000 wei，输出币数量（扣 Hook 前）1925298421255294540932595，Hook 手续费 19252984212552945409325，creator tax 38505968425105890818651，最终输出 1867539468617635704704619。测试严格按最小单位整数比较。

在固定流动性、exact-input、zeroForOne、无跨计算边界时，Q = 2^96：

```text
next_sqrt = ceil(L * Q * sqrt / (L * Q + amount_in * sqrt))
gross_out = floor(L * (sqrt - next_sqrt) / Q)
fee = floor(gross_out * hook_fee_bps / 10000)
tax = floor(gross_out * creator_tax_bps / 10000)
net_out = gross_out - fee - tax
```

公式结果与实际 Swap 的 sqrtPrice、amount1 及 HookFeeCollected 两项收费分别一致。不能先把两个费率相加再统一舍入。本样本 hook_fee_bps=100、creator_tax_bps=200，来自该池冻结参数；不是全链默认费率。

反向 exact-input 对应 token1 输入时，下一价格按 `sqrt + floor(amount_in * Q / L)` 增加，输出 token0 需要按协议 FullMath/SqrtPriceMath 的除法顺序向下取整。T15 仍需增加反向与边界验收；本任务没有把尚未测量的方向标记为真实样本通过。

## 支持边界

- 先支持已核验工厂、对应 Hook 的 exact-input 毕业池，LP 费为零；其他 Hook、动态费、未完成毕业、参数不足明确不支持。
- 纯数学报价必须证明输入不会跨越未加载的 tick/流动性或算法分段边界；不满足时返回不支持，不能假定全程固定流动性。T12/T15 必须携带边界证据。
- Hook 内部手续费兑换/回购可能产生额外 Swap；状态维护必须处理所有相关池事件，不能只跟踪用户最外层成交。
- 转账税、特殊 ERC-20 与输入/收款人特例不能仅凭代币符号排除。具体代币支持状态需结合部署来源与完整模拟，未知成本保留未知。
- 失败样本证明 status=0 且没有生效日志；其失败原因和 Pons 归因未确定，不将其误称 Pons 报价失败。
- 当前取得了工厂与 Hook 源码匹配证据，以及一次 Manager 行为的独立样本核对；这不是对所有交易路径或所有 Manager 行为的形式化证明。

## 可重复检查

```sh
python3 scripts/verify_protocol_samples.py
```

检查样本哈希、代码对照、实际 Swap 整数输出和费用分别舍入。默认离线运行，不依赖浏览器、IPFS 或私有接口。

读取证据时遇到过 Blockscout 访问挑战、旧 Sourcify 路径无结果和 IPFS 超时；最终使用 Sourcify v2 官方公开接口获得验证记录，没有绕过认证。

### T12 固定区块初始化

`RpcSource::bootstrap` 在同一区块号读取并前后核对哈希；核对链 ID、三个运行时代码指纹、工厂和 Hook 元数据，然后读取 Slot0、流动性及当前方向需要的 tick 位图字。所有请求参数与原始回复字节和池元数据一起保存到 SQLite，可离线加载。任何一个池缺失、身份不符或区块变化，整批失败。

`t12_bootstrap` 使用已核验的元数据与可控 RPC 状态测试两池、哈希变化、第二池缺失及重开数据库恢复。第二个池为故障注入用合成实例，不作为主网第二池存在的证据。当前仅读取局部位图；后续报价必须在该证明范围内，跨范围须重新初始化，不能假定全区间流动性恒定。

### T15 双向整数报价

新增真实卖出样本：区块 `56830275`，交易 `0xef0d9edc37fc53534a028f3a8cf1e5d778073875188322b00a9e7b65b85f0c1b`。`reverse-observations.json` 包含从初始化到该卖出的完整池日志范围和成功回执；卖出前价格取该范围内前一个 Swap，期间无额外流动性修改。独立 Python 计算和 Rust 报价均与实际输出及两项单独向下取整费用一致。

报价以 512 位中间值计算，输出已扣 Hook 费和创建者税。仅支持已核验 Hook、零 LP/管理器协议费，以及下一位图边界前的单个 SwapMath 步骤；跨 tick、未证明的位图、特殊溢出回退分支均明确拒绝。该双向样本来自同一个池，不代表已经找到两个可套利池。

范围查询日志的附加 `blockTimestamp` 为 `0x0`，交易回执中为实际值；保存原始回复并核对地址、topics、data、区块/交易哈希、索引及 removed 状态，不以附加时间戳作为执行成功证据。
