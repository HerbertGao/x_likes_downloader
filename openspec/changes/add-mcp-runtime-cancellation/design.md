## 上下文

v2.0 的 design D10 显式 defer 了真实 cancellation，给出三条理由：(1) `download_media` lib 函数签名要求保持稳定（D6 invariant）；(2) 半下载文件清理语义复杂；(3) 测试时间窗成本高。v2.1 的判断是这三条理由都站不住了：

1. **D6 invariant**：原意是"v2.0 不重构 lib"。v2.1 不再受 v2.0 期约束；且加 `Option<CancellationToken>` 字段是**扩展**，所有现有 caller `Default::default()` 仍能跑——这是 D6 字面意义下的破坏，但不是精神意义下的"重构"。
2. **半文件清理**：v2.1 选择了更优雅的设计——下载到 `.partial` + 成功后 atomic rename。Cancel 时不需要清理（`.partial` 自然就是 partial），idempotency 检查也不会误命中。**顺手修了 v2.0 的 `crash-leave-partial bug`**。
3. **测试成本**：tokio mock HTTP server（`hyper` 派生或 `mockito` 等）已成熟。

同时从用户体验观察到：

- v2.0 进度数值字段长时间停在 0（单 item 大文件场景下尤其明显），客户端 UI 渲染为"空进度条"；用户怀疑卡住了
- 大文件下载 cancel 失效（设计上）让用户不得不等带宽跑完
- 一次失败下载在最终路径留下损坏的文件，下次跑沉默 skip——**这是个 silent correctness bug**

利益相关者：

- **Agent 用户（MCP 客户端）**：cancel 立即生效；progress 流畅；下载错误半文件不会沉默 skip
- **人类 CLI 用户（`xld download`）**：行为不变（CLI 路径传 `cancel: None`）；但 `.partial` 命名约定也帮他们处理 Ctrl-C 留下的半文件场景，下次跑能续传
- **现有 lib API caller**：`DownloadOpts::default()` 保持完全等价；`DownloadStatus`/`DownloadSummary` 新字段可被忽略
- **未来贡献者**：cancellation 一旦生效，所有 in-flight 操作（HTTP request、文件 IO、futures）都需要 `select!` cancel future——给 lib 加新功能时要遵循这个 pattern

约束：

- 不破坏 v2.0 ✅ MCP server 的 4 个工具 schema
- 不破坏现有 `xld download`、`xld likes list` 等 human CLI 命令行为
- 不引入新进程、新 daemon、新文件系统约定（除 `.partial` 后缀）
- `.partial` 文件命名要安全（不与合法文件名冲突）

## 目标 / 非目标

**目标：**

- MCP `notifications/cancelled` 真实生效——in-flight `download_media` 工具调用立即停止 chunk 循环，关闭 `.partial` 文件 fd，不删
- HTTP Range 续传从 `.partial` 文件接续，验证 ETag 一致性
- MCP `progress` 数值含 in-flight byte fraction，客户端 UI 实时显示百分比变化
- v2.0 `crash-leave-partial bug` 修复：crash / cancel 后的半文件以 `.partial` 后缀保留，不会与 `idempotency check (skipped_existing)` 冲突
- `DownloadStatus::Cancelled` 让 Agent 区分"被 cancel 的 item"与"网络失败的 item"
- Cancel 后 `CallToolResult` 返回 `isError: false` + 完整 `DownloadOutput`（含 cancelled items），Agent 可继续处理
- ETag 失配自动重头下 + 一条 diagnostic message
- 全部行为有 `tests/cancellation_smoke.rs` 与 `tests/range_resume_smoke.rs` 覆盖

**非目标：**

- 不实施跨进程的 cancellation（`Ctrl-C` MCP client 后用另一个进程接续；当前 v2.1 仅同进程内）
- 不实施 chunk-level retry（HTTP error 在 chunk 中段失败仍走 v2.0 的 fail-item 路径，不重试当前 chunk）
- 不暴露 cancel API 给 human CLI（`xld download` 用户用 Ctrl-C 杀进程即可，他们不调用 lib）
- 不实施 `.partial` 清理 GC（用户/磁盘工具自管；v2.2 视需求加）
- 不修改 list_likes / auth_status / setup_from_curl 工具行为（它们的 round-trip 短，cancel 价值低）
- 不重构 `download_media` 主控制流（保持 `buffer_unordered` 并发模式，仅在 chunk 循环中 select! cancel）

