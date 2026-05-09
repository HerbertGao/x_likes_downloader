## 1. 依赖添加

- [x] 1.1 在 `Cargo.toml` 添加 `tokio-util = { version = "0.7", features = ["rt"] }` 依赖（注：rmcp 1.6 已传递依赖了 tokio-util，本条主要是显式声明本 crate 直接使用 `CancellationToken` 类型）
- [x] 1.2 在 `Cargo.toml` 添加 `wiremock = "0.6"` dev-dependency（mock HTTP server）
- [x] 1.3 在 `Cargo.toml` 添加 `fs2 = "0.4"` 依赖（ETag cache 文件锁）
- [x] 1.4 在 `Cargo.toml` 添加 `sha2 = "0.10"` 依赖（ETag cache key 生成）
- [x] 1.5 跑 `cargo build --release` 验证依赖解析无冲突；rustls-webpki 等版本未被影响

> 注：原 v0 design 列出的 `scopeguard` 依赖在 D6 简化后**不再需要**——in_flight map 已删除，不需要 defer 清理

## 2. types.rs 数据结构扩展

- [x] 2.1 给 `DownloadOpts` 添加 `cancel: Option<tokio_util::sync::CancellationToken>` 字段
- [x] 2.2 `DownloadOpts::default()` 设置 `cancel: None`
- [x] 2.3 `DownloadStatus` 枚举添加 `Cancelled` 变体（带 `#[serde(rename = "cancelled")]` 或继承 rename_all = snake_case）
- [x] 2.4 `DownloadSummary` 添加 `cancelled: usize` 字段，serde 配 `#[serde(default)]`
- [x] 2.5 跑 `cargo check --lib` 验证无编译错误；现有 caller 通过 `DownloadOpts::default()` 自动 cancel: None

## 3. .partial + atomic rename (Phase B)

- [x] 3.1 在 `src/agent/download_media.rs` 中找到 `download_single_item`（或类似单 item 下载函数），把下载流目标改为 `PathBuf::from(format!("{}.partial", final_path.display()))`（字符串拼接追加 `.partial` 后缀；**不要**用 `Path::with_extension("partial")`，后者会替换扩展名而非追加，导致 `alice.mp4` → `alice.partial` 与 spec 要求的 `alice.mp4.partial` 不符）
- [x] 3.2 下载流读尽后，在写入 mtime 之前调用 `tokio::fs::rename(partial_path, final_path)`，rename 失败直接返回 `DownloadStatus::Failed` + error。**不**实现 copy+remove fallback——`.partial` 与 `final_path` 同目录（spec 强制 `<final_path>.partial` 字面后缀），不存在跨文件系统场景；且 copy+remove 非原子，违反 spec.md "rename 必须是 POSIX 原子操作（不存在 final_path 含部分内容的瞬间）"约束
- [x] 3.3 idempotency 检查（`skipped_existing` 判定）只看 final_path 是否存在，不看 .partial
- [x] 3.4 `DownloadResult.path` 字段始终设为 final_path（即使下载失败）
- [x] 3.5 单元测试：模拟 final_path 存在 → skipped；存在 .partial 但无 final_path → 走下载路径（不 skip）（含 `partial_alone_does_not_trigger_skipped_existing` 集成测试）

## 4. ETag cache + Range 续传 (Phase C)

- [x] 4.1 创建 `src/agent/etag_cache.rs` 模块：定义 `EtagCache` 结构、`{load, save, get, set, remove}` 方法
- [x] 4.2 `EtagCache::path()` 用 `dirs::cache_dir()` 构造平台相关路径；目录不存在时创建
- [x] 4.3 `EtagCache::load` 容错解析失败 → 返回空 cache + stderr 诊断；`save` 用 `fs2::FileExt::lock_exclusive` 独占锁
- [x] 4.4 ETag cache 条目 key = `sha256(final_path)` 的 hex 字符串
- [x] 4.5 在 download_single_item 中加 partial 文件大小检查；> 0 时发 HEAD 请求拿当前 server ETag
- [x] 4.6 与 cache 对比 ETag：
  - 一致 → GET with `Range: bytes=N-`
  - 不一致 → 删 `.partial`、删 cache 条目、emit progress message `"tweet <id> ETag changed, restarting from scratch"`、重头下
  - **cache 无对应条目** → 删 `.partial`、emit progress message `"tweet <id> no ETag baseline, restarting from scratch"`、重头下
