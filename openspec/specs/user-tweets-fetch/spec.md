# user-tweets-fetch Specification

## 目的
让 Agent 按 username 拉取任意指定账号（不限于本人）的可见推文列表，产出与点赞列举同构的扁平推文摘要与增量游标，媒体可直接衔接下载。

## 需求

### 需求:CLI 子命令 `xld user tweets`

系统必须提供 `xld user tweets` 子命令，按 username 列出指定账号的推文，以结构化 JSON 信封输出（数据 stdout / 诊断 stderr，退出码由失败信封的 `ErrorKind::exit_code()` 决定，与 `likes list` / `tweet get` 同一套映射）。参数：

- `--username <name>`（必填）：目标账号 screen_name，允许前导 `@`
- `--all`：按游标翻页拉取全部可见推文（默认 false，仅首页）
- `--since-cursor <cursor>`：从上次输出的游标继续拉取（增量同步）
- `--count <N>`：单页条数（默认沿用配置值）
- `--include-raw`：附带原始 GraphQL entry（调试用）
- `--json`：显式声明 JSON 输出（子命令默认即 JSON）

#### 场景:按 username 拉取成功

- **当** 运行 `xld user tweets --username elonmusk --count 3` 且本地凭据有效、账号可见
- **那么** stdout 输出成功信封：`ok: true`，`data.user` 含 `screen_name` / `rest_id`，`data.tweets[]` 为非空列表，`meta.cursor` 为下一页游标（存在更多推文时非 null），退出码 0

#### 场景:游标续拉

- **当** 运行 `xld user tweets --username <name> --since-cursor <上次 meta.cursor>`
- **那么** 返回的 `data.tweets[]` 与上一页不重叠（起点为游标之后），并给出新的 `meta.cursor`；翻到尽头时 `meta.cursor` 为 null

#### 场景:username 非法即拒

- **当** 运行 `xld user tweets --username "a b"`（含空格 / 空串 / 超长 / 非 `[A-Za-z0-9_]` 字符）
- **那么** 以 `invalid_argument` 失败信封退出（码 1），stderr 给出用法提示，不发起任何网络请求

#### 场景:用户不存在

- **当** 运行 `xld user tweets --username <不存在的账号>` 且 X 返回 HTTP 200 + `data` 缺 `user` 键
- **那么** 以 `tweet_unavailable` 失败信封退出（码 1），message 指明目标账号

### 需求:username 校验与归一化

系统必须在发起任何网络请求前，于信任边界校验并归一化 username：剥离可选的前导 `@` 后，主体必须为 `[A-Za-z0-9_]{1,15}`；不满足则以 `invalid_argument` 拒绝。

#### 场景:前导 @ 剥离

- **当** 输入 `@elonmusk`
- **那么** 归一化为 `elonmusk` 并按其发起请求

#### 场景:非法输入拒绝

- **当** 输入为空串、`@`、含空格 / 连字符 / 非 ASCII 字符、或长度超过 15
- **那么** 返回 `invalid_argument`，不做网络往返

### 需求:UserByScreenName 用户解析

系统必须以 `UserByScreenName` GraphQL 端点把 username 解析为用户摘要：请求 variables 为 `{"screen_name": <name>}`，features / fieldToggles 取自 `user_by_screen_name_*` 配置字段（四层解析），鉴权 header 与 `Likes` 一致。成功时必须从 `data.user.result` 提取 `rest_id`、`core.screen_name`（旧 schema 回退 `legacy.screen_name`）与 `core.name`。

用户不可见必须分类为 `tweet_unavailable`（不应重试）：包括 `data` 缺 `user` 键（X 返回 `{"data":{}}`）、`result.rest_id` 缺失（含 `__typename: UserUnavailable`，如注销 / 被封）。HTTP 200 但 `errors[].code ∈ {32,64,89}` 必须分类为 `auth_expired`。

#### 场景:正常解析

- **当** 请求 `screen_name: "elonmusk"` 成功返回
- **那么** 提取 `rest_id`、`screen_name`、`name` 三字段（手工骨架 fixture 覆盖，占位值为 `900000001` / `fixture_user_a` / `Fixture User A`；真实响应只用于一次性核对字段路径，不入仓；live 实测见 tasks 5.2 的 `elonmusk` 记录）

