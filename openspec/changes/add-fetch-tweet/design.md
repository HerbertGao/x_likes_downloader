## 上下文

工具当前只能下载当前账号点过赞的推文（`list_likes` → `Likes` GraphQL 端点）。`Config` 已预留 `tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles` 三个字段并硬编码了默认值，但从未被任何命令引用，且当前仅走 `env::var` 单层解析（与 `likes_*` 的四层解析不一致）。本变更把这套脚手架接通为 `fetch_tweet` 能力。

关键事实（含一次**真实 TweetDetail 抓取实测**；3 份响应经**脱敏后**留存于 `tests/fixtures/tweet_detail/`（git track，**不**放 `data/`——后者被 `.gitignore` 全量忽略；脱敏脚本已替换所有 handle/显示名/user id/正文/媒体 URL/hashtag 为占位，仅保留结构 `__typename`/entryId/variants 形态与 tweet-id↔focalId 对应，测试加 grep 断言无真实 PII）：新 2026 推文、老 2023 推文、老 2024 推文）：
- `src/agent/list_likes.rs` 的 `entry_to_summary(entry, include_raw)` 与 `extract_media(...)` 把一个 `tweet-*` entry 转成扁平 `TweetSummary`，逻辑只依赖 `entry.content.itemContent.tweet_results.result` 结构。
- **实测确认（3 个顶层公开非回复推文样本）**：`TweetDetail` 响应里焦点推文以 `entryId == "tweet-<focalId>"` + `entryType == TimelineTimelineItem` 出现在顶层 `instructions[].TimelineAddEntries.entries[]`；回复链单独位于 `conversationthread-<id>` module。**不假定恒成立**（焦点为回复链中间推文/受保护/引用等形态未取样，故决策 2 加 rest_id 软兜底作 best-effort 防御）。外层路径为 `data.threaded_conversation_with_injections_v2.instructions[]`（与 `Likes` 的 `timeline_v2.timeline.instructions[]` 不同）。
- **实测确认**：较早推文（2023-12 样本）焦点 `tweet_results.result.__typename` 为 `TweetWithVisibilityResults`，真正的 tweet 嵌在 `result.tweet`；`entry_to_summary`（L238 `tweet_result.get("tweet").unwrap_or(tweet_result)`）已正确解包。新推文为 `Tweet`。详见 [[project_tweet_detail_typename_wrapper]]。
- **实测确认**：视频媒体的 `video_info.variants` 同时含 m3u8（`application/x-mpegURL`，bitrate=None）与多个带 bitrate 的 `video/mp4`（如 256k/832k/2176k）；`extract_media` 会选最高 bitrate mp4，`download_media` 纯 GET 可下。新老推文皆然（老推文用 `ext_tw_video` host，结构一致）。
- **时点实测（非持久保证）**：config 现硬编码 query id `_8aYOgEDz35BrBcBal1-_w` 与 X Web query id `6uCvnic3m5reVuehkvHa3w` 当时均返回 HTTP 200；两套 features 集均可用。query id 会被 X 轮换，故 defaults.json 取值仅作当前兜底，过期时靠用户改 env/defaults.json 覆盖。
- `src/x_api.rs` 的鉴权 header 构造对两端点通用，但 `get_liked_tweets_internal` 含 `eprintln!("请求 headers: {:?}", headers)`（L70）会把含 cookie 的 header 打到 stderr。
- `src/error.rs` 的 `exit_code()` 映射：`AuthExpired|EndpointStale|RateLimited|NetworkError => 2`，`NotConfigured|InternalError|SandboxViolation|BinaryMissing|InvalidItem|InvalidArgument => 1`；`classify_status(u16)` 已把 401/403→AuthExpired、404/410→EndpointStale、429→RateLimited、5xx→NetworkError。

## 目标 / 非目标

**目标：**
- 新增 `agent::fetch_tweet(FetchTweetRequest) -> Result<FetchTweetOutput, ErrorPayload>` lib 函数（`FetchTweetOutput { tweet: TweetSummary, schema_version: u32 }` 镜像 `ListOutput`，**不**含 unsupported_media），被 CLI `xld tweet get` 与 MCP `fetch_tweet` 工具共用。CLI 把 `tweet` 放信封 `data`、`schema_version` 放 `meta`（`Meta` 现有字段，无需改 envelope.rs）；MCP 直接返回 `FetchTweetOutput`。
- 复用 `entry_to_summary` / `extract_media`，不复制媒体解析逻辑。
- 输出与 `list_likes` 的 `tweets[]` 元素严格同构。
- `tweet_detail_*` 三字段从 env-only 升级为与 `likes_*` 一致的四层解析（env > private_tokens > defaults.json > 硬编码），真值落 defaults.json，源码只留简短兜底。