- [x] 4.7 处理 server response：206 校验 Content-Range；200 删 partial 重头下；其它走错误路径
- [x] 4.7.1 Content-Range 校验：解析响应头 `Content-Range: bytes N-M/Total`；校验失败 → 删 `.partial`、删 cache 条目、emit progress message、重头下
- [x] 4.8 cache write 时机：**收到 server response headers 后立即**写 cache，写入 `{etag, url, size: server Content-Length 或 Range Total, updated_at: now}`
- [x] 4.8.1 cancel / 失败路径**不**需要额外 cache write
- [x] 4.8.2 单元测试：cache 写入路径覆盖（`fresh_download_writes_to_partial_then_renames` 验证 size = Content-Length）
- [x] 4.9 单元测试 etag_cache 模块：load 不存在文件、load 损坏文件、save 并发安全（含 `concurrent_save_does_not_lose_entries`）
- [x] 4.10 集成测试 `tests/range_resume_smoke.rs`：覆盖 206 / 200 / ETag mismatch / ETag cache 无条目 / Content-Range mismatch 五条主路径

## 5. cancel token 在 chunk 循环生效 (Phase A 核心)

- [x] 5.1 在 `download_single_item` 接收 `cancel: Option<&CancellationToken>` 参数（或通过 opts 传递）
- [x] 5.2 chunk 循环中用 `tokio::select! { biased; _ = cancel.cancelled(), if cancel.is_some() => ...; chunk = stream.next() => ... }`
- [x] 5.3 cancel 触发时：关闭文件 fd（drop）；不删 .partial；**不**写 cache；emit `ProgressEvent::ItemDone { status: Cancelled }`；返回 `Ok(DownloadResult { status: Cancelled })`
- [x] 5.4 `download_media` 主调度循环也响应 cancel：cancel 后队列中尚未启动的 item 标 cancelled（早返回路径在 `buffer_unordered` 内 future 顶部检查 `cancel.is_cancelled()`）
- [x] 5.5 已完成 item 不被覆盖（前 K 个 Downloaded/Skipped/Failed 保留状态）
- [x] 5.6 集成测试：`cancellation_returns_within_1s_with_partial_preserved` 用 wiremock 慢响应 + 200ms cancel 触发，验证 future 在 2s 内 resolve；`cancel_already_completed_item_keeps_status` 验证已完成 item 状态不被改

## 6. McpProgressSink progress 数值化 (Phase D)

- [x] 6.1 在 `McpProgressSink` 添加 `in_flight: Mutex<HashMap<String, (u64, Option<u64>)>>` 字段
- [x] 6.2 `ItemStarted` emit 时 `in_flight.insert(tweet_id, (0, None))`
- [x] 6.3 `ItemProgress` emit 时 `in_flight.insert((bytes_done, bytes_total))`
- [x] 6.4 `ItemDone` emit 时 `in_flight.remove(tweet_id)` 后再 `items_done += 1`
- [x] 6.5 `dispatch` 计算 progress：`progress = items_done as f64 + sum_of_in_flight_fractions`
- [x] 6.6 dispatch 时 progress clamp 到 `[0.0, total_items as f64]`
- [x] 6.7 ETag 失配时通过 sink emit 一条 progress message（实施为 `ProgressEvent::ItemRestart`，dispatch 不变 progress 值）
- [ ] 6.8 单元测试：mock sink，单 item / 多 item / 浮点单调性 / bytes_total=None 兜底 / clamp 边界（基础逻辑由 `compute_in_flight_fraction` 单元覆盖；端到端单调由 cancellation_smoke / range_resume_smoke 间接覆盖。完整专项 unit test 推迟到后续 PR review 反馈时补）
- [x] 6.9 现有 mcp_smoke.rs 的 progress 单调断言改为浮点单调（mcp_smoke 不直接断言 progress 序列；新增 `mcp_server_handles_cancellation_for_unknown_request` 测试覆盖 cancel notification 路由）
- [x] 6.10 区分 `DownloadFinished` 与 `BatchCancelled` 事件：正常结束 emit `DownloadFinished`；cancel 路径 emit `BatchCancelled`；两者互斥

## 7. server tool handler 接入 rmcp 内置 cancel token (Phase E)

> **设计澄清**：rmcp 1.6 已在 `service.rs:987-991` 内置完整 cancel 路由——收到 `CancelledNotification` 时自动 cancel 对应 `RequestContext.ct`。本提案**不**自维护 `in_flight` map；只需把 `ctx.ct` 通过 `DownloadOpts.cancel` 透传给 lib 即可。

- [x] 7.1 修改 `download_media` tool handler 函数签名，加入 `ctx: RequestContext<RoleServer>` 参数
- [x] 7.2 在 handler 内构造 `DownloadOpts` 时填入 `cancel: Some(ctx.ct.clone())`
- [x] 7.3 修改 `on_cancelled` handler：改为 `"xld serve --mcp: cancellation acknowledged for request <id> (reason: <reason>)"`；仅作诊断日志
- [x] 7.4 集成测试覆盖：`mcp_server_handles_cancellation_for_unknown_request` 验证 unknown request id 的 cancel notification 不导致 server 崩溃；真正的 mid-flight cancel 由 lib-level `cancellation_smoke.rs` 覆盖（透传路径 = trivial）
- [x] 7.5 **不需要**维护 `XldMcpServer.in_flight` map；**不需要** `scopeguard::defer!` 清理；**不需要** `RequestId` 作为 map key 的 Hash/Eq 处理

