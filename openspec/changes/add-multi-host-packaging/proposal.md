## 为什么

v2.0 上线后，`skill/` 目录是 v1 OpenClaw 风格的单一 host adapter，但 2026 主流 Agent 生态实际是多 host 共存：Claude Code（plugin + slash commands + MCP）、Codex CLI（含 `interface` 富 manifest 的 plugin + skills）、OpenClaw（保留 v2.0 用户）、Hermes、Cursor 等都各自有 skill 加载机制。继续维护单 host packaging 让用户在每个生态都得手动适配，且 `/x_likes:list` 这类 host-原生 slash command 能力没法体现。同时活体调研发现：5 个工具（Claude Code / Codex CLI / OpenClaw / Hermes / Cursor）共享同一份 `SKILL.md` YAML frontmatter + Markdown 正文格式，差异仅在路径和可选扩展字段——这意味着可以一份 SOT (single source of truth) 派生多份 host 制品，工程成本远低于"为每个 host 各写一份"。

## 变更内容

- **BREAKING (packaging 层，非二进制)**：`skill/*` 目录整体平移到 `packaging/openclaw/x_likes/*`，保 v2.0 OpenClaw 用户路径不变（仅他们 ClawHub 注册的 URL 需重新指向）
- 新增 `packaging/skill/x_likes/`：跨 host 共享的 SOT，含 host-agnostic `SKILL.md` + `README.md` + `defaults.json`
- 新增 `packaging/claude-code/`：完整 Claude Code plugin，含 `.claude-plugin/plugin.json`、`.mcp.json`、`commands/{list,auth,download,setup}.md`（4 个 `/x_likes:*` slash commands）、`skills/x_likes/SKILL.md`（CI 同步生成）
- 新增 `packaging/codex/`：完整 Codex CLI plugin，含 `.codex-plugin/plugin.json`（带富 `interface` 块：displayName、brandColor、defaultPrompt、capabilities）、`.mcp.json`、`skills/x_likes/SKILL.md`
- 新增 `packaging/{hermes,cursor}/README.md`：v2.2 占位，明示已验证 SKILL.md 跨工具兼容
- 新增仓库根 `.claude-plugin/marketplace.json`：自建 GitHub-based marketplace，让用户 `claude plugin marketplace add https://github.com/HerbertGao/x_likes_downloader` + `claude plugin install x_likes` 一行装好
- 新增仓库根 `.agents/plugins/marketplace.json`：Codex CLI 自建 marketplace（schema 与 Claude Code 不同：含 `policy.installation`、`policy.authentication`、`category`、`source` 嵌套对象）
- 新增 `scripts/sync-skill.sh`：从 `packaging/skill/x_likes/SKILL.md` SOT 派生三份 host 副本（Claude Code 不加扩展、Codex 加 `agents/openai.yaml` 兼容、OpenClaw 加 `metadata.openclaw` 块）
- `scripts/check-skill-defaults.sh` → `scripts/check-packaging.sh`：扩展为遍历 `packaging/*` 各子目录、各自校验 manifest schema
- CI（`.github/workflows/`）：PR 时跑 `sync-skill.sh` 然后 `git diff` 校验，diff != 0 则 fail；matrix 校验各 host packaging
- README 顶层加 host 适配状态矩阵表
- **不变**：Rust binary（src/）、Cargo.toml、cargo install / brew tap / GHA release.yml；MCP server 行为；agent-mcp-server 规范

## 功能 (Capabilities)

### 新增功能

- `multi-host-packaging`: 跨 host 的 packaging 抽象——SOT 单一可信源原则、host-adapter 子目录布局、自建 GitHub-based marketplace 入口契约（Claude Code + Codex CLI 双 schema）

### 修改功能

- `agent-skill-package`: 从"OpenClaw-only single-skill 包"扩展为"多 host adapter 容器"——重写需求以引入 host-adapter 抽象层、SOT 派生约定，明示 OpenClaw 实现迁移到 `packaging/openclaw/x_likes/`，Claude Code / Codex CLI 加入为新的 host adapter

## 影响

- **代码**：仅 packaging/、scripts/、.github/workflows/、.claude-plugin/、.agents/、README.md；**不触碰 src/、Cargo.toml、tests/**
- **依赖**：无新增 Rust crate；新增 host 工具的实测依赖（codex CLI 0.128+、claude CLI ≥ 当前）仅在 CI 验收和文档示例处出现
- **用户路径变化**：v2.0 OpenClaw 用户需把 ClawHub URL 从 `<repo>/skill` 更新为 `<repo>/packaging/openclaw/x_likes`；新 Claude Code / Codex 用户走自建 marketplace；现有 `cargo install` / GHA binary release 不变
- **配置**：`.env` / `private_tokens.env` 加载逻辑不变；`mcp-config.json` 在 OpenClaw 路径保留，Claude Code/Codex 用各自 plugin 内 `.mcp.json`
- **CI**：matrix 拆 host 校验；增加 `sync-skill.sh` 一致性 guard
- **文档**：README 顶层增加 host 适配状态表；packaging/README.md 解释架构和 SOT 派生契约
- **不影响**：MCP server 协议行为、4 个 MCP 工具的 schema、cargo 构建、跨平台 binary release