#### 场景:用户不存在返回 tweet_unavailable

- **当** 请求一个不存在的 screen_name，X 返回 HTTP 200 + `{"data":{}}`
- **那么** 以 `tweet_unavailable` 失败，message 说明用户不存在、已注销、被封禁或不可见

#### 场景:UserUnavailable 同样不可重试

- **当** 响应 `data.user.result.__typename` 为 `UserUnavailable`（无 `rest_id`）
- **那么** 同样归类 `tweet_unavailable`

#### 场景:旧 schema 回退

- **当** 响应无 `core.screen_name` 但有 `legacy.screen_name`
- **那么** 以 `legacy.screen_name` 作为 `screen_name` 提取

### 需求:UserTweets 翻页拉取

系统必须以 `UserTweets` GraphQL 端点按 `rest_id` 分页拉取推文：请求 variables 含 `userId` / `count` / `includePromotedContent: false` / `withQuickPromoteEligibilityTweetFields: false` / `withVoice: true`，翻页时附加上页 `cursor-bottom-` 游标。翻页语义必须与 `likes list` 一致：无 `--all` 仅单页；`--all` 按游标连续翻页，游标缺失或与上页相同即停；`--since-cursor` 为续拉起点。

#### 场景:单页返回

- **当** 以有效 `rest_id` 请求一页
- **那么** 返回顶层推文 entries 与 `cursor-bottom-` 游标

#### 场景:all 翻页

- **当** `--all` 且账号有多页推文
- **那么** 持续翻页聚合，直至响应无新游标或游标与上页相同

#### 场景:限流分类

- **当** X 返回 HTTP 429
- **那么** 以 `rate_limited` 失败信封退出，附 `retry_after`（默认 60 秒）

### 需求:UserTweets 响应解析与单页边界

响应解析必须复用点赞列表的既有解析路径：外层 timeline 节点双结构兼容（`timeline_v2` / `timeline`），仅取 `TimelineAddEntries` 的顶层 `tweet-*` entry（不含 `TimelinePinEntry` 置顶推文与 `profile-conversation-*` 折叠线程），每条 entry 以与点赞链路相同的提取逻辑转为同构 `TweetSummary`。`count` 按请求透传且不做客户端侧裁剪；CLI 帮助与 MCP 工具描述必须注明已知边界——X 端忽略过小的 `count`，单页实际约 20 条，分页以 `cursor` 为准。

#### 场景:双结构兼容

- **当** 响应外层节点为 `timeline`（2026-11 现役形态，fixture 覆盖）或 `timeline_v2`（旧形态）
- **那么** 两种形态均解析出 `tweet-*` entries 与游标

#### 场景:同构提取

- **当** 一页 `tweet-*` entry 被解析
- **那么** 每条转为与点赞列表同构的 `TweetSummary`（含 `id` / `author_handle` / `media[]` / `created_at` / `tweet_url`）

#### 场景:折叠线程不进入结果

- **当** 响应含 `profile-conversation-*` module entry
- **那么** 该类 entry 不出现在 `tweets[]` 中（仅顶层推文）

### 需求:输出 schema 与下载衔接

成功输出必须为 `UserTweetsOutput`：`user{screen_name, rest_id, name?}`、`tweets[]`（与 `list_likes` 的 `tweets[]` 元素严格同构）、`cursor`（可 null）、`schema_version`、`include_raw` 时附 `raw_entries[]`。`tweets[].media[]` 必须可直接作为 `download_media` 的 `items[]` 入参。

#### 场景:信封结构

- **当** 拉取成功且解析 stdout JSON 信封
- **那么** 满足 `ok === true`、`data.user.rest_id` 为数字字符串、`Array.isArray(data.tweets)`、`data.schema_version` 为数字

#### 场景:媒体可直接下载

- **当** 把输出中的 `data.tweets[].media[]` 作为 `items[]` 调用 `download_media`
- **那么** 下载照常完成（item schema 与点赞链路完全一致，无需转换）