## 决策

### D1：`DownloadOpts.cancel: Option<CancellationToken>` 字段（破 D6 字面意义）

**选择**：在 `DownloadOpts` 加 `cancel: Option<tokio_util::sync::CancellationToken>` 字段。`Default::default()` 设为 `None`。CLI 路径（`xld download` main.rs 调用）传 `None`；MCP 路径（`mcp_server.rs::download_media` handler）传 `Some(ctx.ct.clone())`，其中 `ctx: RequestContext<RoleServer>` 由 rmcp 1.6 提供，`ctx.ct` 在收到对应 request id 的 `CancelledNotification` 时由 rmcp 内部自动 cancel（详见 D6）。

**理由**：

- `Option<>` 兜底让所有 v1+ caller 零迁移
- `tokio_util` 的 `CancellationToken` 是 tokio 生态约定俗成的 cancellation 原语；`select!` 路径标准
- 把 token 持有放在 server 而非 lib：lib 仅是被动接收方；server 负责生命周期
- v2.0 D10 invariant "lib 不动"在 v2.1 已不再适用——v2.1 是允许 lib 演进的版本

**替代方案**：

- **全局 atomic flag**：被否决——多请求并发时无法精确取消单个请求
- **专门的 `CancellableDownloadOpts` 派生类型**：被否决——增加类型表面积，没有实际收益
- **用 `oneshot::Receiver` 传单次信号**：被否决——CancellationToken 支持级联（`child_token()`），未来扩展性更好

### D2：`.partial` 后缀 + atomic rename，顺手修 crash-leave-partial bug

**选择**：下载流写入 `<final_path>.partial`；下载成功 → `fs::rename(.partial, final_path)`（POSIX 原子）；失败 / cancel → 关闭文件 fd 并保留 `.partial`。idempotency 检查（`skipped_existing`）只看 `final_path`，不看 `.partial`（自然修 v2.0 沉默 skip 的 crash bug）。

**理由**：

- POSIX `rename` 原子（同一文件系统下），错误恢复时间窗 = 0
- v2.0 的 crash-leave-partial 是 silent correctness bug；这个设计自然修复
- `.partial` 后缀让用户在 sandbox 里能直观看到"哪些是半文件"，不需要 binary 自带 GC
- Range resume 的支持文件就是 `.partial`，命名一致
- 不需要专门的 cancel cleanup 路径——cancel 的 partial 跟 crash 的 partial 处理方式相同

**替代方案**：

- **下载到临时 dir 再 mv**：被否决——跨文件系统时不原子，需要额外 fallback；且不利于 Range resume 续传定位
- **下载到 `<final_path>` 直接，crash 后 callers 自己识别**：被否决——这就是 v2.0 的 silent bug，不修
- **后缀用 `.tmp.<random>`**：被否决——Range resume 找 partial 文件时需要稳定命名

### D3：HTTP Range 续传 + ETag 校验

**选择**：

```
download flow:
  1. partial_path = PathBuf::from(format!("{}.partial", final_path.display()))
     // 注：必须用字符串拼接，**不**能用 Path::with_extension("partial")——后者会把
     // alice_123_video.mp4 替换为 alice_123_video.partial（覆盖 .mp4 扩展名），
     // 与 spec.md 要求的 alice_123_video.mp4.partial（追加后缀）不一致
  2. partial_size = partial_path.metadata().map(|m| m.len()).unwrap_or(0)
  3. if partial_size > 0:
       a. HEAD request → 获取 server 当前 ETag、Content-Length
       b. saved_etag = first 8 bytes of partial_path xattr 或 sidecar metadata
            （实际：用户 home dir 下 ~/.cache/x_likes_downloader/etag-cache.json
              key = sha256(final_path)，value = etag string）
       c. if saved_etag != current_etag:
            emit progress message "tweet ID: ETag changed, restarting from scratch"
            fs::remove_file(partial_path)
            partial_size = 0
            (并删除 etag-cache 中的对应条目)
       d. else:
            GET with header Range: bytes=<partial_size>-
            if response is 206 Partial Content:
              open partial_path with O_APPEND
            elif response is 200 OK (server doesn't support Range):
              fs::remove_file(partial_path); 重新下；O_TRUNC
  4. otherwise (partial_size = 0):
       GET; create new partial_path with O_TRUNC; record ETag in cache
```

