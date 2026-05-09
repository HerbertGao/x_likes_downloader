## 为什么

v2.0 通过 design D10 显式 defer 了真实 cancellation 实施，原因有三：(1) 给 `download_media` lib 函数新增 cancellation 参数破坏 D6 "v2 不重构 lib" 承诺；(2) 半下载文件清理语义复杂；(3) 测试时间窗成本高。v2.0 上线后实测 v2.0 download_media 在大文件场景（>20MB 视频）下，活体测试时已观察到无法及时取消的痛点：用户 Ctrl-C MCP client 时 binary 仍跑到完，浪费带宽与磁盘空间。同时活体冒烟还顺手暴露了一个 v2.0 沉默缺陷——下载到一半进程死掉，半文件留在最终路径，下次 `download_media` 看到文件存在就 `skipped_existing` 沉默返回错误的成功（`crash-leave-partial bug`）。MCP `progress` 数值字段在多 in-flight item 时停在 items_done 整数粒度，未利用已有的 `bytes_done/bytes_total` 信息——客户端 UI 长时间显示 0% 直到突然跳到 100%。这三个问题互相耦合：解决 cancellation 必须处理半文件，处理半文件就顺手能修 crash bug，进度数值化让 cancellation 时的 partial state 更可观察，因此一次性解决比分批好。

## 变更内容

- **修改 `download_media` lib 函数签名**：`DownloadOpts` 加 `cancel: Option<tokio_util::sync::CancellationToken>` 字段；`Option<>` 让所有现有 caller（CLI、测试、Agent）零迁移
- **`.partial` 后缀 + atomic rename**：下载流写入 `<final_path>.partial`，成功后 `fs::rename` 到最终路径；失败/取消保留 `.partial` 留待续传；idempotency 检查改为只看最终路径，自然地修复 v2.0 `crash-leave-partial bug`
- **HTTP Range 续传**：下载启动前检查 `.partial` 存在且 size > 0；HTTP HEAD 拿 ETag/Content-Length；GET 带 `Range: bytes=N-` 头打开 append 模式；响应 206 续传、响应 200 重新下、ETag 不匹配重新下（含一条 progress diagnostic message）
- **`McpProgressSink` progress 数值化**：`progress = items_done + Σ_in_flight (bytes_done / bytes_total)`；`bytes_total = None` 兜底贡献 0.5；保持 `[0, total_items]` 量纲与 v2.0 连续
- **新增 `DownloadStatus::Cancelled` 枚举值**：`DownloadOutput.summary` 新增 `cancelled: usize` 字段
- **`CallToolResult` cancel 返回语义**：被 cancel 时返回 `isError: false` + 完整 `DownloadOutput`（含 cancelled items），让 Agent 可继续处理
- **`on_cancelled` 真实生效**：从 v2.0 的"仅 stderr 日志"升级为真实中断进行中的 download。实施利用 rmcp 1.6 内置 cancel 路由——`download_media` tool handler 接收 `RequestContext<RoleServer>` 参数，把 `Some(ctx.ct.clone())` 传给 lib；rmcp 在收到 `notifications/cancelled` 时自动 cancel `ctx.ct`（`rmcp-1.6.0/src/service.rs:987-991`），无需 server 自维护 in_flight map
- **`ETag` mismatch 处理**：续传时 ETag 与首次下载存的不一致，删除 `.partial` 重头下；emit 一条 progress message `"tweet ID: ETag changed, restarting"`；不返回错误
- **测试补全**：tokio JoinHandle 协作测 cancellation 在 chunk 循环中生效；mock HTTP server 测 Range 206/200 路径与 ETag 校验；smoke test 补 `mcp_server_cancellation_actually_cancels` 断言
- **不破坏的内容**：4 个 MCP 工具的 schema、`xld serve --mcp` 子命令、stdio transport、所有 host adapter packaging 结构、cargo build 流程、binary CLI 行为（人类用户的 `xld download` 不感知）

## 功能 (Capabilities)

### 修改功能

- `agent-mcp-server`: progress 字段语义增强（添加 in-flight byte fraction 累加规则）；cancellation 行为升级（v2.0 ignore → v2.1 真实生效）；CallToolResult cancel 返回 shape 明确化
- `media-download-by-items`: lib 函数 `download_media` 签名扩展（`DownloadOpts.cancel` 字段）；下载文件原子性约定（`.partial` + rename）；HTTP Range 续传约定；`DownloadStatus` 增加 `Cancelled` 状态；`DownloadSummary` 增加 `cancelled` 计数字段

## 影响

- **代码**：`src/agent/{download_media,types,mcp_server}.rs`、`src/main.rs`（CLI 路径接 cancel = None）、`tests/mcp_smoke.rs`、新增 `tests/cancellation_smoke.rs` 与 `tests/range_resume_smoke.rs`
- **依赖**：
  - **运行时新增**：`tokio-util = { version = "0.7", features = ["rt"] }`（显式声明本 crate 直接使用 `CancellationToken` 类型；rmcp 1.6 已传递依赖）；`fs2 = "0.4"`（ETag cache 文件锁）；`sha2 = "0.10"`（ETag cache key 生成）
  - **dev-only 新增**：`wiremock = "0.6"`（mock HTTP server，用于 `tests/cancellation_smoke.rs` 与 `tests/range_resume_smoke.rs`）
  - 其他运行时 crate 不变
- **API**：`DownloadOpts` 新增字段（向后兼容，`Default::default()` 保持现有行为）；`DownloadStatus` 新增枚举值（serde rename_all = snake_case 兼容客户端字符串解析）；`DownloadSummary` 新增字段（向后兼容，`#[serde(default)]`）
- **MCP 协议层**：`progress` 数值含义升级为含 byte fraction（数值范围仍 `[0, total_items]`，老 client 字段断言不破）；`notifications/cancelled` 行为从 ignore 升级为真实生效
- **配置**：无变化
- **CI**：新增测试套件 `cargo test --test cancellation_smoke` 和 `cargo test --test range_resume_smoke`；matrix 上需保证 mock HTTP server 可用
- **文档**：`README.md` v2.1 release notes 段说明 cancellation 真实生效与 Range resume；MCP server 章节更新 progress 数值化语义；archive 中的 design D10 标注为已 supersede
- **packaging**：所有 host adapter 不需要改（功能在 binary 层）；`packaging/skill/x_likes/SKILL.md` 加一段说明 cancellation 现真实生效与 partial file 保留行为
- **不影响**：cargo install / GHA release.yml / 6 平台 binary 构建链；`xld serve --mcp` 子命令名 / args；4 个 MCP 工具 schema；human CLI 命令（`xld download`、`xld likes list` 等）行为
