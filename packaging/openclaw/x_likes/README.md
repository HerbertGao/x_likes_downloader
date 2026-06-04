# X Likes Downloader — OpenClaw skill

OpenClaw host adapter，承接 v2.0 `skill/` 目录平移而来。配置 ClawHub URL 指向本目录后，OpenClaw 自动加载 SKILL.md + mcp-config.json。

> **v2.0 → v2.1 路径迁移（breaking）**：v2.0 用户必须把 ClawHub 注册的 URL 从 `<repo>/skill` 改为 `<repo>/packaging/openclaw/x_likes`。binary 行为不变，仅 skill 加载路径变化。

---

## ClawHub 注册（一次性）

```bash
# v2.1+ 路径
clawhub register x_likes \
  --source https://github.com/HerbertGao/x_likes_downloader \
  --skill-path packaging/openclaw/x_likes
```

或在 ClawHub UI 中将 Source 路径改为 `packaging/openclaw/x_likes`（具体命令以 OpenClaw 当前版本为准）。

---

## 前置：安装 binary

本 skill **不打包** binary——它假设 `x_likes_downloader` ≥ 2026.6.0 已在 PATH 中。

```bash
# macOS Apple Silicon
curl -fsSL -o /usr/local/bin/x_likes_downloader \
  https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_macos_arm64
chmod +x /usr/local/bin/x_likes_downloader
```

或 `cargo install x_likes_downloader` / `brew install HerbertGao/tap/x_likes_downloader`。详细见 [SOT README](../../skill/x_likes/README.md)。

首次使用：

```bash
x_likes_downloader setup --curl-file ~/curl_command.txt
```

---

## Skill 内容

| 文件 | 用途 |
|---|---|
| `SKILL.md` | Agent 工具表 + 调用约定（含 `metadata.openclaw.bins`、`min_version` 块） |
| `mcp-config.json` | MCP server 启动配置（`command: x_likes_downloader`、`args: [serve, --mcp]`、`transport: stdio`、`minimum_xld_version: 2026.6.0`） |
| `defaults.json` | 公开协议参数兜底（cURL 导入会覆盖；从 SOT 同步派生） |

`SKILL.md` 与 `defaults.json` 由 `scripts/sync-skill.sh` 从 SOT (`packaging/skill/x_likes/`) 派生；**不要手工编辑**——CI 会通过 `git diff --exit-code` 检测漂移并 fail。

---

## SKILL.md 落地路径

OpenClaw 安装后落地于：

- macOS / Linux：`~/.openclaw/skills/x_likes/SKILL.md`
- Windows：`%USERPROFILE%\.openclaw\skills\x_likes\SKILL.md`

具体路径视 OpenClaw / mcporter 版本而异。

---

## MCP 启动配置详情

```jsonc
{
  "command": "x_likes_downloader",
  "args": ["serve", "--mcp"],
  "transport": "stdio",
  "minimum_xld_version": "2026.6.0"
}
```

OpenClaw 通过 `mcporter` 启动 stdio 子进程；通信走 JSON-RPC 2.0。

---

## 进一步阅读

- [SOT SKILL.md](../../skill/x_likes/SKILL.md) — 真正可信源（host-agnostic）
- [SOT README.md](../../skill/x_likes/README.md) — 完整安装与首次使用流程
- [`packaging/README.md`](../../README.md) — multi-host packaging 架构
- [仓库主 README](../../../README.md) — 人类 CLI + Host 适配状态表
