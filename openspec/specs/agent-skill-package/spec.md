## 目的

在 `skills/x_likes/` 维护一份可独立分发的 Agent Skill（SKILL.md 工具表与调用约定、defaults.json、README.md），定义 5 个 Agent 工具的用途与使用规范，并与 binary 版本绑定。

该目录是 skill 的唯一副本：它以标准 Agent Skill 布局（`skills/<name>/SKILL.md`）位于仓库根，可被 `npx skills add HerbertGao/x_likes_downloader` 直接发现并安装到任意受支持的客户端，不再维护任何 per-host adapter 副本。仓库不得携带其它会被该 CLI 发现的 skill。

## 需求

### 需求:`skills/x_likes/` 目录结构

系统必须在 `skills/x_likes/` 目录维护该 skill 的全部产物，包含以下文件：

- `skills/x_likes/SKILL.md`：host-agnostic Agent 工具表与调用约定（中文）
- `skills/x_likes/defaults.json`：公开协议参数兜底（无敏感字段）
- `skills/x_likes/README.md`：安装与配置指引（中文）

`skills/x_likes/` 禁止包含任何用户私密字段、二进制可执行文件、或 Rust 源码；它必须是平铺目录，不进入 Cargo workspace。`defaults.json` 会被 `src/config.rs` 以 `include_str!` 编译进 binary，因此其路径是 binary 构建契约的一部分。

仓库禁止再维护 per-host 的 skill 副本目录（`packaging/`）、自建 marketplace（`.claude-plugin/marketplace.json`、`.agents/plugins/marketplace.json`）或 SOT 同步脚本（`scripts/sync-skill.sh`）。

#### 场景:目录结构存在

- **当** 在主分支上 `ls skills/x_likes/`
- **那么** 必须看到 `SKILL.md`、`defaults.json`、`README.md` 三个文件

#### 场景:无 per-host 副本

- **当** 检查仓库根与 `scripts/`
- **那么** 必须不存在 `packaging/` 目录、`.claude-plugin/marketplace.json`、`.agents/plugins/marketplace.json`、`scripts/sync-skill.sh`、`scripts/check-packaging.sh`

#### 场景:无仓库自带开发 skill

- **当** 检查 `.claude/skills/`、`.claude/commands/`、`.agents/skills/`
- **那么** 必须不存在任何 `SKILL.md` 或 slash command 定义（`openspec-cn` 生成的 `openspec-*` skill 与 `/opsx:*` 命令均不得入库）——否则 `npx skills add` 会向用户展示与本 skill 无关的开发工具

#### 场景:无用户私密字段

- **当** 在 `skills/x_likes/defaults.json` 或该目录任何文件中查找 `auth_token` / `ct0` / `csrf` / `cookies` / `personalization_id` / `user_id` / `user_agent` 等用户私密字段
- **那么** 这些字段必须不存在（注：`bearer_token` 是 X Web 公开 anonymous bearer，不属于此列；其位置由下方"defaults.json 字段定义"约束）

#### 场景:defaults.json 可被 binary 内嵌

- **当** 编译 crate
- **那么** `src/config.rs` 的 `include_str!` 必须指向存在的 `skills/x_likes/defaults.json`，编译不得因路径缺失而失败

### 需求:`SKILL.md` 工具表声明

`skills/x_likes/SKILL.md` 必须以一节明确声明 Agent 可调用的工具集，每个工具至少包含：名称、用途一句话、输入参数及类型、返回值结构概述、典型错误 `kind` 列表。声明的工具集必须为且仅为以下五个：

- `list_likes(count?, all?, since_cursor?)` → 列点赞
- `download_media(items[], subdir?, concurrency?)` → 按 MediaItem 数组下载到沙箱
- `auth_status()` → 凭据健康自检
- `setup_from_curl(curl_text)` → 首次配置 / 重新导入 cURL
- `fetch_tweet(url? | id?)` → 按 URL 或 tweet_id 抓取任意一条推文的媒体元数据，返回与 `list_likes` 同构的 TweetSummary（含 `media[]`）

`SKILL.md` 必须明确声明 `organize` 与旧 `download` 一把梭**不在**Agent 工具表内。SKILL.md 必须保持 host-agnostic（不含 `~/.claude/...`、`~/.codex/...`、`~/.hermes/...` 等任何 host 特定路径）；host 特定路径仅允许出现在 `README.md`。

