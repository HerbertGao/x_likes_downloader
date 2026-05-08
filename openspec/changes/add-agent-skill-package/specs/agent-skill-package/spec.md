## 新增需求

### 需求:仓库新增 `skill/` 目录结构

系统必须在仓库根目录新增 `skill/` 目录，包含以下文件：

- `skill/SKILL.md`：Agent 工具表与调用约定（中文）
- `skill/defaults.json`：公开协议参数兜底（无敏感字段）
- `skill/README.md`：陌生用户安装与配置指引（中文）

`skill/` 目录禁止包含任何用户私密字段、二进制可执行文件、或 Rust 源码。它必须是平铺目录，不进入 Cargo workspace。

#### 场景:目录结构存在
- **当** 在主分支上 `ls skill/`
- **那么** 必须看到 `SKILL.md`、`defaults.json`、`README.md` 三个文件

#### 场景:无敏感字段
- **当** 在 `skill/defaults.json` 中查找 `auth_token` 或 `ct0` 字段
- **那么** 这些字段必须不存在

### 需求:`SKILL.md` 工具表声明

`skill/SKILL.md` 必须以一节明确声明 Agent 可调用的工具集，每个工具至少包含：名称、用途一句话、输入参数及类型、返回值结构概述、典型错误 `kind` 列表。声明的工具集必须为且仅为以下四个：

- `list_likes(count?, all?, since_cursor?)` → 列点赞
- `download_media(tweet_ids[], subdir?)` → 按 ID 下载到沙箱
- `auth_status()` → 凭据健康自检
- `setup_from_curl(curl_text)` → 首次配置 / 重新导入 cURL

`SKILL.md` 必须明确声明 `organize` 与旧 `download` 一把梭**不在**Agent 工具表内。

#### 场景:工具表完整
- **当** 解析 `SKILL.md` 中的工具声明
- **那么** 必须找到上述四个工具且仅这四个，每个工具均包含名称、用途、参数、返回值、错误种类五要素

#### 场景:明示不暴露的工具
- **当** 解析 `SKILL.md`
- **那么** 必须存在一段说明明示 `organize` 与旧 `download` 不在 Agent 暴露面，并解释原因

### 需求:`SKILL.md` 调用约定

`SKILL.md` 必须明确以下 Agent 调用约定：

- 所有工具的调用形态：通过 spawn `xld <subcommand> --json` CLI 进程拿 stdout JSON
- stdout 必须按 JSON 信封解析，不接受任意文本
- `download_media` 的 stderr 为 NDJSON 进度事件流（schema 见 `media-download-by-items` 能力），其它工具的 stderr 仅含诊断信息，可不解析
- 退出码 0 = 成功，2 = 可重试错误，1 = 不可恢复错误
- 出现 `error.kind: "auth_expired"` 或 `endpoint_stale` 时，Agent 应建议用户重新导入 cURL（即调用 `setup_from_curl`）
- 出现 `error.kind: "binary_missing"` 时，Agent 应将用户引导到 GitHub Releases 安装页

#### 场景:约定可被自动化校验
- **当** 解析 `SKILL.md` 中的调用约定章节
- **那么** 必须包含 stdout JSON 协议、stderr 进度事件协议、退出码语义、典型错误恢复路径四类信息

### 需求:`auth_status` 使用规范

`SKILL.md` 必须明确告知 Agent，`auth_status` 工具执行实际网络请求（约 200ms），不应被频繁调用。文档必须列出且仅列出以下三种允许调用 `auth_status` 的场景：

1. 会话起始的预检（最多一次）
2. 其它工具返回 `auth_expired` 或 `endpoint_stale` 后的确认
3. 用户显式询问凭据状态

`SKILL.md` 必须明确禁止以下使用模式：在循环中调用、在每次工具调用前作为预检调用、为做缓存而连续调用。

#### 场景:Skill 文档含 auth_status 使用规范
- **当** 阅读 `skill/SKILL.md`
- **那么** 必须存在专门一节描述上述三种允许场景与禁止模式

### 需求:`download_media` 分批使用建议

`SKILL.md` 必须建议 Agent 在预期下载量较大时（item 数量超过 20 或预计总字节数超过 50 MB）将下载分成多次 `download_media` 调用，每批 5-10 个 item，以便 stderr 进度事件能驱动用户可见的分阶段反馈，并避免单次调用阻塞过久。

#### 场景:Skill 文档含分批建议
- **当** 阅读 `skill/SKILL.md`
- **那么** 必须存在分批使用建议，明示触发分批的阈值与建议批次大小

### 需求:`defaults.json` 字段定义

`skill/defaults.json` 必须为合法 JSON，包含且仅包含以下字段：

- `likes_api_url`（字符串，X GraphQL `Likes` 端点完整 URL）
- `likes_features`（字符串，features JSON 序列化文本，沿用 X Web 当前值）
- `likes_fieldtoggles`（字符串，fieldToggles JSON 序列化文本）
- `bearer_token`（字符串，X Web 公开 bearer，可选——若省略则从代码硬编码兜底读取）
- `schema_version`（数字，初版为 1）

任何额外字段必须被 CI 校验拒绝。

#### 场景:JSON 合法且字段集封闭
- **当** 解析 `skill/defaults.json`
- **那么** 顶层键集合必须为上述列表的子集，无未声明字段

#### 场景:版本号存在
- **当** 解析 `defaults.json`
- **那么** 必须包含 `schema_version` 字段且其值为大于等于 1 的整数

### 需求:`README.md` 用户旅程覆盖

`skill/README.md` 必须以陌生用户视角描述完整安装与首次使用流程，至少覆盖以下步骤：

1. 检测或安装 `xld` 二进制（含 macOS / Linux / Windows 的下载命令片段，链接到 GitHub Releases）
2. 在 X 网页登录后从浏览器抓取 `Likes` GraphQL 请求的 cURL，并保存到文件
3. 运行 `xld setup --curl-file <path>` 导入凭据与协议参数
4. （可选）通过 `xld setup --download-dir <path>` 自定义沙箱 base dir
5. 在 OpenClaw / Claude Code 中注册本 skill 目录
6. 通过 `xld auth status --json` 验证配置生效

`README.md` 必须明示 ToS 边界由用户承担、凭据本地化、不入仓库等关键约束。

#### 场景:README 包含全部关键步骤
- **当** 阅读 `skill/README.md`
- **那么** 上述六个步骤必须全部出现且顺序合理

#### 场景:README 明示边界
- **当** 阅读 `skill/README.md`
- **那么** 必须包含关于 X ToS、凭据本地化、不入仓库的明确声明

### 需求:Skill 与 binary 版本绑定

`skill/SKILL.md` 与 `skill/defaults.json` 必须随同 `xld` binary 在同一 git tag / GitHub Release 中发布，禁止独立发版。`SKILL.md` 中必须声明该 skill 与 binary 的最低兼容版本（形如 `xld >= x.y.z`）。

#### 场景:版本依赖声明
- **当** 解析 `SKILL.md`
- **那么** 必须包含一行明确的"`xld` 最低版本要求"声明
