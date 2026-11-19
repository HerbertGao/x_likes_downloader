## 修改需求

### 需求:`SKILL.md` 工具表声明

`.agents/skills/x_likes/SKILL.md` 必须以一节明确声明 Agent 可调用的工具集，每个工具至少包含：名称、用途一句话、输入参数及类型、返回值结构概述、典型错误 `kind` 列表。声明的工具集必须为且仅为以下六个：

- `list_likes(count?, all?, since_cursor?)` → 列点赞
- `download_media(items[], subdir?, concurrency?)` → 按 MediaItem 数组下载到沙箱
- `auth_status()` → 凭据健康自检
- `setup_from_curl(curl_text)` → 首次配置 / 重新导入 cURL
- `fetch_tweet(url? | id?)` → 按 URL 或 tweet_id 抓取任意一条推文的媒体元数据，返回与 `list_likes` 同构的 TweetSummary（含 `media[]`）
- `user_tweets(username, all?, since_cursor?, count?, include_raw?)` → 按 username 列出指定账号（不限于本人）的推文列表，返回 `user{screen_name, rest_id, name}` + 与 `list_likes` 同构的 `tweets[]`（含 `media[]`）+ `cursor` 增量游标

`SKILL.md` 必须明确声明 `organize` 与旧 `download` 一把梭**不在**Agent 工具表内。SKILL.md 必须保持 host-agnostic（不含 `~/.claude/...`、`~/.codex/...`、`~/.hermes/...` 等任何 host 特定路径）；host 特定路径仅允许出现在 `README.md`。

`fetch_tweet` 与 `user_tweets` 的工具说明必须含同一 Agent 行为约定：取回 `TweetSummary` / `tweets[]` 后，将其中的 `media[]` 直接作为 `download_media` 的 `items[]` 入参完成下载——两者自身只取元数据、不下载。`user_tweets` 的工具说明还必须声明已知边界：X 端对 UserTweets 的单页条数有下限（实测每页约 20 条，`count` 按请求透传但可能不生效），分页以 `cursor` 为准；仅含顶层推文，不含会话折叠线程。

#### 场景:工具表完整

- **当** 解析 SKILL.md 中的工具声明
- **那么** 必须找到上述六个工具且仅这六个，每个工具均包含名称、用途、参数、返回值、错误种类五要素

#### 场景:fetch_tweet 含下载衔接约定

- **当** 解析 SKILL.md 中 `fetch_tweet` 工具说明
- **那么** 必须存在一句明确约定：把 `fetch_tweet` 返回的 `media[]` 传给 `download_media` 的 `items[]` 来下载

#### 场景:user_tweets 含下载衔接约定

- **当** 解析 SKILL.md 中 `user_tweets` 工具说明
- **那么** 必须存在一句明确约定：把 `user_tweets` 返回的 `tweets[].media[]` 传给 `download_media` 的 `items[]` 来下载，并声明单页条数与折叠线程两条已知边界

#### 场景:明示不暴露的工具

- **当** 解析 SKILL.md
- **那么** 必须存在一段说明明示 `organize` 与旧 `download` 不在 Agent 暴露面，并解释原因

#### 场景:SKILL.md host-agnostic

- **当** 解析 SKILL.md 全文
- **那么** 必须不出现任何 host-specific 路径（`~/.claude/`、`~/.codex/`、`~/.hermes/`、`.cursor/skills/`、`packaging/<host>/` 等）；这些路径只允许出现在 README.md

### 需求:`SKILL.md` 调用约定（双路径）

SKILL.md 必须声明**双路径调用形态**：MCP 优先、CLI 回落，并给出选定依据与两路径的能力差异。

