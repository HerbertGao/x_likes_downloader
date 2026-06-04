## 目的

提供 `xld tweet get` 子命令，按 URL 或 tweet_id 抓取任意一条推文（不限于当前账号点过赞的推文）的媒体元数据，经 `TweetDetail` GraphQL 端点解析为扁平 TweetSummary 并以结构化 JSON 信封输出，复用现有错误分类体系。

## 需求

### 需求:CLI 子命令 `xld tweet get`

系统必须提供 `xld tweet get` 子命令，用于按 URL 或 tweet_id 抓取任意一条推文（不限于当前账号点过赞的推文）的媒体元数据，以结构化 JSON 信封输出。该子命令必须支持以下互斥参数二选一：

- `--url <url>`：推文 URL，形如 `https://x.com/<handle>/status/<id>`（亦接受 `twitter.com` 域名、`i/web/status/<id>` 形态、`/photo/1` 等路径后缀、以及带 query/fragment 的 URL）
- `--id <id>`：纯数字 tweet_id

并支持：

- `--json`：输出 JSON 信封到 stdout（新子命令默认即 JSON，参数仅作显式标记）

子命令必须遵循信封约定:数据写 stdout、诊断日志写 stderr。退出码由失败信封的 `ErrorKind::exit_code()` 决定（见下「错误分类」需求），与现有 `likes list` / `auth status` 命令同一套映射；参数解析阶段（`--url`/`--id` 同缺或同给、id 非数字、URL 无法解析）以 `invalid_argument` 失败。

#### 场景:按 URL 抓取
- **当** 用户运行 `xld tweet get --url https://x.com/ll378458176/status/2061856515569614983 --json` 且本地凭据有效，推文存在且可见
- **那么** 系统调用 X GraphQL `TweetDetail` 端点，将焦点推文序列化为成功信封写入 stdout，进程退出码为 0

#### 场景:按 id 抓取
- **当** 用户运行 `xld tweet get --id 2061856515569614983 --json`
- **那么** 系统行为与按 URL 抓取一致，输出同构信封

#### 场景:URL 与 id 同时缺失或同时提供
- **当** 用户既未提供 `--url` 也未提供 `--id`，或两者同时提供
- **那么** 系统必须以 `invalid_argument` 失败（退出码取 `ErrorKind::InvalidArgument.exit_code()`，当前为 1），并在 stderr 给出参数用法提示

### 需求:URL 与 tweet_id 输入解析

系统必须能从 `--url` 提取 tweet_id：匹配路径中 `/status/<digits>` 段（`<digits>` 后遇非数字即止，因此 `/status/123/photo/1`、`/status/123/video/1` 均能正确截出 `123`），忽略 query string、fragment、尾随斜杠及大小写域名差异（`x.com` / `twitter.com` / `mobile.twitter.com` / `x.com/i/web/status/<id>` 等）。`--id` 必须为纯数字字符串。任一来源解析出的 tweet_id 必须为非空数字串，否则视为非法输入。

#### 场景:从带 query 的 URL 提取 id
- **当** 输入 URL 为 `https://x.com/foo/status/123456789?s=20&t=abc`
- **那么** 解析出的 tweet_id 必须为 `123456789`

#### 场景:从 i/web/status 与 photo 后缀 URL 提取 id
- **当** 输入 URL 为 `https://x.com/i/web/status/123` 或 `https://x.com/foo/status/123/photo/1`
- **那么** 解析出的 tweet_id 必须均为 `123`

#### 场景:非法 URL
- **当** 输入 URL 不含 `/status/<digits>` 段（如 `https://x.com/foo`）
- **那么** 系统必须以 `invalid_argument` 失败，错误信息指明无法从 URL 解析 tweet_id

#### 场景:非数字 id
- **当** `--id` 值含非数字字符（如 `abc` 或 `12a3`）
- **那么** 系统必须以 `invalid_argument` 失败

### 需求:TweetDetail GraphQL 请求构造与鉴权

