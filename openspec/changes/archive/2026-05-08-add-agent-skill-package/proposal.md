## 为什么

当前 `x_likes_downloader` 是一个**面向人类用户的 CLI**——用户必须手动跑 `setup` / `download` / `organize`，整个流程没法被 AI Agent 直接驱动。但项目独有的能力（**用本人浏览器 session 直连 X 内部 GraphQL，零开发者门槛获取自己的完整点赞**）正是 Agent 化场景里最稀缺的：OpenClaw 现有的 X 类 skill 要么只能抓公开内容（无登录），要么要求用户自带 Twitter Developer API key。

把这套能力包装成 **OpenClaw / Claude Code Skill**（v1）并预留 **MCP server**（v2）形态，可以让陌生用户装上即用、让 Agent 用自然语言完成"列我最近的点赞 → 把这几个的媒体下回来"的完整闭环，同时把每个用户的敏感凭据严格本地化、不入仓。

## 变更内容

### CLI 层

- **新增** `xld likes list [--all] [--since-cursor <c>] [--json]`：纯输出点赞列表 JSON 到 stdout
- **新增** `xld media download --ids <id1,id2,...> [--subdir <name>]`：按 tweet ID 下载媒体到沙箱
- **新增** `xld auth status [--json]`：自检 cookie / CSRF / bearer 是否仍然可用
- **保留** `xld download`、`xld organize`、`xld update`、`xld setup`：行为不变，但底层改调新 lib（向后兼容）
- **统一约定**：所有 `--json` 输出走 stdout，所有日志/进度/错误诊断走 stderr（消除当前 `x_api.rs` 里的 `println!` 污染问题）

### Library 层

- **新增** `src/lib.rs` 暴露纯函数能力层：`list_likes` / `download_media` / `auth_status` / `import_curl`
- **重构** `src/x_api.rs`：消除 stdout 调试输出，分离协议层与传输层
- **新增** `src/sandbox.rs`：下载目录沙箱化（base dir 配置 + 路径 jail 校验）

### 配置层

- **增强** `xld setup` 从 cURL 同时提取 `likes_api_url` / `features` / `fieldToggles`，写入本地配置覆盖公开默认值（抗 X 周期性滚动 GraphQL queryId）
- **新增**"公开默认 + 本地 override"双层配置加载：仓库内 vendored 协议参数仅作出厂兜底
- **新增** 沙箱 base dir 配置项（默认值平台相关：macOS `~/Library/Application Support/xld/downloads`、Linux `$XDG_DATA_HOME/xld/downloads`、Windows `%LOCALAPPDATA%\xld\downloads`）

### Skill 包

- **新增** `skill/` 目录：`SKILL.md`（Agent 工具表与调用约定）+ `defaults.json`（公开协议参数）+ `README.md`（陌生用户安装指引）
- 工具表暴露：`list_likes` / `download_media` / `auth_status` / `setup_from_curl`
- **不暴露**：`organize` / `update` / 旧 `download` 一把梭（保留给人类 CLI 用户）

### 范围外（明确不做）

- MCP server 实施（v2，仅在 lib 层留接口便利将来添加）
- 跨账号 / 多用户管理
- 第三方 Twitter API key 模式
- 删除现有 `organize` / `update` 子命令

## 功能 (Capabilities)

### 新增功能

- `likes-listing-json`: CLI 与 lib 层"列点赞为 JSON"的契约——扁平 v1 schema、可选原始 entry（opt-in）、分页、增量游标、错误形态
- `media-download-by-items`: 按 `MediaItem[]` 下载媒体的契约（首选）+ 按 ID 快捷方式（内部走 list_likes），含并发控制、stderr NDJSON 进度事件流、沙箱、断点续传
- `credential-self-check`: 凭据健康自检的契约——轻量真实请求、JSON 输出形态、退出码、不缓存策略
- `curl-import-extended`: `setup` 从 cURL 提取协议参数（url/features/fieldToggles）的契约与 override 优先级
- `download-sandbox`: 下载沙箱目录的配置、默认值（跨平台）、子目录传参的 jail 规则
- `agent-skill-package`: `skill/` 目录结构、`SKILL.md` 工具表、`defaults.json` 字段定义、陌生用户首次使用流程、auth_status 与 download_media 的使用规范

### 修改功能

无。现有 `username-alias` 能力（归档侧）不在本次变更范围内。

## 影响

### 代码

- `src/main.rs`: 新增 3 个子命令路由，所有现有子命令底层切换到调 lib（无外部行为变化）
- `src/x_api.rs`: 重构，剥离 stdout 输出；公开默认值从代码中移到 `skill/defaults.json` 与本地 override 合并
- `src/setup.rs`: cURL 解析扩展，提取 url / features / fieldToggles
- `src/config.rs`: 配置层级新增"公开默认 vendored layer"与沙箱 base dir 字段
- `src/downloader.rs`: 新增"按 ID 下载"入口，复用现有断点续传与进度逻辑
- **新增** `src/lib.rs`、`src/sandbox.rs`
- `Cargo.toml`: 由 bin-only 改为 lib + bin 双 crate；新增 `dirs` 依赖（跨平台路径）

### 仓库

- **新增** `skill/SKILL.md` / `skill/defaults.json` / `skill/README.md`
- `README.md`: 新增"作为 Agent Skill 使用"章节，说明三类用户路径（人类 CLI / Skill 装机用户 / 未来 MCP 集成方）

### 用户

- **现有 CLI 用户**：完全向后兼容，旧命令行为不变；可选享用新子命令
- **新 Skill 用户**：需自行安装 `xld` 二进制（友好错误指引），随后导入 cURL 即可被 Agent 驱动
- **凭据存储**：仍在本地 `.env` / 配置目录，不入仓库；新增的 base dir 默认值用平台标准目录

### 风险与边界

- 公开 `defaults.json` 里的 GraphQL queryId 会随 X 滚动而失效；缓解策略是"用户重导一次 cURL 即恢复"，不依赖 skill 仓库发版救火
- 公开发布后 Agent 调用频率上升，X anti-bot 风险增加；本变更不引入主动节流（依赖现有翻页逻辑），但在 README 里明示 ToS 边界由用户承担
- 沙箱化下载切断了 Agent 写任意路径的能力；现有 `xld download` 默认目录行为保持不变以兼容
