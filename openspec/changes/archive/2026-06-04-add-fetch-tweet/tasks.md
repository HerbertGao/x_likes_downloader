## 1. 类型与错误体系

- [x] 1.1 在 `src/error.rs` 的 `ErrorKind` 枚举新增 `TweetUnavailable` 变体；在 `exit_code()` 把它归入返回 **1** 的分支组（与 `NotConfigured` 同组）；在 `default_hint()` 补中文 hint（如「推文不存在、已删除、受保护或当前凭据不可见」）；补一条单测断言 `TweetUnavailable.exit_code() == 1`（勿改动现有 `exit_codes_match_spec` 已锁定的 auth_expired=2 等）
- [x] 1.2 新增 `FetchTweetRequest { url: Option<String>, id: Option<String> }`，派生 `Deserialize` + `schemars::JsonSchema`（Cargo.toml 已含 schemars，无需加依赖），字段加中文 doc 注释；放置位置与现有 MCP 请求类型（当前 `ListLikesRequest` 等定义在 `mcp_server.rs`）保持一致。同时在 `agent::types` 新增 `FetchTweetOutput { tweet: TweetSummary, schema_version: u32 }`（镜像 `ListOutput`，派生 `Serialize, Deserialize, Debug, Clone` 与 ListOutput 一致；不含 unsupported_media——见 design 决策，不引入影子计数）

## 2. 复用现有解析逻辑

- [x] 2.1 将 `src/agent/list_likes.rs` 的 `fn entry_to_summary` 与 `fn extract_media` 可见性提升为 `pub(crate)`（现有 `#[cfg(test)] mod tests` 的 `use super::*` 调用不受影响）
- [x] 2.2 确认 `entry_to_summary` 的 `result.tweet` 解包（L238）覆盖 `TweetWithVisibilityResults` wrapper；用 `tests/fixtures/tweet_detail/old_2023_TweetWithVisibilityResults.json` 的焦点 entry 作 fixture 补一条单测验证老推文 wrapper 能取到非空 legacy+media

## 3. TweetDetail 请求与配置

- [x] 3.1 在 `src/x_api.rs` 的 `XApi` 新增 `get_tweet_detail(&self, tweet_id: &str) -> Result<(u16, serde_json::Value)>`：用 `tweet_detail_api_url` + `tweet_features` + `tweet_fieldtoggles` + 完整 `variables`（含 `focalTweetId` + `with_rux_injections:false` + `rankingMode` + `includePromotedContent` + `withCommunity` + `withQuickPromoteEligibilityTweetFields` + `withBirdwatchNotes` + `withVoice`，URL 编码）构造 GET，复用鉴权 header 构造（User-Agent 沿用 list_likes.rs 的空值保护写法，勿照抄 x_api.rs）；**先 `let status = response.status().as_u16()`，再 `response.text()`（仅传输错误返 Err→network_error），`serde_json::from_str(&text).unwrap_or(Value::Null)`**——非 JSON body（如 401 HTML 页）不得使函数 Err（否则绕过 classify_status）；非 2xx 不提前 Err；**禁止 `eprintln!` 含 cookie/bearer 的 header 或完整 URL 到 stderr**
- [x] 3.2 在 `src/config.rs` 把 `tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles` 从 `env::var(...).unwrap_or_else(硬编码)` 改为 `resolve_protocol_field`，**(env_key, json_key) 三对逐字钉死**：`("TWEET_DETAIL_API_URL","tweet_detail_api_url")` / `("TWEET_FEATURES","tweet_features")` / `("TWEET_FIELDTOGGLES","tweet_fieldtoggles")`（json_key 必须与 defaults.json 顶层键名一致，否则第 3 层 miss）；**镜像 `likes_*`**：URL 硬编码兜底留有效短 URL、features/fieldtoggles 硬编码兜底留 `{}`（真值移 defaults.json）。**与 7.1 必须同一原子提交**（否则中间态二进制 features 兜底为 `{}`、请求被 X 拒）。补单测：(a) 断言 `VENDORED_DEFAULTS` 含非空 `tweet_detail_api_url`/`tweet_features`/`tweet_fieldtoggles`；(b) 空 private_tokens + 真实 `VENDORED_DEFAULTS` 下 `resolve_protocol_field("TWEET_FEATURES","tweet_features",...)` 返回非 `{}` 且可 `serde_json::from_str` 成 object（验证 json_key 映射正确）

