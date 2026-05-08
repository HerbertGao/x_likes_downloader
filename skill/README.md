# X Likes Downloader Skill — 安装指引

本 skill 让 AI Agent 通过自然语言访问你自己的 X 点赞列表并按需下载媒体。**整个流程在你本机完成，没有任何凭据上传到云端**。

如果你在评估这个 skill 能不能装，请先看 [SKILL.md](./SKILL.md) 了解 Agent 实际能做什么。

---

## 安装六步

### 1. 安装 `xld` 二进制

从 [GitHub Releases](https://github.com/HerbertGao/x_likes_downloader/releases) 下载对应平台版本：

```bash
# macOS Apple Silicon
curl -fsSL -o /usr/local/bin/xld https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_macos_arm64
chmod +x /usr/local/bin/xld

# macOS Intel
curl -fsSL -o /usr/local/bin/xld https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_macos_x86_64
chmod +x /usr/local/bin/xld

# Linux x86_64
sudo curl -fsSL -o /usr/local/bin/xld https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_linux_x86_64
sudo chmod +x /usr/local/bin/xld

# Windows x86_64（PowerShell）
Invoke-WebRequest -Uri https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_windows_x86_64.exe -OutFile $env:USERPROFILE\xld.exe
# 然后将 %USERPROFILE% 加入 PATH
```

验证：

```bash
xld --version
# 期望输出：x_likes_downloader 1.0.6 或更新
```

> 本 skill 要求 `xld` ≥ **1.0.6**（含 Agent Skill 子命令）。

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

### 5. 在 Agent 里注册本 skill

#### Claude Code

把本目录（`skill/`）作为 skill 路径添加到 Claude Code 配置中（具体方式见 Claude Code 文档的 skill 注册一节）。

#### OpenClaw

参考 OpenClaw skill 注册流程，把仓库的 `skill/` 路径或本仓库 release zip 作为来源。

### 6. 验证

```bash
xld auth status --json
```

应输出：

```json
{"ok":true,"data":{"status":"healthy","checked_at":"2026-05-07T..."}, "meta":{"schema_version":1}}
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

- [SKILL.md](./SKILL.md) — Agent 工具表与调用约定
- [defaults.json](./defaults.json) — 公开协议参数（兜底用，cURL 导入时被覆盖）
- [仓库主 README](../README.md) — 人类 CLI 用法与构建说明
