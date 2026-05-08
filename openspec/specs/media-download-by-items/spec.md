### 需求:CLI 子命令 `xld media download`

系统必须提供 `xld media download` 子命令，用于按一组 media item 描述下载对应媒体到沙箱目录。该子命令必须支持以下两种互斥的输入形式：

- `--items <json>`：JSON 字符串或 `@<file>` 文件引用，内容为 `MediaItem[]` 数组（与 `list_likes` 输出的 `data.tweets[].media[]` 同形态）
- `--ids <id1,id2,...>`：逗号分隔的 tweet ID 列表，作为快捷方式；系统在内部隐式调用 `list_likes` 拉取这些 tweet 的详情提取 media items 后再下载

`--items` 与 `--ids` 必须互斥，同时提供时拒绝执行并退出码 1。其它参数：

- `--subdir <name>`：可选，沙箱内子目录名（受 `download-sandbox` 能力的 jail 规则约束）
- `--concurrency <n>`：可选，并发下载数；默认 4，钳位到 [1, 16]，超界拒绝执行
- `--json`：输出 JSON 信封到 stdout

#### 场景:按 items 下载（首选路径）
- **当** Agent 从 `list_likes` 拿到 `media[]` 数组并以 `--items @media.json` 调用
- **那么** 系统不再触发任何 GraphQL 调用，仅对 CDN 发起媒体下载，stdout 输出包含每个文件下载结果的 JSON 信封

#### 场景:按 ID 快捷方式
- **当** 用户运行 `xld media download --ids 1234,5678 --json`
- **那么** 系统先内部调用 `list_likes` 拉取这两条 tweet 的扁平表示，再按其 `media[]` 字段下载

#### 场景:items 与 ids 互斥
- **当** 用户同时提供 `--items` 和 `--ids`
- **那么** 系统拒绝执行，stderr 输出参数错误，退出码 1

#### 场景:并发度钳位
- **当** 用户传入 `--concurrency 100` 或 `--concurrency 0`
- **那么** 系统拒绝执行并提示合法范围 [1, 16]，退出码 1

#### 场景:子目录传参
- **当** 用户运行 `xld media download --items @x.json --subdir 2026-05`
- **那么** 系统在沙箱 base dir 下创建（若不存在）`2026-05` 目录并将文件写入其中

### 需求:`MediaItem` 数据形态

每个 `MediaItem` 对象必须包含以下字段（与 `likes-listing-json` 能力中的 `MediaItem` 定义保持完全一致）：

- `tweet_id`（字符串，必填）：归属的 tweet ID，用于文件命名
- `type`（枚举：`image` | `video` | `gif`，必填）
- `url`（字符串，必填）：要实际下载的最佳 variant URL
- `suggested_filename`（字符串，必填）：服务端建议的目标文件名（不含目录）
- `bytes`（数字，可选）：预期字节数（用于断点续传校验）
- `author_handle`（字符串，可选）：父推文作者 @ 名；提供时被默认命名使用
- `created_at`（字符串，可选）：父推文发布时间（X RFC2822 格式）；提供时 `download_media` 默认会把文件 mtime 设到此时间
- `all_variants`（数组，可选）：所有可用 variants（仅 list_likes 在 `--include-raw` 时携带，下载侧可忽略）

#### 场景:items 字段集封闭
- **当** 解析 `--items` 输入
- **那么** 系统必须接受上述字段，对未知字段静默忽略；缺失任何必填字段时拒绝该条目并记录 `error.kind: "invalid_item"`

### 需求:JSON 输出信封——下载结果

`xld media download --json` 的成功信封 `data` 部分必须包含字段 `downloads[]`，每个元素至少包含 `tweet_id`、`url`、`path`（落盘绝对路径）、`bytes`、`status`（值为 `downloaded` / `skipped_existing` / `failed`）；失败条目必须额外含 `error.kind` 与 `error.message`。`data.summary` 必须包含 `total`、`downloaded`、`skipped`、`failed` 四项计数。

#### 场景:逐项结果可观测
- **当** 下载 3 个 media item，其中 1 个已存在
- **那么** `data.downloads[]` 长度为 3，包含 1 个 `status: "skipped_existing"` 条目；`data.summary` 中 `skipped` 等于 1

#### 场景:部分失败不中断其它下载
- **当** 多个 item 中某个 URL 返回 404
- **那么** 系统继续完成其它下载，失败条目记录 `status: "failed"` 与 `error.kind`，整体退出码：全成功或部分成功退 0，全失败退 2

