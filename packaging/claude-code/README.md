# X Likes Downloader — Claude Code plugin

让 Claude Code 通过 4 个 `/x_likes:*` slash command + MCP server 操作你自己的 X 点赞列表。

---

## 一行装

```bash
claude plugin marketplace add https://github.com/HerbertGao/x_likes_downloader \
  && claude plugin install x_likes
```

> 自建 GitHub-based marketplace；无需上架第三方。`marketplace.json` 在仓库根 `.claude-plugin/marketplace.json`。

---

## 前置：安装 binary

本 plugin **不打包** binary——它假设 `x_likes_downloader` ≥ 2.1.0 已在 PATH 中。

```bash
# macOS Apple Silicon（其他平台见 packaging/skill/x_likes/README.md）
curl -fsSL -o /usr/local/bin/x_likes_downloader \
  https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_macos_arm64
chmod +x /usr/local/bin/x_likes_downloader
```

或：`cargo install x_likes_downloader` / `brew install HerbertGao/tap/x_likes_downloader`（如适用）。详细安装步骤见 [SOT README](../skill/x_likes/README.md)。

首次使用前需导入 cURL：

```bash
x_likes_downloader setup --curl-file ~/curl_command.txt
```

---

## Plugin 内容

| 文件 | 用途 |
|---|---|
| `.claude-plugin/plugin.json` | Plugin manifest（name/version/description/mcpServers 指针） |
| `.mcp.json` | MCP server 启动配置（`command: x_likes_downloader`、`args: [serve, --mcp]`） |
| `commands/auth.md` | `/x_likes:auth` — 凭据健康自检（disable-model-invocation） |
| `commands/list.md` | `/x_likes:list [count]` — 列点赞，渲染压缩表格（disable-model-invocation） |
| `commands/setup.md` | `/x_likes:setup` — 拉起交互 setup（disable-model-invocation） |
| `commands/download.md` | `/x_likes:download <id> [id...]` — LLM 串联 list_likes + download_media |
| `skills/x_likes/SKILL.md` | 由 SOT 同步生成的 SKILL.md 副本（落地于 `~/.claude/plugins/<...>/skills/`） |

---

## SKILL.md 落地路径

装后 Claude Code 把 SKILL.md 链接到：

- macOS / Linux：`~/.claude/plugins/x_likes/skills/x_likes/SKILL.md`
- Windows：`%USERPROFILE%\.claude\plugins\x_likes\skills\x_likes\SKILL.md`

> 副本由 `scripts/sync-skill.sh` 从 `packaging/skill/x_likes/SKILL.md` SOT 派生；**不要手工编辑** `packaging/claude-code/skills/x_likes/SKILL.md`，CI 会通过 `git diff --exit-code` 检测漂移并 fail。

---

## 卸载

```bash
claude plugin uninstall x_likes
claude plugin marketplace remove https://github.com/HerbertGao/x_likes_downloader
```

binary 与 binary 写入的本地凭据不会被卸载——如需清理：

```bash
# binary（取决于你装的方式）
which x_likes_downloader && rm -i "$(which x_likes_downloader)"

# 凭据（macOS）
rm -i ~/Library/Application\ Support/xld/private_tokens.env
```

---

## 进一步阅读

- [SOT SKILL.md](../skill/x_likes/SKILL.md) — Agent 工具表 + MCP 协议约定
- [SOT README.md](../skill/x_likes/README.md) — 完整安装与首次使用流程
- [`packaging/README.md`](../README.md) — multi-host packaging 架构
- [仓库主 README](../../README.md) — 人类 CLI 用法 + Host 适配状态表
