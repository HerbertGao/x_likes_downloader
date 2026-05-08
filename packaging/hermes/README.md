# X Likes Downloader — Hermes adapter

让 [Hermes Agent](https://github.com/NousResearch/hermes) 通过自然语言操作你自己的 X 点赞列表。Hermes 0.12+ 原生支持 Anthropic 风格 SKILL.md frontmatter——SOT (`packaging/skill/x_likes/SKILL.md`) 直接派生过来即可加载，无需额外 host 扩展。

> **状态**：✅ v2.1+（活体冒烟通过：MCP 连接 + 4 工具发现 + SKILL.md 安装）

---

## 前置：安装 binary

本 adapter **不打包** binary——`x_likes_downloader` ≥ 2.1.0 必须在 PATH 中。

```bash
# macOS Apple Silicon
curl -fsSL -o /usr/local/bin/x_likes_downloader \
  https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_macos_arm64
chmod +x /usr/local/bin/x_likes_downloader
```

或：`cargo install x_likes_downloader` / `brew install HerbertGao/tap/x_likes_downloader`（如适用）。详细见 [SOT README](../skill/x_likes/README.md)。

首次使用：

```bash
x_likes_downloader setup --curl-file ~/curl_command.txt
```

---

## 装 SKILL.md（一行命令）

```bash
hermes skills install \
  https://raw.githubusercontent.com/HerbertGao/x_likes_downloader/master/packaging/hermes/skills/x_likes/SKILL.md \
  --yes
```

> Hermes 自动经 community 安全扫；通过后落到 `~/.hermes/skills/x_likes/SKILL.md`。
>
> 验证：`hermes skills list | grep x_likes`

---

## 装 MCP server（手动 YAML 编辑）

⚠️ **Hermes 0.12 已知问题**：`hermes mcp add` CLI 的 argparse 把 `--mcp` 误识为 hermes 顶层 flag。直接编辑 `~/.hermes/config.yaml` 绕开：

```yaml
mcp_servers:
  x_likes_downloader:
    command: /Users/<you>/.cargo/bin/x_likes_downloader   # 或 GitHub Releases binary 的绝对路径
    args:
      - serve
      - --mcp
    enabled: true
```

或用 Python 一键追加（避免手抖）：

```bash
python3 -c "
import yaml
with open('$HOME/.hermes/config.yaml') as f: cfg = yaml.safe_load(f)
cfg.setdefault('mcp_servers', {})['x_likes_downloader'] = {
    'command': '$(command -v x_likes_downloader)',
    'args': ['serve', '--mcp'],
    'enabled': True,
}
with open('$HOME/.hermes/config.yaml', 'w') as f:
    yaml.safe_dump(cfg, f, sort_keys=False, allow_unicode=True)
"
```

验证：

```bash
hermes mcp list                       # 应看到 x_likes_downloader 状态 ✓ enabled
hermes mcp test x_likes_downloader    # 应在 < 1s 内 'Connected' + 'Tools discovered: 4'
```

---

## 触发

任意 Hermes 自然语言会话（`hermes chat` / `hermes -z "..."`）会基于 SKILL.md 描述自动路由：

- "Show my latest X likes" → `list_likes`
- "Download these tweets' media" → `list_likes` + `download_media`
- "Check my X auth" → `auth_status`
- "Import this cURL" → `setup_from_curl`

---

## SKILL.md 落地路径

- macOS / Linux：`~/.hermes/skills/x_likes/SKILL.md`
- Windows：`%USERPROFILE%\.hermes\skills\x_likes\SKILL.md`

> 副本由 `scripts/sync-skill.sh` 从 `packaging/skill/x_likes/SKILL.md` SOT 派生（直接复制，无 host 扩展）；**不要手工编辑** `packaging/hermes/skills/x_likes/SKILL.md`，CI 会通过 `git diff --exit-code` 检测漂移。

---

## 卸装

```bash
hermes skills uninstall x_likes
hermes mcp remove x_likes_downloader
```

binary 与本地凭据由用户独立管理；adapter 卸载不影响。

---

## 已知限制

- **`hermes mcp add` argparse bug**：上方 YAML 手动 / Python 一键编辑是当前 workaround。建议反馈到 Nous Research / Hermes issue tracker（`hermes mcp add NAME --command CMD --args=serve --args=--mcp` 应当解析为两个 plugin args，但 `--mcp` 被 hermes 顶层 argparse 抢走）
- **`hermes skills install` 拒绝 file://**：必须给 HTTP(S) URL 或 registry identifier；本地开发可先 push 分支到 GitHub fork 再用 raw URL 装

---

## 进一步阅读

- [SOT SKILL.md](../skill/x_likes/SKILL.md) — Agent 工具表 + MCP 协议约定
- [SOT README.md](../skill/x_likes/README.md) — 完整安装与首次使用流程
- [`packaging/README.md`](../README.md) — multi-host packaging 架构
- [仓库主 README](../../README.md) — 人类 CLI + Host 适配状态表