### 需求:断点续传保留

按 items 下载必须复用现有 `downloader.rs` 的断点续传逻辑：若目标路径文件已存在且大小与远端 `Content-Length` 一致，则跳过；若大小不一致则覆盖重下；若部分写入则继续从断点续传。

#### 场景:已存在完整文件跳过
- **当** 沙箱内目标路径已有同名文件且字节数与远端一致
- **那么** 系统不重新下载，记录 `status: "skipped_existing"`

#### 场景:部分写入续传
- **当** 沙箱内目标路径已有不完整文件
- **那么** 系统从已下载字节数处续传剩余内容，最终记录 `status: "downloaded"`

### 需求:并发下载

下载实现必须支持配置化的并发度，使用 `futures::stream::buffer_unordered(n)` 或等价机制对 media items 集合并发分发。系统禁止串行下载所有 items（即使在 n=1 时也走相同代码路径，仅 buffer 大小为 1）。

#### 场景:默认并发 4
- **当** 未显式传入 `--concurrency`
- **那么** 系统以并发 4 同时下载多个 item

#### 场景:并发上限保护
- **当** 用户传入 `--concurrency 16`
- **那么** 系统在任意时刻最多保持 16 个 in-flight HTTP 请求

### 需求:默认文件命名约定

`download_media` 在未提供 `filename_format` 时必须使用以下默认命名规则：

- `MediaItem.author_handle` 存在且非空 → 文件名 = `{author_handle}_{tweet_id}_{suggested_filename}`
- `author_handle` 缺失或为空字符串 → 文件名 = `{tweet_id}_{suggested_filename}`
- `suggested_filename` 缺失或为空 → 用 `media` 作为兜底，构成 `..._{tweet_id}_media`

提供 `filename_format`（字符串模板，占位符 `{USERNAME}` 与 `{ID}`）时：

- `{USERNAME}` 替换为 `author_handle`（缺失时替换为空串）
- `{ID}` 替换为 `tweet_id`
- 替换后所有空格转为下划线
- 最终文件名 = `{替换后的模板}_{suggested_filename}`

#### 场景:默认带 author_handle
- **当** `MediaItem { tweet_id: "1234", author_handle: Some("alice"), suggested_filename: "AAA.jpg", .. }` 且未传入 `filename_format`
- **那么** 落盘文件名必须为 `alice_1234_AAA.jpg`

#### 场景:默认无 author_handle 时回退
- **当** `MediaItem.author_handle` 为 `None` 或空字符串，未传 `filename_format`
- **那么** 落盘文件名必须为 `{tweet_id}_{suggested_filename}` 形式（无 author 前缀）

#### 场景:legacy 模板兼容
- **当** 传入 `filename_format = Some("{USERNAME} {ID}")`
- **那么** 落盘文件名 = `<author_handle>_<tweet_id>_<suggested_filename>`（与旧 `xld download` 的默认行为一致）

### 需求:文件 mtime 默认设置到推文发布时间

当 `DownloadOpts.set_mtime == true`（默认）且 `MediaItem.created_at` 提供且可解析时，`download_media` 必须在文件落盘后把文件的 modification time 设置到推文发布时间。无法解析或字段缺失时静默跳过，不报错。

`set_mtime == false` 时不设置 mtime（文件保留写入时间）。

#### 场景:mtime 默认设到推文时间
- **当** `MediaItem.created_at = "Thu Apr 06 15:24:15 +0000 2017"`，`set_mtime = true`
- **那么** 文件 mtime 必须为 unix 时间戳 1491492255（即 2017-04-06 15:24:15 UTC）

#### 场景:created_at 缺失静默跳过
- **当** `MediaItem.created_at = None`，`set_mtime = true`
- **那么** 文件 mtime 保留为写入时间，不报错

#### 场景:set_mtime 关闭
- **当** `set_mtime = false`，即使 `created_at` 提供
- **那么** 文件 mtime 保留为写入时间

### 需求:`DownloadOpts.base_dir` 注入

`DownloadOpts` 必须含可选字段 `base_dir: Option<PathBuf>`。base 来源解析顺序：

1. `opts.base_dir`（若 `Some`）
2. `config.download_sandbox_base_dir`（若已配置）
3. `sandbox::default_base_dir()`（平台默认）

