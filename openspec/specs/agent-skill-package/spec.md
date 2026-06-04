## 目的

在 `packaging/skill/x_likes/` 维护一份 host-agnostic 的 Skill SOT（SKILL.md 工具表与调用约定、defaults.json、README.md、mcp-config.json），定义 5 个 Agent 工具的用途与使用规范，并与 binary 版本绑定，供各 host adapter 派生。

## 需求

### 需求:仓库新增 `packaging/skill/x_likes/` SOT 目录结构

系统必须在 `packaging/skill/x_likes/` 目录维护 SOT（single source of truth），包含以下文件：

- `packaging/skill/x_likes/SKILL.md`：host-agnostic Agent 工具表与调用约定（中文）
- `packaging/skill/x_likes/defaults.json`：公开协议参数兜底（无敏感字段）
- `packaging/skill/x_likes/README.md`：陌生用户安装与配置指引（中文）

`packaging/skill/x_likes/` 是 SOT，**不**直接被任何 host 加载；所有 host adapter 子目录（`packaging/{claude-code,codex,openclaw}/`）中的 SKILL.md 副本必须由 `scripts/sync-skill.sh` 派生。SOT 目录禁止包含任何用户私密字段、二进制可执行文件、或 Rust 源码；它必须是平铺目录，不进入 Cargo workspace。

v2.0 的 `skill/*` 目录已废弃；其功能与文件平移到 `packaging/openclaw/x_likes/` 作为 OpenClaw host adapter；后者的具体结构由 `multi-host-packaging` capability 定义。

#### 场景:SOT 目录结构存在
- **当** 在主分支上 `ls packaging/skill/x_likes/`
- **那么** 必须看到 `SKILL.md`、`defaults.json`、`README.md` 三个文件

#### 场景:SOT 无用户私密字段
- **当** 在 `packaging/skill/x_likes/defaults.json` 或 SOT 任何文件中查找 `auth_token` / `ct0` / `csrf` / `cookies` / `personalization_id` 等用户私密字段
- **那么** 这些字段必须不存在（注：`bearer_token` 是 X Web 公开 anonymous bearer，不属于此列；其位置由下方"defaults.json 字段定义"约束）

#### 场景:v2.0 skill 目录已迁移
- **当** 检查仓库根
- **那么** 必须不存在顶级 `skill/` 目录；其原内容必须存在于 `packaging/openclaw/x_likes/`

### 需求:`SKILL.md` 工具表声明

`packaging/skill/x_likes/SKILL.md`（SOT）必须以一节明确声明 Agent 可调用的工具集，每个工具至少包含：名称、用途一句话、输入参数及类型、返回值结构概述、典型错误 `kind` 列表。声明的工具集必须为且仅为以下五个：

- `list_likes(count?, all?, since_cursor?)` → 列点赞
- `download_media(items[], subdir?, concurrency?)` → 按 MediaItem 数组下载到沙箱
- `auth_status()` → 凭据健康自检
- `setup_from_curl(curl_text)` → 首次配置 / 重新导入 cURL
- `fetch_tweet(url? | id?)` → 按 URL 或 tweet_id 抓取任意一条推文的媒体元数据，返回与 `list_likes` 同构的 TweetSummary（含 `media[]`）

`SKILL.md` 必须明确声明 `organize` 与旧 `download` 一把梭**不在**Agent 工具表内。SOT SKILL.md 必须保持 host-agnostic（不含 `~/.claude/...`、`~/.codex/...` 等任何 host 特定路径）；host-specific 路径仅出现在 host adapter README 或对应 plugin manifest 中。

`fetch_tweet` 的工具说明必须含 Agent 行为约定:取回 `TweetSummary` 后，将其 `media[]` 直接作为 `download_media` 的 `items[]` 入参完成下载——`fetch_tweet` 自身只取元数据、不下载。

#### 场景:工具表完整
- **当** 解析 SOT SKILL.md 中的工具声明
- **那么** 必须找到上述五个工具且仅这五个，每个工具均包含名称、用途、参数、返回值、错误种类五要素

#### 场景:fetch_tweet 含下载衔接约定
- **当** 解析 SOT SKILL.md 中 `fetch_tweet` 工具说明
- **那么** 必须存在一句明确约定:把 `fetch_tweet` 返回的 `media[]` 传给 `download_media` 的 `items[]` 来下载

