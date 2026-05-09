### 需求:CLI 子命令 `xld serve --mcp`

系统必须提供 `xld serve --mcp` 子命令，用于启动一个本地 MCP server，stdio transport 通信，长驻直至客户端关闭。该子命令必须：

- 以 `rmcp`（Rust MCP SDK，官方）为基础实现，绑定 stdio transport
- 在 stdin 读取 MCP 客户端发来的 JSON-RPC 请求；在 stdout 输出响应与 notifications；stderr 仅用于诊断日志（结构化或自由文本均可，但禁止与 stdout 混淆）
- 进程生命周期与 stdin EOF 绑定——stdin 关闭时优雅关闭 server
- 退出码：正常关闭 0；启动失败（端口冲突 / 权限等不应有但兜底）非零

#### 场景:子命令存在
- **当** 运行 `xld serve --help`
- **那么** help 输出必须含 `--mcp` flag 描述

#### 场景:启动后等待 stdin
- **当** 运行 `xld serve --mcp` 且 stdin 未关闭
- **那么** 进程必须持续运行（不立即退出），等待 JSON-RPC 输入

#### 场景:stdin EOF 触发优雅关闭
- **当** stdin 关闭（client 关闭 pipe）
- **那么** server 必须优雅关闭（释放 in-flight 请求 / 清理临时状态），进程以退出码 0 退出

### 需求:tools/list 暴露 4 个工具

`xld serve --mcp` 必须在响应 MCP `tools/list` 请求时返回且仅返回以下 4 个工具，每个工具必须包含名称、中文描述、JSON Schema 形式的输入参数：

- `list_likes`：拉取当前账号的点赞推文列表
- `download_media`：按一组 MediaItem 下载媒体到沙箱目录
- `auth_status`：探测当前凭据是否仍可访问 X Likes 端点
- `setup_from_curl`：从 cURL 文本导入凭据 + 协议参数

工具的 JSON Schema 必须由 `agent::types` 的 Rust 类型自动派生（建议使用 `schemars` crate），保证类型定义与 schema 一致。

`tools/list` 必须**不**包含：`organize`、旧 `xld download` 一把梭、任何下载之外的写操作工具。

#### 场景:工具列表完整
- **当** MCP 客户端发送 `tools/list` 请求
- **那么** 响应必须含 4 个工具且工具名集合等于 `{list_likes, download_media, auth_status, setup_from_curl}`

#### 场景:每个工具有 schema
- **当** 解析 `tools/list` 响应中任一工具
- **那么** 该工具必须有 `inputSchema` 字段，且为合法 JSON Schema 对象

#### 场景:工具描述用中文
- **当** 解析任一工具的 `description` 字段
- **那么** 该字段必须为非空 UTF-8 字符串，描述工具用途与典型场景

### 需求:tools/call 调用语义

MCP `tools/call` 请求必须按以下规则处理：

- 调用 lib 层对应函数（`agent::list_likes` / `agent::download_media` / `agent::auth_status` / `agent::import_curl`），**不得通过 fork 子进程调 CLI**
- 工具参数从请求的 `arguments` 字段解析；`download_media` 的 `items[]` 字段必须能反序列化为 `agent::types::MediaItem` 数组
- 成功响应必须为 `CallToolResult { isError: false, content: [TextContent { text: <serialized data> }] }`，content text 是 lib 函数返回值的 JSON 序列化
- 业务错误（auth_expired / endpoint_stale / invalid_argument 等）必须返回 `CallToolResult { isError: true, content: [TextContent { text: <error JSON> }] }`，错误 JSON 含 `kind` / `message` / `hint` / `retry_after?` 字段
- 真 panic / serde 失败 / 协议解析失败才使用 JSON-RPC level error response（code -32603 InternalError）

#### 场景:正常调用
- **当** MCP 客户端调用 `tools/call { name: "list_likes", arguments: {count: 3} }` 且凭据有效
- **那么** 响应必须为 `CallToolResult { isError: false, content: [{type:"text", text: <ListOutput JSON>}] }`，文本内容反序列化后含 `tweets[]` / `cursor` / `schema_version`

#### 场景:业务错误返回 isError true
- **当** 客户端调用 `tools/call { name: "list_likes" }` 且本地未配置凭据
- **那么** 响应必须为 `isError: true`，content text 反序列化后含 `kind: "not_configured"`、`message`、`hint` 字段

#### 场景:协议错误才用 JSON-RPC error
- **当** 客户端调用未知工具名（如 `tools/call { name: "nonexistent" }`）
- **那么** server 必须以 JSON-RPC error response 形式响应（`code: -32601 Method not found` 或 `-32602 Invalid params`），而不是 isError CallToolResult

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

### 需求:每次工具调用重新加载 Config

每次 MCP 工具调用（`tools/call`）必须由 lib 函数内部重新调用 `Config::load()` 加载凭据，不得在 server 启动时一次性缓存 Config 长期持有。

#### 场景:cookie 热加载
- **当** server 已运行，用户在外部跑 `xld setup --curl-file <new>` 更新凭据
- **那么** 下一次工具调用必须读取到新凭据，无需客户端重启

### 需求:仅 stdio transport

`xld serve --mcp` 必须仅启动 stdio transport，禁止启动 HTTP / SSE 等远程 transport。即便未来 rmcp 支持其它 transport，本子命令必须保持仅 stdio。

#### 场景:子命令不接受 HTTP flag
- **当** 运行 `xld serve --mcp --http <addr>` 或类似变体
- **那么** clap 必须拒绝该参数（unknown argument）；server 不得在任何端口监听

### 需求:凭据 / 敏感字段不通过 MCP 通道泄露

`tools/call` 响应（无论 isError 还是 success）必须不包含 cookie（`auth_token` / `ct0`） / `bearer_token` / `personalization_id` 等敏感字段。`setup_from_curl` 工具的成功返回必须仅含 `written: bool` / `path: string` / `protocol_params_extracted: bool`，不回显输入的 cURL 文本或解析出的凭据值。

#### 场景:setup_from_curl 不回显凭据
- **当** 客户端调用 `setup_from_curl` 传入完整 cURL 文本
- **那么** 响应 content text 反序列化后必须不含 `auth_token`、`ct0`、`bearer_token`、`cookie`、`Authorization` 等任意密钥字段

#### 场景:auth_status 不回显凭据
- **当** 客户端调用 `auth_status`
- **那么** 响应 content text 必须不含 cookie / bearer 等字段；仅含 `status: "healthy" | <error kind>` 与 `checked_at`

### 需求:MCP server 模块独立，不污染人类 CLI 路径

MCP server 实现必须位于 `src/agent/mcp_server.rs`，仅在 `xld serve --mcp` 路径加载；`xld likes list` / `xld media download` / `xld auth status` / `xld setup` / `xld download` / `xld organize` / `xld update` 等所有人类 CLI 子命令的执行路径**禁止**调用 mcp_server 模块的任何函数。

#### 场景:人类 CLI 调用栈不含 mcp_server
- **当** 用户运行 `xld likes list --json`
- **那么** 该子命令的执行路径必须直接调用 `agent::list_likes` lib 函数，不得经过 `mcp_server::*`