系统调用 `TweetDetail` 端点时，必须复用与 `Likes` 请求一致的鉴权头构造:`Authorization: Bearer <bearer_token>`、`Cookie: auth_token=...; ct0=...`、`X-Csrf-Token: <ct0>`、`User-Agent: <user_agent>`。请求 URL 必须由 `tweet_detail_api_url`、`tweet_features`、`tweet_fieldtoggles` 三个配置字段与 `variables` 拼接，三者均须 URL 编码。`variables` 必须包含 X Web 当前 `TweetDetail` 请求所需的完整字段集（实测确认）：至少 `focalTweetId`（目标 tweet_id）、`with_rux_injections:false`、`rankingMode`、`includePromotedContent`、`withCommunity`、`withQuickPromoteEligibilityTweetFields`、`withBirdwatchNotes`、`withVoice`——只发 `focalTweetId` 单字段会被 X 拒（features/variables 不匹配）。`tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles` 必须经现有 `resolve_protocol_field` 解析（层序 env > `private_tokens.env` > `defaults.json` > 代码硬编码）。注意：`setup_from_curl` 当前只解析 Likes 端点 cURL、不写 `TWEET_*` 到 `private_tokens.env`，故对这三字段 private_tokens 层实际为空——有效来源为 env / defaults.json / 硬编码三层；真值由 `defaults.json` 承载，用户如需覆盖走 env 或手改 defaults.json（本变更不扩展 setup 去解析 TweetDetail cURL）。

#### 场景:variables 含完整必填字段
- **当** 抓取 tweet_id 为 `999` 的推文
- **那么** GraphQL 请求 `variables` 中 `focalTweetId` 字段必须等于 `"999"`，且必须含 `with_rux_injections` / `withVoice` 等上述必填项

#### 场景:鉴权头与 Likes 一致
- **当** 构造 `TweetDetail` 请求
- **那么** 请求头必须含 `Authorization`、`Cookie`（含 `auth_token` 与 `ct0`）、`X-Csrf-Token`、`User-Agent`，取值来源与 `Likes` 请求相同；且实现禁止将含 cookie/bearer 的请求头或完整 URL 打印到 stderr

### 需求:焦点推文解析为扁平 TweetSummary

系统必须从 `TweetDetail` 响应的 `data.threaded_conversation_with_injections_v2.instructions[]` 中定位焦点 entry，分两级：**主路径**匹配顶层 `entryId == "tweet-<focalTweetId>"` 的 `TimelineAddEntries` 焦点 entry（实测样本——顶层公开非回复推文——均以此形态出现，回复链单独位于 `conversationthread-*` module）；**软兜底**：主路径未命中时，必须遍历所有 instructions 的 entries（含 `conversationthread-*` module 内的 item），按解包 wrapper 后 `tweet_results.result...rest_id == focalTweetId` 定位（应对焦点为回复链中间节点等未取样形态）。两级均未命中才判 `tweet_unavailable`。定位到的 entry 必须复用与 `list_likes` 相同的 `entry_to_summary` / `extract_media` 逻辑解析。`entry_to_summary` 既有的 `result.tweet` 解包逻辑必须覆盖焦点 `tweet_results.result.__typename == "TweetWithVisibilityResults"` 的情形（实测：较早推文常返回该 wrapper，真正的 tweet 对象嵌在 `result.tweet`）。产出必须为与 `likes list` 输出**严格同构**的扁平 `TweetSummary`（字段集 `id` / `author_handle` / `author_display_name` / `text` / `created_at` / `tweet_url` / `is_retweet` / `is_reply` / `media[]` / `liked_at`）。`media[]` 中 `MediaItem` 的形态与最佳直链选取规则必须**完全复用现有 `extract_media`**（不重新规定选取算法，不引入与 `extract_media` 取数口径不同的并行计数）。`fetch_tweet` lib 函数返回 `FetchTweetOutput { tweet: TweetSummary, schema_version }`（镜像 `list_likes` 的 `ListOutput`）；CLI 信封把 `tweet` 放 `data`、`schema_version` 放 `meta`，故成功信封的 `data` 为单个 `TweetSummary` 对象、`meta` 含 `schema_version`（数字）；MCP 工具直接返回 `FetchTweetOutput`。

**已知边界**（沿用 `extract_media` 既有行为，非本变更引入）：当焦点视频仅有 m3u8 而无带 bitrate 的 mp4 variant 时，`extract_media` 对该项产出空，`media[]` 因此可能为空或缺项——本变更不为此新增诊断字段（避免重造解析），HLS 下载支持须另开 change。该边界与 `list_likes` 一致。

#### 场景:输出 schema 与 list_likes 同构
- **当** 抓取一条含视频的推文成功
- **那么** 信封 `data` 解析后必须满足 `data.id` 为数字字符串、`Array.isArray(data.media)` 为真，且每个 media item 含 `tweet_id` / `type` / `url` / `suggested_filename`，字段语义与 `likes list` 的 `tweets[].media[]` 一致

#### 场景:TweetWithVisibilityResults wrapper 正确解包
- **当** 焦点推文的 `tweet_results.result.__typename` 为 `TweetWithVisibilityResults`（较早推文常见）
- **那么** 系统必须解包 `result.tweet` 后取 legacy，正确产出非空 `id` 与 `media[]`，不得因 wrapper 而漏取媒体

