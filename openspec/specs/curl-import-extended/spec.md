## 目的

扩展 `xld setup` 的 cURL 解析，在认证字段之外同时提取 `Likes` 端点的协议参数（API URL、features、fieldToggles）写入本地配置，使本地 override 公开默认值，并对非 Likes 端点的 cURL 拒绝导入。

## 需求

### 需求:cURL 解析必须同时提取协议参数

`xld setup` 在解析用户提供的 cURL 命令时，除现有的认证字段（`auth_token`、`ct0`、`bearer_token`、`user_agent`、`user_id`）外，必须额外提取以下协议参数并写入本地用户配置：

- `likes_api_url`：cURL 中 `Likes` GraphQL 请求的完整基础 URL（含 queryId 路径段，去除 query string）
- `likes_features`：URL query 参数中 `features` 的解码后 JSON 字符串
- `likes_fieldtoggles`：URL query 参数中 `fieldToggles` 的解码后 JSON 字符串

若 cURL 命令不属于 `Likes` 端点（无法识别 `Likes` 路径段），系统必须提示用户重新捕获正确的请求并以退出码 1 退出，禁止以错误数据写入配置。

#### 场景:从 Likes 端点 cURL 提取协议参数
- **当** 用户提供的 cURL 命令 URL 形如 `https://x.com/i/api/graphql/<queryId>/Likes?variables=...&features=...&fieldToggles=...`
- **那么** 系统将 `https://x.com/i/api/graphql/<queryId>/Likes` 写入 `likes_api_url`，将解码后的 features / fieldToggles JSON 写入对应字段

#### 场景:非 Likes 端点 cURL 拒绝
- **当** 用户提供的 cURL 命令 URL 不包含 `/Likes` 路径段
- **那么** 系统输出说明性错误并退出码 1，本地配置不被修改

### 需求:本地配置 override 公开默认值

系统启动时加载 `likes_api_url` / `likes_features` / `likes_fieldtoggles` 的优先级必须为：环境变量 / `.env` > 用户本地配置（由 setup 写入） > `skill/defaults.json`（vendored 公开默认） > 代码硬编码兜底。任何更高优先级的来源若提供了某字段，必须完整覆盖更低优先级的同名字段（不做合并）。

#### 场景:用户本地配置覆盖 vendored 默认
- **当** 用户已通过 `xld setup` 写入本地 `likes_api_url`，且 `skill/defaults.json` 也含该字段
- **那么** 运行时实际使用的值必须为用户本地配置中的值

#### 场景:无本地配置时 fallback 到 vendored
- **当** 用户尚未导入 cURL（本地配置无 `likes_api_url`）
- **那么** 运行时使用 `skill/defaults.json` 中的值（仅供首次试用，缺乏认证字段时 `auth_status` 仍会返回 `not_configured`）

### 需求:vendored defaults.json 不含敏感字段

`skill/defaults.json` 文件禁止包含任何用户私密字段（`auth_token`、`ct0`、`user_id`、`user_agent`）。该文件必须仅包含协议层公开字段：`likes_api_url`、`likes_features`、`likes_fieldtoggles`，可选包含 `bearer_token`（X Web 公开 bearer 字符串）。CI 检查必须验证此约束。

#### 场景:CI 拒绝包含敏感字段的 defaults.json
- **当** PR 修改 `skill/defaults.json` 引入了 `auth_token` 或 `ct0` 字段
- **那么** CI 必须失败并明确指出违反"vendored defaults 禁止含敏感字段"规则

### 需求:setup 增强对人类用户向后兼容

`xld setup` 的命令行参数（如 `--curl-file`）和成功输出格式对现有人类用户必须保持向后兼容。新增的协议参数提取必须默默完成，不引入额外的必填参数。

#### 场景:旧脚本运行不变
- **当** 用户使用与本变更前完全相同的 `xld setup --curl-file my.txt` 调用
- **那么** 命令成功完成且写入的本地配置除了多出 url/features/fieldToggles 三个字段外，其它字段含义与旧版本一致

### 需求:协议兜底值新鲜度与单源性

vendored `defaults.json` 与 `config.rs` 硬编码层共同构成协议端点兜底（四层解析的第 3、4 层）。任何一次刷新其中任一协议端点 queryId 的变更，必须满足：

- **新鲜度**：被修改的 `*_api_url` 兜底值必须以有效登录态实测可用（完整 URL 发起 GraphQL GET 返回 HTTP 200），实测证据记录于该变更的 proposal 或 tasks；X 已轮换而实测仍可用的旧 queryId 允许暂存于用户层（`private_tokens.env`），但 vendored 兜底不得落后于已知现役值。
- **单源性**：同一端点在 defaults.json 与 `config.rs` 硬编码兜底中的 queryId 必须在同一变更内同步为相同值，禁止两层各自漂移。

#### 场景:兜底刷新须实测

- **当** 一次变更修改了 defaults.json 中任一 `*_api_url` 的 queryId 且准备合入
- **那么** 该变更必须附带对修改后完整 URL 的实测记录（有效登录态下返回 HTTP 200），否则不得合入

#### 场景:硬编码兜底与 vendored 同步

- **当** 一次变更刷新了某端点在 defaults.json 中的 queryId
- **那么** `config.rs` 中同字段的硬编码兜底 URL 必须在同一变更中改为相同 queryId

#### 场景:双源一致可被校验

- **当** 对照 `config.rs` 各协议字段的硬编码兜底 URL 与 defaults.json 同名字段
- **那么** 两者必须指向相同 operation 与相同 queryId 路径段