## 4. fetch_tweet 能力

- [x] 4.1 新建 `src/agent/fetch_tweet.rs`：实现 `parse_tweet_detail_response(resp, focal_id) -> Option<entry>`（纯函数；`resp` 为完整响应 Value，路径是 `resp["data"]["threaded_conversation_with_injections_v2"]["instructions"][]`，勿漏 `["data"]` 这层），主路径匹配顶层 `entryId == "tweet-<focal_id>"`；未命中时软兜底遍历所有 entries（含 `conversationthread-*` module 内 item）取解包 wrapper 后 `tweet_results.result...rest_id == focal_id` 的 entry
- [x] 4.2 在 `fetch_tweet.rs` 实现 `extract_tweet_id(req) -> Result<String, ErrorPayload>`（纯函数）：`--id` 校验纯数字；`--url` 用 `regex` 提取 `/status/(\d+)`，接受 `x.com`/`twitter.com`/`mobile.twitter.com`/`i/web/status`/`/photo|video/N` 后缀；二者恰好其一，否则 `invalid_argument`
- [x] 4.3 实现**阶段一**纯函数 `classify_tweet_detail(status, resp) -> Option<ErrorPayload>`（不持 focal_id）：先 `error::classify_status(status)`；仅 HTTP 200 时查响应体——顶层 `errors[].code ∈ {32,64,89}`→auth_expired、`data` 缺 `threaded_conversation_with_injections_v2` 路径且无鉴权 errors→endpoint_stale。`tweet_unavailable` **不**在此函数产出（它无 focal_id）
- [x] 4.4 在 `fetch_tweet.rs` 实现 `pub async fn fetch_tweet(req: FetchTweetRequest) -> Result<FetchTweetOutput, ErrorPayload>`：`extract_tweet_id` → `XApi::get_tweet_detail`(返回 `(status, resp)`) → `classify_tweet_detail`（失败短路）→ **阶段二**焦点定位 `parse_tweet_detail_response(resp, id)`：返回 None / 焦点 `__typename ∈ {"TweetUnavailable","TweetTombstone"}` / 缺 legacy → 返回 `tweet_unavailable`；否则 `entry_to_summary` → 组装 `FetchTweetOutput { tweet, schema_version: 1 }`。传输错误映射 network_error
- [x] 4.5 在 `src/lib.rs` 的 `pub mod agent { ... }` 内联块加 `pub mod fetch_tweet;`（仓库无 `src/agent/mod.rs`），并导出 `fetch_tweet` / `FetchTweetRequest` / `FetchTweetOutput`

## 5. CLI 子命令

- [x] 5.1 在 `src/main.rs` 的 `Commands` 新增 `Tweet { action: TweetAction }`，`TweetAction::Get { url: Option<String>, id: Option<String>, json: bool }`
- [x] 5.2 实现 `run_tweet_get(url, id, json)`：构造 `FetchTweetRequest` → `agent::fetch_tweet` → 成功时 `OutputEnvelope::success`，`data` = `output.tweet`、`meta.schema_version` = `output.schema_version`（复用现有 `Meta`，**不改 envelope.rs**）；失败 `OutputEnvelope::failure`；退出码由 `ErrorKind::exit_code()` 决定（invalid_argument=1、tweet_unavailable=1、auth_expired/endpoint_stale/network_error=2），不要手写独立退出码表

## 6. MCP 工具注册

- [x] 6.1 在 `src/agent/mcp_server.rs` 注册第 5 个工具 `fetch_tweet`：中文描述、`FetchTweetRequest` 派生 inputSchema、`tools/call` 路由到 `agent::fetch_tweet`，成功/业务错误按现有 CallToolResult isError 约定返回
- [x] 6.2 确认 `tools/list` 返回工具名集合为 `{list_likes, download_media, auth_status, setup_from_curl, fetch_tweet}`
- [x] 6.3 更新现有 `tests/mcp_smoke.rs`：它当前硬编码 4 个工具名并断言「tools/list 返回 4 个」，须加入 `fetch_tweet` 并改断言为 5 个，否则该集成测试会因新工具失败

