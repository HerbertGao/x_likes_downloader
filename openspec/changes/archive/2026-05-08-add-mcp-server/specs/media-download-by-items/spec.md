## 移除需求

### 需求:stderr NDJSON 进度事件流

**Reason**：v2 引入 MCP server 后，`download_media` 作为 Agent 工具的进度反馈走 MCP 原生 `notifications/progress` 协议（参见新能力 `agent-mcp-server`），不再需要也不再支持 stderr NDJSON 形态。NDJSON 是 v1 在 spawn-CLI 形态下的变通方案，在 v2 中是死代码 + 文档负担。

**Migration**：

- Agent 客户端：MCP `notifications/progress` 事件直接消费，详见 `agent-mcp-server` 能力中的进度通知规范
- 人类 CLI 用户：`xld media download`（无 `--json`）的 `indicatif` 进度条行为不变；带 `--json` 时 stdout 仍输出 JSON 信封，但 stderr 不再有 NDJSON 事件流（仅含诊断文本，可不解析）
- 代码清理：删除 `agent::types::ProgressEvent` 中 `Diagnostic` 等 NDJSON-only 变体（如有）；删除 `agent::types::NdjsonStderrSink` 实现；删除针对该协议的所有单元测试与集成 smoke 步骤
- 旧 v1 release（1.0.x）继续保留 NDJSON 行为；任何依赖该形态的脚本可继续使用旧版本

## 修改需求

### 需求:不暴露 organize 与旧 download 一把梭给 Agent

Agent 工具表（无论是 v1 Skill 形态还是 v2 MCP `tools/list`）暴露面禁止包含 `organize`（按用户名归档）与旧版 `xld download`（list+下载耦合）。`download_media` 必须是 Agent 模式下唯一的下载入口。

#### 场景:MCP tools/list 清单
- **当** MCP 客户端发送 `tools/list` 请求
- **那么** 响应工具集必须仅包含 `list_likes`、`download_media`、`auth_status`、`setup_from_curl`，禁止出现 `organize` 或旧 `download` 入口

#### 场景:Skill 工具表清单
- **当** 检视 `skill/SKILL.md` 中声明的 Agent 可调用工具集
- **那么** 该集合必须仅包含 `list_likes`、`download_media`、`auth_status`、`setup_from_curl`，禁止出现 `organize` 或旧 `download` 入口