**理由**：

- ETag 是 HTTP 协议级别的"内容指纹"；X CDN 几乎都给（实测 100%）
- xattr 跨平台不一致（Windows、APFS、ext4 各行为）；用 sidecar JSON 文件简单且可移植
- ETag cache 在用户 cache dir（`~/Library/Caches/x_likes_downloader/` macOS、`~/.cache/x_likes_downloader/` Linux 等）；每条目 < 1KB
- 206 是续传成功；200 是 server 给了完整响应（说明它不支持 Range 或忽略了我们的请求），需要从头来；其他状态走错误路径
- ETag 失配 silent re-download：用户只关心拿到能播的文件，不关心是不是同一份；progress message 提供诊断

**替代方案**：

- **Last-Modified 替代 ETag**：被否决——精度低（秒级），CDN 改一次就更新但内容可能没变
- **xattr 存 ETag**：被否决——跨平台不一致
- **不做 ETag 校验，盲续传**：被否决——CDN 改文件后续传得到拼接坏文件
- **Content-Length 校验代替 ETag**：被否决——长度相同不代表内容相同

### D4：progress 数值化为 `items_done + Σ_in_flight byte fraction`

**选择**：

```rust
// McpProgressSink::dispatch 重写：
let in_flight: HashMap<TweetId, (u64 bytes_done, Option<u64> bytes_total)> = ...
let in_flight_fraction: f64 = in_flight.values()
    .map(|(done, total)| match (done, total) {
        (_, Some(t)) if *t > 0 => *done as f64 / *t as f64,
        (0, _) => 0.0,
        _ => 0.5,  // bytes_total unknown, 兜底中间值
    })
    .sum();
let progress = items_done as f64 + in_flight_fraction;
let total = total_items as f64;
```

range: [0, total_items]；老 v2.0 client 用 progress / total 渲染百分比仍正确，且 progress 数值现在连续。

**理由**：

- 量纲与 v2.0 一致（都是 [0, total_items]），不破老 client 解析
- 单调性：每个 in_flight item 的 (done/total) 单调（bytes_done 单调增、total 不变）；items_done 单调增；和单调
- bytes_total = None 兜底 0.5：既不是 0（看起来卡）也不是 1（看起来快好），UI 显示中间状态
- explore 阶段 D2 章节做过数学证明，这里就是它的实施

**替代方案**：

- **归一化到 `[0, 1]`**：被否决——破老 client 数值预期；丢失"还剩几个 item"信息
- **per-tweet sub-token**：被否决——MCP 协议不支持嵌套 token；客户端要自己维护多个进度条复杂
- **保持 v2.0 整数粒度**：被否决——大文件场景 UX 差，是这次升级的核心动机之一

### D5：`DownloadStatus::Cancelled` + `summary.cancelled` 字段

**选择**：

```rust
pub enum DownloadStatus {
    Downloaded,
    SkippedExisting,
    Failed,
    Cancelled,           // 新增
}

pub struct DownloadSummary {
    pub total: usize,
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub cancelled: usize,  // 新增
}
```

cancel 后的 `CallToolResult { isError: false, content: serialize(DownloadOutput) }`，Agent 收到完整 DownloadOutput 含 cancelled items 与 partial summary。

**理由**：

- Agent 能区分"用户主动取消"和"网络失败"——重试策略不同
- 沿用 v2.0 `Failed` 路径的设计（per-item 状态而非 toplevel error），保持一致
- isError: false 让 Agent 不走 generic error handler，直接拿 partial 结果
- 新字段都用 `#[serde(default)]` 兼容老 client