## 8. CallToolResult cancelled return shape

- [x] 8.1 `download_media` MCP handler 收到 `Ok(DownloadOutput)`（含 cancelled items）后，序列化为 JSON，构造 `CallToolResult { isError: false, content }`
- [x] 8.2 不区分 lib 是否被 cancel——cancel 仍走"成功"路径；"全失败"逻辑修改为 `cancelled` 不视作失败
- [x] 8.3 单元测试：cancellation_smoke 间接覆盖 cancel 路径返回 isError: false（`download_media` 返回 Ok→success_call_result→isError: false）

## 9. 整合 + smoke 测试 (Phase G)

- [x] 9.1 创建 `tests/cancellation_smoke.rs`：本实现采用 lib 层直接调用 `download_media` + wiremock 慢响应 + 200ms cancel；不通过 binary spawn（更稳定、更快），但等价覆盖 ctx.ct 透传后的全部行为
- [x] 9.2 断言：CallToolResult 在 cancel 后 2s 内返回；isError: false（lib Ok 路径）；downloads[] 含至少 1 个 cancelled status；partial 文件如存在则 size <= server total；ETag cache `size` == server Content-Length（如已写入）
- [x] 9.3 断言：lib 路径不直接产生 stderr "cancelled"，但 binary 上的 `on_cancelled` 钩子会写——通过 `mcp_server_handles_cancellation_for_unknown_request` 间接覆盖
- [x] 9.4 创建 `tests/range_resume_smoke.rs`：覆盖 Range 206 续传路径
- [x] 9.5 测试 `tests/range_resume_smoke.rs`：覆盖 200 fallback 路径
- [x] 9.6 测试 `tests/range_resume_smoke.rs`：覆盖 ETag mismatch + ETag cache 无对应条目 + Content-Range mismatch 三条路径
- [x] 9.7 现有 `tests/mcp_smoke.rs` 的 cancellation_ignored 测试更名为 `mcp_server_handles_cancellation_for_unknown_request`，断言收到 unknown id cancel 后 server 仍存活
- [x] 9.8 全套 cargo test --release 跑通：lib 测试（含新 unit）+ mcp_smoke + cancellation_smoke + range_resume_smoke（132 passed）

## 10. 文档更新与 spec 同步

- [x] 10.1 README.md 加 v2.1 release notes 段：cancellation 真实生效、Range resume、progress 数值化
- [x] 10.2 README.md MCP server 章节更新 progress 数值含义说明
- [x] 10.3 `packaging/skill/x_likes/SKILL.md` 加一段说明 cancellation 在 v2.1 真实生效，partial 文件保留行为，Agent 决策建议
- [ ] 10.4 在 design D10 archive `add-mcp-server` design.md 中标注 supersede（不修改 archived 文件，但在新 design.md 引用时说明——本变更 design.md 已在"Context"段说明 D10 supersede）
- [x] 10.5 验证 packaging 层无需改动（Cargo.toml v2.1.0 已存在；marketplace 不需要变更）
- [ ] 10.6 更新 CHANGELOG.md（项目无 CHANGELOG.md；release notes 在 README.md v2.1 段已记录）

## 11. 活体冒烟与回归 (Phase I)

- [x] 11.1 本机活体测：X 真实 15.5MB ext_tw_video 测 cancel 中段触发——cancel 55ms 内返回，partial 800KB+ 保留，progress 单调（0 violations）
- [x] 11.2 本机活体测：cancel 后再次 download_media，Range 续传真实生效（v2.1.x Last-Modified fallback 让 X CDN 场景从"重头下"转为真续传，went_through_restart_path: false）
- [x] 11.3 本机活体测：手改 cache 模拟 fingerprint mismatch 验证重头下 + "Last-Modified changed, restarting from scratch" diagnostic
- [ ] 11.4 本机活体测：3 个 host（OpenClaw / Claude Code / Codex）各跑一遍（Claude Code 已隐式覆盖；Codex / OpenClaw UI 抽查可作为 v2.1.0 release 后的软性验收，不阻塞归档）
- [x] 11.5 跑 cargo clippy --release --all-targets -- -D warnings 通过
- [x] 11.6 跑 cargo test --release（全套 139 passed）通过

## 12. PR + Codex review 循环 (Phase J)

- [x] 12.1 提交 PR #6（runtime cancellation）+ PR #7（Last-Modified fallback hot fix）
- [x] 12.2 跑 codex review 14 轮 + cursor bugbot 4 轮，全部 clear
- [x] 12.3 PR 描述列出 cancellation 真实生效、.partial+rename、HTTP Range 续传（含 Last-Modified fallback）、progress 数值化、新增 4 项依赖
- [ ] 12.4 merge 后 tag v2.1.0 + push 触发 GHA release.yml（归档完成后立即执行）
- [x] 12.5 归档此变更（本步骤）
