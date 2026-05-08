# v2.1.0 — Multi-host plugin packaging

## Highlights

- 新增 **Claude Code plugin**：4 个 `/x_likes:*` slash command（auth / list / setup / download）+ MCP server 一行装 (`claude plugin marketplace add ... && claude plugin install x_likes`)
- 新增 **Codex CLI plugin**：含富 `interface` manifest（displayName / category / capabilities / defaultPrompt / brandColor）+ MCP server，在 codex 交互窗口里说"show my X likes"自动路由
- **OpenClaw skill 仍可用**：v1+ 用户继续工作；**packaging 路径变化**——见下方 Breaking
- 新增 **自建 GitHub-based marketplace**：`.claude-plugin/marketplace.json` + `.agents/plugins/marketplace.json`，无需上架第三方
- 新增 SOT + sync 派生契约：`packaging/skill/x_likes/` 是单一可信源，三家 host 副本由 `scripts/sync-skill.sh` 同步生成；CI 通过 `git diff --exit-code` guard 防漂移
- 顶层 README 加 **Host 适配状态表**（Claude Code / Codex CLI / OpenClaw / Hermes / Cursor）
- Hermes / Cursor v2.2 占位文档已就位，欢迎 PR

## Breaking changes (packaging only — binary 行为不变)

> ⚠️ **v2.0 OpenClaw 用户**：必须把 ClawHub 注册的 URL 从 `<repo>/skill` 改为 **`<repo>/packaging/openclaw/x_likes`**。binary 行为完全不变，仅 skill 加载路径迁移。

迁移示例：

```bash
# v2.0 (失效)
clawhub register x_likes --source https://github.com/HerbertGao/x_likes_downloader --skill-path skill

# v2.1+ (新路径)
clawhub register x_likes --source https://github.com/HerbertGao/x_likes_downloader --skill-path packaging/openclaw/x_likes
```

或在 ClawHub UI 中修改 Source 配置。

不受影响：

- `cargo install x_likes_downloader` / `brew install` / GitHub Releases binary
- MCP server 行为、4 个 MCP 工具的 schema
- `agent-mcp-server` 规范

## Install

```bash
# 1. 安装 binary（任选其一）
cargo install x_likes_downloader
brew install HerbertGao/tap/x_likes_downloader  # 如有 tap
curl -fsSL -o /usr/local/bin/x_likes_downloader https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_macos_arm64
chmod +x /usr/local/bin/x_likes_downloader

# 2. 装 host plugin
## Claude Code
claude plugin marketplace add https://github.com/HerbertGao/x_likes_downloader
claude plugin install x_likes@x_likes_downloader   # 注意 @<marketplace> 限定

## Codex CLI（≥ 0.128）
codex plugin marketplace add https://github.com/HerbertGao/x_likes_downloader
# codex 0.128 无独立 plugin install；编辑 ~/.codex/config.toml 加：
#   [plugins."x_likes@x_likes_downloader"]
#   enabled = true

# 3. 首次使用：在浏览器从 X 复制 cURL，导入凭据
x_likes_downloader setup --curl-file ~/curl_command.txt
```

## What's next (v2.2)

- Hermes / Cursor host adapter 实做
- 评估 GHA 自动版本同步（目前手动跑 `scripts/version.sh`）
- 评估上架第三方 marketplace（Claude Skills Hub / Anthropic 官方）

## Full file changes

- `packaging/skill/x_likes/` — SOT
- `packaging/{claude-code,codex,openclaw,hermes,cursor}/` — host adapters
- `.claude-plugin/marketplace.json` + `.agents/plugins/marketplace.json` — 自建 marketplace
- `scripts/sync-skill.sh` — SOT 派生
- `scripts/check-packaging.sh` — 取代旧 `check-skill-defaults.sh`
- `scripts/version.sh` — 扩展同步所有版本字段
- `.github/workflows/reusable-quality-checks.yml` — 加 packaging matrix（ubuntu + macos）
- 顶层 `README.md` — host 状态表 + 迁移指引
- `Cargo.toml` — version `2.0.1` → `2.1.0`

## Acknowledgements

谢谢早期 v2.0 OpenClaw 用户对路径迁移的耐心；谢谢 codex CLI 0.128 的 plugin schema 文档让 Codex adapter 实施得以落地。