**替代方案**：

- **isError: true + 部分摘要**：被否决——客户端通常把 isError true 当 generic failure 处理，partial 价值丢失
- **不加 cancelled 状态，复用 Failed**：被否决——丢失语义信息，Agent 无法做"重试已 cancel 的，跳过 fail 的"决策
- **toplevel return shape 用新类型**：被否决——破坏现有 `download_media` API 一致性

### D6：直接使用 rmcp 1.6 内置 `RequestContext.ct`，不维护 in_flight map

**选择**：

```rust
// 注：tool handler 函数签名加 RequestContext<RoleServer> 参数
async fn download_media(
    &self,
    Parameters(req): Parameters<DownloadMediaRequest>,
    meta: Meta,
    peer: Peer<RoleServer>,
    ctx: RequestContext<RoleServer>,
) -> Result<CallToolResult, McpError> {
    let opts = DownloadOpts {
        subdir: req.subdir,
        concurrency: req.concurrency.unwrap_or(4),
        cancel: Some(ctx.ct.clone()),  // ctx.ct 由 rmcp 自动管理生命周期与 cancel 触发
        ..Default::default()
    };
    // ... 调用 lib download_media；select! 在 lib 内部检测 ctx.ct.cancelled()
}

impl ServerHandler for XldMcpServer {
    // on_cancelled 仅保留为诊断钩子；rmcp 已在调用本方法**之前**完成 ct.cancel() 路由
    async fn on_cancelled(&self, notification: CancelledNotificationParam, _ctx: NotificationContext<RoleServer>) {
        let reason = notification.reason.as_deref().unwrap_or("(no reason)");
        eprintln!(
            "xld serve --mcp: cancellation acknowledged for request {:?} (reason: {})",
            notification.request_id, reason
        );
    }
}
```

**理由（关键澄清）**：

- **rmcp 1.6 已内置完整 cancellation 路由**：参见 `rmcp-1.6.0/src/service.rs:987-991`——收到 `CancelledNotification` 时，rmcp 内部从 `local_ct_pool` 查到对应 request_id 的 token 并调用 `ct.cancel()`，**这一切在 `on_cancelled` 钩子被调用之前完成**
- `RequestContext<R>` 的 `ct: CancellationToken` 字段（`service.rs:655-657`）注释直接写明 *"this token will be cancelled when the CancelledNotification is received"*；`id: RequestId` 字段提供 request id（如需用于诊断）
- 本提案**不**需要 `in_flight: HashMap<RequestId, CancellationToken>` map——那是重复实现 rmcp 已经做好的事
- `on_cancelled` 钩子仅作诊断日志使用；不需要查 map、不需要主动 cancel
- `RequestId = NumberOrString`（`model.rs:295`），实现 `Hash + Eq`，但本提案不需要将其作为 map key

**替代方案**：

- **自维护 in_flight map（原 v0 design）**：被否决——重复实现 rmcp 1.6 内置功能；多 30+ 行 server 代码、一组 Mutex 锁、scopeguard 清理逻辑、race condition 测试全是不必要的复杂度
- **每个请求 spawn 一个 task，token 跟 task 绑定**：被否决——rmcp 已经管理 task lifecycle
- **用 `tokio::sync::watch` 代替 `CancellationToken`**：被否决——`RequestContext.ct` 已是 `tokio_util::sync::CancellationToken`

### D7：cancel 在 chunk 循环中通过 `tokio::select!` 检测

**选择**：

```rust
// download_media.rs 的下载循环：
loop {
    tokio::select! {
        biased;  // 优先检查 cancel
        _ = cancel_token.cancelled(), if cancel_token.is_some() => {
            sink.emit(ProgressEvent::ItemDone {
                tweet_id: ...,
                status: DownloadStatus::Cancelled,
                bytes: downloaded,
            });
            return Ok((status_cancelled, downloaded));
        }
        chunk_result = stream.next() => {
            match chunk_result {
                Some(Ok(chunk)) => { file.write_all(&chunk).await?; downloaded += chunk.len() as u64; }
                Some(Err(e)) => return Err(...),
                None => break,
            }
        }
    }
}
```

