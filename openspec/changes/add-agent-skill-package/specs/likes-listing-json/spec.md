## 新增需求

### 需求:CLI 子命令 `xld likes list`

系统必须提供 `xld likes list` 子命令，用于以纯结构化 JSON 格式输出当前账号的点赞推文列表。该子命令必须支持以下参数：

- `--all`：拉取全部历史点赞（按 GraphQL 游标翻页直至无更多数据）；缺省时仅拉取首页
- `--since-cursor <cursor>`：从指定游标开始拉取，用于增量同步
- `--count <n>`：单页条数，默认沿用配置值
- `--include-raw`：可选，附带原始 GraphQL entry（默认关闭，开启后大幅增加 token 成本）
- `--json`：输出 JSON 信封到 stdout（在新子命令中此为强制行为，参数仅作显式标记）

#### 场景:首页拉取
- **当** 用户运行 `xld likes list --json` 且本地凭据有效
- **那么** 系统调用 X GraphQL `Likes` 端点拉取一页数据，将结果序列化为成功信封写入 stdout，进程退出码为 0

#### 场景:全量拉取
- **当** 用户运行 `xld likes list --all --json`
- **那么** 系统按 `cursor-bottom-` 游标翻页直至游标为空或与上一次相同，全部 tweet 合并到单个信封的 `data.tweets[]` 中输出

#### 场景:增量拉取
- **当** 用户运行 `xld likes list --since-cursor <c> --json`
- **那么** 系统以 `<c>` 作为初始游标拉取数据，输出新拉取的 tweet 与最新游标值

### 需求:JSON 输出信封格式

`xld likes list --json` 输出的 JSON 必须为单个根对象，禁止使用 NDJSON 或多行流式输出。成功信封必须包含字段 `ok: true`、`data` 与 `meta`；失败信封必须包含 `ok: false` 与 `error`。`meta` 必须包含 `schema_version`（数字，初版为 1）和 `cursor`（最新游标，可能为 null）。

#### 场景:成功信封结构
- **当** 拉取成功且 stdout 输出 JSON
- **那么** 该 JSON 解析后必须满足 `result.ok === true && Array.isArray(result.data.tweets) && typeof result.meta.schema_version === 'number'`

#### 场景:游标可为 null
- **当** 已拉取到列表末尾且无更多数据
- **那么** `meta.cursor` 必须为 `null`

### 需求:扁平 tweet 字段集（v1 schema）

`data.tweets[]` 中每个元素必须为扁平形态对象，**严格包含且仅包含以下字段集**（v1 schema，未声明字段禁止出现）：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `id` | string | 是 | tweet ID（数字字符串） |
| `author_handle` | string | 是 | 用户 @ 名（不含 @） |
| `author_display_name` | string | 是 | 显示名（可空字符串） |
| `text` | string | 是 | tweet 正文文本（无 t.co 短链替换） |
| `created_at` | string | 是 | RFC3339 格式 tweet 发布时间 |
| `tweet_url` | string | 是 | 形如 `https://x.com/<handle>/status/<id>` |
| `is_retweet` | boolean | 是 | 是否为 RT |
| `is_reply` | boolean | 是 | 是否为回复 |
| `media` | MediaItem[] | 是 | 媒体列表（无媒体时为空数组） |
| `liked_at` | string \| null | 否 | 点赞时间（X API 不一定返回，可为 null） |

`MediaItem` 必须为：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `tweet_id` | string | 是 | 与父 tweet 的 `id` 一致 |
| `type` | enum | 是 | `image` / `video` / `gif` 三选一 |
| `url` | string | 是 | 该媒体的"最佳"直链：image 取最大尺寸 `:orig`；video/gif 取所有 mp4 variant 中 bitrate 最高者 |
| `suggested_filename` | string | 是 | 由服务端基于 URL 派生的目标文件名（不含目录） |
| `bytes` | number \| null | 否 | 预期字节数，从 X 响应取得；不可得时为 null |
| `author_handle` | string \| null | 否 | 父推文的作者 @ 名。`list_likes` 总是填（与父 `TweetSummary.author_handle` 一致），Agent 手工构造 MediaItem 时可省略。`download_media` 默认命名格式使用此字段 |
| `created_at` | string \| null | 否 | 父推文发布时间（X 原始 RFC2822 字符串）。`list_likes` 总是填（与父 `TweetSummary.created_at` 一致）。`download_media` 默认会据此设置文件 mtime |
| `all_variants` | object[] \| null | 否 | 仅当 `--include-raw` 启用时携带；包含所有可用 variants |

#### 场景:扁平 tweet 字段完整
- **当** `data.tweets[]` 中存在条目
- **那么** 每个条目必须含有上表所有"必填"字段，未声明字段必须不出现