无论 base 来自哪一层，sandbox jail 校验（拒 `..` / 绝对 subdir / 符号链接逃逸）必须照常执行。

#### 场景:opts.base_dir 优先于 config
- **当** 调用方显式传 `opts.base_dir = Some("/Users/foo/old_downloads")`，且 config 中也有沙箱 base
- **那么** 实际 base 必须为 `/Users/foo/old_downloads`

#### 场景:不绕过 sandbox 校验
- **当** opts.base_dir = Some(任意目录)，subdir = `Some("../escape")`
- **那么** 仍必须返回 `sandbox_violation` 错误

### 需求:lib 层函数 `download_media`

系统必须在 lib crate 中暴露 `download_media(items: &[MediaItem], opts: &DownloadOpts, sink: Arc<dyn ProgressSink>) -> Result<DownloadOutput>` 异步函数。`DownloadOpts` 必须含字段 `subdir: Option<String>`、`concurrency: u32`、`base_dir: Option<PathBuf>`、`filename_format: Option<String>`、`set_mtime: bool`。函数禁止直接写 stdout（stderr 进度事件由本能力下方"NDJSON 进度事件"需求约束）。函数实现内部必须调用 `sandbox` 模块完成路径解析与 jail 校验。

CLI 的 `--ids` 快捷方式由 main.rs 层在调用 `download_media` 前先调 `list_likes` 实现，禁止把 ID 解析逻辑下沉到 lib 函数。

#### 场景:lib 函数仅接受 items
- **当** 任何调用方调用 `download_media`
- **那么** 函数签名必须为 `(items, opts)`，禁止存在接受 ID 列表的重载

#### 场景:lib 函数路径走沙箱
- **当** 任何调用方调用 `download_media(items, DownloadOpts { subdir: Some(...), .. })`
- **那么** 实际写入路径必须由 sandbox 模块返回，且位于已配置 base dir 之内

### 需求:stderr NDJSON 进度事件流

当 `xld media download` 以 `--json` 模式运行时，stderr 必须按 newline-delimited JSON 形态输出进度事件，每行一个独立 JSON 对象。事件类型必须包含且仅包含以下种类：

- `{ "event": "download_started", "total": N, "concurrency": n }`：批次开始
- `{ "event": "item_started", "tweet_id": "...", "url": "...", "index": i, "total": N }`：单项开始
- `{ "event": "item_progress", "tweet_id": "...", "bytes_done": x, "bytes_total": y }`：单项进度（可选发送，建议每秒至多一次）
- `{ "event": "item_done", "tweet_id": "...", "status": "downloaded|skipped_existing|failed", "bytes": z }`：单项结束
- `{ "event": "download_finished", "summary": { total, downloaded, skipped, failed } }`：批次结束
- `{ "event": "diagnostic", "level": "warn|error", "message": "..." }`：诊断/错误（替代自由文本）

stderr 上**仅**输出上述事件。系统禁止在 stderr 输出 ANSI escape codes、`indicatif` 进度条字符或任何非 NDJSON 自由文本。

非 `--json` 模式（人类 CLI）下，stderr 行为不变（继续使用 `indicatif` 渲染人类可读进度条）。

#### 场景:--json 模式 stderr 仅 NDJSON
- **当** 运行 `xld media download --items @x.json --json 2> err.log`
- **那么** `err.log` 每一非空行必须可被 `serde_json::from_str` 单独解析为合法 JSON 对象

#### 场景:批次开始与结束事件必发
- **当** 任何 `--json` 调用产生 stderr 输出
- **那么** 必须存在恰好一个 `download_started` 事件作为首个事件，恰好一个 `download_finished` 事件作为末个事件

#### 场景:人类模式不受影响
- **当** 运行 `xld media download --items @x.json`（无 `--json`）
- **那么** stderr 输出 `indicatif` 进度条，行为与本变更前一致

### 需求:不暴露 organize 与旧 download 一把梭给 Agent

Skill 工具表暴露面禁止包含 `organize`（按用户名归档）与旧版 `xld download`（list+下载耦合）。`xld media download` 必须是 Agent 在 Skill 模式下唯一的下载入口。

#### 场景:Skill 工具表清单
- **当** 检视 `skill/SKILL.md` 中声明的 Agent 可调用工具集
- **那么** 该集合必须仅包含 `list_likes`、`download_media`、`auth_status`、`setup_from_curl`，禁止出现 `organize` 或旧 `download` 入口
