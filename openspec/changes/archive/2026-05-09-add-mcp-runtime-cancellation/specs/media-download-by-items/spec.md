## MODIFIED Requirements

### 需求:lib 层函数 `download_media`

系统必须在 lib crate 中暴露 `download_media(items: &[MediaItem], opts: &DownloadOpts, sink: Arc<dyn ProgressSink>) -> Result<DownloadOutput>` 异步函数。`DownloadOpts` 必须含字段 `subdir: Option<String>`、`concurrency: u32`、`base_dir: Option<PathBuf>`、`filename_format: Option<String>`、`set_mtime: bool`、**`cancel: Option<tokio_util::sync::CancellationToken>`（v2.1 新增）**。函数禁止直接写 stdout（进度事件由本能力下方 progress 事件需求约束）。函数实现内部必须调用 `sandbox` 模块完成路径解析与 jail 校验。

**`cancel` 字段语义**：

- 取值 `None`：函数行为与 v2.0 完全等价（不响应任何外部 cancel 信号）；CLI 路径 `xld download` 必须传 `None`；现有所有 caller 通过 `DownloadOpts::default()` 自动取得 `None`
- 取值 `Some(token)`：函数主下载循环 / chunk 循环必须使用 `tokio::select!` 检测 `token.cancelled()`，cancel 触发后立即停止当前 in-flight item 的 chunk 接收（`.partial` 文件保留），将其状态标记为 `DownloadStatus::Cancelled`；尚未启动的 item 也标记为 `cancelled`；返回的 `DownloadOutput` 含完整 N 项（不抛 `Err`）

CLI 的 `--ids` 快捷方式由 main.rs 层在调用 `download_media` 前先调 `list_likes` 实现，禁止把 ID 解析逻辑下沉到 lib 函数。

#### 场景:lib 函数仅接受 items
- **当** 任何调用方调用 `download_media`
- **那么** 函数签名必须为 `(items, opts, sink)`，禁止存在接受 ID 列表的重载

#### 场景:lib 函数路径走沙箱
- **当** 任何调用方调用 `download_media(items, DownloadOpts { subdir: Some(...), .. })`
- **那么** 实际写入路径必须由 sandbox 模块返回，且位于已配置 base dir 之内

#### 场景:cancel 字段缺省时行为不变
- **当** caller 调用 `download_media(items, &DownloadOpts::default(), sink)`（即 `cancel: None`）
- **那么** 函数行为必须与 v2.0 完全等价；不轮询任何 cancellation 信号；现有测试（不含 cancel 测试）必须无需修改即通过

#### 场景:cancel token cancel 时 in-flight item 立即停止
- **当** caller 用 `cancel: Some(token)` 调用 `download_media`，下载到 in-flight item 的 30% bytes 时调 `token.cancel()`
- **那么** 该 item 必须在 1 秒内停止 chunk 接收循环；其 `.partial` 文件必须保留（不删）；其状态必须标记为 `DownloadStatus::Cancelled`；尚未开始的 item 必须不启动新的 HTTP request；返回的 `DownloadOutput` 含完整 item 数（不抛 Err）

#### 场景:cancel 不影响已完成 item
- **当** `download_media` 已完成 K 个 item 后被 cancel
- **那么** 已完成的 K 个 item 状态保持原值（`downloaded` / `skipped_existing` / `failed`），其文件已通过 atomic rename 落到最终路径；剩余 N-K 个 item 状态为 `cancelled`

### 需求:断点续传保留

按 items 下载必须实施 `.partial` 文件 + atomic rename 协议（v2.1 加强；v2.0 直接写 final_path 的实现已废弃）：

- **下载流写入**：所有下载流写入 `<final_path>.partial`，**不**直接写 `final_path`
- **成功完成**：HTTP 流读完后，`tokio::fs::rename(<final_path>.partial, <final_path>)`（POSIX 原子；同文件系统）；rename 后写入文件 mtime（如 opts.set_mtime）
- **失败 / cancel**：保留 `.partial` 文件不删；下次启动 `download_media` 同 item 时通过 HTTP Range 续传
- **idempotency 检查**：仅看 `final_path`，**不**看 `.partial`；存在且非空 → `skipped_existing`；不存在 → 走下载路径（其中可能因 `.partial` 存在而走续传）
- **HTTP Range 续传**：下载启动前检查 `<final_path>.partial` 是否存在且 size > 0
  - 是：先 HTTP HEAD 拿当前 ETag；与 ETag cache（位置见下）中已记录的对比
    - 一致：GET 带 `Range: bytes=<partial_size>-`；server 响应 206 Partial Content → 校验 `Content-Range` 头（详见下方"Content-Range 校验"小节）；通过则打开 `.partial` 文件 with O_APPEND 续写；server 响应 200 OK → 删 `.partial` 重头下并 emit progress message `"tweet <id> server doesn't support Range, restarting"`；其它响应走错误路径
    - 不一致：删 `.partial`、删 ETag cache 中对应条目、重头下；emit progress message `"tweet <id> ETag changed, restarting from scratch"`
    - **ETag cache 无对应条目**（`.partial` 存在但 cache 中找不到该 final_path 的 sha256 entry，例如 cache 文件被删除或损坏后重置）：必须视为无法安全续传，删除 `.partial`、重头下载；emit progress message `"tweet <id> no ETag baseline, restarting from scratch"`；这一路径与"ETag 不一致"行为等价
  - 否：常规新下载；记录返回的 ETag 到 cache

