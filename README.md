# X Likes Downloader (Rust版本)

一个用 Rust 编写的 X（Twitter）点赞推文媒体下载器：既是面向人类用户的 CLI，也是可被多 host AI Agent（Claude Code / Codex CLI / OpenClaw / 未来 Hermes、Cursor）驱动的自动化工具。**所有凭据本地化保存**，不依赖第三方 API key 或外部抓取服务。

> ## v2.0 → v2.1 迁移指引（packaging breaking）
>
> v2.1 把 host adapter 从单 `skill/` 目录拆分为 `packaging/` 多 host 容器：
>
> | 影响对象 | 变化 |
> |---|---|
> | **v2.0 OpenClaw 用户** | ClawHub 注册的 URL 必须从 `<repo>/skill` 改为 `<repo>/packaging/openclaw/x_likes`。binary 行为不变 |
> | **新 Claude Code 用户** | 走自建 marketplace：`claude plugin marketplace add https://github.com/HerbertGao/x_likes_downloader && claude plugin install x_likes` |
> | **新 Codex CLI 用户** | 走自建 marketplace：`codex plugin marketplace add https://github.com/HerbertGao/x_likes_downloader && codex plugin install x_likes` |
> | **`cargo install` / GHA release** | 不变 |
>
> 详见 [`packaging/README.md`](./packaging/README.md) 与各 host adapter README。

## 功能特性

- 🔐 支持 X 内部 API，无需第三方服务
- 📥 自动下载点赞推文中的图片和视频
- 🔄 支持断点续传，避免重复下载
- 📁 自动文件整理和分类
- 🚀 异步下载，支持进度显示
- 🌐 支持 HTTP 代理
- 📊 详细的下载统计信息
- 🤖 **Agent MCP server 模式**（v2）：暴露 `list_likes` / `download_media` / `auth_status` / `setup_from_curl` 四个 MCP 工具，原生支持 progress notification

## 安装

### 前置要求

- Rust **1.85+**（v2.0 起，rmcp 1.x 是 edition 2024 crate；schemars 1.2.x 声明 rust-version 1.74，整体取最高）
- 有效的X账号和登录状态

### 方法一：下载预编译版本（推荐）