`fetch_tweet` 的工具说明必须含 Agent 行为约定：取回 `TweetSummary` 后，将其 `media[]` 直接作为 `download_media` 的 `items[]` 入参完成下载——`fetch_tweet` 自身只取元数据、不下载。

#### 场景:工具表完整

- **当** 解析 SKILL.md 中的工具声明
- **那么** 必须找到上述五个工具且仅这五个，每个工具均包含名称、用途、参数、返回值、错误种类五要素

#### 场景:fetch_tweet 含下载衔接约定

- **当** 解析 SKILL.md 中 `fetch_tweet` 工具说明
- **那么** 必须存在一句明确约定：把 `fetch_tweet` 返回的 `media[]` 传给 `download_media` 的 `items[]` 来下载

#### 场景:明示不暴露的工具

- **当** 解析 SKILL.md
- **那么** 必须存在一段说明明示 `organize` 与旧 `download` 不在 Agent 暴露面，并解释原因

#### 场景:SKILL.md host-agnostic

- **当** 解析 SKILL.md 全文
- **那么** 必须不出现任何 host-specific 路径（`~/.claude/`、`~/.codex/`、`~/.hermes/`、`.cursor/skills/`、`packaging/<host>/` 等）；这些路径只允许出现在 README.md

### 需求:`SKILL.md` 调用约定（双路径）

SKILL.md 必须声明**双路径调用形态**：MCP 优先、CLI 回落，并给出选定依据与两路径的能力差异。

- **MCP 形态**：通过 MCP 协议的 `tools/call` 调用工具名 `list_likes` / `download_media` / `auth_status` / `setup_from_curl` / `fetch_tweet`，由客户端建立的 stdio MCP 连接转发到 `x_likes_downloader serve --mcp` 子进程
- **CLI 形态**：当 MCP 工具不可用（客户端未注册 MCP server、不支持 MCP、或用户只装了 binary）时，Agent 通过 shell 调用 `x_likes_downloader likes list --json` / `media download` / `auth status --json` / `setup --curl-file` / `tweet get --json`，解析 stdout 的 JSON 信封 `{ok, data?, meta, error?}`
- 两条路径共享同一份 lib 实现，语义、JSON 结构、错误 `kind` 必须一致
- SKILL.md 必须给出形态对照表（操作 → MCP 工具 → CLI 子命令），并要求 Agent 不得在同一轮对话中为同一件事混用两种形态

调用约定章节还必须包含：

- 业务错误以 `CallToolResult { isError: true }` 返回（MCP 形态）或 `ok: false` + 非零退出码（CLI 形态），content 含结构化 `kind` / `message` / `hint` 字段；`tools/call` 不应抛 JSON-RPC level error 除非真协议异常
- `download_media` 的进度通过 MCP `notifications/progress` 推送（仅在请求含 `progressToken` 时）；CLI 形态无进度通知，其它工具均无进度
- MCP 形态下 `notifications/cancelled` 必须真实生效：in-flight item 在 1 秒内停止，返回 `isError: false` + 完整 `DownloadOutput`（中断 item 状态为 `cancelled`，`.partial` 保留供续传）；CLI 形态无取消通道
- 出现 `kind: "auth_expired"` 或 `endpoint_stale` 时，Agent 应建议用户重新导入 cURL（即调用 `setup_from_curl` 工具或让用户跑 `x_likes_downloader setup --curl-file`）
- 出现 `kind: "tweet_unavailable"` 时（仅 `fetch_tweet`），Agent 应告知用户该推文不存在 / 已删除 / 受保护或当前凭据不可见，不应重试
- `fetch_tweet` 成功但 `tweet.media` 为空数组时，Agent 应提示用户该推文可能无媒体、或其视频为 HLS-only（当前不支持下载），而非直接把空 `media[]` 传给 `download_media` 后静默无结果
- 出现 `kind: "binary_missing"` 时，Agent 应将用户引导到 GitHub Releases 安装页
- Agent 必须依信封 `kind` 而非进程退出码区分错误类别（退出码是 `exit_code()` 把多个 `kind` 压成的 2 档粗判，不足以区分如 `tweet_unavailable` 与 `invalid_argument`）