#### 场景:软兜底按 rest_id 定位焦点（best-effort）
- **当** 顶层无 `entryId == "tweet-<focalTweetId>"`，但某 entry（含 `conversationthread-*` module 内 item）的 `tweet_results.result`（解包 wrapper 后）`rest_id == focalTweetId`
- **那么** 系统应（best-effort）经软兜底定位到该 entry 并正常产出 `TweetSummary`，不误判 `tweet_unavailable`（注：该兜底为防御性回退，尚无真实「焦点为回复链中间推文」样本验证；主路径有实测样本支撑）

#### 场景:焦点真无媒体
- **当** 焦点推文 legacy 不含可产出媒体（无 `extended_entities.media`，或仅含 `extract_media` 不支持的形态如纯 m3u8）
- **那么** `data.media` 必须为空数组 `[]`，信封 `ok` 为 `true`（不区分「真无媒体」与「HLS-only」，沿用 `extract_media` 既有边界）

#### 场景:liked_at 字段缺失
- **当** 抓取任意推文成功
- **那么** 序列化输出中 `liked_at` 字段必须缺失或为 `null`（`TweetSummary.liked_at` 带 `skip_serializing_if = "Option::is_none"`，TweetDetail 无点赞时间语义，故该字段不出现）

### 需求:错误分类复用现有体系

`fetch_tweet` 的失败必须复用现有 `ErrorKind` 体系并新增 `tweet_unavailable`。判定必须**分两阶段**：

**阶段一（请求级，不需要 focal_id）**：先按 HTTP 状态码经现有 `error::classify_status` 分类（401/403→`auth_expired`、404/410→`endpoint_stale`、429→`rate_limited`、5xx→`network_error`）；仅当 HTTP 200 时再查响应体——含顶层 `errors[]` 且任一 `code` 为鉴权类（如 32 / 64 / 89）→ `auth_expired`（X 对失效凭据**常返回 HTTP 200 + errors[]**，不可只靠状态码）；`data` 为 null 或缺 `threaded_conversation_with_injections_v2` 路径且无鉴权类 errors → `endpoint_stale`。reqwest 传输层失败（连接 / 超时 / DNS）→ `network_error`。

**阶段二（焦点级，需 focal_id）**：阶段一放行后进入焦点定位（主路径 + rest_id 软兜底）。当主路径与软兜底均未找到焦点 entry，或焦点 `tweet_results.result.__typename ∈ {"TweetUnavailable", "TweetTombstone"}`，或焦点结果缺可用 `legacy` → `tweet_unavailable`。

（实现上阶段一为纯函数 `classify_tweet_detail(status, resp)`，阶段二在焦点定位/校验步骤产出 `tweet_unavailable`——分类器不持有 focal_id，避免签名错位。）

新增的 `ErrorKind::TweetUnavailable` 必须在 `exit_code()`（归入返回 1 的「业务不可用」组，与 `NotConfigured` 同组）与 `default_hint()` 补齐分支。失败信封必须含 `kind` / `message` / `hint` 字段。

退出码语义说明：现有 `exit_code()` 把 10 个 `kind` 压成 2 档（auth_expired/endpoint_stale/rate_limited/network_error→2，其余含 invalid_argument/tweet_unavailable→1），本变更不扩展该映射。因此退出码仅供人类 shell 做成功/失败粗判；调用方（尤其 Agent）必须依信封 `kind` 而非退出码区分错误类别（如 `tweet_unavailable` 不重试、`auth_expired` 提示重新导入）。

#### 场景:推文不可见
- **当** 抓取一条已删除或受保护的推文，HTTP 200 且响应中无鉴权类 errors、无 `entryId == tweet-<focalTweetId>` 焦点 entry
- **那么** 系统必须返回失败信封，`kind` 为 `tweet_unavailable`，退出码为 `ErrorKind::TweetUnavailable.exit_code()`（1）

#### 场景:凭据失效经 HTTP 200 + errors 判定
- **当** `TweetDetail` 请求返回 HTTP 200，但响应体含 `errors[].code == 32`
- **那么** 系统必须返回失败信封，`kind` 为 `auth_expired`（不得误判为 endpoint_stale 或 tweet_unavailable），`hint` 提示重新导入 cURL

#### 场景:凭据失效经 HTTP 状态码判定
- **当** `TweetDetail` 请求返回 HTTP 401/403
- **那么** 系统必须经 `classify_status` 返回 `auth_expired`
