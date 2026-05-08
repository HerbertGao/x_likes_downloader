## 1. Phase A — 依赖与子命令骨架

- [x] 1.1 `Cargo.toml` 新增依赖：`rmcp`（pin 到具体 minor 版本，含 `server` / `macros` / `transport-io` features）、`schemars`（用于派生 JSON Schema）。**不**加 `tokio-util`（cancellation 推迟到 v2.1，参见 D10）
- [x] 1.2 `cargo build` 验证依赖安装无冲突；记录 binary size 基线（v1 vs v2 增量）
- [x] 1.3 修改 `src/main.rs` 的 `Commands` enum，新增 `Serve { #[arg(long)] mcp: bool }` 子命令
- [x] 1.4 实现 `run_serve_mcp()` 占位入口（仅打印 "MCP server starting..." 到 stderr 并阻塞读 stdin），验证 clap 路由通顺
- [x] 1.5 验证 `xld serve --help` 输出含 `--mcp` flag；`xld serve --mcp --http x` 被 clap 拒绝
- [ ] 1.6 提交 commit："chore: 引入 rmcp 依赖与 serve 子命令骨架" *(留给用户)*

## 2. Phase B — MCP server 模块 + tools/list

- [x] 2.1 新建 `src/agent/mcp_server.rs`，定义 `pub struct XldMcpServer`，实现 `rmcp::ServerHandler` trait
- [x] 2.2 在 `XldMcpServer` 上用 `#[tool(tool_box)]` 标注，给 4 个工具占位空实现（返回 unimplemented 文本）
- [x] 2.3 给 `agent::types` 中相关 struct（`ListOpts` / `MediaItem` / `DownloadOpts` 子集）加 `#[derive(JsonSchema)]`，验证 schemars 派生通过
- [x] 2.4 在 `run_serve_mcp()` 中创建 `XldMcpServer` 实例并 `.serve(stdio_transport()).await`
- [x] 2.5 安装 [`mcp-inspector`](https://github.com/modelcontextprotocol/inspector) 或类似 client 工具；连接 `xld serve --mcp` 并发送 `tools/list`，验证 4 个工具返回 + schema 正确（手动一次性活体测试）
- [x] 2.6 单元测试：构造 mock `tools/list` 请求，断言响应 JSON 含 4 个工具名
- [x] 2.7 **CI 集成测试**：写一个 `tests/mcp_smoke.rs`（集成测试），用 `tokio::process::Command` spawn `xld serve --mcp` 子进程 + 喂入手写 JSON-RPC `initialize` 与 `tools/list` 请求 + 解析 stdout 响应；断言响应含 4 个工具及其 schema。**纯 stdio，无外部依赖（mcp-inspector 不需要）**，能进 GitHub Actions
- [ ] 2.8 提交 commit："feat(mcp): tools/list 暴露 4 个工具骨架 + CI smoke test"

## 3. Phase C — 4 个工具的实际实现

- [x] 3.1 实现 `list_likes` 工具：从 `arguments` 反序列化为 `ListOpts`，调用 `agent::list_likes(opts).await`，结果序列化为 `CallToolResult::content_text(json)`
- [x] 3.2 实现 `auth_status` 工具：调用 `agent::auth_status().await`，按 `AuthStatus` 枚举映射到 isError success / error 信封
- [x] 3.3 实现 `setup_from_curl` 工具：从 `arguments` 取 `curl_text` 字符串，调用 `agent::import_curl(text)`；明确**不回显**输入文本与凭据
- [x] 3.4 实现 `download_media` 工具（不含 progress / cancellation）：从 `arguments` 反序列化 `items[]` 与 `opts`，调用 `agent::download_media(items, opts, NullSink).await`
- [x] 3.5 错误模型映射：所有 `ErrorPayload` 转 `CallToolResult { isError: true, content: [{type:"text", text: <error JSON>}] }`；真协议错误（unknown tool、bad args）走 JSON-RPC error response
- [x] 3.6 单元测试：每个工具一个 mock 测试（serde round-trip、错误映射、空 args 处理）
- [ ] 3.7 活体冒烟：用 mcp-inspector 调每个工具一次（list_likes count=2、auth_status、setup_from_curl 用本机已有 cURL、download_media 下 1 个 item），确认结果与 v1 CLI 路径一致
- [ ] 3.8 提交 commit："feat(mcp): 4 个工具实现完成"

## 4. Phase D — Progress notification（仅）

- [x] 4.1 在 `agent::types` 新增 `McpProgressSink` struct 实现既有 `ProgressSink` trait；持有 rmcp 的 progress notification sender 引用
- [x] 4.2 在 `download_media` 工具入口判断请求 `_meta.progressToken`：有则注入 `McpProgressSink`，无则注入 `NullSink`（保持 lib 函数签名不变，无 cancellation 参数）
- [x] 4.3 单元测试：构造 collector sink，跑 mock download，断言 progress 事件序列正确（started → progress* → finished）
- [x] 4.4 实现 `notifications/cancelled` 处理：收到通知时仅在 stderr 输出诊断日志（"received cancellation, ignoring (v2.0)"）+ 该 request id；**不**中断工具、**不**清理半文件
- [x] 4.5 单元测试：mock 发送 cancellation 通知，断言（a）stderr 含诊断行、（b）工具仍返回正常 CallToolResult
- [ ] 4.6 活体测试：用 mcp-inspector 跑 `download_media` 3 个 item，中途发 cancellation，确认下载继续完成 + stderr 有诊断
- [ ] 4.7 提交 commit："feat(mcp): progress notification 支持（cancellation 推迟到 v2.1）"

## 5. Phase E — 删除 v1 stderr NDJSON

- [x] 5.1 删除 `agent::types::ProgressEvent` 中专为 NDJSON 设计的 variant（如 `Diagnostic`，如有）
- [x] 5.2 删除 `agent::types::NdjsonStderrSink` struct 与相应 `impl ProgressSink`
- [x] 5.3 修改 `src/main.rs::run_media_download`：当 `--json` 标志启用时，stderr 不再注入 NdjsonStderrSink；改为注入 NullSink（人类模式仍 IndicatifSink）
- [x] 5.4 删除 `src/agent/download_media.rs::tests` 中针对 NDJSON 的测试（如 `progress_emits_ndjson` 等）
- [x] 5.5 删除 `openspec/specs/media-download-by-items/spec.md` 中 "stderr NDJSON 进度事件流" 整个需求块（移到 archive 之 v2 spec 中已通过 REMOVED）
- [x] 5.6 验证：跑 `xld media download --items @x.json --json`，stderr 应只有诊断文本（如有），无 NDJSON 事件
- [ ] 5.7 提交 commit："refactor: 移除 v1 stderr NDJSON 进度协议（被 MCP progress 替代）"

## 6. Phase F — Skill 包改造

- [x] 6.1 重写 `skill/SKILL.md` 调用约定章节：删除 spawn-CLI / stdout JSON envelope / stderr NDJSON 表述；新增 MCP `tools/call` 协议描述、isError 错误模型、progressToken 进度协议
- [x] 6.2 修改 `skill/SKILL.md` 的 `download_media` 分批使用建议章节：把"stderr 进度"改为"MCP progress notification"
- [x] 6.3 重写 `skill/README.md` 第 5 步：从"在 OpenClaw / Claude Code 注册 skill 目录"改为"配置 mcpServers 段"，给出 Claude Code 与 OpenClaw 两种平台示例
- [x] 6.4 新建 `skill/mcp-config.json`，含 `command`/`args`/`transport`/`minimum_xld_version` 四字段
- [x] 6.5 更新 `scripts/check-skill-defaults.sh`：把校验逻辑扩展到 `mcp-config.json`（字段集封闭、无敏感字段）
- [x] 6.6 更新 `.github/workflows/reusable-quality-checks.yml`：CI step 校验 mcp-config.json
- [ ] 6.7 提交 commit："feat(skill): 升级为 MCP server 发现层"

## 7. Phase G — 主仓 README 与文档

- [x] 7.1 修改根 `README.md` 的"作为 Agent Skill 使用"章节：替换 v1 描述为 v2 MCP 形态；保留人类 CLI 用法描述不变
- [x] 7.2 加入 MCP server 配置示例（Claude Code `mcpServers` 段）
- [x] 7.3 README 顶部"功能特性"列表：把 "Agent Skill 模式" 改为 "Agent MCP server 模式"或更新表述
- [ ] 7.4 提交 commit："docs: 更新 README 介绍 MCP server 集成"

## 8. Phase H — 跨平台 + 活体冒烟

- [x] 8.1 macOS 本机跑 `cargo build --release`，binary 大小相比 v1 增量记录
- [ ] 8.2 用 mcp-inspector 走完整端到端：连接 → tools/list → list_likes → download_media（含 progress + 中途 cancellation）→ auth_status → setup_from_curl
- [ ] 8.3 验证 `xld likes list --json` 等所有 v1 CLI 子命令仍按原 spec 工作（无回归）
- [ ] 8.4 GitHub Actions 触发 6 平台 cross-compile，确认 rmcp 在 Windows / Linux ARM 等平台编译通过
- [ ] 8.5 在 Linux 容器跑一次 MCP server smoke（用 mock cURL 文件先 setup，再用 mcp-inspector）
- [ ] 8.6 在 Windows 至少一次冒烟（手动启 `xld serve --mcp` + 用 Claude Code 客户端连接）

## 9. Phase I — 收尾 + 版本号

- [ ] 9.1 自检 `proposal.md` / `design.md` / `specs/` 与最终实现是否仍一致；如实现中调整了决策（特别是 D5 schemars 的兼容性、D7 progress 字段映射），回填到 design.md
- [x] 9.2 运行 `openspec-cn validate add-mcp-server` 通过
- [x] 9.3 运行 `scripts/version.sh 2.0.0` bump 版本号到 2.0.0
- [ ] 9.4 准备 PR 描述：含改动总览、6 份 spec 影响、breaking change 说明（v1 stderr NDJSON 移除）、测试矩阵
- [ ] 9.5 提交 commit："chore: bump version to 2.0.0" + 创建 git tag `v2.0.0` *(留给用户)*

## 10. Phase J — Release 后归档

- [ ] 10.1 等 master 合入并 release v2.0.0
- [ ] 10.2 运行 `/opsx:archive add-mcp-server`，把 spec 同步进项目长期 baseline，把变更目录移到 `openspec/changes/archive/YYYY-MM-DD-add-mcp-server/`
