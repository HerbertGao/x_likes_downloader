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

- `progress`：**已完成的 item 数**（绝对计数，单调非递减）。批次开始为 0，每完成一个 item 递增 1，批次结束为 `total_items`
- `total`：批次总 item 数；与 `progress` 配对，client 用 `progress / total` 计算百分比
- `message`：当前操作描述，含字节级进度（如 `tweet 12345 1024/4096`）

**事件映射**：

- 批次开始（`DownloadStarted`）→ `progress: 0.0, total: total_items, message: "starting N items, concurrency=n"`
- 单项启动（`ItemStarted`）→ `progress: items_done, total: total_items, message: "starting tweet <id>"`（用 `items_done` 而非 `index` 保单调）
- 单项字节进度（`ItemProgress`）→ `progress: items_done, total: total_items, message: "tweet <id> <bytes_done>/<bytes_total>"`（progress 字段不加 fraction，否则多并发时会回退）
- 单项完成（`ItemDone`）→ `progress: items_done+1, total: total_items, message: "tweet <id> <status>"`
- 批次结束（`DownloadFinished`）→ `progress: total_items, total: total_items, message: "done: ..."`

**关键不变量**：`progress` 单调非递减，无论 `concurrency` 取值。完成时 `progress == total`（client 比例 = 1.0）。

**Flush**：tool handler 在返回前必须 await sink 的 flush，让 worker drain 所有 pending notification——避免 final notification 跟 tool response 竞争被丢弃。

实现层面：`agent::mcp_server` 模块新增 `McpProgressSink` 实现既有 `ProgressSink` trait + 暴露 async `flush(&self)`；内部用 mpsc 单 worker task 串行化保证 FIFO 投递。

#### 场景:有 progressToken 时发 notification
- **当** 客户端发送 `tools/call { name: "download_media", _meta: { progressToken: "..." } }` 下载多个 item
- **那么** server 必须发送至少一个 `notifications/progress` 通知，含相同的 progressToken；`progress` 字段为 `[0, total_items]` 内的非递减整数 / 浮点数；`total` 字段等于 `total_items`

#### 场景:无 progressToken 时不发 notification
- **当** 客户端调用 `download_media` 但请求中无 progressToken
- **那么** server 不发送任何 progress 通知，但必须正常完成下载并返回最终结果

#### 场景:批次开始与结束必发
- **当** 客户端使用 progressToken 调用 `download_media` 下载至少 1 个 item
- **那么** 必须存在至少一个 `progress: 0.0` 起始通知与一个 `progress == total_items` 结束通知（即 client 计算出比例 1.0）

#### 场景:多并发下 progress 单调
- **当** 客户端用 `progressToken` 调用 `download_media`，`concurrency=4` 下载 5 个 item
- **那么** server 发送的 progress notification 序列必须满足 `progress` 字段单调非递减，无任何"回退"或大于 `total_items` 的值

#### 场景:tool 返回前必须 flush
- **当** server 即将为 `download_media` 调用返回 `tools/call` 响应
- **那么** 它必须先确保所有 pending 的 progress notification 已被发送给 transport（即 sink 的 worker task 已 drain）

### 需求:cancellation 通知在 v2.0 被忽略但必须记录

`xld serve --mcp` 在收到 MCP `notifications/cancelled` 通知时必须：

- 解析通知，识别其 request id
- 在 stderr 输出诊断日志，至少含 request id 与一句 "received cancellation notification, ignoring (v2.0 limitation)"
- **不**中断对应 request id 正在运行的工具调用——令其继续完成并返回正常 `CallToolResult`
- **不**清理任何已经写入磁盘的下载文件
- **不**改变 lib 层 `agent::download_media` 等函数的签名（不引入 `CancellationToken` 参数）

客户端如需强制终止 server 正在跑的工具，必须通过关闭 stdin 实现（让 server 走 EOF 优雅关闭路径——参见"stdin EOF 触发优雅关闭"需求）。v2.1 视实战需求实施真实 cancellation 响应，届时再修订此需求。

#### 场景:cancellation 不中断进行中的工具
- **当** 客户端在 `download_media` 进行中（如已下完 2/5 个 item）发 `notifications/cancelled`
- **那么** server 必须继续完成剩余 3 个 item 的下载，最终返回 `CallToolResult { isError: false, ... }` 含 5 个 item 的 summary

#### 场景:cancellation 必须留诊断日志
- **当** 客户端发 `notifications/cancelled` 给一个 request id
- **那么** server 必须在 stderr 输出至少一行包含 "cancellation" 字样的诊断文本，含该 request id

#### 场景:cancellation 不修改 lib 函数签名
- **当** 检视 `agent::download_media` / `agent::list_likes` / `agent::auth_status` / `agent::import_curl` 的函数签名
- **那么** 这些签名必须与 v1 完全一致（不得新增 `cancellation` / `CancellationToken` 参数；本变更不涉及 lib 层契约修改）

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
