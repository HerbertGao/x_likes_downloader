# Proposal

## Why

2026-11 用真实凭据实测发现：X 已轮换 GraphQL queryId，vendored 协议兜底（`.agents/skills/x_likes/defaults.json`）里的 Likes / TweetDetail 兜底值已非现役（实测仍可用，但随时可能被 X 下线）；`config.rs` 第 4 层硬编码兜底还是更老的 queryId，与 defaults.json 形成双源漂移。同时，配套变更 `add-user-tweets` 的能力已在工作区落地，skill 打包物（SKILL.md / README.md / check-skills.sh 白名单）仍停留在「5 个工具」的旧现实。另有一处死代码 `XApi::get_liked_tweets_internal` 的解析器只认旧 `timeline_v2` 外层结构（对当前响应恒解析 0 条），是颗定时炸弹。

## What Changes

- vendored `defaults.json` 的 Likes / TweetDetail queryId 兜底刷新为 2026-11 实测现役值（已在未提交工作区完成，apply 时验证）。
- `config.rs` 第 4 层硬编码兜底对齐现役值，消除双源漂移（未做）。
- skill 打包物同步到 6 工具现实：SKILL.md 工具表与 frontmatter 描述（`5 tools` → `6 tools`）、README 版本底线、`min_binary_version`、`scripts/check-skills.sh` 的 defaults.json 字段白名单追加 `user_by_screen_name_*` / `user_tweets_*` 六个协议字段（未做——白名单目前缺这 6 个键，工作区状态会让该检查失败）。
- 删除死代码 `XApi::get_liked_tweets_internal`（含其 `timeline_v2`-only 解析器）；全仓库无任何调用方（未做）。
- 在 README/文档记录 x-organize `~/.config/x_organize/creds.json` 的 UserByScreenName queryId 同样过期（仍可用，现役 `AMIBMjtxEEATh4z8V9GtRg`）——用户侧数据，本变更只提示不代改。

## Capabilities

### New Capabilities

（无——本变更是既有能力的登记与维护，不引入新行为。）

### Modified Capabilities

- `agent-skill-package`: SKILL.md 工具表由「且仅五个」扩为「且仅六个」（登记 `user_tweets`）；`defaults.json` 字段集新增 `user_by_screen_name_api_url` / `user_by_screen_name_features` / `user_by_screen_name_fieldtoggles` / `user_tweets_api_url` / `user_tweets_features` / `user_tweets_fieldtoggles` 六个协议字段；SKILL.md `min_binary_version` 与 README 安装指引的版本底线同步抬高。
- `curl-import-extended`: 新增「协议兜底值新鲜度与单源性」需求——兜底 queryId 发布前须经真实凭据实测；`config.rs` 硬编码兜底必须与 defaults.json 同步刷新，禁止双源各自漂移。

## Impact

- **代码**：`src/config.rs`（两个旧兜底 URL 对齐现役值）；`src/x_api.rs`（删除 `get_liked_tweets_internal` 及其私有解析器，约 -90 行）；`scripts/check-skills.sh`（allowed 白名单 +6 字段）。
- **打包 / 文档**：`.agents/skills/x_likes/{defaults.json, SKILL.md, README.md}`；仓库根 README 的版本说明区随发布整理。
- **向后兼容**：全部为兜底值刷新与文档登记，不改任何 CLI / MCP 行为与 schema；`private_tokens.env` 用户侧记录不动（实测仍可用，`xld setup` 会在下次导入时自然刷新）；被删除的 pub API `get_liked_tweets_internal` 无任何调用方（含测试），删除属无损清理。
- **X API 交互**：不改任何请求参数或响应解析路径——现役链路（`agent::list_likes` 等）已双兼容 `timeline` / `timeline_v2` 外层结构；本变更只刷兜底值。
- **变更间依赖**：本变更登记的 `user_tweets` 工具由配套变更 `add-user-tweets` 提供实现（已在工作区完成）。应用顺序应为其后或同批发布；若单独先应用本变更，SKILL.md 将短暂描述一个 binary 尚未发布的工具。
