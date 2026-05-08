# X Likes Downloader Skill

让 AI Agent 通过自然语言操作"我自己的 X 点赞"——列点赞、按需下载媒体、自检凭据健康状态。底层调用本地的 `xld` 二进制（Rust 实现），凭据完全保留在用户机器上。

**适用场景**：

- "看一下我最近点赞了哪些和 Rust 有关的内容"
- "把这三条点赞里的视频下载下来"
- "我的 cookie 还能用吗？"

**不适用**：跨账号、批量监控他人、第三方 API key 模式。

---

## 前置依赖

- `xld` 可执行文件 ≥ **1.0.6**（含 Agent Skill 子命令）
- `xld` 必须在 PATH 中。Skill 调用前会执行 `xld --version`；若不存在，将返回 `error.kind: "binary_missing"`，请将用户引导到 [GitHub Releases](https://github.com/HerbertGao/x_likes_downloader/releases)
- 用户已运行 `xld setup` 导入 cURL（首次使用）

---

## Agent 工具表

Skill 暴露**四个**工具，每个底层为一条 `xld` 子命令调用，stdout 为 JSON 信封，stderr 视工具不同含 NDJSON 进度或诊断文本。

### 1. `list_likes`

**用途**：拉取当前账号的点赞推文列表。

**调用形态**：
```
xld likes list [--all] [--since-cursor <c>] [--count <n>] [--include-raw] --json
```

**参数**：

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `all` | bool | 否 | 翻页拉全部历史；默认仅首页 |
| `since_cursor` | string | 否 | 增量同步起点游标（来自上次输出的 `meta.cursor`） |
| `count` | number | 否 | 单页条数；默认 20 |
| `include_raw` | bool | 否 | 同时返回原始 GraphQL entry，**显著增加 token 成本**（5–10 倍），仅用于调试 |

**返回结构**（成功）：

```jsonc
{
  "ok": true,
  "meta": { "schema_version": 1, "cursor": "..." },
  "data": {
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
}
```

**典型错误 `kind`**：`auth_expired` / `endpoint_stale` / `rate_limited` / `network_error` / `not_configured` / `internal_error`

---

### 2. `download_media`

**用途**：按一组 `MediaItem` 下载媒体到沙箱目录。**最佳实践**：直接把 `list_likes` 输出的 `data.tweets[].media[]` 元素拼成数组传回，不要做转换——`MediaItem` 形态完全一致。

**调用形态**：
```
# 首选（item 形态）：
xld media download --items @items.json [--subdir <name>] [--concurrency <n>] --json

# 快捷方式（按 tweet ID，内部触发 list_likes 拉详情）：
xld media download --ids 123,456 [--subdir <name>] [--concurrency <n>] --json
```

**参数**：

| 参数 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `items` | `MediaItem[]`（JSON 字符串或 `@<file>`） | 与 `ids` 二选一 | 见上 |
| `ids` | string（逗号分隔 tweet ID） | 与 `items` 二选一 | 仅含 ASCII 数字；非法即拒 |
| `subdir` | string | 否 | 沙箱内子目录名；不允许 `..` / 绝对路径 / 驱动器盘符 |
| `concurrency` | number | 否 | 并发下载数；钳位 [1, 16]，默认 4 |

**输出（stdout）**：单个 JSON 信封；`data.downloads[]` 含每项结果，`data.summary` 含计数。

**输出（stderr，仅 `--json` 模式）**：NDJSON 进度事件，每行一个 JSON 对象：

```jsonc
{"event":"download_started","total":3,"concurrency":4}
{"event":"item_started","tweet_id":"123","url":"...","index":0,"total":3}
{"event":"item_progress","tweet_id":"123","bytes_done":1024,"bytes_total":4096}
{"event":"item_done","tweet_id":"123","status":"downloaded","bytes":4096}
{"event":"download_finished","summary":{"total":3,"downloaded":2,"skipped":1,"failed":0}}
```

支持的 `event` 类型：`download_started` / `item_started` / `item_progress` / `item_done` / `download_finished` / `diagnostic`。

**分批使用建议**：当 item 数量 > 20 或预计总字节数 > 50 MB 时，请将下载分成多次调用，**每批 5–10 个 item**。这样 stderr NDJSON 进度能驱动用户可见的分阶段反馈，且避免单次调用阻塞过久。

**典型错误 `kind`**：`sandbox_violation`（subdir 路径穿越）/ `invalid_item`（items 格式错误）/ `invalid_argument`（ids/concurrency 越界、items 与 ids 同传）/ `network_error`

---

### 3. `auth_status`

**用途**：探测当前凭据是否仍可访问 Likes 端点。**实际发起一次轻量真实请求**（约 200ms），不是仅做字段检查。

**调用形态**：
```
xld auth status --json
```

**返回（健康）**：
```jsonc
{ "ok": true, "data": { "status": "healthy", "checked_at": "2026-05-07T12:34:56+00:00" } }
```

**返回（失效）**：`error.kind` 为 `auth_expired` / `endpoint_stale` / `rate_limited` / `network_error` / `not_configured`。

#### `auth_status` 使用规范（重要）

`auth_status` 执行真实网络请求，不应被频繁调用。**Agent 应当仅在以下三种场景调用**：

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

**调用形态**：
```
xld setup --curl-file <path> --json
```

`--json` 触发 stdout JSON 信封；不带 `--json` 时是人类文本（`生成 ... 成功！` / `初始化完成。`），Agent 必须使用 `--json`。

或编程式（仅 lib 层，未直接通过 CLI 暴露）：调用 lib 函数 `xld::agent::import_curl(curl_text)`。

**返回结构**（成功）：

```jsonc
{
  "ok": true,
  "meta": { "schema_version": 1 },
  "data": {
    "written": true,
    "path": "data/private_tokens.env",
    "protocol_params_extracted": true
  }
}
```

**典型错误 `error.kind`**：

- `invalid_argument`：cURL 文件读取失败 / cURL 内容不是 Likes 端点 / cookie 缺字段
- `internal_error`：写入 `data/private_tokens.env` 失败

**当 Agent 引导用户时**：告诉用户在浏览器中打开 X，登录后从 DevTools Network 标签页找到任意 `/Likes` API 请求，右键"Copy as cURL"，保存到文件，然后运行 `xld setup --curl-file <path> --json`。

---

## 不暴露的能力

以下 `xld` 子命令**不在** Agent 工具表内，请勿调用：

- `xld download`：旧版"列+下"一把梭，写到 `./downloads`，不走沙箱。保留给人类用户的现有工作流
- `xld organize`：按用户名归档已下载文件。Agent 通常会按自己的逻辑（按主题、时间）组织内容，无需服务端归档
- `xld update`：版本自更新，应由用户手动管理

---

## 调用约定总览

| 项 | 约定 |
|---|---|
| stdout | **单个 JSON 信封**：`{ ok, data?, meta, error? }`。Agent 应直接 `JSON.parse` |
| stderr | `download_media --json` 输出 NDJSON 进度；其它工具仅有诊断文本，可不解析 |
| 退出码 0 | 成功 |
| 退出码 2 | 可重试错误（`auth_expired` / `endpoint_stale` / `rate_limited` / `network_error`） |
| 退出码 1 | 不可恢复（`not_configured` / `invalid_argument` / `invalid_item` / `sandbox_violation` / `internal_error`） |
| `auth_expired` / `endpoint_stale` | 引导用户重新导出 cURL → `xld setup --curl-file <path> --json` |
| `binary_missing` | 引导用户到 GitHub Releases 安装 `xld` |

---

## 安全与边界

- 凭据（auth_token / ct0 / bearer）只存在于用户本地 `data/private_tokens.env`，永远不入仓
- `download_media` 的写入路径被 sandbox 严格限定在 base dir 之内；Agent 即使尝试 `--subdir "../etc"` 也会被拒绝
- 使用 X 内部 GraphQL 端点理论上违反 X ToS，由用户承担合规边界
- 本 skill 不引入主动节流，依赖现有翻页节奏；Agent 大批量调用可能触发 X 的 anti-bot
