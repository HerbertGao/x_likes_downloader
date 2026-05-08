## 上下文

`x_likes_downloader` 当前是单 binary 的 Rust CLI，所有逻辑写在 `main.rs` 里直接调用 `x_api.rs` / `downloader.rs` 等模块。它面向人类用户：用户运行 `xld setup` 导入 cURL，再跑 `xld download` 一键完成"列点赞 + 下媒体"。

本变更要把这套能力暴露给 AI Agent。Agent 化场景的根本约束是：**stdout 必须是结构化的、状态变迁要可被外部观察、所有"我自己玩玩"的边界（路径、状态、状态机）必须被沙箱化**。当前代码不满足任何一条——`x_api.rs:52,70,83` 直接 `println!` 调试信息，`download` 把 list+下载耦合，下载目录默认 `./downloads` 相对路径。

利益相关者：
- **现有人类 CLI 用户**：不能让他们的工作流断掉
- **未来 Skill 装机用户**（OpenClaw / Claude Code）：陌生人，凭据本地化，安装即用
- **未来 MCP 集成方**（Hermes / Cursor 等）：v2 才接入，但 lib 层接口需要为它预留

## 目标 / 非目标

**目标：**

1. 把 `list_likes` / `download_media` / `auth_status` / `import_curl` 抽到 lib 层，让 CLI、未来 MCP server、集成测试可以共用同一份能力实现
2. 所有新 CLI 子命令的 stdout 是**严格结构化 JSON**，调试日志走 stderr
3. 公开仓库**只承载协议默认值**（GraphQL endpoint、features/fieldToggles），**不承载任何用户私密数据**；`setup` 流程从 cURL 提取的字段 override 默认值
4. 下载路径**沙箱化**：base dir 由用户配置（A3），默认值由 lib 选定为平台标准目录（A2）；Agent 只能传子目录名，不能传绝对路径或 `..`
5. 提供一个**最小可用的 Skill 包**（`skill/SKILL.md` + `skill/defaults.json` + `skill/README.md`），陌生用户跟着 README 装 binary + 导 cURL 后即可被 Agent 驱动
6. **向后兼容**：`xld download` / `xld organize` / `xld update` / `xld setup` 行为对人类用户不变

**非目标：**

- 不实施 MCP server（v2 范围，本次仅在 lib API 设计上不阻塞将来添加）
- 不实施跨账号、多用户、共享 daemon
- 不引入主动节流 / 速率限制（依赖现有翻页节奏）
- 不删除 `organize` / `update` 子命令；不在 Skill 工具表里暴露它们
- 不做 binary 自动分发（不下载二进制、不打包到 skill 仓库），仅文档指引
- 不为 `download_media` 维护"已见过 ID"的缓存校验（B1 信任 cookie 边界）

## 决策

### D1：双 crate 化（lib + bin），lib 是 SSOT

**选择**：在 `Cargo.toml` 里同时声明 `[lib]` 和 `[[bin]]`，所有真正的能力实现搬到 `src/lib.rs` 暴露的模块树（`xld::api` / `xld::download` / `xld::auth` / `xld::setup` / `xld::sandbox`）。`src/main.rs` 退化为薄薄的 clap 路由，每个子命令只做"解析参数 → 调 lib → 把返回值序列化到 stdout"。

**理由**：

- v2 的 MCP server 必然要复用同一份 `list_likes` 实现。如果 v1 把逻辑留在 main.rs 里，v2 要么 fork 子进程调自己（丑且慢），要么重复实现（漂移）
- lib 化是把"业务逻辑"和"输出协议"剥离的强迫机制——一旦能力是返回 `Result<Vec<Tweet>>` 的纯函数，它就天然不会 `println!` 了
- 测试性提升：现有 `organize_files_test.rs` 之外，可为 lib 函数加单元测试

**替代方案**：

- 保持 bin-only，MCP 时再 fork 子进程：被否决，原因如上
- 把 lib 拆成独立 crate（workspace）：过度工程，单 crate 双 target 已足够

### D2：JSON 输出协议——单根对象 + 顶层 ok 字段

**选择**：所有 `--json` 子命令的 stdout 输出**单个 JSON 对象**（不是 NDJSON、不是数组），形如：

```json
{ "ok": true, "data": { ... }, "meta": { "schema_version": 1, "cursor": "..." } }
```

或失败时：

```json
{ "ok": false, "error": { "kind": "auth_expired", "message": "...", "hint": "重新导入 cURL" } }
```

退出码：成功 0；可由 Agent 主动重试的错误（认证过期、X 限流）退 2；不可恢复的（参数错、配置缺失）退 1。

**理由**：