**理由**：

- `biased` 模式让 cancel 检查优先级最高，避免被高频 chunk 事件饿死
- per-item 检查；多并发时各 item 独立响应 cancel
- `if cancel_token.is_some()` 保证 v2.0 行为零变化（`cancel: None` 路径走原 `while let Some(chunk)` 等价语义）

**替代方案**：

- **`futures::stream::take_until`**：被否决——错过中段 cancel；只有外层 future 完成才反应
- **每 N chunks 检查一次 atomic flag**：被否决——延迟难调；select! 等价但更优雅

### D8：测试用 `wiremock` mock HTTP server 验证 Range 与 cancel

**选择**：在 `tests/range_resume_smoke.rs`、`tests/cancellation_smoke.rs` 用 `wiremock`（dev dependency）启 mock server，模拟：

- Range 206 响应正确续传
- Range 200 响应触发重新下
- ETag 失配触发 .partial 删除 + diagnostic message
- 慢响应（每 chunk 100ms）+ cancel 中段触发 → 文件应停在 partial size + DownloadStatus::Cancelled

**理由**：

- `wiremock` 是 Rust 生态主流 mock HTTP；对 streaming 响应支持好
- 不依赖 X CDN 真实端点（CI 跑得快、可重复）
- mock server 启动 ~50ms；测试套件总时间应 < 5s

**替代方案**：

- **写自己的 hyper-based mock**：被否决——维护成本高
- **`mockito`**：被否决——对 streaming chunk + 慢响应支持弱

### D9：ETag cache 文件位置与格式

**选择**：

```
位置（按平台）：
  macOS:   ~/Library/Caches/x_likes_downloader/etag-cache.json
  Linux:   ${XDG_CACHE_HOME:-~/.cache}/x_likes_downloader/etag-cache.json
  Windows: %LOCALAPPDATA%\x_likes_downloader\Cache\etag-cache.json

格式：
  {
    "version": 1,
    "entries": {
      "<sha256(final_path)>": {
        "etag": "\"abc123\"",
        "url": "https://video.twimg.com/...",
        "size": 1234567,
        "updated_at": "2026-05-09T13:00:00Z"
      },
      ...
    }
  }
```

并发访问用 `fs2::file_lock` 风格独占锁（写时锁）；读时无锁（先读再校验）。

**理由**：

- 平台缓存 dir 约定（`dirs` crate 已在依赖中）
- JSON 单文件简单；条目数量小（用户级别 ~ 100 条）
- sha256 路径作 key 避免特殊字符 / 空格 / 长路径问题
- 锁仅写时，读多写少不阻塞
- size + updated_at 字段为未来扩展（GC、validation）留口

**替代方案**：

- **每个 partial 文件配 sidecar `.partial.etag`**：被否决——文件数量翻倍；GC 复杂
- **xattr**：被否决——跨平台坑多
- **SQLite**：被否决——为 100 条键值对引入数据库 overkill

### D10：cancel 后 binary 行为与 CLI 一致性

**选择**：human CLI 路径（`xld download`、`xld media download` 等）调用 lib 时 `cancel: None`，行为与 v2.0 完全等价。但所有路径都受益于 `.partial + rename` —— Ctrl-C 杀人类 CLI 进程后，下次 `xld download` 跑也能 Range 续传。

**理由**：

- 不暴露 `--cancel-token` flag 给人类用户（无意义）
- `.partial` 行为下沉到 lib 层；CLI 用户透明受益
- Ctrl-C → SIGINT → tokio runtime drop → 文件 fd 关闭，半文件留 `.partial`，路径自洽

**替代方案**：

- **CLI 加 `--no-resume` flag**：被否决——v2.1 不需要；用户删 `.partial` 即等价
- **CLI 路径也注入 ctrl-c handler 调用 cancel**：被否决——增加 main.rs 复杂度，无明显收益（runtime drop 即等价）

### D11：sandbox path canonicalize 与 .partial 路径一致

**选择**：现有 sandbox jail 检查（`sandbox::resolve_subdir`）对 `final_path` 做 canonicalize。`.partial` 路径用同样的 base + suffix，**不**重新 canonicalize（path 已是 canonical 的 final_path 加 `.partial` 后缀）。

