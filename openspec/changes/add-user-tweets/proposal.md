# Proposal

## Why

现有工具只能取「本账号点赞列表」（`list_likes`）或「单条指定推文」（`fetch_tweet`）——Agent 要按人取推文（例如备份某个画师的全部推文媒体）时无路可走，只能绕到仓库外 x-organize skill 的独立 Python 脚本：凭据另存一份、不走本项目的四层协议解析与统一错误模型、也进不了 MCP 工具面。2026-11 实测确认 `UserByScreenName`（username → rest_id）→ `UserTweets`（游标翻页）链路可用，且现有 likes features 集对两个新端点直接兼容（HTTP 200 + 完整 entries + cursor），补齐这块能力的成本很低。

## What Changes

- 新增 `user_tweets` 能力：按 username（允许前导 `@`）列出任意账号的可见推文，返回 `user{screen_name, rest_id, name}` + 与 `list_likes` 严格同构的 `tweets[]`（含 `media[]`，可直接交给 `download_media`）+ `cursor` 增量游标。
- 新增 CLI 子命令 `xld user tweets --username <name> [--all] [--count N] [--since-cursor C] [--include-raw] [--json]`，输出 `OutputEnvelope`，退出码语义与既有子命令一致。
- 新增 MCP 工具 `user_tweets`（第 6 个），输入 schema 由 `UserTweetsRequest` 经 schemars 派生。
- `Config` 新增 6 个协议字段（`user_by_screen_name_api_url/features/fieldtoggles`、`user_tweets_api_url/features/fieldtoggles`），走既有四层解析；真值入 vendored `defaults.json`。
- `XApi` 抽出 `graphql_get` 公共机械（URL 拼装 / 鉴权 header / 错误分类契约），`TweetDetail` / `UserByScreenName` / `UserTweets` 三个 GraphQL GET 共用。
- 已知边界如实声明：X 端对 UserTweets 忽略过小的 `count`（每页固定约 20 条，分页以 `cursor` 为准）；仅含 `TimelineAddEntries` 的顶层 `tweet-*` entry，不含置顶推文（`TimelinePinEntry`）与会话折叠线程。

**实现状态**：本变更的代码与测试已在未提交工作区完成（含 `mcp_smoke` 6 工具断言与真实凭据三路径实测：正常页 / `--since-cursor` 续拉 / 用户不存在）。apply 工作流的任务以验证与补缺为主。

## Capabilities

### New Capabilities

- `user-tweets-fetch`: 按 username 拉取指定账号（不限于本人）可见推文列表的完整行为契约——username 校验与归一化、UserByScreenName 用户解析与不可用分类、UserTweets 游标翻页、输出 schema 与 `list_likes` 同构、错误分类与已知边界。

### Modified Capabilities

- `agent-mcp-server`: `tools/list` 工具集由「且仅 5 个」扩为「且仅 6 个」（新增 `user_tweets`）；`tools/call` 语义新增 `user_tweets` 的参数反序列化与正常 / 错误返回场景。

## Impact

- **代码**：新增 `src/agent/user_tweets.rs`（校验 / 解析 / 翻页 / 分类纯函数）与 `tests/fixtures/user_tweets/` 三份手工骨架 fixture（仅保留解析器读取的键，占位值全部脱敏；真实响应不入仓）；`src/agent/types.rs` 增 `UserTweetsRequest` / `UserTweetsOutput` / `UserBrief`；`src/x_api.rs` 增 `get_user_by_screen_name` / `get_user_tweets` 并抽取 `graphql_get`；`src/config.rs` 增 6 个四层解析字段；`src/main.rs` 增 `User { Tweets }` 子命令；`src/agent/mcp_server.rs` 注册第 6 个工具；`tests/mcp_smoke.rs` 期望工具集 5→6。
- **配置 / 打包**：vendored `defaults.json` 收录 6 个新协议字段；`scripts/check-skills.sh` 白名单同步收录由配套变更 `update-protocol-and-skill` 承担（该变更同时刷新既有 queryId 兜底），避免两变更重复改动同一文件。
- **向后兼容**：不改既有 5 工具的 schema 与行为；`ErrorKind` 枚举不扩——用户级「不存在 / 注销 / 被封」复用 `tweet_unavailable`（exit_code=1、语义同「目标不可见、不应重试」），Agent 面对的错误词汇表保持 11 种不变。
- **X API 交互**：新增两类 GraphQL GET——`UserByScreenName`（variables `{"screen_name": <name>}`）与 `UserTweets`（variables `{"userId", "count", "includePromotedContent": false, "withQuickPromoteEligibilityTweetFields": false, "withVoice": true}`，翻页加 `cursor`）；鉴权 header 与 `Likes` 完全一致；响应复用 `list_likes` 的 timeline 双结构（`timeline_v2` / `timeline`）解析与 `entry_to_summary`，不新增解析路径。`all=true` 对高产账号会连续翻页，受 X 端速率限制约束（错误分类已含 `rate_limited` + retry_after）。
