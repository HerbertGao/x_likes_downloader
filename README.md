# X Likes Downloader

[![release](https://img.shields.io/github/v/release/HerbertGao/x_likes_downloader?sort=semver&label=release)](https://github.com/HerbertGao/x_likes_downloader/releases)
[![build](https://img.shields.io/github/actions/workflow/status/HerbertGao/x_likes_downloader/build.yml?branch=master&label=build)](https://github.com/HerbertGao/x_likes_downloader/actions/workflows/build.yml)
![rust](https://img.shields.io/badge/rust-1.85%2B-orange?logo=rust)
![platforms](https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-blue)

用 Rust 写的 X（Twitter）点赞推文媒体下载器。两种用法：

- **命令行工具**：一条命令把你点赞过的图片 / 视频全部下载、自动整理。
- **AI Agent 工具（MCP）**：在 Claude Code / Codex CLI 等里用自然语言操作，比如"看我最近点赞了哪些 Rust 相关内容，把视频下回来"。

> 🔐 所有凭据只存在你本机，不经过任何第三方服务或 API key。

---

## 快速开始（命令行）

### 1. 安装

从 [GitHub Releases](https://github.com/HerbertGao/x_likes_downloader/releases) 下载对应平台的版本，或从源码编译：

```bash
# 预编译版（macOS/Linux）
chmod +x x_likes_downloader
./x_likes_downloader --version

# 或从源码
cargo install --path .
```

> 需要 Rust **1.85+**（仅源码编译时）。

### 2. 导入登录凭据（一次性）

工具需要你浏览器里的登录信息：

1. 浏览器打开并登录 [x.com](https://x.com)，按 `F12` 打开开发者工具 → **Network** 标签
2. 刷新页面，随便找一个请求 → 右键 → **Copy → Copy as cURL**
3. 把内容存到 `curl_command.txt`，然后：

```bash
x_likes_downloader setup                       # 默认读 curl_command.txt
x_likes_downloader setup --curl-file my.txt    # 或指定文件
```

### 3. 下载 & 整理

```bash
x_likes_downloader download    # 下载点赞推文里的所有图片/视频
x_likes_downloader organize    # （可选）按用户名分文件夹整理
```

就这些。常用配置见下方 [配置](#配置)。

---

## 作为 AI Agent 工具使用（Skill + MCP）

本项目既是一个 [Agent Skill](https://agentskills.io)，也内置 [MCP server](https://modelcontextprotocol.io/)，让 AI Agent 直接操作你的 X 点赞列表。装好后你可以直接对话：

> "看我最近点赞了哪些 Rust 相关的内容" → "把这两条的视频下回来"

Agent 能用的 5 个工具：`list_likes`（列点赞）、`download_media`（下载）、`fetch_tweet`（按 URL/id 抓任意推文）、`auth_status`（检查凭据）、`setup_from_curl`（导入凭据）。下载进度实时反馈，大文件可中断、可断点续传。

skill 走**双路径**：客户端已注册 MCP server 时用 MCP 工具（有进度通知、下载可取消）；没注册则自动回落 `x_likes_downloader ... --json` 命令行，功能等价。

### 第一步：装好 binary

skill **不自带** binary，先确保 `x_likes_downloader` ≥ 2026.6.0 在 PATH 里，并已 `setup` 过凭据（见上方快速开始）：

```bash
x_likes_downloader --version    # 应 ≥ 2026.6.0
```

### 第二步：装 skill

```bash
npx skills add HerbertGao/x_likes_downloader
```

加 `-g` 装到用户级（所有项目可用），加 `-a claude-code`（或 `codex` / `cursor` / `openclaw` …）指定目标客户端：

```bash
npx skills add HerbertGao/x_likes_downloader -g -a claude-code
```

### 第三步（可选）：注册 MCP server

不注册也能用（Agent 自动回落 CLI）。注册的收益只有两个：下载进度通知，以及取消能真实中断在途下载。把 `x_likes_downloader serve --mcp` 写进你客户端的 MCP 配置即可，配置位置与格式见该客户端自己的文档。

### 客户端支持

只要客户端支持 Agent Skill、或能执行 shell 命令，就可以用。`npx skills` 支持 80+ 客户端（Claude Code、Codex CLI、Cursor、OpenCode、OpenClaw、Hermes、Gemini CLI、GitHub Copilot、Windsurf、Zed、Pi 等），完整列表与各自的安装路径见 [`vercel-labs/skills`](https://github.com/vercel-labs/skills#supported-agents)。

> 工具表、双路径调用约定与错误码语义见 [`.agents/skills/x_likes/SKILL.md`](./.agents/skills/x_likes/SKILL.md)，安装步骤与常见问题见 [`.agents/skills/x_likes/README.md`](./.agents/skills/x_likes/README.md)。

---

## 配置

最常用的几个配置项，写在项目根目录 `.env` 文件里（`cp env.example .env`）或用环境变量：

```ini
COUNT=50                     # 每次获取的推文数量
ALL=true                     # 是否下载全部点赞推文
DOWNLOAD_DIR=data/downloads  # 下载目录
FILE_FORMAT={USERNAME}_{ID}  # 文件命名格式
AUTO_ORGANIZE=true           # 下载后自动整理
TARGET_DIR=data/organized    # 整理目标目录
```

优先级：`.env` 文件 > 环境变量 > 默认值。如果无法直连 X，配置 HTTP 代理即可。

### 多账号归档（别名）

同一个人有多个 X 账号时，在 `DOWNLOAD_DIR` 下建 `username_aliases.txt`，把不同账号归到同一文件夹：

```text
# 每行一组，逗号分隔，首个为主名称（= 目标文件夹）
alice, alice_art, alice_photo
bob, bob_backup
```

`alice_art` / `alice_photo` 的文件会自动归到 `alice` 文件夹（文件名保留原始用户名）。文件不存在时忽略，`#` 开头为注释。

---

## 故障排除

| 问题 | 排查 |
| --- | --- |
| 认证失败 | 检查 `data/private_tokens.env` 是否存在且正确；重新 `setup` |
| 网络错误 / 连不上 X | 检查或更换代理 |
| 下载失败 | 检查网络和磁盘空间 |
| 整理出错 | 确认目标目录存在且可写 |

开调试日志：`RUST_LOG=debug x_likes_downloader download`

清理断点续传残留文件：

```bash
find ~/Pictures/x_likes -name "*.partial" -delete           # 部分下载残留
rm -rf ~/Library/Caches/x_likes_downloader                  # ETag 缓存（macOS）
rm -rf "${XDG_CACHE_HOME:-$HOME/.cache}/x_likes_downloader"  # ETag 缓存（Linux）
```

---

## 注意事项

- `data/private_tokens.env` 含敏感凭据，请妥善保管。
- 合理控制请求频率，避免触发 X 限流。
- 下载大量媒体会占用较多磁盘空间。

---

## 版本说明

<details open>
<summary><b>v2026.6.0 更新（新增 fetch_tweet：下载任意推文媒体）</b></summary>

- **新增 `fetch_tweet` 能力**：按 URL 或 tweet_id 抓取**任意**推文（不再限于自己点赞过的）的媒体元数据，返回与 `list_likes` 同构的 `TweetSummary`（含 `media[]`）。
  - CLI：`x_likes_downloader tweet get --url <url> | --id <id> [--json]`
  - MCP：新增第 5 个工具 `fetch_tweet`（`tools/list` 4 → 5）；取回后把 `media[]` 传给 `download_media` 完成下载。
- **两阶段错误分类**：HTTP 200 + GraphQL `errors[code:32]` 识别为 `auth_expired`；端点过期 `endpoint_stale` / 推文不可见 `tweet_unavailable` / 网络 `network_error` 各归其类；非 JSON body 容错不误判。
- **网络健壮性**：TweetDetail 请求 30s 超时；构造期失败归 `internal_error`、传输期归 `network_error`，不再一刀切 network。
- **兼容老推文**：覆盖 `TweetWithVisibilityResults` wrapper；焦点定位含 `rest_id` 软兜底。
- **版本 2026.9.0`。

不变：CLI 既有行为、`download_media` 沙箱/续传/取消、各 host packaging 结构、凭据本地化。
</details>

<details>
<summary><b>v2.1 更新（runtime cancellation + Range 续传 + 数值化进度）</b></summary>

- **MCP cancellation 真实生效**：`serve --mcp` 收到 `notifications/cancelled` 后，in-flight `download_media` 在 1 秒内停止；返回含 `cancelled` 状态的完整 `DownloadOutput`，Agent 可据此重试。
- **`.partial` + 原子 rename**：下载先写 `<path>.partial`，成功后原子 rename；失败/取消保留 partial 供续传。顺带修复 v2.0 crash 后半文件被静默 skip 的 bug。
- **HTTP Range 续传 + ETag 校验**：ETag 失配 / 缓存缺失自动重下并 emit 诊断消息。
- **进度数值化**：`progress = items_done + Σ_in_flight(bytes_done/bytes_total)`，大文件不再卡 0%。
- **新增字段**：`DownloadStatus::Cancelled`、`summary.cancelled`（`#[serde(default)]` 兼容老 client）。

不变：4 个 MCP 工具 schema、`serve --mcp` 子命令、stdio transport、各 host packaging 结构、CLI 行为。
</details>

---

## 命令行接口速查

所有功能也提供 `--json` 模式（输出 JSON 信封，便于 shell pipeline / jq）：

| 命令 | 用途 |
| --- | --- |
| `setup` | 从 cURL 导入凭据 |
| `download` | 下载点赞媒体 |
| `organize` | 按用户名整理文件 |
| `update` | 检查并自动更新 binary |
| `likes list` / `media download` / `auth status` | 细粒度子命令 |
| `serve --mcp` | 启动 MCP server（一般由 MCP client 自动拉起，无需手动运行） |

---

## 许可证

MIT License。欢迎 Issue / PR。

> 本工具仅供学习和个人使用，请遵守 X 的服务条款和相关法律法规，使用风险自负。