**理由**：

- `PathBuf::from(format!("{}.partial", final_path.display()))` 不引入符号链接逃逸（path 已 canonicalized；字符串拼接仅追加后缀，不解析 path 组件）
- 避免在每次 partial → final rename 时重复做 jail 校验

**替代方案**：

- **每次 rename 前 jail 校验**：被否决——重复工作

## 风险 / 权衡

| 风险 | 缓解 |
|---|---|
| `tokio_util` 0.7 与 tokio 1.x 兼容性窗口 | 已在 Rust ecosystem 稳定 ≥ 2 年；`features = ["rt"]` 仅引入 CancellationToken；行为约定俗成 |
| ETag cache 文件并发损坏（多个 binary 同时跑） | `fs2` 文件锁写时独占；读时无锁但用 `serde_json::from_reader` 容错（解析失败 → cache 重置而非 panic） |
| `.partial` 文件遗留磁盘空间无人清理 | v2.1 不实施 GC；README 添加"如何清理 partial 文件"指引（`find <sandbox> -name "*.partial"`）；v2.2 视需求加 `xld media gc` 子命令 |
| ETag cache miss 导致全量重新下（cache 文件被删 / 损坏） | 行为正确（safety > efficiency），但用户带宽多用；emit progress message 提示 "no ETag cache, re-downloading" |
| Range 206 响应但 server 实际给了不同 byte 范围（partial-mismatch） | 已升级为正式需求（参见 `media-download-by-items` spec 的 `Content-Range 校验` 小节与 `场景:Content-Range 校验失败，重头下`）：校验 `Content-Range` 头的 N/M/Total 三字段，任一不符即删 `.partial` 重新下 |
| 单元测试时间窗：mock server chunk 间 sleep 导致测试时间长 | 设计 chunk size = 1 KB，间隔 5-10ms，避免单测 > 1s；CI 总时间 < 30s |
| `on_cancelled` 在 download_media 还没 register token 前到达（race） | rmcp 1.6 内部 `local_ct_pool` 在 request 进入时即注册条目；race 由 rmcp 内部处理，应用层无需关心；server `on_cancelled` 钩子仅作诊断日志 |
| RequestId 类型在 rmcp 中可能不是 Hash + Eq | 检查 rmcp 1.6 model.rs；如必要包一层 NewType；测试覆盖 |
| 多个 in-flight item 中一个 cancel，其它继续 | per-item token 通过 `cancel_token.child_token()` 派生；root cancel 级联到 children；child fail/cancel 不影响 root |
| `progress > total` (浮点数边界 case 由 u64 → f64 损失) | progress dispatch 时 clamp 到 [0, total_items_f]；测试覆盖大数（u64::MAX）边界 |
| ETag cache 跨用户 / 跨 home 不可移植 | cache 是 host-local；不应跨机迁移；README 声明 |
| Range resume 在 X CDN 上失效 (CDN 拒绝 Range header) | 测试覆盖 200 response fallback 路径；sandboxed mock 充分覆盖；活体冒烟在 v2.1 实施期至少跑 3 个 ETag 不变重启 round |
| 已 download 完成的文件被外部程序删除后，下次运行误认为需要 download | 这跟 v2.0 行为相同；v2.1 不变；属于 "外部状态被改" 的合理 fall-through |

## Migration Plan

