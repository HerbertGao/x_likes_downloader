## 新增需求

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
