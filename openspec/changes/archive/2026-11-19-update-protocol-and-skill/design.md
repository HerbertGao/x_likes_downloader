# Design

## Context

参见 proposal.md - Why。现状要点：现役请求链路（`agent::list_likes` / `agent::fetch_tweet`）已双兼容 `timeline` / `timeline_v2` 外层结构，协议兜底四层解析（env > private_tokens > defaults.json > 硬编码）已由 `resolve_protocol_field` 统一承载；`add-user-tweets`（配套变更）已把 6 个新协议字段接入同一机制。本变更不新增任何请求路径或解析逻辑。

## Goals / Non-Goals

- Goals：兜底值回到现役且双源一致；skill 打包物登记 6 工具现实；移除已死的旧解析路径。
- Non-Goals：不改任何 CLI / MCP 行为；不刷新用户层 `private_tokens.env`（仍可用，归 `xld setup` 流程）；不修 x-organize 仓库外数据（见 proposal Impact）。

## Decisions

1. **死代码处置：删除 `XApi::get_liked_tweets_internal`，而非补 `timeline` 兼容。**
   备选：模仿 `agent::list_likes::find_outer_timeline` 给它加双结构回退。弃选理由：该函数全仓库无调用方（`xld download` 早已改走 `agent::list_likes`），修复死代码只是让它继续以第二条解析路径存在，违背「同一解析逻辑只有一份」的现状；删除约 -90 行且无行为影响。
2. **兜底双源策略：`config.rs` 硬编码层保留完整 URL，与 defaults.json 逐值同步，而非退化为占位。**
   备选：按 add-fetch-tweet 提案里"只留简短兜底"的思路把第 4 层换成无效占位（强制依赖 defaults.json）。弃选理由：第 4 层是 defaults.json 意外缺失时的最后防线，保留有效值使 `XLD_CREDENTIALS_FILE` 自定义部署仍可裸跑；以新增的单源同步需求（见 specs/curl-import-extended delta）约束漂移即可，成本只是每次刷新改两处。
3. **登记性文档变更集中在 skill 打包物，不另立新能力 spec。**
   工具表 / 调用约定 / defaults 字段集均为 `agent-skill-package` 既有需求的枚举更新，走 MODIFIED delta；不新增 `protocol-maintenance` 之类的杂烩能力，新鲜度约束落在 `curl-import-extended`（它已拥有四层解析需求）。

## Risks / Trade-offs

- [旧 queryId 随时可能被 X 下线] → 本变更把 vendored 兜底刷到现役值并留下实测记录；用户层旧值失效时 `auth_status` / `endpoint_stale` 分类会引导重新 setup。
- [与 `add-user-tweets` 存在应用顺序耦合：SKILL.md 若先于 binary 发布，会描述一个尚未存在的工具] → 同批发布或按「先 `add-user-tweets`、后本变更」顺序应用；README 版本底线任务以携带两变更的发布版本为准。
- [check-skills.sh 白名单与 defaults.json 字段集目前不同步（工作区新增 6 字段会让检查失败）] → tasks 中安排「先复现失败、改白名单、再复跑通过」，把修复固化为可验证步骤。

## Migration Plan

无部署迁移：纯值刷新与文档登记，旧配置零改动。回滚即还原 git diff 涉及文件。
