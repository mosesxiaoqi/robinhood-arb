# T01 网络核验

核验时间：2026-09-07T12:04:29.800113+00:00。目标为 Robinhood Chain 主网。

## 官方身份与入口

[官方连接文档](https://docs.robinhood.com/chain/connecting/)列明主网链 ID 4663（0x1237）、测试网 46630。主网公开 RPC 为 `https://rpc.mainnet.chain.robinhood.com`，Feed 为 `wss://feed.mainnet.chain.robinhood.com`。测试网端点以 `testnet` 替代 `mainnet`；本次未探测测试网，也不混用测试网数据。

## 实际探测

完整只读请求与响应见 [network-probe.json](network-probe.json)。公开端点不含密钥。

| 能力 | 结果 |
| --- | --- |
| 网络身份 | eth_chainId 返回 0x1237，与官方一致 |
| 区块读取 | 区块 56819909，哈希 `0xf86eca534014a124c63b8d1582e11774ea9355d0ff3914310b16fed652653d45` |
| 哈希交叉核对 | 按 number 与按 hash 返回的 hash 一致 |
| 区块日志 | eth_getLogs 按 blockHash 返回 12 条 |
| 批量回执 | eth_getBlockReceipts 返回 10 条，与区块交易数一致 |
| 早期历史状态 | 在区块 1 查询零地址余额返回 -32000 / metadata is not found, 4；不能视为 archive 可用 |
| Pons 工厂代码 | 读取到非空代码；这只证明地址有代码，不完成协议核验 |
| eth_simulateV1 | 空 calls 探测返回结果；尚不证明能完成真实原子双腿模拟 |
| Feed 握手 | 2026-09-07T12:04:55Z 返回 HTTP 101，4 秒收取约 10.6 MB；curl 按时限退出 28 是主动终止观测 |
| RPC WebSocket 订阅 | 本次没有核验无密钥 RPC WebSocket；Feed 不是 eth_subscribe 接口 |

Feed 仅确认握手与有数据，解码、序号与恢复留在 T25。历史状态限制由 T22 在目标位置进一步探测，不能用最新状态替代。

复现方式：按 JSON 文件逐个发送 request 至 endpoint；只读方法不含广播。Feed 用标准 WebSocket Upgrade 握手，设置有限时间和接收上限，握手头中的 cookie 不保存。

## 下游约束

T07 可以实现并在线验证当前区块采集。公开接口存在限额，不以本次成功推断稳定吞吐量。完整模拟、协议身份与 Feed 语义仍由各自任务验收。

## T07 实现验收

`cargo test -p arb-adapters --test t07_rpc verified_mainnet_block -- --ignored --exact` 已对上述固定区块通过，只读采集器核对了交易、回执、日志与区块哈希。默认测试使用本地有限响应 HTTP 服务，覆盖错误链、缺回执、HTTP 429 重试上限和过大响应。

实现采用 Alloy JSON-RPC 请求/响应类型与通用哈希原语；HTTP 层显式限制流式响应大小并保留原始字节，避免高层 SDK 重编码丢失原始观测。没有启用签名或广播接口。当前批量回执能力是接入前提，不支持时返回明确错误。

## T08 采集闭环验收

CLI 对主网区块 56819909 采集并提交 3 条原始记录，重开 SQLite 核对游标为 56819910。故障测试在第二块写入时触发 SQLite ABORT，确认第一块保留、第二块全部回滚，重启只补采第二块。默认测试共 8 项通过；显式主网测试另行通过。

### T25 Feed 协议与边界

T01 捕获的第一个完整未压缩文本帧保存为 `tests/data/verified/feed-message.json`（manifest 附 SHA-256）。其 JSON `version=1`、首消息 `sequenceNumber=56819749`，内容为 Nitro `message.message.header` 与 base64 `l2Msg`。格式及重连请求头依据 [Nitro 消息结构](https://github.com/OffchainLabs/nitro/blob/master/broadcaster/message/message.go)、[客户端](https://github.com/OffchainLabs/nitro/blob/master/broadcastclient/broadcastclient.go) 和 [服务端头定义](https://github.com/OffchainLabs/nitro/blob/master/wsbroadcastserver/wsbroadcastserver.go)（2026-09-07 核对）。客户端头版本 2 与 JSON 消息版本 1 属于不同层级。

Feed 序号不是区块号，header.blockNumber 是 L1 消息头字段；本实现不据此生成 ChainPosition。每个原始帧保留原字节与本地接收时间；RawRecord.sequence 为持久化帧游标，消息序号保留在 payload 与 feed_seen。重复输入仍保留观测，但按网络/序号/消息摘要去重；同序号冲突整体回滚。缺失区间单独保存在 feed_gaps，不冒充 RPC 区块缺口，也不宣称后续普通回执能够补回首次观测时间。

重连发送最后已持久化的下一消息序号；公共 relay 缓存覆盖未经保证，跳号保留“恢复未核验”缺口。即使晚到消息补齐内容，历史观测缺口仍保留。连接/读超时与重试有界；WebSocket 帧和聚合消息都有大小上限。不协商压缩，拒绝意外扩展，因此没有隐含解压放大。L2 内部消息仍是不透明内容，不解码交易、不验证 sequencer 签名；来源身份为配置来源，缺少服务端 chain header 时不视为密码学身份验证。所有 Feed 执行和确认状态保持 Unknown，完全不进入已确认状态更新管线。

`collect-feed --config <path> --frames <1..10000>` 使用持续连接与逐帧确认写入提供背压，最长 300 秒。失败返回显式 feed disabled 错误；RPC collect/replay 独立可用。运行预算统一保护由 T29 接入，当前命令仅供有限验收。离线 `t25_feed` 覆盖 10→12 缺口、重复/冲突、未知状态与大小上限；公网验收须显式 `cargo test -p arb-adapters --test t25_feed live_feed_acceptance -- --ignored --nocapture`。

2026-09-07 T25 显式 Rust 公网验收通过，读取消息序号 56905363，原帧另存本地 `data/t25-live-feed.json`；未修改 Unknown 语义。随后 CLI 有限接收 3 帧，退出后逐帧数据与 Feed 游标留存在 SQLite。
