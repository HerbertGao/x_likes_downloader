## 修改需求

### 需求:`SKILL.md` 工具表声明

`packaging/skill/x_likes/SKILL.md`（SOT）必须以一节明确声明 Agent 可调用的工具集，每个工具至少包含：名称、用途一句话、输入参数及类型、返回值结构概述、典型错误 `kind` 列表。声明的工具集必须为且仅为以下五个：

- `list_likes(count?, all?, since_cursor?)` → 列点赞
- `download_media(items[], subdir?, concurrency?)` → 按 MediaItem 数组下载到沙箱
- `auth_status()` → 凭据健康自检
- `setup_from_curl(curl_text)` → 首次配置 / 重新导入 cURL
- `fetch_tweet(url? | id?)` → 按 URL 或 tweet_id 抓取任意一条推文的媒体元数据，返回与 `list_likes` 同构的 TweetSummary（含 `media[]`）

`SKILL.md` 必须明确声明 `organize` 与旧 `download` 一把梭**不在**Agent 工具表内。SOT SKILL.md 必须保持 host-agnostic（不含 `~/.claude/...`、`~/.codex/...` 等任何 host 特定路径）；host-specific 路径仅出现在 host adapter README 或对应 plugin manifest 中。

`fetch_tweet` 的工具说明必须含 Agent 行为约定:取回 `TweetSummary` 后，将其 `media[]` 直接作为 `download_media` 的 `items[]` 入参完成下载——`fetch_tweet` 自身只取元数据、不下载。

#### 场景:工具表完整
- **当** 解析 SOT SKILL.md 中的工具声明
- **那么** 必须找到上述五个工具且仅这五个，每个工具均包含名称、用途、参数、返回值、错误种类五要素

#### 场景:fetch_tweet 含下载衔接约定
- **当** 解析 SOT SKILL.md 中 `fetch_tweet` 工具说明
- **那么** 必须存在一句明确约定:把 `fetch_tweet` 返回的 `media[]` 传给 `download_media` 的 `items[]` 来下载

#### 场景:明示不暴露的工具
- **当** 解析 SOT SKILL.md
- **那么** 必须存在一段说明明示 `organize` 与旧 `download` 不在 Agent 暴露面，并解释原因

#### 场景:SOT host-agnostic
- **当** 解析 SOT SKILL.md 全文
- **那么** 必须不出现任何 host-specific 路径（`~/.claude/`、`~/.codex/`、`~/.hermes/`、`.cursor/skills/`、`packaging/<host>/` 等）；这些路径只允许出现在各自 host adapter 的 README

### 需求:`SKILL.md` 调用约定

`packaging/skill/x_likes/SKILL.md`（SOT）必须明确以下 Agent 调用约定，所有派生副本继承之：

- 所有工具的调用形态：通过 MCP 协议的 `tools/call` 调用工具名 `list_likes` / `download_media` / `auth_status` / `setup_from_curl` / `fetch_tweet`，由客户端建立的 stdio MCP 连接转发到 `x_likes_downloader serve --mcp` 子进程
- 业务错误以 `CallToolResult { isError: true }` 返回，content 含结构化 `kind` / `message` / `hint` 字段；`tools/call` 不应抛 JSON-RPC level error 除非真协议异常
- `download_media` 的进度通过 MCP `notifications/progress` 推送（仅在请求含 `progressToken` 时）；其它工具无进度通知
- 出现 `kind: "auth_expired"` 或 `endpoint_stale` 时，Agent 应建议用户重新导入 cURL（即调用 `setup_from_curl` 工具或让用户跑 `x_likes_downloader setup --curl-file`）
- 出现 `kind: "tweet_unavailable"` 时（仅 `fetch_tweet`），Agent 应告知用户该推文不存在 / 已删除 / 受保护或当前凭据不可见，不应重试
- `fetch_tweet` 成功但 `tweet.media` 为空数组时，Agent 应提示用户该推文可能无媒体、或其视频为 HLS-only（当前不支持下载），而非直接把空 `media[]` 传给 `download_media` 后静默无结果
- 出现 `kind: "binary_missing"` 时，Agent 应将用户引导到 GitHub Releases 安装页
- Agent 必须依信封 `kind` 而非进程退出码区分错误类别（退出码是 `exit_code()` 把多个 `kind` 压成的 2 档粗判，不足以区分如 `tweet_unavailable` 与 `invalid_argument`）