#### 场景:media 数组结构稳定
- **当** 任意 tweet 含媒体
- **那么** `media[]` 中每个元素的 `tweet_id` 必须等于父 tweet 的 `id`，`type` 必须为枚举三值之一，`url` 必须为完整 https URL

#### 场景:video 选最高码率 mp4
- **当** 一个 video tweet 在 X 响应中含有多个 mp4 variants 与一个 m3u8
- **那么** `media[].url` 必须为 mp4 variants 中 `bitrate` 最高者的 URL；m3u8 不出现在 `url`

#### 场景:image 选原始尺寸
- **当** 一个 image tweet 的 X 响应给出 `https://pbs.twimg.com/media/XXX.jpg`
- **那么** `media[].url` 必须为 `https://pbs.twimg.com/media/XXX.jpg?format=jpg&name=orig` 或等价的最大尺寸形式

### 需求:可选原始 entry 携带（opt-in）

`--include-raw` 标志启用时，成功信封的 `data` 部分必须额外包含 `raw_entries[]` 数组，每个元素为 X GraphQL 原始 entry 对象（即当前 `parse_likes_response` 收集到的 `tweet-` 前缀 entry 的完整 JSON）。`raw_entries[]` 与 `tweets[]` 的元素顺序必须一一对应。

`--include-raw` 默认关闭。文档必须明示该选项会显著增加 stdout token 成本（典型情况 5-10 倍），仅供调试或一次性深度提取使用。

#### 场景:默认不含原始 entry
- **当** 运行 `xld likes list --json`（不带 `--include-raw`）
- **那么** `data` 部分必须不包含 `raw_entries` 字段

#### 场景:opt-in 原始 entry 一一对应
- **当** 运行 `xld likes list --include-raw --json`
- **那么** `data.raw_entries.length === data.tweets.length`，且对任意索引 `i`，`raw_entries[i]` 派生出的 tweet ID 必须等于 `tweets[i].id`

### 需求:stdout/stderr 输出严格分离

执行 `xld likes list --json` 期间，所有调试日志、进度提示、HTTP 请求详情必须输出到 stderr；stdout 必须仅包含最终的 JSON 信封一次。系统禁止在 stdout 输出任何非 JSON 字符（包括但不限于"请求 URL"、"本页获取到 N 条"等当前 `x_api.rs` 中的 `println!` 文案）。

#### 场景:stdout 严格 JSON
- **当** 重定向 `xld likes list --json > out.json 2> err.log`
- **那么** `out.json` 必须可被 `serde_json::from_str` 一次性解析为有效信封；`err.log` 可包含任意诊断文本

#### 场景:错误时 stdout 仍输出有效 JSON
- **当** 凭据失效导致请求失败
- **那么** stdout 必须输出失败信封 JSON，stderr 可包含诊断信息，进程以非零退出码结束

### 需求:错误分类与退出码

`xld likes list --json` 必须将错误归类为有限种类，使用结构化 `error.kind` 字段表达，并对应特定退出码。`error.kind` 必须为以下值之一：`auth_expired`、`endpoint_stale`、`rate_limited`、`network_error`、`not_configured`、`internal_error`。退出码规则：成功 0；可由用户/Agent 行动后重试的错误（`auth_expired` / `endpoint_stale` / `rate_limited` / `network_error`）退 2；不可恢复的（`not_configured` / `internal_error`）退 1。

#### 场景:认证过期分类
- **当** X 返回 401 或 403
- **那么** stdout 输出 `{ ok: false, error: { kind: "auth_expired", ... } }`，进程退出码为 2

#### 场景:协议过期分类
- **当** X 返回 404 或 410（GraphQL queryId 滚动）
- **那么** stdout 输出 `{ ok: false, error: { kind: "endpoint_stale", ... } }`，进程退出码为 2

#### 场景:配置缺失
- **当** 本地未导入过 cURL，必要字段为空
- **那么** stdout 输出 `{ ok: false, error: { kind: "not_configured", ... } }`，进程退出码为 1

### 需求:lib 层函数 `list_likes`

系统必须在 `xld` lib crate 中暴露 `list_likes(opts: ListOpts) -> Result<ListOutput>` 异步函数，作为 CLI 子命令与未来 MCP server 共用的能力实现。`ListOpts` 必须支持 `all: bool`、`since_cursor: Option<String>`、`count: Option<u32>`、`include_raw: bool`。`ListOutput` 必须等价于 JSON 信封中 `data` 部分的强类型版本。函数禁止直接写 stdout/stderr。

#### 场景:lib 函数无副作用输出
- **当** 调用 `list_likes(opts)` 且不通过 CLI 进入
- **那么** 函数返回结果对象，不向标准输出/标准错误写入任何内容

#### 场景:lib 函数与 CLI 共享实现
- **当** `xld likes list --json` 与未来 MCP server 同时存在
- **那么** 两者必须最终调用同一份 `list_likes` 实现，禁止存在第二份等价逻辑
