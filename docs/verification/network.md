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