SOT SKILL.md 禁止描述"通过 spawn `xld <subcmd> --json` CLI 进程"作为 Agent 调用形态——v1 的 stdout JSON envelope 协议在 v2+ 中**仅作为人类 CLI 接口存在**，不被 Agent 使用。

#### 场景:约定可被自动化校验
- **当** 解析 SOT SKILL.md 中的调用约定章节
- **那么** 必须包含 MCP 协议调用形态、CallToolResult isError 错误模型、progressToken 进度协议、典型错误恢复路径四类信息

#### 场景:不再描述 spawn-CLI 形态作为 Agent 接口
- **当** 解析 SOT SKILL.md 全文
- **那么** 任何 Agent 调用约定章节必须不出现"spawn xld 子命令"、"--json"、"stdout JSON envelope"等 v1 描述；这些表述若出现仅能作为"人类 CLI 用户参考"出现

### 需求:`defaults.json` 字段定义

`packaging/skill/x_likes/defaults.json`（SOT defaults）必须为合法 JSON，包含且仅包含以下字段：

- `likes_api_url`（字符串，X GraphQL `Likes` 端点完整 URL）
- `likes_features`（字符串，features JSON 序列化文本，沿用 X Web 当前值）
- `likes_fieldtoggles`（字符串，fieldToggles JSON 序列化文本）
- `tweet_detail_api_url`（字符串，X GraphQL `TweetDetail` 端点完整 URL）
- `tweet_features`（字符串，TweetDetail features JSON 序列化文本，沿用 X Web 当前值）
- `tweet_fieldtoggles`（字符串，TweetDetail fieldToggles JSON 序列化文本）
- `bearer_token`（字符串，X Web 公开 bearer，可选——若省略则从代码硬编码兜底读取）
- `schema_version`（数字，初版为 1）

上述字段中，**必填**为 `schema_version` / `likes_api_url` / `likes_features` / `likes_fieldtoggles` / `tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles`（共 7 个），`bearer_token` 为唯一可选字段。`tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles` 经 `resolve_protocol_field` 解析（env > private_tokens > defaults.json > 硬编码），其中 defaults.json 承载真值、`config.rs` 硬编码层仅作非权威兜底（镜像 `likes_*`：URL 兜底为有效短 URL，features/fieldtoggles 兜底为 `{}`）；`setup_from_curl` 不写 `TWEET_*`，故 private_tokens 层对这三字段为空。任何 required∪optional 之外的字段必须被 `scripts/check-packaging.sh` 校验拒绝。各 host adapter（`packaging/{claude-code,codex,openclaw}/`）若需要 defaults.json，必须直接复用 SOT 文件路径或由 `sync-skill.sh` 同步派生，禁止独立维护。

#### 场景:JSON 合法且字段集封闭
- **当** 解析 `packaging/skill/x_likes/defaults.json`
- **那么** 7 个必填键必须全部存在，且顶层键集合必须为 required∪{bearer_token} 的子集，无此范围外的字段

#### 场景:含 TweetDetail 协议字段
- **当** 解析 `defaults.json`
- **那么** 必须包含 `tweet_detail_api_url`、`tweet_features`、`tweet_fieldtoggles` 三个字符串字段

#### 场景:版本号存在
- **当** 解析 `defaults.json`
- **那么** 必须包含 `schema_version` 字段且其值为大于等于 1 的整数
