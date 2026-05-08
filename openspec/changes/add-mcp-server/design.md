## 上下文

v1 的 Agent 集成形态（Skill markdown + spawn `xld <subcmd> --json` + stdout JSON envelope + stderr NDJSON）刚刚通过 16 轮 Codex review 闭环并 merge 到 master，活体冒烟全过。但 v1 这套设计跟 2026 主流 Agent 生态错位：OpenClaw 65% 的 active skill 已是 MCP server wrapper，Claude Code / Hermes 把 MCP 作为一等公民。继续维护"自造 stdout JSON envelope + stderr NDJSON"协议的边际价值低，且 MCP 协议原生支持的 progress / cancellation / 长连接复用是 v1 拼凑实现的。

利益相关者：
- **现有 v1 Skill 装机用户**（少量）：用了"装 skill 目录 + spawn CLI"的早期接入；v2 后他们的客户端（OpenClaw / Claude Code）会自动迁移到 MCP server 注册形态
- **未来 Agent 集成用户**（多数）：体验更好——配置一次 `mcpServers` 就行，无需理解 spawn-CLI / JSON envelope 等概念
- **现有人类 CLI 用户**：完全无感——`xld likes list --json | jq` 等用法不变
- **开发方**：维护一份 lib + 一份 MCP adapter，比维护 lib + JSON envelope + NDJSON 协议简单

约束：

- 凭据完全本地化（spec D3 不可破坏）→ MCP 必须 stdio transport，不接受 HTTP
- v1 已发布的 1.0.x release 不能 break——CLI 子命令行为必须不变
- 现有 `agent::list_likes` / `download_media` / `auth_status` / `import_curl` lib API 已稳定（spec 规定），v2 改造**不动 lib 函数签名**

## 目标 / 非目标

**目标：**

1. 把 4 个 lib 函数（list_likes / download_media / auth_status / import_curl）薄薄地包成 4 个 MCP 工具，stdio transport 暴露
2. 把 v1 `ProgressSink` 抽象的第三个实现 `McpProgressSink` 接到 MCP `notifications/progress` 上，让进度反馈走 MCP 原生通道
3. 把 Skill 包从"spawn-CLI 协议层"升级为"MCP server 发现层 + 工具描述"
4. 保留 v1 所有 CLI 子命令行为（向后兼容人类 / shell 脚本）
5. 整个变更不引入新平台、不破坏 6 平台 release artifact 构建

**非目标：**

1. 不实施 HTTP / SSE transport（凭据本地化原则）
2. 不实施 MCP `Tasks` 异步原语（download_media 实测体感够快）
3. 不增加新工具（diff_since / search_likes 等）——v2.1 视实际反馈再加
4. 不重构 lib 层——`agent::*` 4 个函数签名不变
5. 不删除 v1 的 stdout JSON envelope 实现（保留供人类用户 `--json` 模式）
6. 不为 v2 单独发版——跟 2.0 release 一起发布

## 决策

### D1：选 `rmcp` 而非自实现 MCP 协议

