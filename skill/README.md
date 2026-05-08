# X Likes Downloader Skill — 安装指引

本 skill 让 AI Agent 通过自然语言访问你自己的 X 点赞列表并按需下载媒体。**整个流程在你本机完成，没有任何凭据上传到云端**。

如果你在评估这个 skill 能不能装，请先看 [SKILL.md](./SKILL.md) 了解 Agent 实际能做什么。

---

## 安装六步

### 1. 安装 `x_likes_downloader` 二进制

binary 实际名为 `x_likes_downloader`（与 Cargo `[[bin]]` 配置一致）。后续示例为简洁同时给"短名 `xld`"——你可以选择直接用全名，或建一个 `xld` 软链。

从 [GitHub Releases](https://github.com/HerbertGao/x_likes_downloader/releases) 下载对应平台版本（保持 binary 名 `x_likes_downloader`）：

```bash
# macOS Apple Silicon
curl -fsSL -o /usr/local/bin/x_likes_downloader https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_macos_arm64
chmod +x /usr/local/bin/x_likes_downloader

# macOS Intel
curl -fsSL -o /usr/local/bin/x_likes_downloader https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_macos_x86_64
chmod +x /usr/local/bin/x_likes_downloader

# Linux x86_64
sudo curl -fsSL -o /usr/local/bin/x_likes_downloader https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_linux_x86_64
sudo chmod +x /usr/local/bin/x_likes_downloader

# Windows x86_64（PowerShell）—— 把 %USERPROFILE% 加入 PATH
Invoke-WebRequest -Uri https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_windows_x86_64.exe -OutFile $env:USERPROFILE\x_likes_downloader.exe
```

可选——加 `xld` 短名（让后续命令更短）：

```bash
# macOS / Linux
sudo ln -s "$(which x_likes_downloader)" /usr/local/bin/xld
# 之后 `xld setup ...` 等命令都能用
```

验证：

```bash
x_likes_downloader --version
# 期望输出：x_likes_downloader 2.0.0 或更新
```

> 本 skill v2 要求 binary ≥ **2.0.0**（含 `serve --mcp` MCP server 子命令）。

### 2. 从浏览器抓取 cURL

1. 在浏览器打开 [https://x.com](https://x.com) 并登录你的账号
2. 进入"个人资料 → Likes"页面
3. 打开 DevTools（F12 / ⌥⌘I），切到 Network 标签
4. 触发翻页（往下滚一两屏）
5. 在请求列表里找到 URL 包含 `/Likes` 的 GraphQL 请求
6. 右键 → Copy → Copy as cURL（**bash** 风格）
7. 把 cURL 文本粘贴到一个文件，例如 `~/curl_command.txt`

### 3. 导入凭据

```bash
xld setup --curl-file ~/curl_command.txt
```

成功输出 `初始化完成。`。这一步会把 cookie / bearer / queryId / features 全部本地化到 `data/private_tokens.env`，**不会上传到任何地方**。

### 4. （可选）自定义沙箱下载目录

默认下载目录：

| 平台 | 路径 |
|---|---|
| macOS | `~/Library/Application Support/xld/downloads` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/xld/downloads` |
| Windows | `%LOCALAPPDATA%\xld\downloads` |

如需改到其它磁盘（比如外接硬盘），重新跑 setup 时加参数：

```bash
xld setup --curl-file ~/curl_command.txt --download-dir /Volumes/Archive/xld
```

### 5. 注册 MCP server

v2 的接入形态是 **MCP server**——客户端启动时以子进程形式 spawn `xld serve --mcp`，通过 stdio pipe 通信。配置一次即可。

#### Claude Code

在 `~/.claude/settings.json`（用户级）或项目根 `.mcp.json`（项目级）加：

```jsonc
{
  "mcpServers": {
    "xld": {
      "command": "x_likes_downloader",
      "args": ["serve", "--mcp"]
    }
  }
}
```

`command` 必须是 `cargo install` 或 GitHub Releases 安装的实际 binary 名 `x_likes_downloader`。如果你想用更短的命令名 `xld`，自己软链（一次性）：

```bash
# macOS / Linux
sudo ln -s "$(which x_likes_downloader)" /usr/local/bin/xld
# 然后配置可改为 "command": "xld"
```

重启 Claude Code 后，`list_likes` / `download_media` / `auth_status` / `setup_from_curl` 4 个工具自动可用。

#### OpenClaw

通过 `mcporter` 注册（推荐）：

```bash
# 让 OpenClaw 发现并连接到本地 xld MCP server
mcporter add xld --command xld --args "serve --mcp"
```

或手动编辑 OpenClaw 的 MCP 配置文件（具体路径见 [OpenClaw MCP 文档](https://docs.openclaw.ai/cli/mcp)），加入与上面 Claude Code 相同的配置内容。

#### 其它 MCP 客户端（Hermes / Cursor / etc.）

通用配置：`command = xld`、`args = ["serve", "--mcp"]`、`transport = stdio`。详见 `skill/mcp-config.json`。

### 6. 验证

启动 Agent 客户端后，让 Agent 调用 `auth_status` 工具——应当返回 `status: "healthy"`。

或手动验证（不通过 Agent）：

```bash
xld auth status --json
```

应输出：

```json
{"ok":true,"data":{"status":"healthy","checked_at":"2026-05-08T..."},"meta":{"schema_version":1}}
```

如果看到 `"kind":"auth_expired"` 或 `"endpoint_stale"`，回到第 2 步重新抓 cURL。

---

## 常见问题

**Q: cURL 重新抓多久一次？**
A: cookie 大约能用一两周，X 偶尔滚动 GraphQL queryId 后也需要重抓。看到 `auth_expired` / `endpoint_stale` 时再处理即可，不必预防性更新。

**Q: 下载的文件能放进 iCloud / Dropbox 同步目录吗？**
A: 可以。第 4 步把 `--download-dir` 设到同步目录即可。注意不要设到云端只读目录。

**Q: 我能装在多台机器上让多个 Agent 共用吗？**
A: 每台机器各装一份，凭据独立。本 skill 不支持把 cookie 同步到云端——这是设计选择。

**Q: 这违反 X 的 ToS 吗？**
A: 使用 X 内部 GraphQL + cookie 严格意义上违反 X 的开发者协议。本 skill 仅作个人合理使用工具，**风险与责任由用户承担**。请勿用于商业产品、转售数据、或大规模自动化。

---

## 进一步阅读

- [SKILL.md](./SKILL.md) — Agent 工具表与 MCP 协议调用约定
- [mcp-config.json](./mcp-config.json) — MCP server 启动配置（client 用它配置 mcpServers 段）
- [defaults.json](./defaults.json) — 公开协议参数（兜底用，cURL 导入时被覆盖）
- [仓库主 README](../README.md) — 人类 CLI 用法与构建说明