- **MCP 形态**：通过 MCP 协议的 `tools/call` 调用工具名 `list_likes` / `download_media` / `auth_status` / `setup_from_curl` / `fetch_tweet` / `user_tweets`，由客户端建立的 stdio MCP 连接转发到 `x_likes_downloader serve --mcp` 子进程
- **CLI 形态**：当 MCP 工具不可用（客户端未注册 MCP server、不支持 MCP、或用户只装了 binary）时，Agent 通过 shell 调用 `x_likes_downloader likes list --json` / `media download` / `auth status --json` / `setup --curl-file` / `tweet get --json` / `user tweets --json`，解析 stdout 的 JSON 信封 `{ok, data?, meta, error?}`
- 两条路径共享同一份 lib 实现，语义、JSON 结构、错误 `kind` 必须一致
- SKILL.md 必须给出形态对照表（操作 → MCP 工具 → CLI 子命令），并要求 Agent 不得在同一轮对话中为同一件事混用两种形态

调用约定章节还必须包含：

- 业务错误以 `CallToolResult { isError: true }` 返回（MCP 形态）或 `ok: false` + 非零退出码（CLI 形态），content 含结构化 `kind` / `message` / `hint` 字段；`tools/call` 不应抛 JSON-RPC level error 除非真协议异常
- `download_media` 的进度通过 MCP `notifications/progress` 推送（仅在请求含 `progressToken` 时）；CLI 形态无进度通知，其它工具均无进度
- MCP 形态下 `notifications/cancelled` 必须真实生效：in-flight item 在 1 秒内停止，返回 `isError: false` + 完整 `DownloadOutput`（中断 item 状态为 `cancelled`，`.partial` 保留供续传）；CLI 形态无取消通道
- 出现 `kind: "auth_expired"` 或 `endpoint_stale` 时，Agent 应建议用户重新导入 cURL（即调用 `setup_from_curl` 工具或让用户跑 `x_likes_downloader setup --curl-file`）
- 出现 `kind: "tweet_unavailable"` 时（`fetch_tweet` 与 `user_tweets`），Agent 应告知用户目标推文 / 目标账号不存在、已删除或注销、受保护或当前凭据不可见，不应重试
- `fetch_tweet` 成功但 `tweet.media` 为空数组时，Agent 应提示用户该推文可能无媒体、或其视频为 HLS-only（当前不支持下载），而非直接把空 `media[]` 传给 `download_media` 后静默无结果
- 出现 `kind: "binary_missing"` 时，Agent 应将用户引导到 GitHub Releases 安装页
- Agent 必须依信封 `kind` 而非进程退出码区分错误类别（退出码是 `exit_code()` 把多个 `kind` 压成的 2 档粗判，不足以区分如 `tweet_unavailable` 与 `invalid_argument`）

#### 场景:双路径均有声明

- **当** 解析 SKILL.md 中的调用约定章节
- **那么** 必须同时出现 MCP `tools/call` 形态与 CLI `--json` 子命令形态，并给出二者的选择依据与能力差异说明

#### 场景:形态对照表完整

- **当** 解析 SKILL.md 中的形态对照表
- **那么** 六个 Agent 工具必须各自映射到一个 MCP 工具名与一条 CLI 子命令

#### 场景:约定可被自动化校验

- **当** 解析 SKILL.md 中的调用约定章节
- **那么** 必须包含 MCP 调用形态、CLI 信封形态、CallToolResult isError 错误模型、progressToken 进度协议、cancellation 生效范围、典型错误恢复路径六类信息

#### 场景:tweet_unavailable 指引覆盖两类工具

- **当** 解析 SKILL.md 中的 `tweet_unavailable` 错误恢复指引
- **那么** 必须同时覆盖 `fetch_tweet`（推文级不可见）与 `user_tweets`（账号级不存在 / 注销 / 被封），且两类均明示不应重试

#### 场景:cancellation 描述不与实现矛盾

- **当** 解析 SKILL.md 的约定总览表
- **那么** Cancellation 一行必须声明 v2.1 起 MCP `notifications/cancelled` 真实生效，不得再出现"被忽略（仅 stderr 日志记录）"这类 v2.0 描述

### 需求:`defaults.json` 字段定义

