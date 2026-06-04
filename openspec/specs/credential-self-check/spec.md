## 目的

提供 `xld auth status` 子命令，通过一次轻量真实请求探测本地凭据是否仍能访问 X GraphQL `Likes` 端点，并以分类的 JSON 信封区分健康、认证过期、端点过期、速率限制等状态。

## 需求

### 需求:CLI 子命令 `xld auth status`

系统必须提供 `xld auth status` 子命令，用于探测当前本地凭据是否仍可用于访问 X GraphQL `Likes` 端点。该子命令必须支持 `--json` 输出 JSON 信封到 stdout。其行为必须包含一次轻量真实请求（`count=1`，不翻页），而非仅做字段静态校验。

#### 场景:健康
- **当** 本地凭据完整且 X 返回 200 并能解析出 timeline 结构
- **那么** stdout 输出 `{ ok: true, data: { status: "healthy", checked_at: "..." } }`，退出码为 0

#### 场景:认证过期
- **当** X 返回 401 或 403
- **那么** stdout 输出 `{ ok: false, error: { kind: "auth_expired", hint: "..." } }`，退出码为 2

#### 场景:端点过期
- **当** X 返回 404 或 410（GraphQL queryId 已滚动）
- **那么** stdout 输出 `{ ok: false, error: { kind: "endpoint_stale", hint: "..." } }`，退出码为 2

#### 场景:速率限制
- **当** X 返回 429
- **那么** stdout 输出 `{ ok: false, error: { kind: "rate_limited", retry_after: <秒数或 null> } }`，退出码为 2

#### 场景:网络错误
- **当** 请求因 DNS / 连接 / 超时等网络层原因失败
- **那么** stdout 输出 `{ ok: false, error: { kind: "network_error", message: "..." } }`，退出码为 2

#### 场景:配置缺失
- **当** 本地未导入过 cURL，凭据字段为空或不完整
- **那么** 系统不发起网络请求，stdout 直接输出 `{ ok: false, error: { kind: "not_configured", hint: "请运行 xld setup" } }`，退出码为 1

### 需求:错误诊断的 hint 字段必须可执行

`error.hint` 字段必须给出用户/Agent 可立即采取的下一步操作，且必须为字符串。`auth_expired` 和 `endpoint_stale` 的 hint 必须明确指向"重新导入 cURL"流程；`not_configured` 的 hint 必须指向 `xld setup`。

#### 场景:hint 引导重导 cURL
- **当** 错误为 `auth_expired` 或 `endpoint_stale`
- **那么** `error.hint` 字符串必须包含可识别的"重新导入 cURL"或"xld setup"等引导

### 需求:lib 层函数 `auth_status`

系统必须在 lib crate 中暴露 `auth_status() -> Result<AuthStatus>` 异步函数，返回的 `AuthStatus` 类型必须为枚举，覆盖 `Healthy`、`AuthExpired`、`EndpointStale`、`RateLimited { retry_after }`、`NetworkError { message }`、`NotConfigured` 六种状态。函数禁止直接写 stdout/stderr。

#### 场景:lib 函数返回结构化状态
- **当** 调用 `auth_status()` 时本地凭据缺失
- **那么** 函数返回 `AuthStatus::NotConfigured`，不发起任何网络请求

#### 场景:lib 函数与 CLI 共享实现
- **当** CLI 子命令与未来 MCP server 同时存在
- **那么** 两者必须最终调用同一份 `auth_status` 实现，禁止存在第二份等价逻辑

### 需求:无缓存策略

`auth_status` 默认不缓存探测结果。每次调用必须发起一次新的真实请求（除非状态判定不需要请求，如 `not_configured`）。

#### 场景:连续两次调用各发起一次请求
- **当** 同一进程或同一 Skill 会话内连续调用 `auth_status` 两次
- **那么** 必须实际产生两次 HTTP 请求（除非首次即判定为 `not_configured`）
