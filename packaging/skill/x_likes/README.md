# X Likes Downloader Skill — 安装指引（host-agnostic）

本 skill 让 AI Agent 通过自然语言访问你自己的 X 点赞列表并按需下载媒体。**整个流程在你本机完成，没有任何凭据上传到云端**。

如果你在评估这个 skill 能不能装，请先看 [SKILL.md](./SKILL.md) 了解 Agent 实际能做什么。

> 本目录是 SOT（single source of truth），**不**直接被任何 host 加载；具体注册步骤（marketplace 装命令、SKILL.md 落地路径）见对应 host adapter 的 README：
> - Claude Code → [`packaging/claude-code/README.md`](../../claude-code/README.md)
> - Codex CLI → [`packaging/codex/README.md`](../../codex/README.md)
> - OpenClaw → [`packaging/openclaw/x_likes/README.md`](../../openclaw/x_likes/README.md)
> - Hermes → [`packaging/hermes/README.md`](../../hermes/README.md)（v2.2 占位）
> - Cursor → [`packaging/cursor/README.md`](../../cursor/README.md)（v2.2 占位）

---

## 安装五步（host-agnostic 部分）

### 1. 安装 `x_likes_downloader` 二进制

binary 实际名为 `x_likes_downloader`（与 Cargo `[[bin]]` 配置一致）。

从 [GitHub Releases](https://github.com/HerbertGao/x_likes_downloader/releases) 下载对应平台版本：

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

或：

```bash
cargo install x_likes_downloader  # 如发布到 crates.io
brew install HerbertGao/tap/x_likes_downloader  # 如有 tap
```

验证：

```bash
x_likes_downloader --version
# 期望输出：x_likes_downloader 2.1.0 或更新
```

> 本 skill v2.1 要求 binary ≥ **2.1.0**（`SKILL.md` frontmatter 中 `min_binary_version` 字段约束）。

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
x_likes_downloader setup --curl-file ~/curl_command.txt
```

成功输出 `初始化完成。`。这一步会把 cookie / bearer / queryId / features 全部本地化到 binary 选择的稳定路径，**不会上传到任何地方**。

### 4. （可选）自定义沙箱下载目录

如需改到其它磁盘（比如外接硬盘），重新跑 setup 时加参数：

```bash
x_likes_downloader setup --curl-file ~/curl_command.txt --download-dir /Volumes/Archive/xld
```

### 5. 注册到对应 host 客户端

具体步骤见各 host adapter README（链接见本文顶部）。注册之后让 Agent 调用 `auth_status` 工具验证；亦可手动跑：

```bash
x_likes_downloader auth status --json
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

## 边界声明

- **凭据本地化**：cookies / bearer / queryId 仅写入本机用户目录，**永不入仓**
- **ToS**：使用 X 内部 GraphQL + cookie 理论上违反 X 开发者协议，由用户承担合规边界
- **沙箱**：`download_media` 的写入路径被严格限定在 base dir 之内，禁止 `..` / 绝对路径

---

## 进一步阅读

- [SKILL.md](./SKILL.md) — Agent 工具表与 MCP 协议调用约定
- [defaults.json](./defaults.json) — 公开协议参数（兜底用，cURL 导入时被覆盖）
- [仓库主 README](../../../README.md) — 人类 CLI 用法与构建说明
- [`packaging/README.md`](../../README.md) — multi-host packaging 架构