- **Content-Range 校验**：当 server 对 `Range: bytes=N-` 请求返回 206 Partial Content 时，必须解析响应的 `Content-Range` 头（形如 `bytes N-M/Total`）并校验：
  - 起始字节必须等于请求的 `partial_size`（即 N）；不等则 server 给的不是我们要的范围
  - 结束字节 + 1 必须等于 `Total`（即 server 必须把剩余全部字节作为续传响应的一部分）
  - `Total` 必须等于 ETag cache 中记录的 `size` 字段（如有）；不等说明 server 端 size 已变，等价于 ETag mismatch
  - 任一校验失败 → 删 `.partial`、删 ETag cache 对应条目、重头下；emit progress message `"tweet <id> Content-Range mismatch, restarting from scratch"`；不返回错误
- **ETag cache 位置**（按平台，由 `dirs` crate 解析）：
  - macOS: `~/Library/Caches/x_likes_downloader/etag-cache.json`
  - Linux: `${XDG_CACHE_HOME:-~/.cache}/x_likes_downloader/etag-cache.json`
  - Windows: `%LOCALAPPDATA%\x_likes_downloader\Cache\etag-cache.json`
- **ETag cache 格式**：`{ "version": 1, "entries": { "<sha256(final_path)>": { "etag": "...", "url": "...", "size": N, "updated_at": "ISO-8601" } } }`
- **`size` 字段语义**：`size` 是 server 资源完整大小（HTTP `Content-Length` 头或 `Content-Range: bytes N-M/Total` 中的 `Total`），用于下次续传时 Content-Range 校验。**不**是当前 partial 文件大小；当前 partial 大小直接从 `partial_path.metadata().len()` 读取
- **并发安全**：写入 cache 文件前必须用 `fs2::file_lock`（独占锁）；读时无锁但用容错解析（解析失败 → cache 重置而非 panic）
- **cache write 时机**：收到 server response headers 后**立即**写 cache（写入 `{etag, url, size: Content-Length, updated_at: now}`），然后才进入 chunk 循环。cancel / 失败路径**不**需要额外 cache write——cache 在 headers 阶段已写完整条目；cancel 不改变 server Total

**v2.0 兼容性说明**：v2.0 的"`final_path` 文件存在 + size 与 Content-Length 一致 → skipped_existing"语义在 v2.1 仍然适用——只是文件**只可能**通过 atomic rename 才能存在于 `final_path`；任何 v2.0 时代留下的损坏的 `final_path` 文件会被 v2.1 误判为 skipped_existing（因为 v2.1 不再 verify 文件 size 与 Content-Length 一致——这是 v2.0 的逻辑被本需求接管）。用户从 v2.0 升 v2.1 时如发现错误的 skipped_existing，可手工删除 sandbox 中可疑文件触发重新下。

#### 场景:已存在完整文件跳过
- **当** 沙箱内 `final_path` 已有同名文件（v2.1 下意味着前次下载成功 atomic rename）
- **那么** 系统不重新下载，记录 `status: "skipped_existing"`

#### 场景:partial 文件触发 Range 续传
- **当** 沙箱内 `<final_path>.partial` 存在且 size > 0，ETag cache 中记录的 ETag 与当前 server HEAD 返回的一致
- **那么** 系统发起 GET with `Range: bytes=<partial_size>-` 请求；如收到 206 响应，打开 `.partial` with O_APPEND 续写剩余字节；下载成功后 atomic rename 到 final_path；最终 status: "downloaded"

#### 场景:server 不支持 Range，重头下
- **当** 系统对 partial 文件发 Range 请求，但 server 响应 200 OK（完整响应而非 206 Partial）
- **那么** 系统必须删 `.partial` 文件，重新下载，emit progress message 含 `"server doesn't support Range, restarting"` 字样