**非目标：**
- 不做下载。下载继续走 `download_media`，复用沙箱/续传/取消。
- 不动 `list_likes` / `download_media` 的现有 schema 与行为；不改既有 `ErrorKind::exit_code()` 映射（仅为新变体补分支）。
- 不改 `download_media --ids` 现有快捷方式（它仍走 list_likes；若将来需要再单开 change）。
- 不支持纯 m3u8（无 mp4 variant）推文的下载——`extract_media` 本就要求 mp4 variant，此为其既有行为，本变更沿用（见风险）。
- MVP 只支持单条 tweet（单 `url` 或单 `id`）；多 id 批量留待后续。
- 不修复 `x_api.rs::get_liked_tweets_internal` 既有的 stderr cookie 泄露（既有 bug，可单列），但本变更新增代码不得复制该泄露。

## 决策

**决策 1：复用 `entry_to_summary` / `extract_media`，提升可见性到 `pub(crate)`。** 这两个函数对 entry 结构的依赖与时间线类型无关。替代方案是复制一份解析——会产生两份须同步演进的媒体解析逻辑，否决。媒体最佳直链选取规则**完全沿用 `extract_media` 现状**（video 取最高 bitrate mp4、animated_gif 取首个 mp4、image 附加 `?format=jpg&name=orig`），spec 不重新规定算法，避免措辞与实现漂移。

**决策 2：焦点 entry 定位用 `entryId == "tweet-<focalTweetId>"` 顶层精确匹配，并加 `rest_id` 软兜底（best-effort）。** 实测 3 个顶层公开非回复样本焦点均在顶层；为防未取样形态（回复链中间推文、entryId 后缀/变体）找不到，回退遍历所有 entries（含 module items）取解包 wrapper 后 `rest_id == focalId` 的 entry。该兜底无真实回复推文样本验证，属防御性回退、非契约保证；仍找不到 → `tweet_unavailable`（不产出错数据）。

**决策 3：新增独立解析器 `parse_tweet_detail_response(resp, focal_id) -> Option<entry>`，不复用 `parse_likes_response`。** 入参 `resp` 是**完整响应 Value**（含顶层 `data` 键），焦点路径为 `resp["data"]["threaded_conversation_with_injections_v2"]["instructions"][]`——注意是 `resp.data.threaded_...` 两层，勿漏 `["data"]`（参数命名用 `resp` 而非 `data` 以避免与 JSON 顶层 `data` 键混淆）。主路径匹配顶层 `entryId == "tweet-<focal_id>"`，未命中则软兜底遍历所有 entries（含 module items）按解包后 `rest_id == focal_id` 定位。该解析器与错误分类均做成**纯函数**（输入 `serde_json::Value` + HTTP 状态码），与网络发送分离，单测用 `tests/fixtures/tweet_detail/` 的真实 fixture 经 `include_str!` 编译期内联离线覆盖（对齐 `list_likes` 纯函数测试 + `VENDORED_DEFAULTS` 内联模式；fixture 放 `tests/`（被 git track）而非 `data/`（被 `.gitignore` 全量忽略））。

**决策 4：`FetchTweetRequest { url: Option<String>, id: Option<String> }`，恰好其一。** schemars 派生的 JSON Schema 无法表达「oneOf 互斥」（两字段都是 optional），互斥由 lib 层运行时校验兜底 + 单测覆盖，不手写 schemars oneOf。tweet_id 提取用 `regex`（已在 Cargo.toml）匹配 `/status/(\d+)`；`--id` 校验纯数字。`FetchTweetRequest` 与现有 MCP 请求类型（当前定义在 `mcp_server.rs`）放一致位置。

**决策 5：错误分类分两阶段（避免与 `classify_status` 双判、避免分类器签名错位）。** **阶段一**纯函数 `classify_tweet_detail(status, resp) -> Option<ErrorPayload>`（不持有 focal_id）：先 `classify_status(status)`；**仅当 HTTP 200** 查响应体——顶层 `errors[]` 含鉴权 code（32/64/89）→ `auth_expired`（X 对失效凭据常返回 200+errors，只靠状态码会漏判误报，给错 hint）；`data` 缺路径且无鉴权 errors → `endpoint_stale`。reqwest 传输错误 → `network_error`。**阶段二**在焦点定位/校验步骤（持有 focal_id）产出 `tweet_unavailable`：主路径+软兜底均无焦点 entry、焦点 `__typename ∈ {"TweetUnavailable","TweetTombstone"}`、或缺可用 `legacy`。新增 `ErrorKind::TweetUnavailable` 归入 `exit_code()` 返回 1 的组（与 `NotConfigured` 同组），并补 `default_hint()`。退出码语义债（2 档压缩）不在本变更扩展，规范要求调用方依 `kind` 而非退出码分支（见 spec 与 SKILL.md）。