`.agents/skills/x_likes/defaults.json` 必须为合法 JSON，包含且仅包含以下字段：

- `likes_api_url`（字符串，X GraphQL `Likes` 端点完整 URL）
- `likes_features`（字符串，features JSON 序列化文本，沿用 X Web 当前值）
- `likes_fieldtoggles`（字符串，fieldToggles JSON 序列化文本）
- `tweet_detail_api_url`（字符串，X GraphQL `TweetDetail` 端点完整 URL）
- `tweet_features`（字符串，TweetDetail features JSON 序列化文本，沿用 X Web 当前值）
- `tweet_fieldtoggles`（字符串，TweetDetail fieldToggles JSON 序列化文本）
- `user_by_screen_name_api_url`（字符串，X GraphQL `UserByScreenName` 端点完整 URL）
- `user_by_screen_name_features`（字符串，UserByScreenName features JSON 序列化文本，沿用 X Web 当前值）
- `user_by_screen_name_fieldtoggles`（字符串，UserByScreenName fieldToggles JSON 序列化文本）
- `user_tweets_api_url`（字符串，X GraphQL `UserTweets` 端点完整 URL）
- `user_tweets_features`（字符串，UserTweets features JSON 序列化文本，沿用 X Web 当前值）
- `user_tweets_fieldtoggles`（字符串，UserTweets fieldToggles JSON 序列化文本）
- `bearer_token`（字符串，X Web 公开 bearer，可选——若省略则从代码硬编码兜底读取）
- `schema_version`（数字，初版为 1）

上述字段中，**必填**为 `schema_version` 与 `likes_api_url` / `likes_features` / `likes_fieldtoggles` / `tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles` / `user_by_screen_name_api_url` / `user_by_screen_name_features` / `user_by_screen_name_fieldtoggles` / `user_tweets_api_url` / `user_tweets_features` / `user_tweets_fieldtoggles`（共 13 个），`bearer_token` 为唯一可选字段。`tweet_detail_*` 与 `user_by_screen_name_*` / `user_tweets_*` 九个字段经 `resolve_protocol_field` 解析（env > private_tokens > defaults.json > 硬编码），其中 defaults.json 承载真值、`config.rs` 硬编码层仅作非权威兜底（镜像 `likes_*`：URL 兜底为有效短 URL，features 兜底为 `{}`，fieldtoggles 兜底为与 defaults.json 真值等价的非空 JSON）；`setup_from_curl` 不写 `TWEET_*` 与 `USER_*`，故 private_tokens 层对这些字段为空。任何 required∪optional 之外的字段必须被 `scripts/check-skills.sh` 校验拒绝。

#### 场景:JSON 合法且字段集封闭

- **当** 解析 `.agents/skills/x_likes/defaults.json`
- **那么** 13 个必填键必须全部存在，且顶层键集合必须为 required∪{bearer_token} 的子集，无此范围外的字段

#### 场景:含 TweetDetail 协议字段

- **当** 解析 `defaults.json`
- **那么** 必须包含 `tweet_detail_api_url`、`tweet_features`、`tweet_fieldtoggles` 三个非空字符串字段

#### 场景:含 UserByScreenName 与 UserTweets 协议字段

- **当** 解析 `defaults.json`
- **那么** 必须包含 `user_by_screen_name_api_url` / `user_by_screen_name_features` / `user_by_screen_name_fieldtoggles` / `user_tweets_api_url` / `user_tweets_features` / `user_tweets_fieldtoggles` 六个非空字符串字段，且两个 `*_features` 值必须是合法 JSON object 序列化文本

#### 场景:白名单同步收录新字段

- **当** 运行 `bash scripts/check-skills.sh`
- **那么** `user_by_screen_name_*` 与 `user_tweets_*` 六个字段必须通过其 `allowed` 白名单校验（不得被当作未知字段拒绝）

#### 场景:版本号存在

- **当** 解析 `defaults.json`
- **那么** 必须包含 `schema_version` 字段且其值为大于等于 1 的整数
