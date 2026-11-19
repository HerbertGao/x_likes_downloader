# Tasks

> 实现与测试已在未提交工作区完成（proposal 已声明）。以下任务以验证与补缺为主；标注「已入工作区」的项在 apply 时核验后勾选。`scripts/check-skills.sh` 白名单与 SKILL.md / skill README 的登记由配套变更 `update-protocol-and-skill` 承担，本变更不重复改动这两个文件。

## 1. 配置层（`src/config.rs`、`.agents/skills/x_likes/defaults.json`）

- [x] 1.1 核验 6 个新协议字段接入四层解析：`USER_BY_SCREEN_NAME_API_URL` / `USER_BY_SCREEN_NAME_FEATURES` / `USER_BY_SCREEN_NAME_FIELDTOGGLES` / `USER_TWEETS_API_URL` / `USER_TWEETS_FEATURES` / `USER_TWEETS_FIELDTOGGLES`（env > private_tokens > defaults.json > 硬编码，硬编码兜底为 2026-11 实测现役 queryId / `{}`）——已入工作区
  - 文件：`src/config.rs`
- [x] 1.2 核验 vendored defaults.json 已收录同名 6 字段且值为实测可用集（两个 `*_features` 为合法 JSON object）——已入工作区
  - 文件：`.agents/skills/x_likes/defaults.json`
- [x] 1.3 跑 `cargo test config::` 确认新字段 defaults 真值测试通过（含 `vendored_defaults_has_user_tweets_fields`）
  - 验证命令：`cargo test config::`

## 2. API 层（`src/x_api.rs`）

- [x] 2.1 核验 `graphql_get` 私有公共方法：URL 拼装 / 鉴权 header / internal-vs-network 错误分类 / 非 JSON body 容错（契约自 `get_tweet_detail` 原文搬移），`get_tweet_detail` 改为其调用方且行为不变——已入工作区
- [x] 2.2 核验 `get_user_by_screen_name`（variables `{"screen_name"}`）与 `get_user_tweets`（userId + count + cursor 翻页）两个方法——已入工作区
- [x] 2.3 确认无 header / 完整 URL 泄漏到 stderr（沿用既有禁令，grep `eprintln` 于 `src/x_api.rs` 的 graphql_get 路径无输出敏感构造）
  - 验证命令：`grep -n "eprintln" src/x_api.rs`

## 3. Agent 层（`src/agent/user_tweets.rs`、`src/agent/types.rs`、`src/agent/list_likes.rs`）

- [x] 3.1 核验 `UserTweetsRequest` / `UserTweetsOutput` / `UserBrief` 类型与 MCP 输入 schema 派生——已入工作区
  - 文件：`src/agent/types.rs`
- [x] 3.2 核验 `user_tweets` 能力：username 归一化、UserByScreenName 用户解析（含 `{"data":{}}` / `UserUnavailable` / 旧 schema 回退三类不可见）、UserTweets 翻页语义（与 `list_likes` 一致：单页 / all 翻页 / since_cursor / 游标重复即停）、复用 `parse_likes_response` + `entry_to_summary`、`count` 透传附 `shortcut:` 已知边界注释、置顶推文（`TimelinePinEntry`）排除——已入工作区
  - 文件：`src/agent/user_tweets.rs`
- [x] 3.3 核验单测 30 个（含纯函数分类与翻页；fixtures 为手工骨架，仅保留解析器读取的键：ubsn_ok / ubsn_notfound / usertweets_ok）——已入工作区
  - 文件：`tests/fixtures/user_tweets/`

## 4. CLI / MCP 层（`src/main.rs`、`src/agent/mcp_server.rs`、`src/lib.rs`、`tests/mcp_smoke.rs`）

- [x] 4.1 核验 `xld user tweets` 子命令（`--username` 必填，`--all` / `--since-cursor` / `--count` / `--include-raw` / `--json`，信封 cursor 写入 `meta.cursor`，`invalid_argument` 时 stderr 用法提示）——已入工作区
  - 文件：`src/main.rs`
- [x] 4.2 核验 MCP 第 6 个工具 `user_tweets` 注册（描述含单页约 20 条 / cursor 分页 / 折叠线程边界与 `tweet_unavailable` 语义）与 `agent::user_tweets` 导出——已入工作区
  - 文件：`src/agent/mcp_server.rs`、`src/lib.rs`
- [x] 4.3 核验 `tests/mcp_smoke.rs` 期望工具集扩为 6（`EXPECTED_TOOLS` + 握手断言）——已入工作区
  - 验证命令：`cargo test --test mcp_smoke`

## 5. 验证（手动 + 自动）

- [x] 5.1 `cargo test` 全绿（lib 189 + 集成套件）
- [x] 5.2 真实凭据实测三条路径并在本文件记录结果：正常页（`xld user tweets --username <公开账号> --count 3`，exit 0、`data.user` 与 `meta.cursor` 就位）；游标续拉（`--since-cursor <上次 cursor>`，exit 0、条目不重叠）；用户不存在（不存在账号名，exit 1、`kind: "tweet_unavailable"`）
  - 已入工作区的实测记录：elonmusk 正常页 20 条 + cursor；`--since-cursor` 续拉 exit 0；`zzqq9988xxyy77w` 返回 `tweet_unavailable`——apply 时已复跑确认（正常页 20 条 + cursor、续拉首条与首页无重叠、不存在账号 exit 1 kind=tweet_unavailable）
- [x] 5.3 与 `update-protocol-and-skill` 同批发布：SKILL.md 版本底线与发布版本一致（该变更 tasks 4.3 互为镜像）

## 6. 文档

- [x] 6.1 核验仓库根 README 已含本能力说明（v2026.11 区块、MCP 工具清单 5→6、CLI 速查表 `user tweets` 行）——已入工作区
  - 文件：`README.md`
- [x] 6.2 skill 侧文档（SKILL.md 工具表 / README 版本底线）由 `update-protocol-and-skill` 承担，此处不重复改动
