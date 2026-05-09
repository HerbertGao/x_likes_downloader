## MODIFIED Requirements

### 需求:`download_media` 进度通过 MCP notifications/progress 推送

当 MCP 客户端调用 `download_media` 工具且请求中含 `progressToken` 字段时，`xld serve --mcp` 必须在下载过程中向客户端发送 MCP `notifications/progress` 通知。

**字段语义**（与 [MCP 规范的 `ProgressNotificationParam`](https://modelcontextprotocol.io/specification/2025-11-25) 一致）：

- `progress`：**已完成的 item 数 + 所有 in-flight item 的 byte fraction 之和**（浮点数，单调非递减，范围 `[0, total_items]`）。批次开始为 0.0，每完成一个 item 整数部分递增 1，in-flight item 通过其 `bytes_done / bytes_total` 贡献 `[0.0, 1.0]` 浮点 fraction
- `total`：批次总 item 数；与 `progress` 配对，client 用 `progress / total` 计算百分比
- `message`：当前操作描述，含字节级进度与状态文本（如 `tweet 12345 1024/4096`、`tweet 12345 ETag changed, restarting from scratch`）

**byte fraction 计算公式**：

```
in_flight_fraction(item) = match (bytes_done, bytes_total):
    (_, Some(t)) if t > 0 => bytes_done / t        // 正常路径
    (0, _)                => 0.0                    // 还没开始
    _                     => 0.5                    // bytes_total 未知，兜底中间值

progress = items_done + Σ_in_flight in_flight_fraction(item)
```

**事件映射**：

- 批次开始（`DownloadStarted`）→ `progress: 0.0, total: total_items, message: "starting N items, concurrency=n"`
- 单项启动（`ItemStarted`）→ `progress: items_done + Σ其他 in_flight fractions, total: total_items, message: "starting tweet <id>"`
- 单项字节进度（`ItemProgress`）→ `progress: items_done + Σ in_flight fractions（含本项更新后的 fraction）, total: total_items, message: "tweet <id> <bytes_done>/<bytes_total>"`
- 单项 ETag 失配重启（v2.1 新增）→ `progress: 不变, total: total_items, message: "tweet <id> ETag changed, restarting from scratch"`
- 单项完成（`ItemDone`）→ `progress: (items_done+1) + Σ其他 in_flight fractions, total: total_items, message: "tweet <id> <status>"`（status ∈ {downloaded, skipped_existing, failed, cancelled}）
- 批次正常结束（`DownloadFinished`）→ `progress: total_items, total: total_items, message: "done: downloaded=X skipped=Y failed=W"`（不在 cancel 路径上发送）
- 批次取消结束（`BatchCancelled`，v2.1 新增）→ `progress: items_done（含已 cancelled 的整数项）, total: total_items, message: "cancelled: downloaded=X cancelled=Y failed=W"`；其中 `items_done < total_items`（cancel 时尚有未启动 item，item 计数严格小于总数）；in-flight item 在 cancel 触发瞬间已通过其 `ItemDone { status: Cancelled }` 事件被计入 items_done，因此 `BatchCancelled` 发送时 `Σ in_flight fractions == 0`

**关键不变量**：

- `progress` **单调非递减**（含浮点路径：每个 in_flight item 的 fraction 单调；items_done 单调；和单调）
- `progress` **不超过 total_items**（实施层面 dispatch 时 clamp 到 [0, total_items_f]）
- 正常结束（`DownloadFinished`）时 `progress == total`（client 比例 = 1.0）
- 取消结束（`BatchCancelled`）时 `progress < total`（client 比例严格小于 1.0；含义为"批次提前终止"），且 `progress >= 已完成 item 数 K`

**Flush**：tool handler 在返回前必须 await sink 的 flush，让 worker drain 所有 pending notification——避免 final notification 跟 tool response 竞争被丢弃。

实现层面：`agent::mcp_server::McpProgressSink` 必须维护 in_flight HashMap，键为 tweet_id，值为 `(bytes_done: u64, bytes_total: Option<u64>)`；ItemProgress 与 ItemStarted/ItemDone 需更新该 map；emit 时按公式聚合。多并发下 in_flight 集合可能含多项；ItemDone 时从 map 移除该项。

#### 场景:有 progressToken 时发 notification
- **当** 客户端发送 `tools/call { name: "download_media", _meta: { progressToken: "..." } }` 下载多个 item
- **那么** server 必须发送至少一个 `notifications/progress` 通知，含相同的 progressToken；`progress` 字段为 `[0, total_items]` 内的非递减浮点数；`total` 字段等于 `total_items`

#### 场景:无 progressToken 时不发 notification
- **当** 客户端调用 `download_media` 但请求中无 progressToken
- **那么** server 不发送任何 progress 通知，但必须正常完成下载并返回最终结果

#### 场景:批次开始与结束必发（区分正常 vs 取消路径）
- **当** 客户端使用 progressToken 调用 `download_media` 下载至少 1 个 item
- **那么** 必须存在至少一个 `progress: 0.0` 起始通知；并满足以下二选一的结束通知：
  - **正常路径**（无 cancel）：必须发送 `DownloadFinished` 结束通知，`progress == total_items`（client 计算出比例 1.0）
  - **取消路径**（cancel 触发）：必须发送 `BatchCancelled` 结束通知，`progress < total_items` 且 `progress >= 已完成 item 数 K`（client 计算出比例严格小于 1.0；message 含 `"cancelled"` 字样）
  - 两类结束通知**互斥**：cancel 路径**不**发 `DownloadFinished`；正常路径**不**发 `BatchCancelled`

#### 场景:多并发下 progress 单调（含 byte fraction）
- **当** 客户端用 `progressToken` 调用 `download_media`，`concurrency=4` 下载 5 个 item
- **那么** server 发送的 progress notification 序列必须满足 `progress` 字段单调非递减；任何相邻通知 `p_i, p_{i+1}` 必须满足 `p_{i+1} >= p_i - 0.001`（浮点 epsilon 容忍）；任何通知 `progress` 必须满足 `0 <= progress <= total_items`

#### 场景:tool 返回前必须 flush
- **当** server 即将为 `download_media` 调用返回 `tools/call` 响应
- **那么** 它必须先确保所有 pending 的 progress notification 已被发送给 transport（即 sink 的 worker task 已 drain）

#### 场景:bytes_total 未知时 fraction 兜底
- **当** 单个 item 下载时 server 没有提供 Content-Length（`bytes_total = None`）但已开始接收数据
- **那么** McpProgressSink 计算 progress 时该 item 贡献 fraction 0.5；进度仍单调非递减（item 完成时 fraction 跳到 1.0）

#### 场景:cancellation 时发 BatchCancelled 而非 DownloadFinished
- **当** `download_media` 被 cancel 中断，已完成 K 个 item（含状态为 downloaded / skipped_existing / failed / cancelled 的全部已结束 item），N-K 个尚未启动 item 也标 cancelled
- **那么** server **不**发送 `DownloadFinished` 通知；必须发送 `BatchCancelled` 通知（progress 字段 `>= K` 但 `< total_items`，message 含 `"cancelled"` 字样及 `downloaded=X cancelled=Y` 等 partial summary）

### 需求:cancellation 通知在 v2.0 被忽略但必须记录

`xld serve --mcp` 在收到 MCP `notifications/cancelled` 通知时必须**真实生效**地中断对应 request id 正在运行的工具调用。具体路由由 rmcp 1.6 内部完成（参见 `rmcp-1.6.0/src/service.rs` 中 `local_ct_pool` + `CancelledNotification` 路径）；本 server 实施层面的契约：

- `download_media` tool handler 必须接收 `RequestContext<RoleServer>` 参数，并把 `Some(ctx.ct.clone())` 通过 `DownloadOpts.cancel` 传给 lib 层 `download_media`。`ctx.ct` 由 rmcp 在收到对应 request id 的 `CancelledNotification` 时自动 cancel，无需 server 自维护 token map
- `on_cancelled` 钩子仅作诊断日志使用：在 stderr 输出至少一行包含 `"cancellation"` 或 `"cancelled"` 字样的文本，含 request id 与可选 reason；**不**主动调 `token.cancel()`（rmcp 已自动 cancel `ctx.ct`，且 `on_cancelled` 钩子在 rmcp 内部 cancel 路由完成**之后**才被调用）
- `on_cancelled` 必须不抛错，无论 request id 当前是否在 in-flight（race condition：notification 可能晚于 tool response 到达，rmcp 内部 `local_ct_pool.remove` 找不到对应条目时静默处理）

被 cancel 的 `download_media` 工具调用必须返回 `CallToolResult { isError: false, content: [text(serialize(DownloadOutput))] }`，其中 `DownloadOutput` 含完整 `downloads[]`：已完成的 item 状态保持原值（`downloaded` / `skipped_existing` / `failed`）；被 cancel 中断的 in-flight item 状态为 `cancelled`；尚未启动的 item 状态也为 `cancelled`。`summary.cancelled` 字段反映 cancelled item 计数。Agent 据此可决定是否对部分 item 重试。

**实施约束**：本 server **不**自维护 `in_flight: HashMap<RequestId, CancellationToken>` map——那是重复实现 rmcp 1.6 已经做好的事。原 v0 design 草案中的 in_flight map 已在 design D6 中明确删除。

**v2.1 supersede 说明**：本需求（v2.1）取代 v2.0 design D10 的"忽略 cancellation"策略；v2.0 archived design `add-mcp-server` 中 D10 的"v2.1 实施真实 cancellation"承诺由本变更兑现。

#### 场景:cancellation 真实中断进行中的下载
- **当** 客户端在 `download_media` 进行中（如已下完 2/5 个 item，第 3 个正在下到 30% bytes）发 `notifications/cancelled`
- **那么** server 必须在 1 秒内停止第 3 个 item 的 chunk 接收循环，关闭其 partial 文件 fd（`.partial` 后缀的文件保留），并向 4 / 5 这两个未启动的 item 也标记 cancelled；最终 CallToolResult 含 5 个 item，其中 2 个 downloaded、3 个 cancelled

#### 场景:cancellation 仍记 stderr 诊断日志
- **当** 客户端发 `notifications/cancelled` 给一个 request id
- **那么** server 必须在 stderr 输出至少一行包含 "cancellation" 或 "cancelled" 字样的诊断文本，含该 request id

#### 场景:对 unknown / 已结束 request 的 cancellation silent ignore
- **当** 客户端对一个 request id 发 `notifications/cancelled`，但该请求已经返回（race condition：notification 晚于 tool response 到达，rmcp 内部 `local_ct_pool` 中无对应条目）
- **那么** server 不抛错；rmcp 内部静默处理（cancel 已无效）；server 的 `on_cancelled` 钩子仍会被调用，stderr 必须输出至少一行 "cancellation" 或 "cancelled" 字样的诊断文本，含 request id

#### 场景:cancellation 后 CallToolResult 含 partial summary
- **当** `download_media` 被 cancel 中断时已完成 1 个 item，1 个 in-flight 中断到 30% bytes，1 个尚未启动
- **那么** CallToolResult 必须满足：`isError: false`；`content` 含 serialize 的 DownloadOutput；DownloadOutput.downloads 长度 = 3；status 分布为 1 个 downloaded + 2 个 cancelled；`summary.cancelled == 2`；`summary.downloaded == 1`；`summary.total == 3`；in-flight 的那个 item 在 sandbox 留下对应 `<filename>.partial` 文件