- Agent 拿到 stdout 后第一件事是 `JSON.parse`——单对象比 NDJSON 更稳，不会因翻页过程中报错半路输出半个流
- `ok` 字段让 Agent 不必依赖退出码也能分支决策（很多 MCP 客户端拿不到退出码）
- `error.kind` 用枚举而非自由文本，便于将来 MCP 工具映射到结构化错误
- `meta.schema_version` 留给将来格式演进

**替代方案**：

- NDJSON 流式输出：被否决——Agent 端解析复杂度上升，错误半行问题难处理
- 直接输出 X 原始 GraphQL JSON：被否决——schema 不稳，且暴露 X 内部结构会让 skill 用户依赖将来会变的字段

### D3：公开默认值放 `skill/defaults.json`，运行时与本地配置三层合并

**合并优先级**（高 → 低）：

```
.env / 环境变量  >  ~/.config/xld/config.json  >  skill/defaults.json (vendored)  >  hard-coded fallback
                          ▲                              ▲
                          │                              │
                  setup 解析 cURL 写这里         仓库公开兜底
```

`setup` 流程从 cURL 同时提取的字段：`auth_token`、`ct0`、`bearer_token`、`user_agent`、`user_id`（这些进 user config，绝不入仓）+ `likes_api_url`、`features`、`fieldToggles`（这些**也写**进 user config，覆盖 defaults.json）。

**理由**：

- X 的 GraphQL queryId 会被周期性滚动；用户重新导一次 cURL 就同步更新协议参数，不依赖 skill 仓库发版救火
- `defaults.json` 的存在让首次安装能"开箱即用"，但它只是兜底——任何用户已经导过 cURL 后就走自己的本地值
- 拆 `defaults.json` 出独立文件而非嵌进 `SKILL.md`，方便 CI 校验 / 工具脚本读取

**替代方案**：

- 把协议参数硬编码在 Rust 源码里：被否决——X 改一次就要发新 binary
- 启动时从远程 endpoint 拉最新 defaults：被否决——增加供应链风险（remote spoofing），且对离线环境不友好

### D4：沙箱 base dir 用 `dirs` crate 选平台标准目录