## 7. 打包与文档

- [x] 7.1 在 `packaging/skill/x_likes/defaults.json` 新增 `tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles` 三个字段，值为**实测当前可用的完整真值**（query id + 完整 features/fieldtoggles JSON；defaults.json 是这三者的权威来源，config.rs 只留非权威短兜底）。**与 3.2 同一原子提交**
- [x] 7.2 在 `scripts/check-packaging.sh` 的 defaults.json 字段白名单（硬编码 `allowed` JSON 数组，现含 `schema_version,likes_api_url,likes_features,likes_fieldtoggles,bearer_token`）追加 `tweet_detail_api_url,tweet_features,tweet_fieldtoggles`；验收 `bash scripts/check-packaging.sh` 须绿
- [x] 7.3 在 `packaging/skill/x_likes/SKILL.md` 工具表新增 `fetch_tweet`（名称/用途/参数/返回/错误 kind 五要素 + 「media[] 传给 download_media」衔接约定），调用约定章节补 `fetch_tweet` 与 `tweet_unavailable` 恢复路径
- [x] 7.4 运行 `scripts/sync-skill.sh`（若存在）同步派生各 host 副本，确保不漂移

## 8. 测试与验证

- [x] 8.1 单测:`extract_tweet_id` 覆盖带 query 的 URL、尾斜杠、twitter.com 域名、`i/web/status/<id>`、`/status/<id>/photo/1`、非法 URL、非数字 id
- [x] 8.2 单测:`parse_tweet_detail_response` 用 `tests/fixtures/tweet_detail/` 三份脱敏 fixture 经 `include_str!` 内联（new_2026_Tweet→顶层焦点正确选中 / old_2023_TweetWithVisibilityResults→wrapper 解包取非空 legacy+media / old_2024_Tweet_multimedia→多媒体）；再手搓「主路径无 tweet-<id> 但某 entry rest_id 匹配→软兜底命中（best-effort，手搓 fixture）」「主+兜底均无→None」「焦点 TweetTombstone/TweetUnavailable→tweet_unavailable」三例。同时离线断言 `fetch_tweet` 产出 `tweet` 字段集与 `list_likes` 的 `tweets[]` 元素一致（schema 同构，不必等真网络）
- [x] 8.3 单测:阶段一 `classify_tweet_detail` 覆盖 401/403→auth_expired、404→endpoint_stale、**HTTP 200 + errors[code:32]→auth_expired**、**非 JSON body（HTML）+ 401→auth_expired（不变 network_error）**；阶段二焦点 tweet_unavailable 在 8.2 已覆盖（纯函数，离线可测，无需 mock 网络）
- [x] 8.4 `cargo test` 全绿、`cargo clippy` 无新警告；**手动**（依赖真实凭据+网络，CI 不覆盖）`xld tweet get --url <真实推文> --json` 端到端验证，并把 `media[]` 喂给 `download_media` 真实下到 mp4
- [x] 8.5 `openspec-cn validate add-fetch-tweet --strict` 通过
- [x] 8.6 fixture 隐私守卫：`tests/fixtures/tweet_detail/*.json` 已脱敏（handle/显示名/正文/媒体 URL/hashtag/in_reply_to_screen_name 替换为占位；所有数字账号 id——`user_id_str`/`in_reply_to_user_id_str`/Base64 `User:<id>` node id——及 ≥15 位 tweet/conversation id 均替换为合成占位，**注意 9-10 位 legacy 账号 id 不能漏**；保留结构/__typename/variants/tweet-id↔focalId 相等）；加一条 CI/单测断言这三文件不含真实 handle、露骨词、`twimg`/`pic.x.com` 域名、以及 `user_id_str`/Base64 `User:` 中的非占位（非 `9000000xx`）账号 id，防后续误提交未脱敏样本
