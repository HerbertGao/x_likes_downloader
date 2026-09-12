# X Likes Downloader Skill — 安装指引

让 AI Agent 通过自然语言操作你自己的 X 点赞列表并按需下载媒体。整个流程在你本机完成，**没有任何凭据上传到云端**。

如果你在评估这个 skill 能不能装，请先看 [SKILL.md](./SKILL.md) 了解 Agent 实际能做什么。

---

## 1. 安装 skill

```bash
npx skills add HerbertGao/x_likes_downloader
```

加 `-g` 装到用户级（所有项目可用），加 `-a claude-code`（或 `codex` / `cursor` / `openclaw` …）指定目标客户端：

```bash
npx skills add HerbertGao/x_likes_downloader -g -a claude-code
```

先预览不安装：

```bash
npx skills add HerbertGao/x_likes_downloader --list
```

安装路径由 skills CLI 决定（`skills` 源目录 → 各客户端 `<agent>/skills/x_likes/`），无需手工放置。

---

## 2. 安装 `x_likes_downloader` binary

skill **不打包** binary——它假设 `x_likes_downloader` ≥ **2026.6.0** 已在 `PATH` 中。

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
```

Windows（PowerShell，把 `%USERPROFILE%` 加入 PATH）：

```powershell
Invoke-WebRequest -Uri https://github.com/HerbertGao/x_likes_downloader/releases/latest/download/x_likes_downloader_windows_x86_64.exe -OutFile $env:USERPROFILE\x_likes_downloader.exe
```

或 `cargo install x_likes_downloader` / `brew install HerbertGao/tap/x_likes_downloader`。

验证：

```bash
x_likes_downloader --version
# 期望输出：x_likes_downloader 2026.6.0 或更新
```

---

## 3. 从浏览器抓取 cURL

1. 在浏览器打开 [https://x.com](https://x.com) 并登录你的账号
2. 进入"个人资料 → Likes"页面
3. 打开 DevTools（F12 / ⌥⌘I），切到 Network 标签页
4. 触发翻页（往下滚一两屏）
5. 在请求列表里找到 URL 包含 `/Likes` 的 GraphQL 请求
6. 右键 → Copy → Copy as cURL（**bash** 风格）
7. 把 cURL 文本粘贴到一个文件，例如 `~/curl_command.txt`

---

## 4. 导入凭据

```bash
x_likes_downloader setup --curl-file ~/curl_command.txt
```

成功输出 `初始化完成。`。这一步把 cookie / bearer / queryId / features 全部本地化到 binary 选择的稳定路径，**不会上传到任何地方**。

（可选）自定义沙箱下载目录，比如放到外接硬盘：

```bash
x_likes_downloader setup --curl-file ~/curl_command.txt --download-dir /Volumes/Archive/xld
```

验证：

```bash
x_likes_downloader auth status --json
```

应输出：

```json
{"ok":true,"data":{"status":"healthy","checked_at":"2026-05-08T..."},"meta":{"schema_version":1}}
```

看到 `"kind":"auth_expired"` 或 `"endpoint_stale"` 就回到第 3 步重新抓 cURL。

---

## 5. （可选）注册 MCP server

**不注册也能用**——SKILL.md 约定 Agent 在 MCP 工具不可用时回落到 `x_likes_downloader ... --json` 命令行，功能等价。

注册 MCP 的收益只有两个：下载时的实时进度通知，以及取消能真实中断在途下载。如果你的客户端支持 MCP 配置，把 `x_likes_downloader serve --mcp` 注册进去即可，注册后重启客户端：

```jsonc
// 多数客户端的 MCP 配置形态（键名与文件位置随客户端而异）
{
  "mcpServers": {
    "x_likes_downloader": {
      "command": "x_likes_downloader",
      "args": ["serve", "--mcp"],
      "transport": "stdio"
    }
  }
}
```

需要绝对路径时用 `command -v x_likes_downloader` 的结果替换 `command`。各客户端的配置文件位置请查该客户端自己的文档——本 skill 不再维护逐客户端的注册脚本。

注册成功后 Agent 的工具列表里会出现 `list_likes` / `download_media` / `auth_status` / `setup_from_curl` / `fetch_tweet`。验证：

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | x_likes_downloader serve --mcp
```

---

## 常见问题

**Q: cURL 要多久重抓一次？**
A: cookie 大约能用一两周，X 偶尔滚动 GraphQL queryId 后也需要重抓。看到 `auth_expired` / `endpoint_stale` 时再处理即可，不必预防性更新。

**Q: 不注册 MCP 会少什么功能？**
A: 只少下载进度通知和中途取消。列点赞、下载、抓推文、凭据自检、导入 cURL 五项能力完全一样。

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

- [SKILL.md](./SKILL.md) — Agent 工具表、双路径调用约定、错误码语义
- [defaults.json](./defaults.json) — 公开协议参数兜底（cURL 导入时被覆盖；同时被 binary 编译进去作为最后兜底）
- [仓库主 README](../../README.md) — 人类 CLI 用法与构建说明
