# `packaging/` — Multi-host plugin packaging

本目录承载 `x_likes_downloader` 在不同 AI Agent host 平台的 plugin / skill 实做。架构基于 **SOT + host adapter 派生** 模式。

---

## 架构

```
packaging/
├── skill/x_likes/              ← SOT (single source of truth, host-agnostic)
│   ├── SKILL.md                  Agent 工具表 + 调用约定 + min_binary_version frontmatter
│   ├── README.md                 host-agnostic 安装与首次使用流程
│   └── defaults.json             公开协议参数兜底（无敏感字段）
│
├── claude-code/                ← Claude Code plugin
│   ├── .claude-plugin/plugin.json
│   ├── .mcp.json                 MCP server 启动配置
│   ├── commands/                 4 个 /x_likes:* slash commands
│   │   ├── auth.md, list.md, setup.md   (disable-model-invocation: 确定性 shell)
│   │   └── download.md                  (LLM 介入 + MCP 工具调用)
│   ├── skills/x_likes/SKILL.md   ← 由 sync-skill.sh 从 SOT 派生（直接复制）
│   └── README.md
│
├── codex/                      ← Codex CLI plugin
│   ├── .codex-plugin/plugin.json   含富 interface 块 (displayName / category / capabilities / defaultPrompt / brandColor)
│   ├── .mcp.json                   与 Claude Code 同
│   ├── skills/x_likes/SKILL.md     ← 由 sync-skill.sh 从 SOT 派生（直接复制）
│   └── README.md
│
├── openclaw/x_likes/           ← OpenClaw skill (v2.0 skill/ 目录平移而来)
│   ├── SKILL.md                  ← 由 sync-skill.sh 从 SOT 派生（注入 metadata.openclaw 块）
│   ├── mcp-config.json
│   ├── defaults.json             ← 由 sync-skill.sh 从 SOT 复制（剥除 bearer_token）
│   └── README.md
│
├── hermes/                     ← Hermes adapter (v2.1+ active)
│   ├── skills/x_likes/SKILL.md   ← 由 sync-skill.sh 从 SOT 派生（直接复制，无 host 扩展）
│   └── README.md
│
├── cursor/README.md            ← v2.2 占位
└── README.md                   ← 本文件
```

仓库根的两份 marketplace.json 是自建 GitHub-based marketplace 入口：

- `.claude-plugin/marketplace.json` — Claude Code marketplace schema
- `.agents/plugins/marketplace.json` — Codex CLI marketplace schema

---

## SOT 派生契约

**SOT (`packaging/skill/x_likes/SKILL.md`) 是真正可信源**；其它 host 子目录中的 `SKILL.md` 副本由 `scripts/sync-skill.sh` 派生。CI 通过 `bash scripts/sync-skill.sh && git diff --exit-code` 检测漂移。

派生规则：

| Host | 派生策略 |
|---|---|
| Claude Code | 直接复制 SOT，无 host 扩展 |
| Codex CLI | 直接复制 SOT（无 host 扩展；frontmatter 已含 `name`/`description` 即可） |
| Hermes | 直接复制 SOT（Hermes 0.12+ 原生消费 Anthropic 风格 frontmatter） |
| OpenClaw | 复制 SOT，在 frontmatter 注入 `metadata.openclaw.{bins,min_version}` 块；defaults.json 复制时剥除 bearer_token |

工具链：bash + awk + sed + jq（GHA `ubuntu-latest` / `macos-latest` 默认自带，**不依赖 yq**）。

`scripts/check-packaging.sh` 校验：

- 各 manifest 必填字段、命令一致性（`command == "x_likes_downloader"`、`args == ["serve","--mcp"]`、`transport == "stdio"`）
- 无敏感字段穿透（auth_token / ct0 / csrf / cookies / personalization_id；bearer_token 仅允许在 SOT defaults.json）
- 版本字段一致性（Cargo.toml ↔ marketplace.json ↔ plugin.json ↔ mcp-config.json ↔ SOT SKILL.md.min_binary_version）

---

## 当前 host 状态

| Host | Status | 实做位置 |
|---|---|---|
| Claude Code | ✅ v2.1+ | `claude-code/` |
| Codex CLI | ✅ v2.1+ | `codex/` |
| OpenClaw | ✅ v2.1+ 📦 v1.x+ | `openclaw/x_likes/` |
| Hermes | ✅ v2.1+ | `hermes/` |
| Cursor | 🔜 v2.2 | `cursor/` (placeholder) |

---

## 如何添加新 host adapter

参考 `packaging/codex/` 的形态——下面是骨架：

```
packaging/<your-host>/
├── <host>-plugin/manifest.json    # host-specific manifest (如有)
├── .mcp.json                      # MCP server 启动配置（如 host 支持 MCP）
├── skills/x_likes/SKILL.md        # 由 sync-skill.sh 派生（不要手工编辑）
└── README.md                      # 一行装命令、SKILL.md 落地路径、binary 安装
```

实施步骤：

1. **建目录骨架**：`mkdir packaging/<host>/skills/x_likes`
2. **写 manifest**：参考 [`codex/.codex-plugin/plugin.json`](./codex/.codex-plugin/plugin.json) 或 [`claude-code/.claude-plugin/plugin.json`](./claude-code/.claude-plugin/plugin.json)
3. **写 .mcp.json**（如适用）：与现有 host 一致（`command: x_likes_downloader`、`args: [serve, --mcp]`、`transport: stdio`）
4. **扩展 sync 脚本**：在 [`scripts/sync-skill.sh`](../scripts/sync-skill.sh) 中加一份派生路径；视 host 需要决定直接复制还是注入 frontmatter 块
5. **扩展 check 脚本**：在 [`scripts/check-packaging.sh`](../scripts/check-packaging.sh) 中加 manifest schema 校验分支
6. **扩展 marketplace**（如有）：在仓库根 marketplace.json 中加 plugin 条目
7. **扩展 version.sh**：在 [`scripts/version.sh`](../scripts/version.sh) 同步版本字段中加新增 manifest 的 version 字段
8. **更新 README**：顶层 `README.md` 状态表 + `packaging/README.md` 架构图 + `packaging/<host>/README.md` 一行装命令
9. **CI**：`scripts/sync-skill.sh && git diff --exit-code` + `scripts/check-packaging.sh` 应当通过；如新 host 需要专门的 lint，加到 `.github/workflows/`

实施例子可参考 git log 中 `add-multi-host-packaging` 变更如何加 codex / claude-code 三家 host。

---

## 设计参考

- [`openspec/changes/archive/add-mcp-server/`](../openspec/changes/archive/add-mcp-server/) — v2 MCP server 迁移
- `openspec/changes/add-multi-host-packaging/` — 本次 multi-host 抽象的 proposal / design / specs
- [`packaging/skill/x_likes/SKILL.md`](./skill/x_likes/SKILL.md) — Agent 工具表与协议约定
