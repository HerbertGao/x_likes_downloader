# Design

## Context

参见 proposal.md - Why。现状要点：`list_likes` 已具备 timeline 双结构解析（`timeline_v2` / `timeline` 外层均兼容）与 `entry_to_summary` 同构提取；`fetch_tweet` 已确立「lib 层纯函数分类 + XApi 返回 `(status, body)` 不提前 Err」的错误分类契约；协议字段四层解析（env > private_tokens > defaults.json > 硬编码）由 `resolve_protocol_field` 统一承载。本变更在其上补一条「按人取推文」链路。代码与测试已在未提交工作区完成，本设计记录既定决策供 apply 与后续维护参照。

## Goals / Non-Goals

- Goals：username → 用户摘要 → 推文列表的完整链路进入本项目统一解析与错误模型；CLI 与 MCP 同一份 lib 实现。
- Non-Goals：不做「按人点赞 / 按人回复 / 媒体时间线」等其它 profile tab（`UserTweetsAndReplies` 等端点留待真实需求）；不改 `download_media`；不新增 ErrorKind；不维护独立解析器。

## Decisions

1. **两跳链路 `UserByScreenName → UserTweets`，而非「按 screen_name 直取推文」。**
   备选：X 无单端点可按 screen_name 直拉时间线；或用 web 页面抓取解析（脆弱、绕开协议层）。弃选理由：GraphQL 两跳是 X Web 客户端自身的做法，端点与鉴权模式和既有 `Likes` / `TweetDetail` 完全同构，实测一次即通。
2. **复用 `list_likes::parse_likes_response` 与 `entry_to_summary`，不新写解析器。**
   实测 `UserTweets` 响应外层同为 `data.user.result.timeline.timeline.instructions`（双结构兼容已覆盖），`tweet-*` entry 结构与点赞链路一致。备选：为 UserTweets 单独复制一套提取逻辑——只会造成第二份解析路径随 X 改版各自漂移。仅取 `TimelineAddEntries` 的顶层 `tweet-*` entry（不含置顶推文（`TimelinePinEntry`）与会话折叠线程），以代码注释与 fixture 断言声明该取舍。
3. **`XApi` 抽出 `graphql_get` 私有公共机械，三个 GraphQL GET 共用。**
   `TweetDetail` 原有 ~90 行构造（URL 编码 / 鉴权 header / internal-vs-network 错误分类 / 禁止 header 泄漏 stderr）在新增两个端点时面临三份复制；抽为私有方法后 `get_tweet_detail` / `get_user_by_screen_name` / `get_user_tweets` 各只保留自己的 variables 构造。行为契约（先取状态码再读 body、非 2xx 不提前 Err、非 JSON body → `Value::Null`）原样搬移，既有 `fetch_tweet` 集成测试不动。
4. **用户级不可用复用 `ErrorKind::TweetUnavailable`，不新增变体。**
   备选：新增 `UserUnavailable`（更精确的错误词汇，但扩大 Agent 面对的错误枚举、动 `exit_code()` 与 `default_hint` 两处映射）。弃选理由：语义同构——「目标不存在 / 已删除（注销） / 受保护 / 当前凭据不可见，不应重试」，复用使 MCP 错误词汇表保持 11 种，`user_tweets` 工具描述中明示该 `kind` 兼指账号级不可见。
5. **`count` 透传 + 已知边界注释，而非客户端补齐条数。**
   实测 X 端忽略过小的 `count`（每页固定约 20 条）。备选：在 lib 层按 `count` 裁剪返回数组凑足语义——假象式修复，掩盖 X 端行为且多一次内存拷贝。弃选理由：保留透传、`shortcut:` 注释声明限制、CLI 帮助与 MCP 描述如实注明，Agent 以 `cursor` 做真实分页。

## Risks / Trade-offs

- [新端点 queryId 同样会被 X 轮换] → 与既有机制同病同治：四层解析 + `update-protocol-and-skill` 变更建立的新鲜度实测流程兜底；`endpoint_stale` 分类引导重新 setup。
- [`all=true` 对高产账号连续翻页易触速率限制] → 沿用 `rate_limited` + `retry_after`（60s）分类；`since_cursor` 支持断点续拉，Agent 可分批。
- [被封 / 注销账号的响应形态可能与 fixture 有别] → 解析按「`data.user` 缺失或 `rest_id` 缺失」双条件归 `tweet_unavailable`，`UserUnavailable` typename 用真实形态场景固化；出现新形态时分类仍落在不可重试桶，不会误判为可重试网络错误。
- [旧 schema（`legacy.screen_name`）回退未被当前线上用到] → 一条 `or_else` 成本换旧响应 / 奇怪部署下的兼容性，已有单测固化。

## Migration Plan

无迁移：新增子命令与工具对既有调用方零影响；未导入新凭据字段的旧部署走 defaults.json / 硬编码兜底即可用。回滚即还原 git diff 涉及文件。

归档提示：sync/archive 时需把主 spec `openspec/specs/agent-mcp-server/spec.md` 的需求标题「tools/list 暴露 5 个工具」重命名为 6（delta 按仓库惯例以旧标题作 MODIFIED 锚点，同 add-fetch-tweet 归档先例 05e4b7e 的 4→5 处理）。