#### 场景:明示不暴露的工具
- **当** 解析 SOT SKILL.md
- **那么** 必须存在一段说明明示 `organize` 与旧 `download` 不在 Agent 暴露面，并解释原因

#### 场景:SOT host-agnostic
- **当** 解析 SOT SKILL.md 全文
- **那么** 必须不出现任何 host-specific 路径（`~/.claude/`、`~/.codex/`、`~/.hermes/`、`.cursor/skills/`、`packaging/<host>/` 等）；这些路径只允许出现在各自 host adapter 的 README

### 需求:`SKILL.md` 调用约定

`packaging/skill/x_likes/SKILL.md`（SOT）必须明确以下 Agent 调用约定，所有派生副本继承之：

- 所有工具的调用形态：通过 MCP 协议的 `tools/call` 调用工具名 `list_likes` / `download_media` / `auth_status` / `setup_from_curl` / `fetch_tweet`，由客户端建立的 stdio MCP 连接转发到 `x_likes_downloader serve --mcp` 子进程
- 业务错误以 `CallToolResult { isError: true }` 返回，content 含结构化 `kind` / `message` / `hint` 字段；`tools/call` 不应抛 JSON-RPC level error 除非真协议异常
- `download_media` 的进度通过 MCP `notifications/progress` 推送（仅在请求含 `progressToken` 时）；其它工具无进度通知
- 出现 `kind: "auth_expired"` 或 `endpoint_stale` 时，Agent 应建议用户重新导入 cURL（即调用 `setup_from_curl` 工具或让用户跑 `x_likes_downloader setup --curl-file`）
- 出现 `kind: "tweet_unavailable"` 时（仅 `fetch_tweet`），Agent 应告知用户该推文不存在 / 已删除 / 受保护或当前凭据不可见，不应重试
- `fetch_tweet` 成功但 `tweet.media` 为空数组时，Agent 应提示用户该推文可能无媒体、或其视频为 HLS-only（当前不支持下载），而非直接把空 `media[]` 传给 `download_media` 后静默无结果
- 出现 `kind: "binary_missing"` 时，Agent 应将用户引导到 GitHub Releases 安装页
- Agent 必须依信封 `kind` 而非进程退出码区分错误类别（退出码是 `exit_code()` 把多个 `kind` 压成的 2 档粗判，不足以区分如 `tweet_unavailable` 与 `invalid_argument`）

SOT SKILL.md 禁止描述"通过 spawn `xld <subcmd> --json` CLI 进程"作为 Agent 调用形态——v1 的 stdout JSON envelope 协议在 v2+ 中**仅作为人类 CLI 接口存在**，不被 Agent 使用。

#### 场景:约定可被自动化校验
- **当** 解析 SOT SKILL.md 中的调用约定章节
- **那么** 必须包含 MCP 协议调用形态、CallToolResult isError 错误模型、progressToken 进度协议、典型错误恢复路径四类信息

#### 场景:不再描述 spawn-CLI 形态作为 Agent 接口
- **当** 解析 SOT SKILL.md 全文
- **那么** 任何 Agent 调用约定章节必须不出现"spawn xld 子命令"、"--json"、"stdout JSON envelope"等 v1 描述；这些表述若出现仅能作为"人类 CLI 用户参考"出现

### 需求:`auth_status` 使用规范

SOT SKILL.md 必须明确告知 Agent，`auth_status` 工具执行实际网络请求（约 200ms），不应被频繁调用。文档必须列出且仅列出以下三种允许调用 `auth_status` 的场景：

1. 会话起始的预检（最多一次）
2. 其它工具返回 `auth_expired` 或 `endpoint_stale` 后的确认
3. 用户显式询问凭据状态

SOT SKILL.md 必须明确禁止以下使用模式：在循环中调用、在每次工具调用前作为预检调用、为做缓存而连续调用。

#### 场景:Skill 文档含 auth_status 使用规范
- **当** 阅读 `packaging/skill/x_likes/SKILL.md`
- **那么** 必须存在专门一节描述上述三种允许场景与禁止模式

### 需求:`download_media` 分批使用建议

SOT SKILL.md 必须建议 Agent 在预期下载量较大时（item 数量超过 20 或预计总字节数超过 50 MB）将下载分成多次 `download_media` 调用，每批 5-10 个 item，以便 MCP `notifications/progress` 进度事件能驱动用户可见的分阶段反馈，并避免单次调用阻塞过久。

