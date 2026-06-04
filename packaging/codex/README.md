# X Likes Downloader — Codex CLI plugin

让 Codex CLI 通过 SKILL.md + MCP server 操作你自己的 X 点赞列表。Codex 没有 slash command 概念——LLM 在交互窗口里看 SKILL.md 自动决定调用哪个 MCP 工具。

---

## 一行装

```bash
codex plugin marketplace add https://github.com/HerbertGao/x_likes_downloader
```

> 自建 GitHub-based marketplace；`marketplace.json` 在仓库根 `.agents/plugins/marketplace.json`。
>
> ⚠️ **codex 0.128 行为说明**：`codex plugin marketplace` 子命令仅有 `add / upgrade / remove`，**没有独立 `install`**。`marketplace add` 完成后，编辑 `~/.codex/config.toml` 加入 plugin 启用条目：
>
> ```toml
> [plugins."x_likes@x_likes_downloader"]
> enabled = true
> ```
>
> 之后 `codex` 自然语言会话中即可触发 plugin（"show my X likes"、"download these tweets"等）。

### 本地开发安装（fork / PR 验证）

```bash
git clone https://github.com/HerbertGao/x_likes_downloader.git
codex plugin marketplace add "$(pwd)/x_likes_downloader"
# 然后按上方说明手动启用
```

---

## 前置：安装 binary

本 plugin **不打包** binary——它假设 `x_likes_downloader` ≥ 2026.6.0 已在 PATH 中。

```bash
# macOS Apple Silicon
curl -fsSL -o /usr/local/bin/x_likes_downloader \
  https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_macos_arm64
chmod +x /usr/local/bin/x_likes_downloader
```

或：`cargo install x_likes_downloader` / `brew install HerbertGao/tap/x_likes_downloader`（如适用）。详细见 [SOT README](../skill/x_likes/README.md)。

首次使用前导入 cURL：

```bash
x_likes_downloader setup --curl-file ~/curl_command.txt
```

---

## Plugin 内容

| 文件 | 用途 |
|---|---|
| `.codex-plugin/plugin.json` | Plugin manifest（name/version + 富 `interface` 块：displayName / category / capabilities / defaultPrompt / brandColor） |
| `.mcp.json` | MCP server 启动配置（与 Claude Code 一致） |
| `skills/x_likes/SKILL.md` | 由 SOT 同步生成的 SKILL.md 副本 |

Codex CLI 本身没有 slash command 机制。LLM 看 SKILL.md 描述并自动路由到合适的 MCP 工具：

- "show my X likes" / "list my likes" → `list_likes`
- "download these tweets' media" → `list_likes` + `download_media`
- "check my X auth" → `auth_status`
- "import this cURL" → `setup_from_curl`

`interface.defaultPrompt` 提供 3 个 onboarding 示例，在 Codex marketplace 详情页展示。

---

## SKILL.md 落地路径

装后 Codex CLI 把 SKILL.md 链接到：

- macOS / Linux：`~/.codex/plugins/x_likes/skills/x_likes/SKILL.md`
- Windows：`%USERPROFILE%\.codex\plugins\x_likes\skills\x_likes\SKILL.md`

> 副本由 `scripts/sync-skill.sh` 从 `packaging/skill/x_likes/SKILL.md` SOT 派生；**不要手工编辑** `packaging/codex/skills/x_likes/SKILL.md`，CI 会通过 `git diff --exit-code` 检测漂移。

---

## 卸载

```bash
codex plugin uninstall x_likes
codex plugin marketplace remove https://github.com/HerbertGao/x_likes_downloader
```

binary 与本地凭据由用户独立管理；plugin 卸载不影响。

---

## 进一步阅读

- [SOT SKILL.md](../skill/x_likes/SKILL.md) — Agent 工具表 + MCP 协议约定
- [SOT README.md](../skill/x_likes/README.md) — 完整安装与首次使用流程
- [`packaging/README.md`](../README.md) — multi-host packaging 架构
- [仓库主 README](../../README.md) — 人类 CLI 用法 + Host 适配状态表