从 [GitHub Releases](https://github.com/HerbertGao/x_likes_downloader/releases) 下载对应平台的预编译版本：

- **macOS ARM64** (Apple Silicon): `x_likes_downloader_macos_arm64`
- **macOS x86_64** (Intel): `x_likes_downloader_macos_x86_64`
- **Linux x86_64**: `x_likes_downloader_linux_x86_64`
- **Linux ARM64**: `x_likes_downloader_linux_arm64`
- **Windows x86_64**: `x_likes_downloader_windows_x86_64.exe`
- **Windows ARM64**: `x_likes_downloader_windows_arm64.exe`

下载后解压并运行：

```bash
# macOS/Linux
chmod +x x_likes_downloader
./x_likes_downloader --help

# Windows
x_likes_downloader.exe --help
```

### 方法二：从源码编译

```bash
# 克隆项目
git clone <repository-url>
cd x_likes_downloader

# 编译当前平台版本
cargo build --release

# 安装到系统
cargo install --path .
```

## 使用方法

### 1. 初始化配置

首先需要从浏览器中获取X的API请求信息：

1. 打开X网站并登录
2. 打开开发者工具（F12）
3. 进入Network标签页
4. 刷新页面，找到任意一个API请求
5. 右键点击请求 -> Copy -> Copy as cURL
6. 将cURL命令保存到 `curl_command.txt` 文件中

然后运行初始化命令：

```bash
# 使用默认的curl_command.txt文件
x_likes_downloader setup

# 或指定自定义文件
x_likes_downloader setup --curl-file my_curl.txt
```

### 2. 下载媒体文件

```bash
# 开始下载
x_likes_downloader download
```

### 3. 整理文件（可选）

```bash
# 使用默认目录
x_likes_downloader organize

# 或指定自定义目录
x_likes_downloader organize --source-dir downloads --target-dir organized
```

## 作为 Agent MCP server 使用（v2.1+）

本项目内置 **MCP server**（[Model Context Protocol](https://modelcontextprotocol.io/)），让 AI Agent 通过自然语言操作你自己的 X 点赞列表：

- **典型对话**："看我最近点赞了哪些 Rust 相关的内容" → "把这两条的视频下回来"
- **完整安装/配置流程（host-agnostic）**：[`packaging/skill/x_likes/README.md`](./packaging/skill/x_likes/README.md)
- **Agent 工具表与调用约定**：[`packaging/skill/x_likes/SKILL.md`](./packaging/skill/x_likes/SKILL.md)
- **Multi-host packaging 架构**：[`packaging/README.md`](./packaging/README.md)

### Host 适配状态

| Host | Status | Path | Notes |
|---|---|---|---|
| Claude Code | ✅ v2.1+ | [`packaging/claude-code/`](./packaging/claude-code/) | Plugin + 4 个 `/x_likes:*` slash command + MCP server |
| Codex CLI | ✅ v2.1+ | [`packaging/codex/`](./packaging/codex/) | Plugin（含 `interface` 富 manifest）+ MCP server，LLM 路由 |
| OpenClaw | ✅ v2.1+ 📦 v1.x+ | [`packaging/openclaw/x_likes/`](./packaging/openclaw/x_likes/) | v2.0 用户需把 ClawHub URL 改为新路径 |
| Hermes | 🔜 v2.2 | [`packaging/hermes/`](./packaging/hermes/) | SKILL.md 已验证兼容，host adapter 排期 v2.2 |
| Cursor | 🔜 v2.2 | [`packaging/cursor/`](./packaging/cursor/) | SKILL.md 已验证兼容，host adapter 排期 v2.2 |

### 通过自建 marketplace 安装

仓库根 `.claude-plugin/marketplace.json` 与 `.agents/plugins/marketplace.json` 是自托管 marketplace，无需上架第三方。

```bash
# Claude Code
claude plugin marketplace add https://github.com/HerbertGao/x_likes_downloader
claude plugin install x_likes

# Codex CLI（需 codex CLI ≥ 0.128）
codex plugin marketplace add https://github.com/HerbertGao/x_likes_downloader
codex plugin install x_likes
```

Plugin **不打包** binary——先确保 `x_likes_downloader` 在 PATH 中（`cargo install` / brew tap / GHA release binary 任选）。详细见 [`packaging/skill/x_likes/README.md`](./packaging/skill/x_likes/README.md)。

### 三类用户路径

| 用户类型 | 入口 | 特点 |
|---|---|---|
| **人类 CLI**（始终可用）| `x_likes_downloader download / setup / organize / update / likes list / media download / auth status` | 行为对老用户向后兼容；`--json` 模式输出 JSON 信封供 shell pipeline / jq 调试 |
| **Agent via MCP**（v2 主推）| `x_likes_downloader serve --mcp`（由 MCP client 自动 spawn）| MCP `tools/list` 暴露 4 个工具，原生 `notifications/progress` 进度反馈，凭据完全本地化 |
| **lib 集成方** | `use x_likes_downloader::agent::*;` | 直接调 `list_likes` / `download_media` / `auth_status` / `import_curl` 异步函数 |

所有路径共享同一份 lib 实现，任何 bug 修复同时受益。

## 配置选项

### 方法一：使用 .env 文件（推荐）

1. 复制示例配置文件：

    ```bash
    cp env.example .env
    ```

2. 编辑 `.env` 文件，根据需要修改配置：

    ```ini
    # 下载配置
    COUNT=50                    # 每次获取的推文数量
    ALL=true                    # 是否下载所有点赞推文
    DOWNLOAD_DIR=data/downloads # 下载目录
    FILE_FORMAT={USERNAME}_{ID} # 文件命名格式

    # 自动整理
    AUTO_ORGANIZE=true          # 下载完成后自动整理
    TARGET_DIR=data/organized   # 整理目标目录
    ```

### 方法二：环境变量

也可以通过环境变量设置配置：

```bash
# 下载配置
export COUNT=50                    # 每次获取的推文数量
export ALL=true                    # 是否下载所有点赞推文
export DOWNLOAD_DIR="downloads"    # 下载目录
export FILE_FORMAT="{USERNAME} {ID}"  # 文件命名格式

# 自动整理
export AUTO_ORGANIZE=true          # 下载完成后自动整理
export TARGET_DIR="organized"      # 整理目标目录
```

## 项目结构

```text
x_likes_downloader/
├── src/
│   ├── main.rs           # 主程序入口和命令行界面
│   ├── config.rs         # 配置管理
│   ├── setup.rs          # 初始化工具
│   ├── x_api.rs          # X API 调用
│   ├── downloader.rs     # 媒体下载器
│   ├── updater.rs        # 版本检查与自动更新
│   └── organize_files.rs # 文件整理工具
├── data/                  # 运行时自动生成
│   └── private_tokens.env    # 私有令牌配置
├── .env                      # 环境配置文件（用户创建）
├── env.example               # 示例配置文件
└── Cargo.toml
```

## 主要模块说明

### config.rs

- 加载和管理配置信息
- 从.env文件、环境变量和私有令牌文件读取配置
- 支持代理、下载目录、文件格式等配置
- 优先级：.env文件 > 环境变量 > 默认值

### setup.rs

- 解析cURL命令提取认证信息
- 生成私有令牌配置文件
- 支持cookie解析和URL解码

### x_api.rs

- 调用X内部GraphQL API
- 支持分页获取点赞推文
- 处理API响应和错误

### downloader.rs

- 异步下载媒体文件
- 支持图片和视频下载
- 断点续传和进度显示
- 文件完整性验证

### organize_files.rs

- 根据文件名解析用户信息
- 自动分类整理文件
- 处理重复文件
- 支持用户名别名映射（多账号归档到同一文件夹）

### updater.rs

- 检查 GitHub 上的最新版本
- 判断当前版本是否需要更新
- 下载并替换可执行文件（自动更新）

### 用户名别名（多账号归档）

如果同一个人拥有多个X账号，可以通过别名文件将不同账号的文件归档到同一文件夹。

在下载目录（`DOWNLOAD_DIR`）下创建 `username_aliases.txt` 文件：

```text
# 每行一组，逗号分隔，首个为主名称（匹配目标文件夹）
alice, alice_art, alice_photo
bob, bob_backup
```

这样 `alice_art` 和 `alice_photo` 账号的文件会自动归档到 `alice` 对应的文件夹中。

- 文件不存在时自动忽略，不影响正常使用
- 支持 `#` 开头的注释行
- 归档后文件名保留原始用户名

## 注意事项

1. **认证信息安全**: `data/private_tokens.env` 包含敏感信息，请妥善保管
2. **API限制**: 请合理控制请求频率，避免触发X的限流
3. **代理设置**: 如果无法直接访问X，请配置有效的代理
4. **存储空间**: 下载大量媒体文件会占用较多存储空间

## 故障排除

### 常见问题

1. **认证失败**: 检查 `data/private_tokens.env` 文件是否存在且内容正确
2. **网络错误**: 确认代理设置正确，或尝试更换代理
3. **下载失败**: 检查网络连接和存储空间
4. **文件整理错误**: 确认目标目录存在且有写入权限

### 调试模式

设置环境变量启用详细日志：

```bash
export RUST_LOG=debug
x_likes_downloader download
```

## 许可证

MIT License

## 贡献

欢迎提交Issue和Pull Request！

## 免责声明

本工具仅供学习和个人使用，请遵守X的服务条款和相关法律法规。使用者需自行承担使用风险。