| 平台 | 默认 base dir |
|---|---|
| macOS | `~/Library/Application Support/xld/downloads` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/xld/downloads` |
| Windows | `%LOCALAPPDATA%\xld\downloads` |

用户可在 `setup` 时通过 `--download-dir <path>` 覆盖（A3）。`download_media(ids, subdir)` 接受可选 `subdir`，用 `Path::components()` 校验：拒绝绝对路径、拒绝 `..`、拒绝包含驱动器盘符（Windows）；解析后必须是 `base_dir` 的子路径，否则返回结构化错误。

**理由**：

- 平台标准目录更符合用户预期、不污染 cwd、卸载时易清理
- 现有的 `./downloads` 是相对 cwd——人类用户可能依赖这个行为，所以**老 `xld download` 子命令默认仍写到 `./downloads`**（保持兼容），只有新的 `xld media download --ids` 走沙箱

**替代方案**：

- `~/xld/`：被否决——污染 home，不符合 XDG / Apple 规范
- 让 Agent 自己传完整路径：被否决——路径穿越漏洞

### D5：`auth_status` 走"轻量真实请求"判定

**选择**：`auth_status` 不光检查字段是否存在，而是**实际打一次** `likes_api_url`（`count=1`，不翻页），根据 HTTP 状态码 + 响应结构判定：

- 200 + 能解析出 timeline 结构 → `healthy`
- 401 / 403 → `auth_expired`（cookie 失效，引导用户重导 cURL）
- 404 / 410 → `endpoint_stale`（X 滚了 queryId，同样引导重导 cURL）
- 429 → `rate_limited`
- 网络错误 → `network_error`
- 字段缺失 → `not_configured`

**理由**：

- 字段静态校验只能告诉用户"配置是否填全"，不能告诉用户"现在能不能用"——后者才是 Agent 排障真正需要的
- 调用一次极轻量（count=1），但能区分多种失效原因，让用户/Agent 选不同的恢复路径

**替代方案**：

- 仅做字段静态校验：信息量太低
- 调"GET /1.1/account/verify_credentials.json"：被否决——那是另一个端点的鉴权域，不能反映 likes endpoint 真实健康度

### D6：Skill 不分发 binary，README 引导用户从 GitHub Releases 安装

**选择**：`skill/README.md` 在"安装"一节写明：

1. 检查 `xld --version`，未安装则跳到第 2 步
2. 给出 macOS / Linux / Windows 的从 [Releases](https://github.com/HerbertGao/x_likes_downloader/releases) 下载的命令片段
3. `xld setup` 导入 cURL
4. 把 `skill/` 注册到 OpenClaw / Claude Code

Skill 的脚本胶水（如果有）启动时检查 `xld` 在 PATH，没有就返回结构化错误 `{ "ok": false, "error": { "kind": "binary_missing", "hint": "..." } }`。

**理由**：

- 主流 OpenClaw skill 都这么做（见 `x-research-skill`、`twclaw`），用户已习惯
- 不打包二进制避免供应链风险（若有人 fork 改坏后重发）和仓库膨胀（6 个平台 binary 大几十 MB）
- 不写自动 `install.sh` 拉远程 binary，避免"chmod + 远程脚本"的安全反模式

**替代方案**：见 proposal "binary 分发" 那一节，已讨论。

### D7：Cargo workspace 暂不引入；`skill/` 是平铺目录非 crate

**选择**：`skill/` 仅含 markdown + json + 可选 shell 脚本。它不是 Rust crate、不进 `Cargo.toml` workspace。skill 仓库内容靠 GitHub release zip 或 OpenClaw 注册指向当前 repo 子目录。

**理由**：保持仓库结构平直；skill 内容版本与 binary 版本天然绑定（同一 release tag）。

### D8：`download_media` 接受 `MediaItem[]` 而非裸 ID

**选择**：lib 层 `download_media(items: &[MediaItem], opts) -> Result<DownloadOutput>`。CLI 同时支持 `--items <json|@file>`（首选）和 `--ids <csv>`（快捷方式），互斥。`--ids` 实现为 main.rs 层先调 `list_likes` 拿到扁平 tweets 再提取 `media[]`，最终都走同一份 `download_media`。

**理由**：

- 闭环更自然：`list_likes` 输出的 `data.tweets[].media[]` 元素结构与 `MediaItem` 完全一致，Agent 可零成本转手传回
- 解耦：CDN 下载不再依赖 GraphQL 凭据状态；只要 cookie 失效后已经拿到的 items 仍可下载
- Agent 更灵活：可在 list 与 download 之间插入过滤（"只下载 video"、"先看一眼标题再决定"）
- 避免重复 GraphQL 调用：原来按 ID 下载需要再打一次端点拉详情；新接口将该次调用收敛到 `list_likes`，整体调用次数减半

**替代方案**：

- 仅接受 ID：被否决——见上
- 接受 ID 或 items 的 union 类型：被否决——lib 签名要清晰，让 CLI 层做适配即可

### D10：legacy `xld download` 切到 lib，通过 opts 适配旧行为

**选择**：旧 `xld download` 子命令的实现切到调 `agent::list_likes` + `agent::download_media`，并通过 `DownloadOpts` 的三个新字段 (`base_dir` / `filename_format` / `set_mtime`) 表达"写入 `./downloads`、用 `{USERNAME} {ID}` 命名、设置 mtime"等旧默认。

为支撑这个迁移，`MediaItem` 增加两个 optional 字段：

- `author_handle: Option<String>`：`list_likes` 总是填，让 `download_media` 默认命名能拼出 `{handle}_{id}_{file}` 形态
- `created_at: Option<String>`：`list_likes` 总是填，让 mtime 设置不需要额外 metadata 查询

**演进过程**：v0 评估时认为 6.7 任务不可做（旧版的 `{USERNAME} {ID}` 文件命名约定无法在新 lib 接口表达，强行迁移会污染 Agent 接口）。重新评估发现：

- 旧 `{USERNAME} {ID}` 作为默认其实是 `{handle}_{id}` 的一个特例
- 把它升格为新 lib 的**默认命名约定**（而非 legacy-only 适配层），既消解了旧版的特殊性，又给 Agent 输出更友好（含 username 的文件名跟 organize 模块天然兼容）
- `MediaItem` 多 2 个 optional 字段、`DownloadOpts` 多 3 个 optional 字段——比之前担心的"4 个 legacy-only 字段污染"少一半，且语义对 Agent 也是有用的

**理由**：

- 实际净 LoC 减少（删掉 `Downloader::call_media_downloader` 等 ~150 行业务逻辑后，新增的 ~50 行逻辑摊销在两个用户路径上）
- HTTP 重试 / 状态分类 / 断点续传 / 并发逻辑只有一份实现，未来改一处即可
- `data/downloaded_tweet_ids.txt` 兼容性保留：legacy 路径继续读写，新 path 用文件级去重（Content-Length 比对）作为更细粒度补充
- `auto_organize` 工作流不变：默认命名格式 `{handle}_{id}_{file}` 让 `organize_files.rs` 的 parser 能继续识别 username

**替代方案**：

- 保留旧 `Downloader::call_media_downloader` 的双实现并行：被 D10 推翻——单实现更易维护
- 不加 `MediaItem.author_handle` / `created_at`，靠 Agent 在调用前自己拼接：被否决——增加 Agent 端心智负担、丢失"`list_likes` → `download_media` 直接转手"的简洁性

### D9：进度反馈用 stderr NDJSON，不引入 progress 工具

**选择**：`xld media download --json` 模式下 stderr 输出 NDJSON 事件流（`download_started` / `item_started` / `item_progress` / `item_done` / `download_finished` / `diagnostic`）；非 `--json` 模式保持现有 `indicatif` 进度条。**不**引入 `download_progress(job_id)` 一类需要状态的轮询工具。

**理由**：

- CLI subprocess 模型本质 stateless；引入"job 状态"会带来存储位置、过期、并发去重等一系列附加问题，对 v1 性价比低
- NDJSON 让支持解析 stderr 的 harness（部分 Claude Code / OpenClaw 客户端）能渲染进度，不支持也无伤
- v2 上 MCP 时直接用 MCP 原生 progress notification 协议，比自造轮询机制更体面

**替代方案**：

- 不发任何机器可读进度：被否决——长任务时用户体验差
- `indicatif` 输出原样保留：被否决——含 ANSI escape codes 与回车符，Agent harness 难以解析
- 引入轮询工具：被否决——破坏 stateless 模型，复杂度雪球

## 风险 / 权衡

| 风险 | 缓解措施 |
|---|---|
| X 滚动 GraphQL queryId 导致 `defaults.json` 失效 | `setup` 从 cURL 提取协议参数 override；`auth_status` 区分 `endpoint_stale`，提示用户重导 cURL；README 写明此为已知行为 |
| 公开 skill 后 X anti-bot 风险升高 | README 明示 ToS 边界由用户承担；不引入主动加速请求；保留现有翻页节奏 |
| Agent 通过 `subdir` 参数尝试路径穿越 | `sandbox.rs` 用 `Path::components()` 强校验，单元测试覆盖 `..` / 绝对路径 / Windows 驱动器盘符 / 符号链接 |
| 双 crate 化引入意外的 build 中断 | 拆解任务的第一步就是双 crate 切换（不动业务逻辑），过 CI 后再继续 |
| JSON schema v1 设计错误，将来要 break | `meta.schema_version` 字段就位；v1 范围克制（仅必要字段），有变更时新增字段不破坏 |
| `xld download` 老用户期待"一键"行为，重构后产生回归 | 新 lib 的 `list_likes` + `download_media` 组装出与旧行为等价的代码路径；保留现有手动 / 集成测试用例 |
| Skill 用户找不到 binary 报错体验差 | Skill 脚本统一返回 `binary_missing` 结构化错误，hint 字段直链 Releases 页面 |

## 迁移计划

1. **Phase A — lib 化（无外部行为变化）**：`Cargo.toml` 加 `[lib]`；把 `x_api` / `downloader` / `setup` / `config` 暴露为 `pub mod`；`main.rs` 调用方式从 `crate::xxx` 改为 `xld::xxx`。CI 全绿。
2. **Phase B — JSON 输出层**：剥离 `x_api.rs` 内部 `println!`，全部改 `eprintln!` 或经由 `tracing` 走 stderr；引入 `OutputEnvelope` 类型用于 `--json` 输出。
3. **Phase C — 新子命令**：`xld likes list` / `xld media download` / `xld auth status` 上线；`xld setup` 增强 cURL 解析。
4. **Phase D — 沙箱**：`sandbox.rs` 落地，`media download` 接入；老 `download` 路径不变。
5. **Phase E — Skill 包**：`skill/SKILL.md` / `defaults.json` / `README.md` 编写；自测一次"陌生用户"流程。
6. **Phase F — 文档**：`README.md` 主文新增"Agent 集成"章节。

每个 phase 独立可回滚（git revert），且都不影响现有 release artifact 的产出。

## Open Questions

所有 v1 范围内的开放问题均已收敛，决议见 D1–D9。简要回顾：

1. ~~JSON schema 字段命名~~ → **D2 + likes-listing-json spec**：扁平 v1 schema 字段集已钉死（`id` / `author_handle` / `author_display_name` / `text` / `created_at` / `tweet_url` / `is_retweet` / `is_reply` / `media[]` / `liked_at?`）；原始 entry 通过 `--include-raw` opt-in。
2. ~~`download_media` 的并发度~~ → **D8**：接口改为接受 `MediaItem[]`，避免重复 GraphQL；CDN 下载默认并发 4，参数化钳位 [1, 16]。
3. ~~`auth_status` 缓存~~ → **D5 + agent-skill-package spec**：不缓存。SKILL.md 限定三种允许场景（会话起始预检 / 错误后确认 / 用户显式询问），禁止循环或预检式调用。
4. ~~`download_progress` 工具~~ → **D9**：不引入轮询工具。`--json` 模式下 stderr 输出 NDJSON 进度事件流；非 `--json` 模式保持 `indicatif`。

v2（MCP）阶段将自然出现的问题（progress notification、状态化连接、并发任务调度）留待 MCP 设计时再处理。
