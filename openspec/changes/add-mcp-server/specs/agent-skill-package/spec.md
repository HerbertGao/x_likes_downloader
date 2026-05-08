## 修改需求

### 需求:仓库新增 `skill/` 目录结构

系统必须在仓库根目录新增 `skill/` 目录，包含以下文件：

- `skill/SKILL.md`：Agent 工具表与调用约定（中文）
- `skill/defaults.json`：公开协议参数兜底（无敏感字段）
- `skill/README.md`：陌生用户安装与配置指引（中文）
- `skill/mcp-config.json`：MCP server 启动配置（让 Agent 客户端知道如何启动 `xld serve --mcp`）

`skill/` 目录禁止包含任何用户私密字段、二进制可执行文件、或 Rust 源码。它必须是平铺目录，不进入 Cargo workspace。

#### 场景:目录结构存在
- **当** 在主分支上 `ls skill/`
- **那么** 必须看到 `SKILL.md`、`defaults.json`、`README.md`、`mcp-config.json` 四个文件

#### 场景:无敏感字段
- **当** 在 `skill/defaults.json` 或 `skill/mcp-config.json` 中查找 `auth_token` / `ct0` / `bearer_token` 等字段
- **那么** 这些字段必须不存在

### 需求:`SKILL.md` 调用约定

`SKILL.md` 必须明确以下 Agent 调用约定：

- 所有工具的调用形态：通过 MCP 协议的 `tools/call` 调用工具名 `list_likes` / `download_media` / `auth_status` / `setup_from_curl`，由客户端建立的 stdio MCP 连接转发到 `xld serve --mcp` 子进程
- 业务错误以 `CallToolResult { isError: true }` 返回，content 含结构化 `kind` / `message` / `hint` 字段；`tools/call` 不应抛 JSON-RPC level error 除非真协议异常
- `download_media` 的进度通过 MCP `notifications/progress` 推送（仅在请求含 `progressToken` 时）；其它工具无进度通知
- 出现 `kind: "auth_expired"` 或 `endpoint_stale` 时，Agent 应建议用户重新导入 cURL（即调用 `setup_from_curl` 工具或让用户跑 `xld setup --curl-file`）
- 出现 `kind: "binary_missing"` 时，Agent 应将用户引导到 GitHub Releases 安装页

`SKILL.md` 禁止描述"通过 spawn `xld <subcmd> --json` CLI 进程"作为 Agent 调用形态——v1 的 stdout JSON envelope 协议在 v2 中**仅作为人类 CLI 接口存在**，不被 Agent 使用。

#### 场景:约定可被自动化校验
- **当** 解析 `SKILL.md` 中的调用约定章节
- **那么** 必须包含 MCP 协议调用形态、CallToolResult isError 错误模型、progressToken 进度协议、典型错误恢复路径四类信息

#### 场景:不再描述 spawn-CLI 形态作为 Agent 接口
- **当** 解析 `SKILL.md` 全文
- **那么** 任何 Agent 调用约定章节必须不出现"spawn xld 子命令"、"--json"、"stdout JSON envelope"等 v1 描述；这些表述若出现仅能作为"人类 CLI 用户参考"出现

### 需求:`download_media` 分批使用建议

`SKILL.md` 必须建议 Agent 在预期下载量较大时（item 数量超过 20 或预计总字节数超过 50 MB）将下载分成多次 `download_media` 调用，每批 5-10 个 item，以便 MCP `notifications/progress` 进度事件能驱动用户可见的分阶段反馈，并避免单次调用阻塞过久。

#### 场景:Skill 文档含分批建议
- **当** 阅读 `skill/SKILL.md`
- **那么** 必须存在分批使用建议，明示触发分批的阈值与建议批次大小，并明确进度反馈走 MCP progress 通道

### 需求:`README.md` 用户旅程覆盖

`skill/README.md` 必须以陌生用户视角描述完整安装与首次使用流程，至少覆盖以下步骤：

1. 检测或安装 `xld` 二进制（含 macOS / Linux / Windows 的下载命令片段，链接到 GitHub Releases）
2. 在 X 网页登录后从浏览器抓取 `Likes` GraphQL 请求的 cURL，并保存到文件
3. 运行 `xld setup --curl-file <path>` 导入凭据与协议参数
4. （可选）通过 `xld setup --download-dir <path>` 自定义沙箱 base dir
5. 在客户端注册 MCP server——同时给出至少两种主流客户端的配置示例：
   - Claude Code：在 `~/.claude/settings.json` 或项目 `.mcp.json` 加 `mcpServers` 段，command 为 `xld`、args 为 `["serve", "--mcp"]`
   - OpenClaw：通过 `mcporter` 注册或在配置文件中加同等内容
6. 通过 Agent 调用 `auth_status` 工具验证配置生效；亦可手动跑 `xld auth status --json` 做离线 sanity check

`README.md` 必须明示 ToS 边界由用户承担、凭据本地化、不入仓库等关键约束。

#### 场景:README 包含全部关键步骤
- **当** 阅读 `skill/README.md`
- **那么** 上述六个步骤必须全部出现且顺序合理；第 5 步必须含至少 Claude Code 与 OpenClaw 两种客户端的 MCP server 配置示例

#### 场景:README 明示边界
- **当** 阅读 `skill/README.md`
- **那么** 必须包含关于 X ToS、凭据本地化、不入仓库的明确声明

## 新增需求

### 需求:`mcp-config.json` 字段定义

`skill/mcp-config.json` 必须为合法 JSON，描述如何启动 `xld` 的 MCP server 模式。该文件必须包含：

- `command`（字符串，必填，值为 `"xld"` 或绝对路径）
- `args`（字符串数组，必填，值为 `["serve", "--mcp"]`）
- `transport`（字符串，必填，值为 `"stdio"`）
- `minimum_xld_version`（字符串，必填，遵循 SemVer，初版为 `"2.0.0"`）

任何额外字段必须被 CI 校验拒绝。该文件**不得**包含 cookies / bearer / 任何用户敏感字段，因其会被打包进公开 skill 仓库。

#### 场景:文件存在且字段集封闭
- **当** 解析 `skill/mcp-config.json`
- **那么** 顶层键集合必须为上述列表的子集，无未声明字段

#### 场景:无敏感字段
- **当** 在 `skill/mcp-config.json` 中查找 `auth_token` / `ct0` / `bearer_token` / `personalization_id` 等
- **那么** 这些字段必须不存在

#### 场景:命令与 transport 一致
- **当** 解析 `mcp-config.json`
- **那么** `command` 必须为 `"xld"`，`args[0]` 必须为 `"serve"`，`args` 必须含 `"--mcp"`，`transport` 必须为 `"stdio"`
