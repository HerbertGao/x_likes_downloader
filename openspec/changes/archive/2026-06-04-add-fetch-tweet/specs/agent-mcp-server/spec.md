## 修改需求

### 需求:tools/list 暴露 4 个工具

`xld serve --mcp` 必须在响应 MCP `tools/list` 请求时返回且仅返回以下 5 个工具（本次变更由 4 个扩为 5 个，新增 `fetch_tweet`），每个工具必须包含名称、中文描述、JSON Schema 形式的输入参数：

- `list_likes`：拉取当前账号的点赞推文列表
- `download_media`：按一组 MediaItem 下载媒体到沙箱目录
- `auth_status`：探测当前凭据是否仍可访问 X Likes 端点
- `setup_from_curl`：从 cURL 文本导入凭据 + 协议参数
- `fetch_tweet`：按 URL 或 tweet_id 抓取任意一条推文的媒体元数据，返回与 `list_likes` 同构的 TweetSummary

工具的 JSON Schema 必须由对应 Rust 请求类型自动派生（`schemars` crate），保证类型定义与 schema 一致。`fetch_tweet` 的输入类型 `FetchTweetRequest` 必须与现有 MCP 请求类型放在一致位置（沿用现有 server 端请求类型的组织方式）。

`tools/list` 必须**不**包含：`organize`、旧 `xld download` 一把梭、任何下载之外的写操作工具。

#### 场景:工具列表完整
- **当** MCP 客户端发送 `tools/list` 请求
- **那么** 响应必须含 5 个工具且工具名集合等于 `{list_likes, download_media, auth_status, setup_from_curl, fetch_tweet}`

#### 场景:每个工具有 schema
- **当** 解析 `tools/list` 响应中任一工具
- **那么** 该工具必须有 `inputSchema` 字段，且为合法 JSON Schema 对象

#### 场景:工具描述用中文
- **当** 解析任一工具的 `description` 字段
- **那么** 该字段必须为非空 UTF-8 字符串，描述工具用途与典型场景

### 需求:tools/call 调用语义

MCP `tools/call` 请求必须按以下规则处理：

- 调用 lib 层对应函数（`agent::list_likes` / `agent::download_media` / `agent::auth_status` / `agent::import_curl` / `agent::fetch_tweet`），**不得通过 fork 子进程调 CLI**
- 工具参数从请求的 `arguments` 字段解析；`download_media` 的 `items[]` 字段必须能反序列化为 `agent::types::MediaItem` 数组；`fetch_tweet` 的 `arguments` 必须能反序列化为 `FetchTweetRequest`（含互斥的 `url` 或 `id` 字段；schemars 派生的 JSON Schema 无法表达「恰好其一」的互斥，互斥由 lib 层运行时校验兜底并由单测覆盖）
- 成功响应必须为 `CallToolResult { isError: false, content: [TextContent { text: <serialized data> }] }`，content text 是 lib 函数返回值的 JSON 序列化
- 业务错误（auth_expired / endpoint_stale / tweet_unavailable / invalid_argument 等）必须返回 `CallToolResult { isError: true, content: [TextContent { text: <error JSON> }] }`，错误 JSON 含 `kind` / `message` / `hint` / `retry_after?` 字段
- 真 panic / serde 失败 / 协议解析失败才使用 JSON-RPC level error response（code -32603 InternalError）

#### 场景:正常调用
- **当** MCP 客户端调用 `tools/call { name: "list_likes", arguments: {count: 3} }` 且凭据有效
- **那么** 响应必须为 `CallToolResult { isError: false, content: [{type:"text", text: <ListOutput JSON>}] }`，文本内容反序列化后含 `tweets[]` / `cursor` / `schema_version`

#### 场景:fetch_tweet 正常调用
- **当** MCP 客户端调用 `tools/call { name: "fetch_tweet", arguments: {id: "2061856515569614983"} }` 且凭据有效、推文可见
- **那么** 响应必须为 `CallToolResult { isError: false, content: [{type:"text", text: <FetchTweetOutput JSON>}] }`，文本内容反序列化后含 `tweet`（其下含 `id` / `media`）与 `schema_version` 字段（结构镜像 `list_likes` 的 `ListOutput`）

#### 场景:业务错误返回 isError true
- **当** 客户端调用 `tools/call { name: "list_likes" }` 且本地未配置凭据
- **那么** 响应必须为 `isError: true`，content text 反序列化后含 `kind: "not_configured"`、`message`、`hint` 字段

#### 场景:fetch_tweet 推文不可见返回 isError true
- **当** 客户端调用 `tools/call { name: "fetch_tweet", arguments: {id: "<已删除推文>"} }`
- **那么** 响应必须为 `isError: true`，content text 反序列化后含 `kind: "tweet_unavailable"`

#### 场景:协议错误才用 JSON-RPC error
- **当** 客户端调用未知工具名（如 `tools/call { name: "nonexistent" }`）
- **那么** server 必须以 JSON-RPC error response 形式响应（`code: -32601 Method not found` 或 `-32602 Invalid params`），而不是 isError CallToolResult