#### 场景:双路径均有声明

- **当** 解析 SKILL.md 中的调用约定章节
- **那么** 必须同时出现 MCP `tools/call` 形态与 CLI `--json` 子命令形态，并给出二者的选择依据与能力差异说明

#### 场景:形态对照表完整

- **当** 解析 SKILL.md 中的形态对照表
- **那么** 五个 Agent 工具必须各自映射到一个 MCP 工具名与一条 CLI 子命令

#### 场景:约定可被自动化校验

- **当** 解析 SKILL.md 中的调用约定章节
- **那么** 必须包含 MCP 调用形态、CLI 信封形态、CallToolResult isError 错误模型、progressToken 进度协议、cancellation 生效范围、典型错误恢复路径六类信息

#### 场景:cancellation 描述不与实现矛盾

- **当** 解析 SKILL.md 的约定总览表
- **那么** Cancellation 一行必须声明 v2.1 起 MCP `notifications/cancelled` 真实生效，不得再出现"被忽略（仅 stderr 日志记录）"这类 v2.0 描述

### 需求:`auth_status` 使用规范

SKILL.md 必须明确告知 Agent，`auth_status` 工具执行实际网络请求（约 200ms），不应被频繁调用。文档必须列出且仅列出以下三种允许调用 `auth_status` 的场景：

1. 会话起始的预检（最多一次）
2. 其它工具返回 `auth_expired` 或 `endpoint_stale` 后的确认
3. 用户显式询问凭据状态

SKILL.md 必须明确禁止以下使用模式：在循环中调用、在每次工具调用前作为预检调用、为做缓存而连续调用。

#### 场景:Skill 文档含 auth_status 使用规范

- **当** 阅读 `skills/x_likes/SKILL.md`
- **那么** 必须存在专门一节描述上述三种允许场景与禁止模式

### 需求:`download_media` 分批使用建议

SKILL.md 必须建议 Agent 在预期下载量较大时（item 数量超过 20 或预计总字节数超过 50 MB）将下载分成多次 `download_media` 调用，每批 5-10 个 item，以便 MCP `notifications/progress` 进度事件能驱动用户可见的分阶段反馈，并避免单次调用阻塞过久。文档必须说明 CLI 形态下分批是唯一能给用户阶段性反馈的办法。

#### 场景:Skill 文档含分批建议

- **当** 阅读 `skills/x_likes/SKILL.md`
- **那么** 必须存在分批使用建议，明示触发分批的阈值与建议批次大小，并明确进度反馈走 MCP progress 通道

### 需求:`defaults.json` 字段定义

`skills/x_likes/defaults.json` 必须为合法 JSON，包含且仅包含以下字段：

- `likes_api_url`（字符串，X GraphQL `Likes` 端点完整 URL）
- `likes_features`（字符串，features JSON 序列化文本，沿用 X Web 当前值）
- `likes_fieldtoggles`（字符串，fieldToggles JSON 序列化文本）
- `tweet_detail_api_url`（字符串，X GraphQL `TweetDetail` 端点完整 URL）
- `tweet_features`（字符串，TweetDetail features JSON 序列化文本，沿用 X Web 当前值）
- `tweet_fieldtoggles`（字符串，TweetDetail fieldToggles JSON 序列化文本）
- `bearer_token`（字符串，X Web 公开 bearer，可选——若省略则从代码硬编码兜底读取）
- `schema_version`（数字，初版为 1）

上述字段中，**必填**为 `schema_version` / `likes_api_url` / `likes_features` / `likes_fieldtoggles` / `tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles`（共 7 个），`bearer_token` 为唯一可选字段。`tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles` 经 `resolve_protocol_field` 解析（env > private_tokens > defaults.json > 硬编码），其中 defaults.json 承载真值、`config.rs` 硬编码层仅作非权威兜底（镜像 `likes_*`：URL 兜底为有效短 URL，features/fieldtoggles 兜底为 `{}`）；`setup_from_curl` 不写 `TWEET_*`，故 private_tokens 层对这三字段为空。任何 required∪optional 之外的字段必须被 `scripts/check-skills.sh` 校验拒绝。

#### 场景:JSON 合法且字段集封闭

- **当** 解析 `skills/x_likes/defaults.json`
- **那么** 7 个必填键必须全部存在，且顶层键集合必须为 required∪{bearer_token} 的子集，无此范围外的字段

