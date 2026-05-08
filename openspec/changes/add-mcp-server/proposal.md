## 为什么

v1 把 Agent 接口设计成"Skill markdown + spawn `xld <subcmd> --json` CLI 子进程 + stdout JSON envelope + stderr NDJSON 进度事件"。这条路径在 v1 上线时是务实选择（无须新依赖），但跟 2026 主流 Agent 生态错位：

- **OpenClaw 主流 active skill 已是 MCP server 的 wrapper**，而非 spawn-CLI；ClawHub 上社区 MCP server 已是事实标准
- **Claude Code / Hermes / Cursor 等客户端**对 MCP 是一等公民支持，对 spawn-CLI 形态是降级支持
- **MCP 协议原生**支持 progress notification、cancellation、长连接复用——这些 v1 都用 stderr NDJSON / fork 进程的变通方式实现，又重又脆

v2 改造为以 [`rmcp`](https://docs.rs/rmcp)（官方 Rust MCP SDK）为基础的本地 MCP server，让 Skill 包从"调 CLI 的 markdown"升级为"MCP server 的发现层 + 工具描述"。这是把已有 lib 层（`agent::list_likes` / `agent::download_media` / `agent::auth_status` / `agent::import_curl`，4 个 pure async fn）薄薄地包一层 rmcp 适配，**不改业务逻辑**。

## 变更内容

### 新增

- **新增** `xld serve --mcp` 子命令：rmcp-based MCP server，stdio transport，长驻直到 client 关闭
- **新增** `src/agent/mcp_server.rs` 模块：`#[tool]` 宏适配 4 个 lib 函数为 MCP 工具
- **新增** `ProgressSink` 第三实现 `McpProgressSink`：把 `ProgressEvent` 转换为 MCP `notifications/progress` 通知（替代 v1 的 stderr NDJSON）
- **新增** Cargo 依赖 `rmcp`（含 `server` / `macros` / `transport-io` features）
- **新增** `skill/mcp-config.json`（或同等 manifest）：让 client 知道如何启动 MCP server

### 修改

- **MODIFIED** Skill 包形态：`skill/SKILL.md` 工具调用约定从"spawn `xld <subcmd> --json` + stdout JSON envelope"改写为"通过 MCP 协议调用工具名 `list_likes` / `download_media` / `auth_status` / `setup_from_curl`"
- **MODIFIED** `skill/README.md` 第 5 步"在 Agent 里注册本 skill"：从"把 skill 目录路径加到 Claude Code skill 注册"改写为"在 `~/.claude/settings.json` 的 `mcpServers` 段加配置"
- **MODIFIED** v1 的 4 条能力 spec（`likes-listing-json` / `media-download-by-items` / `credential-self-check` / `curl-import-extended`）：删除"调用形态由 stdout JSON envelope + 退出码"那部分契约（这些保留为人类 CLI 的事实，但不再是 Agent 接口规范）；删除"stderr NDJSON 进度事件流"约束（被 MCP progress notification 替代）

### 移除

- **REMOVED** `media-download-by-items` 中"stderr NDJSON 进度事件流"需求：被 MCP progress notification 替代。**保留**人类模式下 `indicatif` 进度条行为不变

### 范围外（明确不做）

- HTTP / SSE transport（仅 stdio；远程部署本质不适合本项目"凭据本地化"原则）
- 新工具（`diff_since` / `search_likes` 等）：v2.1 视实际使用反馈再加
- 2026 新加的 MCP `Tasks` 异步原语：`download_media` 实测体感够快（4-8 MB 单文件秒级），暂不需要异步任务化
- **真实 cancellation 主动响应**：v2.0 收到 MCP `notifications/cancelled` 时仅记录到 stderr 诊断日志，**不**中断正在跑的工具、**不**清理半下载文件。客户端如需强制终止可关闭 stdin（走 EOF 优雅关闭路径）。理由：实现真实 cancellation 需要给 `download_media` 加可选参数（违反 D6 lib 签名稳定承诺）+ 半文件清理语义复杂度，与 v2.0"最薄 MCP 适配层"目标冲突；v2.1 视需求实施
- v1 的 `xld <subcmd> --json` 子命令**保留**：仍服务于人类用户 / shell 脚本 / CI；spec 层降级为"人类接口"，不再标注为"Agent 接口"
- v1 的 `OutputEnvelope` 类型与 stdout JSON 信封代码**保留**：仍服务人类 `--json` 模式（jq pipeline 等）。**删除**的仅有 `NdjsonStderrSink` 与 stderr NDJSON 协议（v2 后无用户）

### 用户面影响

- **Skill 装机用户**：安装步骤更简单——从"装 binary + 注册 skill 目录路径"变成"装 binary + 在 settings.json 加 5 行 `mcpServers` 配置"；体验更好（长连接 + 原生进度反馈）
- **现有人类 CLI 用户**：完全无感——`xld likes list --json` / `xld setup` / `xld media download` / `xld download` / `xld organize` 全部行为不变
- **开发方**：新增 `rmcp` 依赖（约 100KB compile-time，运行时复用 tokio），CI 构建时间预计 +5-10 秒

## 功能 (Capabilities)

### 新增功能

- `agent-mcp-server`: rmcp-based 本地 MCP server 的全部契约——`xld serve --mcp` 子命令、stdio transport、4 个工具的 schema、进度通知映射、错误模型映射、生命周期与凭据热加载策略

### 修改功能

- `agent-skill-package`: Skill 包形态从"spawn-CLI markdown"升级为"MCP server 发现层"；新增 mcp-config.json 文件；改写 SKILL.md 调用约定与 README.md 注册指引
- `media-download-by-items`: 移除"stderr NDJSON 进度事件流"需求（被 MCP progress notification 替代；人类模式 indicatif 行为不变）

### 不修改

- `likes-listing-json` / `credential-self-check` / `curl-import-extended`: 这些 spec 规定的是 CLI 子命令（`xld likes list` / `xld auth status` / `xld setup`）的人类接口契约——v2 后 CLI 子命令行为不变，spec 无需修订。MCP 工具是**新接口**（由 `agent-mcp-server` 能力规定），不取代这些 CLI 契约
- `download-sandbox`: 路径校验跟调用形态无关，spec 不动
- `username-alias`: 与本变更无关

## 影响

### 代码

- **新增** `src/agent/mcp_server.rs`（约 200-300 行）：rmcp adapter，含 4 个 `#[tool]` 标注的工具方法
- **新增** `McpProgressSink` 实现：约 50 行
- **修改** `src/main.rs`：新增 `Commands::Serve { protocol: McpProtocol }` 分支与 `run_serve_mcp()` 入口；约 +50 行
- **修改** `Cargo.toml`：新增 `rmcp = { version = "...", features = ["server", "macros", "transport-io"] }`，可能需要 `schemars` 派生 JSON schema
- **不动**：所有 `agent::list_likes` / `download_media` / `auth_status` / `import_curl` lib 函数；所有 v1 CLI 子命令实现

### 文档与 Skill 包

- **重写** `skill/SKILL.md` 调用约定章节（约 80 行变化）
- **修改** `skill/README.md` 第 4-6 步（MCP 注册指引）
- **新增** `skill/mcp-config.json` 或同等 manifest（约 10 行）
- **更新** 主仓 `README.md` 的"作为 Agent Skill 使用"章节，加入 MCP server 配置示例

### 发布

- 本变更最终随 2.0 版本号发布（与 MCP 这个里程碑级特性匹配；按 SemVer 是 minor 但 v1→v2 是项目演进的自然节点）
- 6 平台 release artifact 不变（rmcp 不引入跨平台编译麻烦）
- skill/ 包随 binary 同 tag 发布

### 风险

- **rmcp 版本演进风险**：rmcp 仍在 0.x（0.16.x / 0.8.x 两个版本线），API 可能有 breaking。缓解：选定一个稳定 minor 版本，pin 在 Cargo.toml；升级走单独 PR
- **MCP 协议本身演进风险**：2026 在加 Tasks 等新原语，未必影响 v2.0 但需关注。缓解：v2.0 仅用稳定的 tools/list、tools/call、notifications/progress、notifications/cancelled
- **OpenClaw skill manifest 标准未冻结**：MCP server config 在不同 client 里写法略有差异（Claude Code 用 `~/.claude/settings.json` mcpServers，OpenClaw 用 mcporter / .mcp.json）。缓解：skill/README.md 同时给两种平台示例
- **沉没成本认知**：v1 的 stderr NDJSON 协议代码 + spec + 测试约 200 行 + 15 项 test，在 v2 删除。**已学到的 ProgressSink trait 抽象迁移到 McpProgressSink 直接复用**，所以不算白干