**决策 6：模块落点、status 数据流与 header 构造单一来源。** 新建 `src/agent/fetch_tweet.rs`，在 `src/lib.rs` 的 `pub mod agent { ... }` 内联块加 `pub mod fetch_tweet;`（仓库无 `src/agent/mod.rs`，agent 子模块均在 lib.rs 声明）。请求发送须**同时携带 HTTP 状态码与 body** 供分层分类——`XApi::get_tweet_detail(&self, tweet_id) -> Result<(u16, Value)>`：先 `let status = response.status().as_u16()`，再 `let text = response.text().await`（传输错误才 `Err` → network_error），`let resp = serde_json::from_str(&text).unwrap_or(Value::Null)`——**非 JSON body（如 401 返回的 HTML 错误页）不得使函数 Err**，否则会绕过 `classify_status` 把 401 误成 network_error；分类阶段一靠 status 仍能正确出 `auth_expired`。非 2xx **不**提前 `Err`。这与 `list_likes`（agent 路径在 list_likes.rs 内联 send、先取 status 才解析、对空 User-Agent 有保护）的范式一致——不要照抄旧 `get_liked_tweets_internal`（它对非 2xx 直接 `Err(anyhow!)`、无 UA 空值保护、且 `eprintln!` headers）。header 构造逻辑复用但**不复制** `eprintln!` headers/URL——新方法禁止把含 cookie/bearer 的 header 或完整 URL 打到 stderr。

## 风险 / 权衡

- **[X 改 GraphQL query id / features]** → 三字段经 `resolve_protocol_field` 解析；但 `setup_from_curl` 不写 `TWEET_*`（只解析 Likes cURL），故 private_tokens 层对 tweet_* 恒空，用户覆盖只能走 env 或手改 defaults.json（不能像 likes_* 那样靠 setup 重导）。过期时需改代码发版或用户手动覆盖。实测时点 config query id 与 X Web query id 均有效；defaults.json 取一份当前可用值即可。
- **[纯 m3u8 推文返回空 media]** → `extract_media` 要求 mp4 variant，纯 m3u8/无 bitrate-mp4 的 video 会得到空产出。曾考虑在 `fetch_tweet` 层加 `meta.unsupported_media` 诊断，但那需要一段与 `extract_media` 取数口径不同（漏 `entities.media` 回退、需自行解包 wrapper legacy）的并行计数，违反「不重造」决策且逼改 `Meta` struct——故**撤销该诊断**，改为在 spec 正文显式声明此边界为 `extract_media` 既有行为（与 `list_likes` 一致）、HLS 下载支持另开 change。实测目标推文及新老样本均含 mp4 variant，常见场景不触发空产出。
- **[stderr cookie 泄露扩散]** → 决策 6 已约束新代码不得打印 header/cookie；既有 `get_liked_tweets_internal` 的泄露属既有 bug，本变更不扩大它（可另列修复）。
- **[ErrorKind 新增 TweetUnavailable 影响现有匹配]** → 穷举 `match` 会被编译器强制补分支，编译期暴露；`exit_code()` 现有测试 `exit_codes_match_spec` 不锁 TweetUnavailable，新增分支不破坏既有断言。
- **[config.rs 硬编码与 defaults.json 双源 + 原子性]** → 升级四层后 defaults.json 优先于硬编码，长 features 串若两处都留会双源漂移。处置**镜像现有 `likes_*` 范式**:config.rs 硬编码兜底——`tweet_detail_api_url` 留一个有效短 URL（如 `likes_api_url` 那样），`tweet_features`/`tweet_fieldtoggles` 留 `{}` 等最小兜底（与 `likes_features` 兜底为 `{}` 一致）；真 features/fieldtoggles 值进 defaults.json。**关键**:`resolve_protocol_field` 在 defaults 层缺该 key 时直接落 hardcoded，故「砍 config 硬编码」(tasks 3.2) 与「defaults.json 加 3 字段」(tasks 7.1) 必须同一原子提交——否则中间态二进制 `tweet_features` 兜底为 `{}`、请求被 X 拒。并加单测断言 `VENDORED_DEFAULTS` 含非空 `tweet_detail_api_url`/`tweet_features`/`tweet_fieldtoggles`，把「defaults.json 必含 3 字段」从未验证前提变成测试期强约束（`VENDORED_DEFAULTS` 是 `include_str!` 编译期内联，运行时恒在）。
- **[`scripts/check-packaging.sh` defaults.json 白名单]** → 脚本内硬编码 `allowed` 数组须加 3 字段，否则打包校验 fail（tasks 给确切改点）。

## 迁移计划

纯新增能力。`tweet_detail_*` 从 env-only 升级为四层解析、真值移 defaults.json——对终端用户行为等价（默认值实测可用），但需在 proposal「影响」如实标注这是解析层变更而非纯接通。旧 `private_tokens.env` 无需改动。回滚:删除 `tweet get` 子命令与 `fetch_tweet` 工具注册即可。

## 待解决问题

- 多 id 批量（逗号分隔）：MVP 先单条；若高频再开后续 change，输出可改 `TweetSummary[]` 复用 `download_media` 批量。
- 既有 `get_liked_tweets_internal` 的 stderr cookie 泄露是否在本变更外单独修复——建议单列 issue。
