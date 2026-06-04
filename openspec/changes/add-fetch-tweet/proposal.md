## 为什么

现有工具只能下载「当前账号点过赞的推文」——`list_likes` 走 X GraphQL `Likes` 端点，无法获取任意一条公开/未点赞推文的媒体。用户要下载某条没点赞的推文（如别人分享的链接）时，工具完全无能为力，只能退回到 yt-dlp 等外部手段。

`Config` 里其实已经预留了 `tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles` 三个字段（指向 X GraphQL `TweetDetail` 端点），但从未接到任何命令或工具上，且当前仅走 `env::var` 单层解析。本变更把这套已存在的脚手架接通并对齐到与 `likes_*` 一致的四层解析，补齐「按 URL 或 tweet_id 取任意推文媒体」的能力。

## 变更内容

- 新增 `fetch_tweet` 能力：接受 URL（如 `https://x.com/<handle>/status/<id>`）或纯 tweet_id，调用 `TweetDetail` GraphQL 端点，解析出焦点推文，返回与 `list_likes` **同构**的扁平 `TweetSummary`（含 `media[]`）。
- 新增 CLI 子命令 `xld tweet get --url <url> | --id <id> [--json]`，输出 `OutputEnvelope`（stdout 数据 / stderr 日志 / 退出码语义化）。
- 新增 MCP 工具 `fetch_tweet`，使 MCP 工具集由 4 个变为 **5 个**。
- `download_media` **不改动**：下载仍走现有工具，复用沙箱 / 续传 / 取消机制。本能力只负责取元数据，Agent 拿到 `media[]` 后照常传给 `download_media`。
- 把 `tweet_detail_*` 三字段从 env-only 升级为与 `likes_*` 一致的四层解析（env > private_tokens > defaults.json > 硬编码）；真值移入 `packaging/skill/x_likes/defaults.json`，`config.rs` 源码只留简短兜底（避免长硬编码串与 defaults.json 双源漂移）。
- SOT `SKILL.md` 工具表新增 `fetch_tweet` 声明与 Agent 行为约定（fetch 完把 `media[]` 传给 `download_media`）。

## 功能 (Capabilities)

### 新增功能
- `tweet-detail-fetch`: 按 URL 或 tweet_id 调用 X `TweetDetail` 端点，解析焦点推文为扁平 `TweetSummary`（含 `media[]`），并以 CLI 子命令 `xld tweet get` 暴露；输入解析、错误分类（`auth_expired` / `endpoint_stale` / `tweet_unavailable` / `network_error`）、JSON 信封输出均归本能力。

### 修改功能
- `agent-mcp-server`: MCP 工具集由「且仅 4 个」扩为「且仅 5 个」，新增 `fetch_tweet` 工具及其 `tools/call` 调用语义与输入 schema。
- `agent-skill-package`: SOT `SKILL.md` 工具表由「且仅四个」扩为「且仅五个」（新增 `fetch_tweet`）；`defaults.json` 字段集新增 `tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles` 三个协议字段。

## 影响

- **代码**：新增 `src/agent/fetch_tweet.rs`（TweetDetail 请求构造 + 响应解析 + URL/id 输入解析 + 分层错误分类，解析与分类为纯函数）；复用 `list_likes.rs` 的 `entry_to_summary` / `extract_media`（提升为 `pub(crate)`）；`x_api.rs` 增加 `get_tweet_detail`（复用鉴权 header 但禁止打印含 cookie 的 header/URL 到 stderr）；`error.rs` 新增 `ErrorKind::TweetUnavailable`（exit_code=1，补 default_hint）；`main.rs` 新增 `Tweet { Get }` 子命令；`agent/mcp_server.rs` 注册 `fetch_tweet`；新增 `FetchTweetRequest`（位置与现有 MCP 请求类型一致）。
- **配置 / 打包**：`config.rs` 三字段升级四层解析并把长硬编码替换为简短兜底；`packaging/skill/x_likes/defaults.json` 新增 3 字段；`scripts/check-packaging.sh` 的 `allowed` 白名单数组追加 3 字段；SOT `SKILL.md` 工具表与调用约定更新；`sync-skill.sh` 派生副本同步。
- **向后兼容**：不改 `list_likes` / `download_media` 的现有 schema 与行为，不改既有 `ErrorKind::exit_code()` 映射（仅为新变体补分支）。`tweet_detail_*` 从 env-only 升级为四层解析属解析层变更，但默认值实测等价，对终端用户行为不变，旧配置无需迁移。
- **X API 交互**：新增一类 GraphQL 请求（`TweetDetail`），鉴权 header 与 `Likes` 完全一致；响应走 `threaded_conversation_with_injections_v2.instructions[]`，需独立解析器定位顶层 `entryId == tweet-<focalId>` 焦点 entry（实测：含 `TweetWithVisibilityResults` wrapper 与 m3u8+mp4 多 variant，均已被现有 `entry_to_summary`/`extract_media` 覆盖）。