#### 场景:Skill 文档含分批建议
- **当** 阅读 `packaging/skill/x_likes/SKILL.md`
- **那么** 必须存在分批使用建议，明示触发分批的阈值与建议批次大小，并明确进度反馈走 MCP progress 通道

### 需求:`defaults.json` 字段定义

`packaging/skill/x_likes/defaults.json`（SOT defaults）必须为合法 JSON，包含且仅包含以下字段：

- `likes_api_url`（字符串，X GraphQL `Likes` 端点完整 URL）
- `likes_features`（字符串，features JSON 序列化文本，沿用 X Web 当前值）
- `likes_fieldtoggles`（字符串，fieldToggles JSON 序列化文本）
- `tweet_detail_api_url`（字符串，X GraphQL `TweetDetail` 端点完整 URL）
- `tweet_features`（字符串，TweetDetail features JSON 序列化文本，沿用 X Web 当前值）
- `tweet_fieldtoggles`（字符串，TweetDetail fieldToggles JSON 序列化文本）
- `bearer_token`（字符串，X Web 公开 bearer，可选——若省略则从代码硬编码兜底读取）
- `schema_version`（数字，初版为 1）

上述字段中，**必填**为 `schema_version` / `likes_api_url` / `likes_features` / `likes_fieldtoggles` / `tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles`（共 7 个），`bearer_token` 为唯一可选字段。`tweet_detail_api_url` / `tweet_features` / `tweet_fieldtoggles` 经 `resolve_protocol_field` 解析（env > private_tokens > defaults.json > 硬编码），其中 defaults.json 承载真值、`config.rs` 硬编码层仅作非权威兜底（镜像 `likes_*`：URL 兜底为有效短 URL，features/fieldtoggles 兜底为 `{}`）；`setup_from_curl` 不写 `TWEET_*`，故 private_tokens 层对这三字段为空。任何 required∪optional 之外的字段必须被 `scripts/check-packaging.sh` 校验拒绝。各 host adapter（`packaging/{claude-code,codex,openclaw}/`）若需要 defaults.json，必须直接复用 SOT 文件路径或由 `sync-skill.sh` 同步派生，禁止独立维护。

#### 场景:JSON 合法且字段集封闭
- **当** 解析 `packaging/skill/x_likes/defaults.json`
- **那么** 7 个必填键必须全部存在，且顶层键集合必须为 required∪{bearer_token} 的子集，无此范围外的字段

#### 场景:含 TweetDetail 协议字段
- **当** 解析 `defaults.json`
- **那么** 必须包含 `tweet_detail_api_url`、`tweet_features`、`tweet_fieldtoggles` 三个字符串字段

#### 场景:版本号存在
- **当** 解析 `defaults.json`
- **那么** 必须包含 `schema_version` 字段且其值为大于等于 1 的整数

### 需求:`README.md` 用户旅程覆盖

SOT `packaging/skill/x_likes/README.md` 必须以陌生用户视角描述完整安装与首次使用流程的核心步骤（host-agnostic 部分），至少覆盖：

1. 检测或安装 `x_likes_downloader` 二进制（含 macOS / Linux / Windows 的下载命令片段，链接到 GitHub Releases）
2. 在 X 网页登录后从浏览器抓取 `Likes` GraphQL 请求的 cURL，并保存到文件
3. 运行 `x_likes_downloader setup --curl-file <path>` 导入凭据与协议参数
4. （可选）通过 `x_likes_downloader setup --download-dir <path>` 自定义沙箱 base dir
5. 通过 Agent 调用 `auth_status` 工具验证配置生效；亦可手动跑 `x_likes_downloader auth status --json` 做离线 sanity check

host-specific 注册步骤（如何在 Claude Code / Codex CLI / OpenClaw 等客户端注册 MCP server）必须由各 host adapter 的 README（`packaging/<host>/README.md`）描述，**不**在 SOT README 中维护。SOT README 必须明示 ToS 边界由用户承担、凭据本地化、不入仓库等关键约束。

每个 host adapter 的 README（claude-code / codex / openclaw 至少三家）必须包含至少以下内容：

- 一行装命令（marketplace `add` + `plugin install` 或 ClawHub 注册等价命令）
- 该 host 下 SKILL.md 的最终落地路径
- binary 安装前置说明（链接到 GitHub Releases / `cargo install` / `brew tap`）

#### 场景:SOT README 含核心步骤
- **当** 阅读 `packaging/skill/x_likes/README.md`
- **那么** 上述五个核心步骤必须全部出现且顺序合理