#### 场景:ETag 失配，重头下
- **当** 沙箱内 `<final_path>.partial` 存在，HTTP HEAD 返回的 ETag 与 cache 中记录的不一致
- **那么** 系统必须删 `.partial`、删 cache 对应条目、重新下载；progress sink 必须 emit 一条 message 含 `"tweet <id> ETag changed, restarting from scratch"` 字样

#### 场景:ETag cache 无对应条目，重头下
- **当** 沙箱内 `<final_path>.partial` 存在，但 ETag cache 中找不到对应 sha256 key 的 entry（cache 文件被删 / 损坏后重置 / 该 item 从未被 v2.1+ 下载过）
- **那么** 系统必须删 `.partial`、重新下载；progress sink 必须 emit 一条 message 含 `"tweet <id> no ETag baseline, restarting from scratch"` 字样；行为与"ETag 不一致"等价

#### 场景:Content-Range 校验失败，重头下
- **当** server 对 `Range: bytes=N-` 请求返回 206 Partial Content，但响应的 `Content-Range` 头与请求范围不匹配（起始字节 != N，或结束字节 + 1 != Total，或 Total != cache 中记录的 size）
- **那么** 系统必须删 `.partial`、删 cache 对应条目、重新下载；progress sink 必须 emit 一条 message 含 `"tweet <id> Content-Range mismatch, restarting from scratch"` 字样；不返回错误

#### 场景:atomic rename 完成下载
- **当** 单个 item 下载完成（HTTP body 读尽）
- **那么** 系统必须先关闭 `.partial` 文件 fd，再 `fs::rename` 到 `final_path`；rename 必须是 POSIX 原子操作（即不存在 `final_path` 含部分内容的瞬间）；`.partial` 文件在 rename 后必须不存在

#### 场景:crash 后 .partial 不被沉默 skip
- **当** 上一次 `download_media` 进程在写入 `.partial` 中段被 SIGKILL 终止；下次 `download_media` 跑同 item
- **那么** 系统必须不报告 `skipped_existing`（因为 `final_path` 不存在）；必须走 Range 续传路径或重头下；最终 status 为 `downloaded`

#### 场景:ETag cache 文件损坏时不 panic
- **当** ETag cache 文件存在但 JSON 解析失败（如格式损坏）
- **那么** 系统必须将其视为 cache 缺失（从空 cache 起），所有 item 走全量重新下载路径；不抛错或 panic；可在 stderr 输出诊断信息

### 需求:JSON 输出信封——下载结果

`xld media download --json` 的成功信封 `data` 部分必须包含字段 `downloads[]`，每个元素至少包含 `tweet_id`、`url`、`path`（落盘绝对路径）、`bytes`、`status`（值为 `downloaded` / `skipped_existing` / `failed` / **`cancelled`** ）；失败条目必须额外含 `error.kind` 与 `error.message`；cancelled 条目无 `error` 字段。`data.summary` 必须包含 `total`、`downloaded`、`skipped`、`failed`、**`cancelled`** 五项计数（v2.1 加 `cancelled`，对应 `DownloadStatus::Cancelled` 状态）。

**向后兼容性**：

- `DownloadStatus` 用 `serde(rename_all = "snake_case")`，新增 `Cancelled` 序列化为 `"cancelled"`；老 client 解析未知字符串值时 fallback 行为由 client 自行决定，但服务端不刻意降级
- `DownloadSummary.cancelled` 字段在反序列化时用 `#[serde(default)]`，老 client 的旧 JSON 可被新代码解析（缺失字段 = 0）；新 client 的新 JSON 不会破坏老 client 解析（额外字段被 serde 忽略）

#### 场景:逐项结果可观测
- **当** 下载 3 个 media item，其中 1 个已存在
- **那么** `data.downloads[]` 长度为 3，包含 1 个 `status: "skipped_existing"` 条目；`data.summary` 中 `skipped` 等于 1

#### 场景:部分失败不中断其它下载
- **当** 多个 item 中某个 URL 返回 404
- **那么** 系统继续完成其它下载，失败条目记录 `status: "failed"` 与 `error.kind`，整体退出码：全成功或部分成功退 0，全失败退 2

#### 场景:cancelled item 在 downloads[] 中可观察
- **当** MCP 路径下 `download_media` 被 cancel 中断时已完成 1 个、1 个 in-flight 中断、1 个未启动
- **那么** `data.downloads[]` 长度 = 3；含 1 个 `status: "downloaded"`、2 个 `status: "cancelled"`；`data.summary` 中 `total: 3, downloaded: 1, skipped: 0, failed: 0, cancelled: 2`