**选择**：使用 [`modelcontextprotocol/rust-sdk`](https://github.com/modelcontextprotocol/rust-sdk) 的 `rmcp` crate，启用 `server` / `macros` / `transport-io` features。

**理由**：

- 官方 SDK，4.7M+ crates.io 下载，活跃维护
- `#[tool]` 宏自动生成 JSON-RPC 派发代码 + JSON Schema（基于 schemars 派生 from Rust types）
- Tokio 异步原生集成，跟项目现有运行时无缝
- 自实现 JSON-RPC 2.0 + tools/list + tools/call + notifications 协议层成本极高（数千行）且没价值

**替代方案**：

- 自实现 JSON-RPC：被否决——重复造轮子，且 MCP 协议在演进，跟不上
- 用 mcp-rs 等社区 SDK：被否决——非官方，未来兼容性弱
- 选定一个特定 minor 版本（如 `rmcp = "0.16"` 或 `"0.8"` 视实施时最新）pin 在 Cargo.toml；升级走单独 PR

### D2：单 binary 子命令（`xld serve --mcp`）而非独立 binary

**选择**：在现有 `x_likes_downloader` binary 加 `Commands::Serve { protocol: McpProtocol }` 分支，调用 `run_serve_mcp()` 异步入口。

**理由**：

- 一份 release artifact，6 平台 cross-compile 不变
- 跟现有 lib 同 crate，零边界
- rmcp 引入的 binary size 增量可接受（~1-2 MB）
- 符合 D2 的"binary 是平台标准目录里的可执行文件"分发模型，跟 v1 完全兼容

**替代方案**：

- 独立 `xld-mcp-server` binary：被否决——维护成本翻倍、Cargo workspace 引入复杂度、客户端配置要写两个不同 command
- Cargo workspace 拆 lib + bin + mcp-server：过度工程，单 crate 双 target 已足够

### D3：仅 stdio transport，不实施 HTTP

**选择**：`xld serve --mcp` 仅支持 stdio transport（child process spawned by MCP client，stdin/stdout pipe 通信）。

**理由**：

- 凭据本地化是项目核心设计原则（spec 既有 D3）。HTTP transport 只在"中央部署 + 多用户共享"场景有价值，跟本项目"用户本机工具 + 自己 cookies"不匹配
- HTTP transport 对开发方意味着要解决多租户 / 鉴权 / 配额 / 部署运维——这是另一种产品形态，v2.0 不做
- stdio 是 MCP 主流（Claude Code / Hermes 默认形态），覆盖度足够

**替代方案**：

- stdio + HTTP 双支持：被否决——v2.0 加 HTTP 引入认证 / 部署设计的副作用
- 仅 HTTP：被否决——本质上跟项目原则冲突

### D4：4 个工具 1:1 映射 v1 lib 函数

**选择**：MCP 工具表跟 v1 工具表完全一致：

| MCP tool | 底层 lib 函数 | 参数 |
|---|---|---|
| `list_likes` | `agent::list_likes(opts)` | `all?`, `since_cursor?`, `count?`, `include_raw?` |
| `download_media` | `agent::download_media(items, opts, sink)` | `items[]`, `subdir?`, `concurrency?` (默认 4，钳位 [1,16]) |
| `auth_status` | `agent::auth_status()` | （无参数） |
| `setup_from_curl` | `agent::import_curl(text)` | `curl_text` |

**理由**：

- v1 spec 已稳，活体冒烟通过；改 schema 等于让 v1/v2 不一致徒增维护成本
- 新工具（`diff_since` / `search_likes` 等）在 v2.0 加是过度发明——等 Agent 实战反馈
- `setup_from_curl` 工具是新加的——v1 Skill 实际只描述了它的存在但调用形态是"用户跑 `xld setup`"。v2 让 Agent 直接以 cURL 文本调用变得更顺

**替代方案**：

- 借机加 `diff_since(cursor)` 等工具：被否决——`list_likes` 的 `since_cursor` 参数已覆盖增量场景
- 借机做 `search_likes(query)`：被否决——服务端做语义搜索引入 LLM 调用 / index 结构等复杂度，超出范围

### D5：MCP 工具参数 schema 用 `schemars` 自动派生

**选择**：在 `agent::types` 现有 struct 上加 `#[derive(JsonSchema)]`（来自 `schemars` crate），rmcp 的 `#[tool]` 宏会自动用它生成工具的 JSON Schema 给客户端 `tools/list` 看。

**理由**：

- 避免维护"Rust 类型定义 + JSON Schema 手写"的同步漂移
- `schemars` 是 Rust 生态主流（被 cargo / git2 / 等广泛用），跟 rmcp 兼容性好
- 给客户端的 schema 准确——Agent 调用工具前能看到字段、类型、必填、说明
- 不动 v1 lib 函数签名（D6 约束）

**替代方案**：

- 手写 JSON Schema：被否决——枯燥且容易跟 Rust 类型漂移
- 自实现 schema 派生：完全没必要

### D6：`agent::*` lib 函数签名不变

**选择**：v2 适配层（`mcp_server.rs`）调用现有 4 个函数完全不动。`McpProgressSink` 实现现有 `ProgressSink` trait，不改 trait。

**理由**：

- 已通过 16 轮 codex review + 活体冒烟的 lib API 应当被视为稳定 baseline
- 改 lib 函数签名等于让 v1 CLI 也跟着改——跟 v1 向后兼容承诺冲突

**替代方案**：

- 借机重构 lib（如 `download_media` 加 cancellation token）：被否决——v2 仅做适配，重构留给独立 change

### D7：`McpProgressSink` 把 v1 `ProgressEvent` 转换为 MCP progress notification

**选择**：

```rust
// 伪代码
struct McpProgressSink {
    progress_token: rmcp::ProgressToken,
    sender: rmcp::NotificationSender,
}

impl ProgressSink for McpProgressSink {
    fn emit(&self, event: ProgressEvent) {
        match event {
            ProgressEvent::DownloadStarted { total, .. } => self.sender.notify_progress(0.0, Some(format!("starting {} items", total))),
            ProgressEvent::ItemProgress { tweet_id, bytes_done, bytes_total, .. } => {
                let pct = bytes_total.map(|t| bytes_done as f32 / t as f32);
                self.sender.notify_progress(pct.unwrap_or(0.0), Some(tweet_id))
            }
            ProgressEvent::DownloadFinished { summary } => self.sender.notify_progress(1.0, Some(format!("done: {}/{}", summary.downloaded, summary.total))),
            // 其余事件略
        }
    }
}
```

具体 API 待 rmcp 0.x 的 progress sender 形态确认。

**理由**：

- v1 的 ProgressSink trait 抽象就是为这个时刻设计的——v2 不改抽象，加新实现
- 不需要再发 NDJSON 文本——MCP client 收到结构化 notification 直接渲染
- 单元测试：注入 `VecSink` collector 仍可工作（v1 的测试模式不变）

### D8：每次工具调用都重新 `Config::load()`

**选择**：MCP server 是长驻进程，但 4 个 tool handler 内部仍调用 `Config::load()`（与 v1 lib 函数行为一致）。

**理由**：

- 用户重跑 `xld setup` 后凭据热更新——长驻 MCP server 自动用新凭据，无需重启 client
- `Config::load()` 性能开销 < 1ms（fs read + serde parse），单次工具调用频率不高，无优化压力
- 跟 spec `credential-self-check` 既有契约一致

**替代方案**：

- 启动时缓存 Config，长期持有：被否决——设置过期/失效逻辑徒增复杂

### D9：错误模型——MCP `isError: true` 而非 JSON-RPC error

**选择**：所有 lib 函数返回的 `ErrorPayload` 转换为 MCP `CallToolResult { isError: true, content: [...] }`，content 含结构化 JSON（`{ kind, message, hint, retry_after? }`）。**不**用 JSON-RPC level error response（code -32603 等）。

**理由**：

- MCP 推荐 `isError: true` 表示"tool 跑了但有问题，agent 可读 detail"；JSON-RPC error 表示"server 故障"，agent 当作 transport-level error 处理
- 我们的错误（auth_expired / endpoint_stale / ...）都是"tool-level 业务错误"，应该让 agent 看到 kind 字段决定下一步
- 真 panic / serde 失败 / 协议解析失败才用 JSON-RPC error response

**替代方案**：

- 全部用 JSON-RPC error：被否决——agent 看不到结构化 kind，恢复路径全靠错误消息正则匹配

### D10：v2.0 不实施真实 cancellation 响应

**选择**：v2.0 收到 MCP `notifications/cancelled` 时**忽略**——既不停止正在跑的工具调用，也不清理半下载文件。Server 仅在 stderr 诊断日志中记录 "received cancellation notification, ignoring (v2.0)"；正在跑的工具继续完成并返回正常 `CallToolResult`。客户端如需强制终止，必须通过关闭 stdin（让 server 走 EOF 优雅关闭路径）实现。v2.1 视实战需求添加真实 cancellation 支持。

**理由**：

- v2.0 核心目标是"最薄的 MCP 适配层"——cancellation 真实实施会带来：(1) 给 `download_media` lib 函数新增 `cancellation: Option<CancellationToken>` 参数，违反 D6 "lib 函数签名不动"承诺；(2) 半下载文件清理语义复杂度（cancel 时刻 buffer_unordered 中各 in-flight item 状态不一致）；(3) 单元测试需要 mock cancellation 时间窗口
- `download_media` 实测大多 30-60s 内完成（D8 决策已有数据），用户中途取消的实际场景罕见
- 紧急停止有兜底路径——关闭 MCP client 进程会让 server 走 stdin EOF 优雅关闭
- 留 v2.1 跟其它"long-running 改造"（如 Tasks 异步原语）一起设计，避免 v2.0 提前承诺约束

**替代方案**：

- 实施真实 cancellation：被否决——上述 3 点成本不抵 v2.0 的 marginal value
- 完全静默忽略 cancellation 通知：被否决——至少要记录 stderr 日志便于排障，否则用户看不到"为什么我取消了它没停"
- 用全局 atomic flag 半实现：被否决——半实现给后续 v2.1 真实 cancellation 留兼容性陷阱

### D11：Skill 包形态——`skill/SKILL.md` 重写、新增 `skill/mcp-config.json`

**选择**：

- `skill/SKILL.md`：工具表保留，但"调用形态"段从"spawn `xld <subcmd> --json`"改为"通过 MCP 协议工具调用 `list_likes` / ..."；删除 stderr NDJSON 一节；保留 auth_status / 分批使用建议等
- `skill/mcp-config.json`：新增 `{ "command": "xld", "args": ["serve", "--mcp"], "minimum_xld_version": "2.0.0" }` 一类内容；OpenClaw / Claude Code 的注册脚本读它
- `skill/README.md` 第 5 步注册改写为 `mcpServers` 配置示例（同时给 Claude Code 和 OpenClaw 两种平台）

**理由**：

- skill 仍是 Agent 发现入口，但执行层换成 MCP——这是 OpenClaw 65% skills 的主流做法
- 复用 v1 的 SKILL.md 工具描述、auth_status 使用规范、分批建议等内容（这些 client 形态无关）

### D12：v1 stderr NDJSON 协议代码 + spec 立刻删除

**选择**：

- 删除 `agent::types::ProgressEvent::Diagnostic` 等 NDJSON 专用变体（如有）
- 删除 `agent::types::NdjsonStderrSink` 实现
- 删除 `media-download-by-items` spec 中"stderr NDJSON 进度事件流"需求
- 保留 `IndicatifSink`（人类模式仍用）和 trait 抽象

**理由**：

- v2 后 NDJSON 没有用户——人类模式用 indicatif，Agent 模式用 MCP notification
- 留着是死代码 + 文档负担，且容易诱导后人误用

**替代方案**：

- 留 NDJSON 作为 `--ndjson-progress` 可选 flag：被否决——需求可疑，给"理论上的脚本用户"留口袋反而增加维护面

## 风险 / 权衡

| 风险 | 缓解措施 |
|---|---|
| rmcp 0.x API breaking change | pin 版本到具体 minor；升级走独立 PR + manual 测试；spec 不引用 rmcp 内部类型，仅描述 MCP 协议层契约 |
| MCP 协议演进引入新原语（Tasks 等） | v2.0 仅依赖稳定的 tools/list、tools/call、notifications/progress、notifications/cancelled；新原语视实战需求 v2.1+ 添加 |
| OpenClaw / Claude Code MCP server 配置格式不一致 | skill/README.md 同时给两种平台示例；mcp-config.json 用最通用格式（兼容主流 client） |
| v1 stderr NDJSON 删除可能影响某个未知用户 | v1 release 已经在 1.0.x 发布；任何依赖该协议的脚本可继续用 1.0.x；2.0 release notes 明确说明此 breaking change |
| Cancellation 在 download_media 中途生效但已写入文件不一致 | 文件系统层面：取消时 `tokio::fs::remove_file` 清理半文件（与 v1 416 处理类似）；spec 加场景"cancellation 必须清理半文件" |
| `schemars` 自动派生跟 rmcp 不兼容 | 实施时若发现冲突，回退手写 schema（增加 ~50 行模板代码）；不影响整体设计 |
| MCP 工具描述中文 vs 英文 | 工具 description 字段用中文（与 SKILL.md / spec / 错误信息保持一致）；rmcp `#[tool(description = "...")]` 应支持任意 UTF-8 字符串 |
| 凭据敏感字段意外通过 MCP 通道泄露 | tool handler 不返回 cookie/bearer 字段；ImportOutput 仅含 path 和 written 标志；spec 加禁止条款 |

## 迁移计划

### 实施阶段（顺序）

1. **Phase A**：依赖 + 子命令骨架——`Cargo.toml` 加 `rmcp` + `schemars` + `tokio-util`；`main.rs` 加 `Commands::Serve` 分支返回 unimplemented，验证编译通过
2. **Phase B**：`mcp_server.rs` 模块——空 server 启动 + tools/list 返回 4 个工具的 schema（无实际实现），用 mcp-inspector 或 Claude Code 验证连接
3. **Phase C**：4 个工具 handler 实施——逐个 `#[tool]` 包 lib 函数，先不接 progress / cancellation；活体测试（用真实 X cookies）
4. **Phase D**：`McpProgressSink` 实施 + download_media 进度通知；`tokio CancellationToken` 桥接 + cancellation 测试
5. **Phase E**：删除 v1 stderr NDJSON 代码 + spec 删除；保留 IndicatifSink；删除冗余测试
6. **Phase F**：Skill 包改造——重写 SKILL.md / README.md / 新增 mcp-config.json
7. **Phase G**：主仓 README + 更新 CI（如有 mcp-inspector smoke test 等）
8. **Phase H**：跨平台回归 + 活体冒烟（按 v1 同样的方式做端到端 MCP 协议验证）

每个 phase 独立可回滚（git revert）。

### 回滚策略

如 v2 整体发现严重问题：

- master 已 merge：revert 整个变更回到 v1 形态
- 已发布 release：rollback 到上一个 1.0.x，用户 `xld update` 自动降级到旧版（updater.rs 既有逻辑）

## Open Questions

1. **rmcp 版本锁定**：实施 Phase A（依赖与子命令骨架）时锁定具体 minor 版本。锁定标准：(a) mcp-inspector 实测连接成功；(b) 对应 minor.patch.x 至少有 1 个月稳定期（无近期 yanked 或 critical bug）；(c) `schemars` 与 rmcp `#[tool]` 宏协作派生 schema 通过。锁定后回填到本设计文档与 proposal.md 的 Cargo.toml 示例。当前提案中 `rmcp = "..."` 占位符会在 Phase A 收尾时替换为实际版本号
2. **schemars 是否真能跟 rmcp `#[tool]` 宏无缝**：实施 Phase B 时验证；不行就退到手写 schema（约 +50 行模板代码，不影响整体架构）
3. **OpenClaw 的 mcporter 如何加载我的 skill 包**：需要查 OpenClaw 文档或在 Phase F 时实测；如发现 mcporter 期望的 manifest 格式与 `mcp-config.json` 字段不一致，按需调整字段名（不影响其它部分）
4. **`xld <subcmd> --json` 模式的 OutputEnvelope 实现要不要彻底封存为 deprecated**？或者继续作为人类调试接口推荐？倾向后者——`--json` 对 jq pipeline 仍有价值，且 v2.0 不删除其代码（参见 proposal "范围外"）
5. **`mcp-config.json` 的字段集需要标准化吗**？目前 Claude Code `mcpServers` 段与 OpenClaw `.mcp.json` 格式不完全一致。Phase F 时给两种平台各写一份示例 / 各自的 README 段落