#### 场景:含 TweetDetail 协议字段

- **当** 解析 `defaults.json`
- **那么** 必须包含 `tweet_detail_api_url`、`tweet_features`、`tweet_fieldtoggles` 三个非空字符串字段

#### 场景:版本号存在

- **当** 解析 `defaults.json`
- **那么** 必须包含 `schema_version` 字段且其值为大于等于 1 的整数

### 需求:`README.md` 安装旅程覆盖

`skills/x_likes/README.md` 必须以陌生用户视角描述完整安装与首次使用流程，至少覆盖：

1. 通过 `npx skills add HerbertGao/x_likes_downloader` 安装 skill（含 `-g` / `-a <agent>` 变体）
2. 检测或安装 `x_likes_downloader` 二进制（含 macOS / Linux / Windows 的下载命令片段，链接到 GitHub Releases）
3. 在 X 网页登录后从浏览器抓取 `Likes` GraphQL 请求的 cURL，并保存到文件
4. 运行 `x_likes_downloader setup --curl-file <path>` 导入凭据与协议参数
5. （可选）通过 `x_likes_downloader setup --download-dir <path>` 自定义沙箱 base dir
6. （可选）注册 MCP server；必须明示不注册也能用（Agent 回落 CLI），并说明注册的收益仅为进度通知与真实取消

README 必须以 host-agnostic 方式给出 MCP 注册配置示例（JSON 形态），并明确说明各客户端的配置文件位置需查阅该客户端自身文档——不得维护逐客户端的注册脚本。README 必须明示 ToS 边界由用户承担、凭据本地化、不入仓库等关键约束。

#### 场景:README 含核心步骤

- **当** 阅读 `skills/x_likes/README.md`
- **那么** 上述六个核心步骤必须全部出现且顺序合理

#### 场景:README 明示边界

- **当** 阅读 README
- **那么** 必须包含关于 X ToS、凭据本地化、不入仓库的明确声明

#### 场景:MCP 注册为可选

- **当** 阅读 README 的 MCP 章节
- **那么** 必须明示"不注册也能用，Agent 会回落 CLI"，且不得包含任何 host-specific 的一行装 / 注册命令

### 需求:Skill 与 binary 版本绑定

`skills/x_likes/SKILL.md` 必须随同 `x_likes_downloader` binary 在同一 git tag / GitHub Release 中发布，禁止独立发版。

SKILL.md 必须声明 `min_binary_version` 字段（形如 `min_binary_version: 2026.6.0`）。该字段必须由 `scripts/version.sh` 在升级 `Cargo.toml` 时同步更新到一致值。

`scripts/check-skills.sh` 必须验证 `min_binary_version` 与 `Cargo.toml` 的 `[package].version` 一致；不一致时 exit code 非 0 并在输出中列出两处取值。

#### 场景:版本依赖声明

- **当** 解析 SKILL.md frontmatter
- **那么** 必须包含 `min_binary_version` 字段且其值符合 SemVer

#### 场景:版本字段同步

- **当** 运行 `bash scripts/version.sh patch`（或 minor / major / 显式版本号）
- **那么** `Cargo.toml` 与 `skills/x_likes/SKILL.md` 的 `min_binary_version` 必须更新到同一值

#### 场景:check 检测漂移

- **当** 手工修改 `Cargo.toml` 版本但未运行 `version.sh`
- **那么** `bash scripts/check-skills.sh` 退出码必须非 0，输出必须列出不一致的版本字段路径

### 需求:Frontmatter 符合 Agent Skills 规范

SKILL.md 的 YAML frontmatter 必须满足 [Agent Skills 规范](https://agentskills.io) 的可发现性要求：必须包含 `name`（值为 `x_likes`）与 `description`（非空，说明 skill 做什么以及何时使用）。`min_binary_version` 是本项目附加的字段，规范外的字段必须被客户端忽略而非报错。

#### 场景:发现所需字段齐全

- **当** 解析 SKILL.md frontmatter
- **那么** `name` 必须为 `x_likes`，`description` 必须非空

#### 场景:skills CLI 可发现

- **当** 在仓库根运行 `npx skills add . --list`
- **那么** 发现的 skill 列表必须**只**包含 `x_likes` 一个