1. **Phase A（lib 改造）**：`DownloadOpts.cancel` 字段 + 下载循环 select! cancel + partial path 切换。本机 cargo test 通过现有测试套件（cancel: None 路径等价）
2. **Phase B（atomic rename + ETag cache）**：lib 加 partial → rename + ETag cache 读写；新增 `tests/partial_rename_smoke.rs`
3. **Phase C（HTTP Range resume）**：lib 加 Range request 路径；新增 `tests/range_resume_smoke.rs` 覆盖 206 / 200 / ETag mismatch 三条主路径
4. **Phase D（progress 数值化）**：`McpProgressSink` 加 in_flight map + byte fraction；新增 unit test 覆盖单调性、bytes_total=None 兜底、单 item / 多 item 场景
5. **Phase E（server token wiring）**：`download_media` handler 加 `ctx: RequestContext<RoleServer>` 参数，把 `Some(ctx.ct.clone())` 透传给 lib；`on_cancelled` 改为诊断日志钩子（不主动 cancel；rmcp 已自动处理）；**不**需要 in_flight map / scopeguard
6. **Phase F（DownloadStatus::Cancelled + summary.cancelled）**：types.rs 加新枚举 / 字段；mcp_server 序列化路径覆盖
7. **Phase G（cancellation smoke test）**：`tests/cancellation_smoke.rs` 启 mock server 慢响应，发 cancel notification，断言 partial size 不为 0 且 status=Cancelled
8. **Phase H（文档 + spec sync）**：README / SKILL.md / CHANGELOG.md 更新；packaging/skill/x_likes/SKILL.md 加 cancellation + partial 说明
9. **Phase I（活体冒烟）**：本机 binary 跑大文件下载（>20MB），测 cancel + Range resume；OpenClaw / Claude Code / Codex 各 host 跑一遍
10. **Phase J（PR + Codex review 循环）**：到 codex clear；merge；tag v2.1.0（与 Change B packaging 合并发，或单独发 v2.1.0 patch）

回滚策略：因为 lib 改动是扩展（`Option<>` 兜底），回滚 = `git revert`，老 caller 仍能跑。`tokio-util` 依赖与 `wiremock` dev-dependency 同步移除。`.partial` 文件如果用户已经在用，回滚后会被遗弃在 sandbox（不会被 v2.0 binary 识别），需要用户手工清理——这是回滚的代价，要在 release notes 说明。

## 已关闭决策（原 Open Questions）

- **D-OQ1（关闭）ETag cache GC**：v2.1 **不**实施 GC。`README.md` 添加一节"如何清理 ETag cache 与 partial 文件"，给出手工命令：`find ~/Library/Caches/x_likes_downloader -size +1M -delete`（macOS）/ `find ~/.cache/x_likes_downloader -size +1M -delete`（Linux）。v2.2 评估是否加 `xld media gc` 子命令；届时再确定阈值（倾向 1MB 或 10000 条目）。本提案不引入新的代码路径或子命令。
- **D-OQ2（关闭）cache.size 语义 = server 资源完整大小，cancel 路径不写 cache**：明确两个量不混用——
  - `cache.entries[key].size` = server 资源完整大小（HTTP `Content-Length` 头或 `Content-Range: bytes N-M/Total` 中的 `Total`），用于下次续传时 Content-Range 校验中与 server 返回的 Total 比较
  - 当前 partial 文件大小 = 直接从 `partial_path.metadata().len()` 读取，**不**进 cache
  - 写 cache 时机：**收到 server response headers 后立即写**（无论是首次 GET 还是续传 Range GET），写入 `{etag, url, size: Content-Length, updated_at: now}`，然后才进入 chunk 循环
  - cancel 路径：**不需要**额外 cache write——cache 在收到 headers 时已写完整条目；cancel 不改变 server Total
  - 这一约定排除了 Codex 原 Q2 提出的"cancel 时记 partial size 到 cache"方案（该方案会让 cache.size 在两处含义冲突）
- **D-OQ3（关闭）测试时间窗**：`tests/cancellation_smoke.rs` 用 chunk size = 1KB、chunk 间隔 50ms 模拟慢下载；cancel 触发后 future 应在 1s 内 resolve（断言用 `tokio::time::timeout(Duration::from_secs(1), ...)`）；CI macos 偶尔波动到 200ms 仍宽裕。`tests/range_resume_smoke.rs` 同此参数。timer-based 误判风险极低。
- **D-OQ4（关闭）Claude Code 端 cancel UI**：v2.1 **不**在 plugin 层做 cancel UI。cancel 触发完全依赖 Claude Code 客户端发送 MCP `notifications/cancelled`（用户 Ctrl-C 或客户端 UI 触发）；本 server 已真实生效响应该通知（D6）。Anthropic 改进客户端 UI 时本 server 自动透明继承。