#### 场景:summary.cancelled 字段必须存在（即使为 0）
- **当** `download_media` 正常完成，无任何 cancel 发生
- **那么** `data.summary` 必须含 `cancelled: 0`（不是省略字段）；老 client 不依赖此字段；新 client 可总是访问

## ADDED Requirements

### 需求:.partial 文件命名约定

`download_media` 在写入下载流时必须使用 `<final_path>.partial` 命名约定，其中：

- `final_path` 是 sandbox 模块已 canonicalize 的合法路径（不含 symlink 逃逸）
- `.partial` 是字面后缀（不是占位符）；最终文件名为 `<final_path>.partial`
- `<final_path>.partial` 必须不与任何合法的 X 媒体文件名冲突（X 媒体文件不会以 `.partial` 结尾）
- sandbox jail 校验对 `<final_path>.partial` 路径自然成立（因为它只是 final_path + 后缀，base dir 限定继承）

`.partial` 文件不进入 idempotency 检查（即 `skipped_existing` 路径只看 `final_path`，看不到 `.partial`）。`.partial` 文件不暴露给 `DownloadResult.path` 字段——该字段始终是 `final_path`，无论实际文件状态如何。

`.partial` 文件无 GC 机制（v2.1）；用户可手工 `find <sandbox> -name "*.partial"` 清理；v2.2 视需求加 `xld media gc` 子命令。

#### 场景:partial 后缀字面
- **当** 一个 item 的 final_path 是 `/sandbox/alice_123_video.mp4`，下载启动
- **那么** 实际写入文件路径必须是 `/sandbox/alice_123_video.mp4.partial`；不得是 `/sandbox/.alice_123_video.mp4.swp` 或其它命名

#### 场景:partial 不破 sandbox jail
- **当** sandbox 解析的 final_path 已通过 jail 校验，对应的 `.partial` 路径
- **那么** `.partial` 路径必须仍在同一 sandbox base dir 内；`.partial` 不需要单独的 jail 校验

#### 场景:idempotency 仅看 final_path
- **当** 一次先前的下载在 `.partial` 阶段被中断；下次 `download_media` 跑同 item
- **那么** 系统不报告 `skipped_existing`；必须继续走下载路径（Range 续传或重头下，依 ETag）

#### 场景:DownloadResult.path 始终是 final_path
- **当** 一个 item 因 cancel 而停在 `.partial` 阶段
- **那么** `DownloadResult.path` 字段必须等于 `final_path`（无 `.partial` 后缀），即使该路径下当前文件不存在；这让 caller 不需要根据 status 字段决定 path 后缀

### 需求:DownloadStatus::Cancelled 状态

`DownloadStatus` 枚举必须含变体 `Cancelled`，serde 序列化为字符串 `"cancelled"`。该状态仅在 `download_media` 接收到 `DownloadOpts.cancel = Some(token)` 且 token 被 cancel 时产生：

- 被 cancel 中断的 in-flight item 的 status 为 `Cancelled`；其 `.partial` 文件保留以便下次续传
- 尚未启动的 item（在 buffer_unordered 队列中等待）的 status 也为 `Cancelled`；它们没有 `.partial` 文件
- 已完成 item（status = `Downloaded` / `SkippedExisting` / `Failed`）不受 cancel 影响，状态不被覆盖

`DownloadResult { status: Cancelled, error: None }` —— cancelled 不携带 error 字段；caller 区分 cancelled 与 failed 以决定重试策略。

`DownloadSummary` 必须含 `cancelled: usize` 字段，反映 status 为 `Cancelled` 的 item 数量；`total = downloaded + skipped + failed + cancelled`。

#### 场景:cancelled status 序列化
- **当** 一个 item 被 cancel；序列化其 DownloadResult 为 JSON
- **那么** 必须含 `"status": "cancelled"`；不得含 `error` 字段（或 error 为 null）

#### 场景:summary 计数包含 cancelled
- **当** 一次下载有 1 个 downloaded、2 个 cancelled
- **那么** `DownloadSummary { total: 3, downloaded: 1, skipped: 0, failed: 0, cancelled: 2 }`；`total == downloaded + skipped + failed + cancelled` 必须成立

#### 场景:cancelled 不触发 error
- **当** caller 检查 cancelled item 的 `DownloadResult.error`
- **那么** error 必须为 `None`；cancelled 不视为 error

#### 场景:已完成 item 不被 cancel 覆盖
- **当** 5 个 item 中前 2 个已 downloaded 或 skipped 后再 cancel
- **那么** 前 2 个 item 的 status 保持原值（`downloaded` / `skipped_existing`），不被改写为 `cancelled`；后 3 个 item 的 status 为 `cancelled`