#### 场景:host adapter README 含一行装命令
- **当** 阅读 `packaging/{claude-code,codex,openclaw}/README.md`（任意一个）
- **那么** 必须包含至少一条 marketplace 注册或 plugin install 命令、SKILL.md 落地路径、binary 安装前置说明

#### 场景:SOT README 明示边界
- **当** 阅读 SOT README
- **那么** 必须包含关于 X ToS、凭据本地化、不入仓库的明确声明

### 需求:Skill 与 binary 版本绑定

SOT `packaging/skill/x_likes/SKILL.md` frontmatter 与所有 host adapter manifest（plugin.json、mcp-config.json、marketplace.json）必须随同 `x_likes_downloader` binary 在同一 git tag / GitHub Release 中发布，禁止独立发版。

SOT SKILL.md 必须声明 `min_binary_version` 字段（形如 `min_binary_version: 2.1.0`）。所有 host adapter manifest 中的版本字段——`packaging/{claude-code,codex}/. <host>-plugin/plugin.json` 的 `version`、`packaging/openclaw/x_likes/mcp-config.json` 的 `minimum_xld_version`、`.claude-plugin/marketplace.json` 的 `metadata.version`——必须由 `scripts/version.sh` 在升级 `Cargo.toml` 时同步更新到一致值。

`scripts/check-packaging.sh` 必须验证所有版本字段一致；不一致时 fail。

#### 场景:版本依赖声明（SOT）
- **当** 解析 SOT SKILL.md frontmatter
- **那么** 必须包含 `min_binary_version` 字段且其值符合 SemVer

#### 场景:版本字段同步
- **当** 运行 `bash scripts/version.sh patch`（或 minor / major / 显式版本号）
- **那么** Cargo.toml、所有 plugin.json、所有 marketplace.json、所有 mcp-config.json 的 version 字段必须更新到同一值

#### 场景:check 检测漂移
- **当** 手工修改 `Cargo.toml` 版本但未运行 `version.sh`
- **那么** `bash scripts/check-packaging.sh` 退出码必须非 0，输出必须列出所有不一致的版本字段路径

### 需求:`mcp-config.json` 字段定义

每个使用 stdio MCP server 启动配置的 host adapter 必须维护一份合法 JSON 描述如何启动 `x_likes_downloader serve --mcp`。该文件位置因 host 而异：

- **OpenClaw**：`packaging/openclaw/x_likes/mcp-config.json`（保 v2.0 兼容）
- **Claude Code**：`packaging/claude-code/.mcp.json`（被 `.claude-plugin/plugin.json.mcpServers` 字段引用）
- **Codex CLI**：`packaging/codex/.mcp.json`（被 `.codex-plugin/plugin.json.mcpServers` 字段引用）

各 host 的 MCP 启动配置文件必须包含且仅包含以下字段：

- `command`（字符串，必填，值为 `"x_likes_downloader"` 或绝对路径）
- `args`（字符串数组，必填，值为 `["serve", "--mcp"]`）
- `transport`（字符串，必填，值为 `"stdio"`）
- `minimum_xld_version`（字符串，OpenClaw mcp-config.json 必填；Claude Code / Codex `.mcp.json` 可省略，因为 plugin.json 自身有 version 字段）

任何额外字段必须被 `scripts/check-packaging.sh` 校验拒绝。这些文件**不得**包含 cookies / bearer / 任何用户敏感字段，因其会被打包进公开 plugin / skill 仓库。

#### 场景:OpenClaw mcp-config.json 字段集封闭
- **当** 解析 `packaging/openclaw/x_likes/mcp-config.json`
- **那么** 顶层键集合必须为上述列表的子集，无未声明字段；必须含 `minimum_xld_version`

#### 场景:Claude Code .mcp.json 与 Codex .mcp.json 一致
- **当** 比较 `packaging/claude-code/.mcp.json` 与 `packaging/codex/.mcp.json`
- **那么** `command` / `args` / `transport` 三个字段必须相等

#### 场景:无敏感字段
- **当** 在任何 host 的 MCP 启动配置文件中查找 `auth_token` / `ct0` / `bearer_token` / `personalization_id` 等
- **那么** 这些字段必须不存在

#### 场景:命令与 transport 一致
- **当** 解析任意 host 的 MCP 启动配置文件
- **那么** `command` 必须为 `"x_likes_downloader"`，`args[0]` 必须为 `"serve"`，`args` 必须含 `"--mcp"`，`transport` 必须为 `"stdio"`
