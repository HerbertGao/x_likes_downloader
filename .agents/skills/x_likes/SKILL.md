---
name: x-likes
description: Operate the user's own X (Twitter) Likes via natural language — list likes, download media, fetch any tweet's media, check credential health. Prefers the local `x_likes_downloader serve --mcp` MCP server (5 tools) and falls back to the `x_likes_downloader ... --json` CLI when MCP is unavailable. Works in any agent with shell access. All credentials stay on the user's machine.
min_binary_version: 2026.6.1
---

# X Likes Downloader Skill

让 AI Agent 通过自然语言操作"我自己的 X 点赞"——列点赞、按需下载媒体、抓取任意推文的媒体、自检凭据健康状态。全部逻辑在本机执行，凭据从不离开用户机器。

**适用场景**：

- "看一下我最近点赞了哪些和 Rust 有关的内容"
- "把这三条点赞里的视频下载下来"
- "这条推文里的视频帮我存下来"（不限于点赞过的推文）
- "我的 cookie 还能用吗？"

**不适用**：跨账号、批量监控他人、第三方 API key 模式。

---

## 前置依赖

- `x_likes_downloader` 可执行文件 ≥ **2026.6.0**，且位于 `PATH` 中。未安装时引导用户到 [GitHub Releases](https://github.com/HerbertGao/x_likes_downloader/releases)
- 用户已导入 cURL 凭据（首次使用）。缺失时见下方 `setup_from_curl`：`x_likes_downloader setup --curl-file <path>`

---

## 调用形态：MCP 优先，CLI 回落

两种形态共享同一份 lib 实现（`agent::list_likes` / `agent::download_media` / `agent::auth_status` / `agent::import_curl` / `agent::fetch_tweet`），语义、JSON 结构、错误 `kind` 完全一致。**任选其一即可，不要在同一轮对话里为同一件事混用。**

**判断用哪个**：

- **MCP 可用** → 用 MCP 工具。`x_likes_downloader serve --mcp` 已注册到客户端时，你的工具列表里会出现 `list_likes` / `download_media` / `auth_status` / `setup_from_curl` / `fetch_tweet`。**优先选它**：下载有 `notifications/progress` 实时进度，`notifications/cancelled` 能真实中断
- **MCP 不可用**（工具不在列表里 / 客户端不支持 MCP / 用户只装了 binary）→ 用 shell 调用同名子命令，读 stdout 的 JSON 信封。功能等价，但没有进度推送、无法中途取消

| 操作 | MCP 工具 | CLI 回落 |
| --- | --- | --- |
| 列点赞 | `list_likes` | `x_likes_downloader likes list --json` |
| 下载媒体 | `download_media` | `x_likes_downloader media download --items @items.json --json` |
| 凭据自检 | `auth_status` | `x_likes_downloader auth status --json` |
| 导入凭据 | `setup_from_curl` | `x_likes_downloader setup --curl-file <path>` |
| 抓任意推文 | `fetch_tweet` | `x_likes_downloader tweet get --url <url> --json` |

CLI 回落统一输出信封 `{ok, data?, meta: {schema_version, cursor?}, error?}`：`ok == false` 时 `error` 含 `{kind, message, hint?}`，语义与 MCP 错误一致。退出码非零即失败，**不要**只看 stdout 是否为空。

> MCP 注册方式因客户端而异，见本 skill 目录下的 [README.md](./README.md)；注册后需重启客户端。用户不想配 MCP 时直接走 CLI 回落，功能不受影响。

---

## Agent 工具表

五个工具。MCP 形态下客户端启动后调用 `tools/list` 自动发现它们；CLI 形态下即上表的子命令。

### 1. `list_likes`

**用途**：拉取当前账号的点赞推文列表（扁平 schema：id / author_handle / text / media[] / created_at 等）。返回值含 cursor 用于增量同步。Agent 应当把 `tweets[].media[]` 元素直接传给 `download_media` 下载。

**调用**：

```bash
x_likes_downloader likes list --json --count 20
x_likes_downloader likes list --json --since-cursor "<cursor>"
x_likes_downloader likes list --json --all
```

**参数**（MCP JSON Schema 经 `tools/list` 暴露；CLI 有同名 flag，下划线转连字符）：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `all` | bool | 否 | 翻页拉全部历史；默认仅首页 |
| `since_cursor` | string | 否 | 增量同步起点游标（来自上次输出的 `cursor` 字段） |
| `count` | number | 否 | 单页条数；默认 20 |
| `include_raw` | bool | 否 | 同时返回原始 GraphQL entry，**显著增加 token 成本**（5–10 倍），仅用于调试 |

**返回内容**（MCP：`CallToolResult { isError: false, content: [{type: "text", text: "<JSON>"}] }`，text 反序列化后；CLI：stdout 信封的 `data` 字段）：

```jsonc
{
  "tweets": [
    {
      "id": "1234567890",
      "author_handle": "alice",
      "author_display_name": "Alice",
      "text": "tweet 正文（无 t.co 替换）",
      "created_at": "Thu Apr 06 15:24:15 +0000 2017",
      "tweet_url": "https://x.com/alice/status/1234567890",
      "is_retweet": false,
      "is_reply": false,
      "media": [
        {
          "tweet_id": "1234567890",
          "type": "image",            // image | video | gif
          "url": "https://pbs.twimg.com/media/AAA.jpg?format=jpg&name=orig",
          "suggested_filename": "AAA.jpg",
          "bytes": null
        }
      ],
      "liked_at": null
    }
  ],
  "cursor": "...",
  "schema_version": 1
}
```

**典型错误 `kind`**：`auth_expired` / `endpoint_stale` / `rate_limited` / `network_error` / `not_configured` / `internal_error`

---

### 2. `download_media`

**用途**：把一组 `MediaItem` 下载到沙箱目录。**最佳实践**：直接把 `list_likes` / `fetch_tweet` 输出的 `media[]` 元素拼成数组传回——`MediaItem` 形态完全一致。

**调用**：

```bash
# 从 list_likes / fetch_tweet 的输出里取出 media[] 写成 items.json
x_likes_downloader media download --items @items.json --json --concurrency 4
x_likes_downloader media download --items '<MediaItem[] JSON>' --json
x_likes_downloader media download --ids 1234,5678 --json          # 快捷方式：内部拉详情
```

**参数**：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `items` | `MediaItem[]` | 是（与 `ids` 二选一） | 来自 `list_likes` / `fetch_tweet` 的 media 元素列表 |
| `subdir` | string | 否 | 沙箱内子目录名；不允许 `..` / 绝对路径 / 驱动器盘符 |
| `concurrency` | number | 否 | 并发下载数；钳位 [1, 16]，默认 4 |

`ids` 是 CLI 专属快捷方式，MCP 形态没有对应参数。

**进度反馈（重要 / v2.1 数值化）**：MCP 形态下，请求带 `_meta.progressToken` 时 server 通过 `notifications/progress` 实时推送进度。每条 notification 含：

- `progress`（浮点数，**单调非递减**）：`items_done + Σ_in_flight (bytes_done / bytes_total)`。完成的 item 贡献整数 1；in-flight item 贡献其字节比例 `[0, 1]`。范围 `[0, total_items]`，浮点容差 0.001
- `total`（数值）：批次总 item 数，与 `progress` 配对使用——client 计算百分比 = `progress / total`
- `message`（字符串）：当前操作描述，含字节级进度细节（如 "starting tweet 12345"、"tweet 12345 1024/4096"、"tweet 12345 downloaded"）

这与 [MCP 规范的 `ProgressNotificationParam`](https://modelcontextprotocol.io/specification/2025-11-25) 一致。完成时发出 `progress == total_items`（比例 1.0）。**v2.1 升级**：相比 v2.0 的整数 item 计数，progress 现含 in-flight item 的字节比例分量，UI 在大文件场景下不再"卡 0%"直到 100%——会连续过渡。`bytes_total = None` 时 fraction 兜底 0.5（既不卡 0 也不假装快好）。

**CLI 形态无进度通知**：工具调用阻塞直到所有 item 完成。此时应把批次切得更小（见下方分批建议），让用户至少看到阶段性的命令返回。

**v2.1 cancellation 真实生效**：MCP client 发 `notifications/cancelled` 时，对应 `download_media` 工具调用必须在 1 秒内返回。返回的 `CallToolResult` 满足：

- `isError: false`（cancel **不**视作错误）
- `content` 含完整 `DownloadOutput`：已完成 item 状态保留（`downloaded` / `skipped_existing` / `failed`），in-flight 中断 item 状态为 `cancelled`（`.partial` 文件保留供下次 Range 续传），未启动 item 状态也为 `cancelled`
- `summary.cancelled` 字段反映被 cancel 的 item 数

Agent 据此可决定**重试策略**：`cancelled` item 通常表示用户主动取消，可直接重试（lib 自动 Range 续传）；`failed` item 表示真实错误，需要根据 `error.kind` 决定是重试还是放弃。

**`.partial` 文件保留行为**：v2.1 起，所有下载流写入 `<final_path>.partial`，成功后 `fs::rename` 到最终路径。失败 / cancel 保留 `.partial`：

- 下次同 item 下载时自动 HTTP HEAD 探测 ETag → 一致则发 Range 续传请求，从断点接续
- ETag 不一致 / cache 缺失 / Content-Range mismatch 自动删除 `.partial` 重头下，并通过 progress message 提示原因
- 用户清理：`find <sandbox> -name "*.partial" -delete`

**返回内容**（成功 / cancelled 都走此路径，因 `cancelled` 不视作错误）：

```jsonc
{
  "downloads": [
    { "tweet_id": "...", "url": "...", "path": "...", "bytes": 12345, "status": "downloaded" },
    { "tweet_id": "...", "url": "...", "path": "...", "bytes": 0, "status": "cancelled" },
    // ... status 可为 "downloaded" / "skipped_existing" / "failed" / "cancelled"
  ],
  "summary": { "total": 5, "downloaded": 3, "skipped": 1, "failed": 0, "cancelled": 1 }
}
```

**典型错误 `kind`**：`sandbox_violation`（subdir 路径穿越）/ `invalid_item`（items 字段不全）/ `invalid_argument`（concurrency 越界）/ `network_error`

**分批使用建议**：当 item 数量 > 20 或预计总字节数 > 50 MB 时，请将下载分成多次调用，**每批 5–10 个 item**。MCP 形态下这样能让 `notifications/progress` 驱动用户可见的分阶段反馈；CLI 形态下这是唯一能给用户阶段反馈的办法。

---

### 3. `auth_status`

**用途**：探测当前凭据是否仍可访问 Likes 端点。**实际发起一次轻量真实请求**（约 200ms），不是仅做字段检查。

**调用**：

```bash
x_likes_downloader auth status --json
```

**参数**：（无）

**返回内容（健康）**：

```jsonc
{ "status": "healthy", "checked_at": "2026-05-08T12:34:56+00:00" }
```

**返回内容（失效）**：`isError: true`（CLI：`ok: false` + 非零退出码），content 含 `{ kind, message, hint }` 形态错误，`kind` 为 `auth_expired` / `endpoint_stale` / `rate_limited` / `network_error` / `not_configured`

#### `auth_status` 使用规范（重要）

`auth_status` 执行真实网络请求，**不应被频繁调用**。**Agent 应当仅在以下三种场景调用**：

1. **会话起始预检**（最多一次）：用户开始一段操作 X 点赞的对话时，先 ping 一下确认环境健康
2. **错误后确认**：其它工具返回 `auth_expired` 或 `endpoint_stale` 时，调一次确认是否真的需要重导 cURL
3. **用户显式询问**："我的 cookie 还能用吗？"这类直接问题

**禁止以下使用模式**：

- ❌ 在循环中调用
- ❌ 每次工具调用前作为预检调用
- ❌ 为做缓存而连续调用（server 不缓存，重复调用就是重复打 X 的 API）

如果 Agent 需要更新鲜的状态，重新调用 `list_likes` 即可——它本身就会反映出 cookie 是否有效。

---

### 4. `setup_from_curl`

**用途**：从用户提供的 cURL 文本导入凭据 + 协议参数（首次配置或 cookie 失效后重导）。

**调用**：

```bash
# MCP 形态：把 cURL 文本作为 curl_text 参数传给 setup_from_curl
# CLI 形态：把 cURL 文本存成文件后导入
x_likes_downloader setup --curl-file ~/curl_command.txt
```

CLI 形态也支持不带 `--curl-file` 的交互式粘贴：`x_likes_downloader setup`。

**参数**：

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `curl_text` | string | 是 | 完整的 cURL 文本（从浏览器 DevTools "Copy as cURL (bash)" 拷贝） |

`curl_text` 是 MCP 专属参数；CLI 形态走文件或交互式输入。

**安全保证**：函数内存解析，**不**写临时文件；返回值**不**回显任何凭据。

**返回内容（成功）**：

```jsonc
{
  "written": true,
  "path": "<host-managed credentials path>",
  "protocol_params_extracted": true
}
```

**典型错误 `kind`**：

- `invalid_argument`：cURL 内容不是 Likes 端点 / cookie 缺字段 / Bearer 缺失
- `internal_error`：写入凭据文件失败

**当 Agent 引导用户时**：告诉用户在浏览器中打开 X，登录后从 DevTools Network 标签页找到任意 `/Likes` API 请求，右键"Copy as cURL"，MCP 形态把整个文本作为 `curl_text` 传入，CLI 形态则先存成文件再 `--curl-file`。

**凭据内容不要回显给用户，也不要写进任何文件或日志。**

---

### 5. `fetch_tweet`

**用途**：按 URL 或 tweet_id 抓取任意一条推文的媒体元数据，返回与 `list_likes` 同构的 `TweetSummary`（含 `media[]`）。**只取元数据、自身不下载**——Agent 应当把返回的 `tweet.media[]` 元素直接作为 `download_media` 的 `items[]` 入参来完成下载。

**调用**：

```bash
x_likes_downloader tweet get --url https://x.com/alice/status/1234567890 --json
x_likes_downloader tweet get --id 1234567890 --json
```

**参数**（恰好提供 `url` / `id` 其一）：

| 参数 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `url` | string | 二选一 | 推文 URL，如 `https://x.com/alice/status/1234567890`（接受 `twitter.com` / `mobile.twitter.com` / `i/web/status/<id>` / 尾随 `/photo/1`、`/video/1` 等形态，带 query 也可） |
| `id` | string | 二选一 | 纯数字 tweet_id，如 `1234567890` |

`url` 与 `id` 必须**恰好提供其一**；都给或都不给均返回 `invalid_argument`。

**返回内容**（MCP：`CallToolResult { isError: false }`，text 反序列化后为 `FetchTweetOutput`；CLI：stdout 信封的 `data` 字段）：

```jsonc
{
  "tweet": {
    "id": "1234567890",
    "author_handle": "alice",
    "author_display_name": "Alice",
    "text": "tweet 正文",
    "created_at": "Thu Apr 06 15:24:15 +0000 2017",
    "tweet_url": "https://x.com/alice/status/1234567890",
    "is_retweet": false,
    "is_reply": false,
    "media": [
      {
        "tweet_id": "1234567890",
        "type": "image",            // image | video | gif
        "url": "https://pbs.twimg.com/media/AAA.jpg?format=jpg&name=orig",
        "suggested_filename": "AAA.jpg",
        "bytes": null
      }
    ],
    "liked_at": null
  },
  "schema_version": 1
}
```

`tweet` 字段集与 `list_likes` 输出的 `tweets[]` 元素**完全同构**（同一 `TweetSummary` 结构）。

**典型错误 `kind`**：`invalid_argument`（`url`/`id` 缺失/同给/格式非法）/ `auth_expired` / `endpoint_stale` / `tweet_unavailable`（推文不存在/已删除/受保护/当前凭据不可见）/ `network_error`

**Agent 行为约定**：取回 `tweet.media[]` 后直接作为 `download_media` 的 `items[]` 下载即可（`MediaItem` 形态完全一致）；`fetch_tweet` 只取元数据，**不下载**。

---

## 不暴露的能力

以下 `x_likes_downloader` 子命令**不在** Agent 工具表内（也不在 MCP `tools/list` 暴露），请勿调用：

- `x_likes_downloader download`：旧版"列+下"一把梭，写到 `./downloads`，不走沙箱。保留给人类用户的现有工作流
- `x_likes_downloader organize`：按用户名归档已下载文件。Agent 通常会按自己的逻辑（按主题、时间）组织内容，无需服务端归档
- `x_likes_downloader update`：版本自更新，应由用户手动管理

---

## 调用约定总览

| 项 | 约定 |
| --- | --- |
| 协议（MCP 形态） | MCP（JSON-RPC 2.0，stdio transport） |
| 工具调用（MCP 形态） | `tools/call { name, arguments }` |
| 成功（MCP 形态） | `CallToolResult { isError: false, content: [{type:"text", text: <JSON>}] }`，text 反序列化即结果 |
| 业务错误（MCP 形态） | `CallToolResult { isError: true, content: [{type:"text", text: <error JSON>}] }`，text 含 `{kind, message, hint, retry_after?}` |
| 协议错误（MCP 形态） | JSON-RPC level error response（unknown tool、bad args 等） |
| 成功 / 失败（CLI 形态） | stdout 信封 `{ok: true, data, meta}` / `{ok: false, error, meta}`，失败时退出码非零 |
| 进度反馈 | 仅 MCP 形态：`download_media` 在请求带 `_meta.progressToken` 时通过 MCP `notifications/progress` 推送；其它工具无进度 |
| Cancellation | 仅 MCP 形态：**v2.1 起收到 `notifications/cancelled` 真实生效**——in-flight item 在 1s 内停止并返回 `isError: false` + 完整 `DownloadOutput`（`cancelled` 状态，`.partial` 保留供续传）。CLI 形态无取消通道 |
| `auth_expired` / `endpoint_stale` | Agent 应建议用户重新导出 cURL 并调用 `setup_from_curl` |
| `tweet_unavailable`（仅 `fetch_tweet`） | Agent 应告知用户该推文不存在 / 已删除 / 受保护或当前凭据不可见，**不应重试** |
| `fetch_tweet` 成功但 `tweet.media` 为空 | Agent 应提示用户该推文可能无媒体、或其视频为 HLS-only（当前不支持下载），而非把空 `media[]` 直接喂给 `download_media` 后静默无结果 |
| `binary_missing` | Agent 应将用户引导到 GitHub Releases 安装页 |

---

## 安全与边界

- 凭据（auth_token / ct0 / bearer / personalization_id）**永远不出工具通道**——`tools/call` 响应与 CLI stdout 都不含任何凭据字段；`setup_from_curl` 返回值仅含 path 与 written 标志
- `download_media` 的写入路径被 sandbox 严格限定在 base dir 之内；Agent 即使尝试 `subdir = "../etc"` 也会被拒绝（返回 `sandbox_violation`）
- 凭据存储在用户本地稳定路径（由 binary 选择，host-managed），永远不入仓
- 使用 X 内部 GraphQL 端点理论上违反 X ToS，由用户承担合规边界
- 本 skill 不引入主动节流，依赖现有翻页节奏；Agent 大批量调用可能触发 X 的 anti-bot

---

## 排查：工具不可用

| 现象 | 处置 |
| --- | --- |
| MCP 工具不在列表里 | 走 CLI 回落；或让用户按 [README.md](./README.md) 注册 `x_likes_downloader serve --mcp` 后重启客户端 |
| CLI 报 command not found | binary 未安装或不在 `PATH`；引导到 GitHub Releases |
| 报版本过低 | 需 ≥ `2026.6.0`；让用户升级 binary |
| `not_configured` | 凭据未导入；引导用户跑 `x_likes_downloader setup` |
| `auth_expired` / `endpoint_stale` | 凭据或 GraphQL queryId 过期；重新导出 cURL 并 `setup_from_curl` |
