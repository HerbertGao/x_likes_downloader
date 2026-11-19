# Tasks

## 1. 协议兜底层（`src/config.rs`、`.agents/skills/x_likes/defaults.json`）

- [x] 1.1 验证 defaults.json 的 Likes / TweetDetail queryId 已刷新为 2026-11 实测现役值（`ZjyvI1V9gajq5eclHCObzA/Likes`、`dQDdKtt882nl-Km98FJGSQ/TweetDetail`）——工作区已完成此编辑，本任务核对值与 proposal 记录一致
  - 文件：`.agents/skills/x_likes/defaults.json`
- [x] 1.2 把 `src/config.rs` 第 4 层硬编码兜底对齐同值：`likes_api_url` 兜底 `XHn_Tw60c6pi0n3DGhpwiA/Likes` → `ZjyvI1V9gajq5eclHCObzA/Likes`；`tweet_detail_api_url` 兜底 `zoF7_t363wZyzylk-BLfZQ/TweetDetail` → `dQDdKtt882nl-Km98FJGSQ/TweetDetail`（与 defaults.json 逐值相同，见 design 决策 2）
  - 文件：`src/config.rs`
- [x] 1.3 跑 `cargo test config::` 确认四层解析行为不变（含 `vendored_defaults_has_user_tweets_fields`、`hardcoded_used_when_all_layers_empty`）
  - 验证命令：`cargo test config::`

## 2. 死代码处置（`src/x_api.rs`）

- [x] 2.1 删除 `XApi::get_liked_tweets_internal` 及其私有 `parse_likes_response`（全仓库无调用方；现役解析在 `src/agent/list_likes.rs` 的双结构 `find_outer_timeline` 路径，勿动）
  - 文件：`src/x_api.rs`
- [x] 2.2 验证无残留引用：`grep -rn "get_liked_tweets_internal" src/ tests/` 无命中；`cargo build && cargo test` 全绿
  - 验证命令：`grep -rn get_liked_tweets_internal src/ tests/ ; cargo build && cargo test`

## 3. Skill 打包物（`.agents/skills/x_likes/`、`scripts/`）

- [x] 3.1 SKILL.md frontmatter：`description` 中 `(5 tools)` → `(6 tools)`；`min_binary_version` 保持与 `Cargo.toml` 同步的机制不变（发布时经 `scripts/version.sh` 统一抬升，见 agent-skill-package「Skill 与 binary 版本绑定」需求）
  - 文件：`.agents/skills/x_likes/SKILL.md`
- [x] 3.2 SKILL.md 工具表：新增 `user_tweets(username, all?, since_cursor?, count?, include_raw?)` 条目，含「`tweets[].media[]` 传给 `download_media`」衔接约定与两条已知边界（X 端单页约 20 条、不含会话折叠线程）——对齐本变更 specs/agent-skill-package delta 的场景「user_tweets 含下载衔接约定」
  - 文件：`.agents/skills/x_likes/SKILL.md`
- [x] 3.3 SKILL.md 调用约定：MCP 工具清单与 CLI 形态清单各追加 `user_tweets` / `user tweets --json`；形态对照表补一行；`tweet_unavailable` 恢复指引改为同时覆盖 `fetch_tweet`（推文级）与 `user_tweets`（账号级）
  - 文件：`.agents/skills/x_likes/SKILL.md`
- [x] 3.4 skill README.md：二进制版本底线从 `≥ 2026.6.0` 抬升至携带 `user_tweets` 的发布版本；全文出现「5 个工具 / 五个工具」处同步为 6
  - 文件：`.agents/skills/x_likes/README.md`
- [x] 3.5 `scripts/check-skills.sh`：`allowed` 白名单追加 `user_by_screen_name_api_url` / `user_by_screen_name_features` / `user_by_screen_name_fieldtoggles` / `user_tweets_api_url` / `user_tweets_features` / `user_tweets_fieldtoggles` 六键（当前 defaults.json 已含这 6 键而白名单未收录，先复现一次检查失败，改后复跑通过）
  - 文件：`scripts/check-skills.sh`
  - 验证命令：`bash scripts/check-skills.sh`

## 4. 发布前验证（手动）

- [x] 4.1 实测兜底可用性（对应 specs/curl-import-extended 新增需求）：以有效登录态对 defaults.json 四个 `*_api_url`（Likes / TweetDetail / UserByScreenName / UserTweets）完整 URL 各发一次 GraphQL GET，确认 HTTP 200；结果记录回本变更 tasks 或 proposal
  - 实测记录（2026-11-19）：四个兜底 URL 全部 HTTP 200 + JSON——`ZjyvI1V9gajq5eclHCObzA/Likes`（新 qid 首测）、`dQDdKtt882nl-Km98FJGSQ/TweetDetail`（新 qid 首测）、`AMIBMjtxEEATh4z8V9GtRg/UserByScreenName`、`D6z-FJI2pKuwEeYYxVIGbg/UserTweets`
- [x] 4.2 `cargo test` 与 `bash scripts/check-skills.sh` 全绿；`xld likes list --count 1` 与 `xld user tweets --username <任一公开账号> --count 1` 实跑通过
- [x] 4.3 确认与 `add-user-tweets` 同批发布（或其后应用），SKILL.md 版本底线与实际发布版本一致
