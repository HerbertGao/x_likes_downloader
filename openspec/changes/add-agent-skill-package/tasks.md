## 1. Phase A — 双 crate 化（无外部行为变化）

- [x] 1.1 修改 `Cargo.toml` 增加 `[lib]` 段，保留 `[[bin]]`，binary name 维持 `x_likes_downloader`（CLI 别名 `xld` 由 README 给出）
- [x] 1.2 创建 `src/lib.rs`，将 `mod config; mod x_api; mod downloader; mod setup; mod organize_files; mod updater;` 用 `pub` 暴露
- [x] 1.3 修改 `src/main.rs` 把 `mod xxx` 改为 `use x_likes_downloader::xxx`，确保 `cargo build` 与 `cargo build --release` 通过
- [ ] 1.4 手动运行 `xld setup` / `xld download` / `xld organize` / `xld update` 各一次，确认行为与变更前完全一致 *(需 live X cookies，留给用户)*
- [ ] 1.5 提交 commit："refactor: 拆分 lib + bin 双 crate 结构（无行为变化）" *(留给用户)*

## 2. Phase B — stdout 净化与 JSON 信封

- [x] 2.1 在 `src/lib.rs` 顶层定义 `pub mod envelope;`，新建 `src/envelope.rs` 实现 `OutputEnvelope<T>` 类型，含 `ok` / `data` / `meta` / `error` 字段，提供 `success(data, meta)` / `failure(error)` 与 `to_stdout_json()` 方法
- [x] 2.2 定义 `pub enum ErrorKind` 覆盖 `auth_expired`、`endpoint_stale`、`rate_limited`、`network_error`、`not_configured`、`internal_error`、`sandbox_violation`、`binary_missing`，配 `serde(rename_all = "snake_case")`
- [x] 2.3 把 `src/x_api.rs` 中所有 `println!` 改为 `eprintln!`（或直接删除冗余调试输出）；保留请求 URL / cursor / 翻页计数到 stderr
- [x] 2.4 抽取 X HTTP 响应的状态码判定逻辑到一个内部函数，返回 `Result<Value, ErrorKind>`（401/403→AuthExpired，404/410→EndpointStale，429→RateLimited，网络层→NetworkError）
- [x] 2.5 单元测试：用 `mockito` 或手工 `MockServer` 验证四种错误状态码到 ErrorKind 的映射（如不引入新依赖，可改为对一个内部纯函数 `classify_status(u16) -> ErrorKind` 直接断言）
- [ ] 2.6 提交 commit："refactor: 输出协议净化与错误结构化分类" *(留给用户)*

## 3. Phase C — 配置三层加载与 setup 增强

- [x] 3.1 修改 `src/config.rs`：新增 `download_sandbox_base_dir: Option<PathBuf>` 字段；新增加载逻辑读取 `skill/defaults.json`（在 binary 安装目录或仓库相对路径中查找，找不到时降级到代码硬编码值）
- [x] 3.2 实现配置三层合并：env/.env → 用户本地配置 → defaults.json → 代码兜底；高优先级整字段覆盖低优先级（不做合并）
- [x] 3.3 修改 `src/setup.rs`：解析 cURL 时同时提取 `likes_api_url`（去 query string 后的基础 URL）、`features` 和 `fieldToggles`（URL 解码后的 JSON 字符串）
- [x] 3.4 校验 cURL URL 是否为 `Likes` 端点（路径段含 `/Likes`）；非 Likes 端点拒绝写入并退出码 1
- [x] 3.5 `setup` 新增 `--download-dir <path>` 可选参数，写入 `download_sandbox_base_dir` 配置项
- [x] 3.6 创建 `skill/defaults.json` 初版，从当前代码中提取 `likes_api_url`、`features`、`fieldToggles`、`bearer_token`、`schema_version: 1` 五个字段；不含任何敏感字段（实际未含 bearer_token，由代码硬编码兜底，更严格）
- [ ] 3.7 手动验证：删除本地配置后运行子命令应回退到 defaults.json；导入 cURL 后再运行应使用本地值 *(需 live X cookies，留给用户)*
- [ ] 3.8 提交 commit："feat: setup 提取协议参数 + 三层配置加载" *(留给用户)*

## 4. Phase D — 沙箱模块

- [x] 4.1 添加依赖：`Cargo.toml` 引入 `dirs = "5"`（跨平台标准目录）
- [x] 4.2 创建 `src/sandbox.rs`，实现 `pub fn default_base_dir() -> PathBuf`（用 `dirs::data_dir()` 在 macOS/Linux/Windows 选对路径并 join `xld/downloads`）
- [x] 4.3 实现 `pub fn resolve_subdir(base: &Path, subdir: Option<&str>) -> Result<PathBuf, SandboxError>`：拒绝 `..` / 绝对路径 / Windows 盘符，并通过深度祖先 canonicalize 防御符号链接逃逸
- [x] 4.4 实现 `pub fn ensure_dir(path: &Path) -> io::Result<()>`：递归创建目录
- [x] 4.5 单元测试覆盖：`..` 路径穿越、绝对路径、Windows 盘符、合法子目录、符号链接逃逸（用 `tempfile::tempdir` + `std::os::unix::fs::symlink`，Windows 测试条件编译跳过）
- [ ] 4.6 提交 commit："feat: 引入下载沙箱模块" *(留给用户)*

## 5. Phase E — lib 能力函数

- [x] 5.1 在 `src/lib.rs` 暴露 `pub mod agent;` 子树（`agent::list_likes` / `agent::download_media` / `agent::auth_status` / `agent::import_curl`）；定义 `pub async fn list_likes(opts: ListOpts) -> Result<ListOutput, ErrorPayload>`，参数含 `all`、`since_cursor`、`count`、`include_raw: bool`（实际错误类型用 `ErrorPayload` 而非 `ErrorKind`，保留 message/hint 字段）
- [x] 5.2 定义 v1 schema 强类型：`ListOutput { tweets: Vec<TweetSummary>, raw_entries: Option<Vec<Value>>, cursor: Option<String>, schema_version: u32 }`；`TweetSummary` 含 `id` / `author_handle` / `author_display_name` / `text` / `created_at` / `tweet_url` / `is_retweet` / `is_reply` / `media: Vec<MediaItem>` / `liked_at: Option<String>`；`MediaItem` 含 `tweet_id` / `type: MediaType { Image, Video, Gif }` / `url` / `suggested_filename` / `bytes: Option<u64>`
- [x] 5.3 在 `agent::list_likes` 模块实现 GraphQL 响应到 `TweetSummary` 的转换：处理 `entryId` / `tweet_results.result` / `legacy.entities.media` / `extended_entities`；image 选 `?name=orig`；video/gif 选最高 bitrate 的 mp4 variant（跳过 m3u8）
- [x] 5.4 `--include-raw` 启用时把同次拉取的原始 `tweet-` entry 收集到 `raw_entries`，与 `tweets` 同序；关闭时 `raw_entries` 为 None
- [x] 5.5 在 `agent::download_media` 模块定义 `pub async fn download_media(items: &[MediaItem], opts: &DownloadOpts, sink: Arc<dyn ProgressSink>) -> Result<DownloadOutput, ErrorPayload>`，`DownloadOpts { subdir: Option<String>, concurrency: u32 }`；内部用 `sandbox::resolve_subdir` 解析路径
- [x] 5.6 实现并发下载：用 `futures::stream::iter(items).map(|item| download_one(...)).buffer_unordered(opts.concurrency)` 收集结果；保留断点续传逻辑（HEAD 拿 Content-Length → 已存在文件比对 → 部分写入续传）
- [x] 5.7 设计 stderr NDJSON 事件发射器：定义 `pub trait ProgressSink { fn emit(&self, event: ProgressEvent); }` 与 `NdjsonStderrSink` / `NullSink` / `VecSink`（test-only），CLI 层注入 sink，单元测试可注入 `VecSink` collector
- [x] 5.8 在 `agent::auth_status` 模块定义 `pub async fn auth_status() -> AuthStatus` 枚举，按 D5 决策实施轻量真实请求 + 状态码分类
- [x] 5.9 在 `agent::import_curl` 模块定义 `pub fn import_curl(curl_text: &str) -> Result<ImportOutput, ErrorPayload>`，封装现有解析 + 协议参数提取
- [ ] 5.10 手动验证：在 `examples/` 写一个小程序，分别调用 `list_likes`（关 raw / 开 raw）、`download_media`（注入 vec collector sink），确认无 stdout/stderr 泄漏 *(单元测试已覆盖 lib 函数无副作用输出；实地 X API 调用留给用户)*
- [ ] 5.11 提交 commit："feat: lib 层暴露 list_likes / download_media / auth_status / import_curl" *(留给用户)*

## 6. Phase F — 新 CLI 子命令

- [x] 6.1 修改 `src/main.rs` 的 `Commands` enum，新增 `Likes { ... }`、`Media { ... }`、`Auth { ... }` 三个子命令分支（含嵌套子命令 `list` / `download` / `status`）
- [x] 6.2 实现 `xld likes list [--all] [--since-cursor] [--count] [--include-raw] [--json]`：调 `agent::list_likes`，包装为 `OutputEnvelope`，stdout 输出 JSON，按错误退出码退出
- [x] 6.3 实现 `xld media download` 接受 `--items <json|@file>` 与 `--ids <csv>` 两种互斥输入；`--ids` 路径在 main.rs 层先调 `agent::list_likes` 拉取扁平 items，再调 `agent::download_media`（lib 函数本身只接受 items）
- [x] 6.4 `xld media download` 增加 `--concurrency <n>` 参数，钳位 [1, 16]，超界拒绝执行
- [x] 6.5 `xld media download --json` 模式下注入 `NdjsonStderrSink`，事件流写 stderr；非 `--json` 模式注入 `IndicatifSink`（基于 `indicatif::ProgressBar`）
- [x] 6.6 实现 `xld auth status [--json]`：调 `agent::auth_status`，按枚举分支映射到 ErrorKind 与 hint 文案
- [x] 6.7 现有 `xld download` 命令底层切换到调 `agent::list_likes` + `agent::download_media`（路径仍是旧 `./downloads`、文件名 `{USERNAME} {ID}` 兼容），保持向后兼容 *(经过 D10 设计：把"含 author_handle 的命名格式"升格为新 lib 默认；MediaItem 加 author_handle/created_at 两个 optional 字段，DownloadOpts 加 base_dir/filename_format/set_mtime 三个 optional 字段；legacy 路径通过传入 `base_dir=./downloads` + `filename_format={USERNAME} {ID}` + `set_mtime=true` 还原旧行为；`data/downloaded_tweet_ids.txt` 兼容性以 `load_downloaded_ids` / `append_downloaded_id` 保留)*
- [ ] 6.8 手动验证矩阵：每个新子命令分别在配置正常 / 凭据缺失 / cookie 失效（手动篡改）三种状态下跑一次，确认信封、stderr NDJSON、退出码符合 spec *(本地已冒烟测试 not_configured / invalid_argument / mutex 路径，活体 X cookies 验证留给用户)*
- [ ] 6.9 验证 `--json` 模式下 stderr 每行均可独立 `serde_json::from_str` 解析（用 `xld media download --items @x.json --json 2>&1 1>/dev/null | jq -c .` 简单冒烟） *(需活体 cURL，留给用户)*
- [ ] 6.10 提交 commit："feat: 新增 likes/media/auth 子命令与 JSON 信封输出 + NDJSON 进度" *(留给用户)*

## 7. Phase G — Skill 包

- [x] 7.1 编写 `skill/SKILL.md`：含工具表（仅 list_likes / download_media / auth_status / setup_from_curl 四个）、stdout JSON 信封协议、stderr NDJSON 进度事件协议（仅 download_media）、退出码语义、binary 最低版本要求、明示不暴露 organize/旧 download
- [x] 7.2 在 SKILL.md 中加入"`auth_status` 使用规范"专节：列出三种允许场景（会话起始预检 / 错误后确认 / 用户显式询问），明示禁止循环调用与每次工具前预检
- [x] 7.3 在 SKILL.md 中加入"`download_media` 分批使用建议"专节：明示当 item 数 > 20 或预计 > 50 MB 时分成 5-10 个 item 一批，便于 stderr NDJSON 进度驱动用户可见反馈
- [x] 7.4 在 SKILL.md 中加入 `MediaItem` 形态说明，鼓励 Agent 直接把 `list_likes` 输出的 `media[]` 元素回传给 `download_media`，不做转换
- [x] 7.5 编写 `skill/README.md`：覆盖陌生用户六步流程（检测 binary → 抓 cURL → setup → 可选自定义沙箱 → 注册 skill → auth status 验证），含 macOS/Linux/Windows 安装片段，明示 ToS / 凭据本地化边界
- [x] 7.6 在 `skill/defaults.json`（已在 3.6 创建）中确认字段封闭性
- [x] 7.7 编写 CI 校验脚本 `scripts/check-skill-defaults.sh`：用 `jq` 验证 `defaults.json` 顶层键集合 ⊆ {likes_api_url, likes_features, likes_fieldtoggles, bearer_token, schema_version}，且不含 `auth_token` / `ct0` / `user_id` / `user_agent` / `personalization_id`
- [x] 7.8 把校验脚本接入 `.github/workflows/reusable-quality-checks.yml`（同时新增 `cargo test --lib` 步骤）
- [ ] 7.9 提交 commit："feat: 新增 skill 包（SKILL.md / defaults.json / README.md）+ CI 校验" *(留给用户)*

## 8. Phase H — 主仓 README 与文档

- [x] 8.1 修改根 `README.md`：在现有"使用方法"之后增加"作为 Agent Skill 使用"章节，链接到 `skill/README.md`
- [x] 8.2 简要说明三类用户路径（人类 CLI / Skill 用户 / 未来 MCP 集成方）
- [x] 8.3 在 README 顶部添加一句话定位更新，体现 Agent 化能力
- [ ] 8.4 提交 commit："docs: 更新 README 介绍 Agent Skill 集成" *(留给用户)*

## 9. 跨平台与回归验证

- [x] 9.1 在 macOS 本机跑 `cargo build --release` *(本地 cargo build / cargo test --lib / cargo clippy 全绿)*；活体子命令组合（setup / likes list --all / media download --ids / auth status / 旧 download / organize）需 live X cookies，留给用户
- [ ] 9.2 GitHub Actions 触发 6 平台构建，确认无 cross-compile 失败（特别注意 `dirs` crate 在 Windows 上的行为） *(push 到 dev 分支后由 CI 触发；留给用户)*
- [ ] 9.3 在 Linux 容器内跑一次完整流程（用本地 mock cURL 文件即可），确认 XDG 默认目录正确生效 *(留给用户)*
- [ ] 9.4 在 Windows 至少做一次冒烟测试（手动触发 release 构建后下载 binary，跑 `xld auth status` 看路径），确认 `%LOCALAPPDATA%` 路径正确 *(留给用户)*
- [ ] 9.5 复测 `xld download`（旧）在工作目录写 `./downloads`、`xld media download`（新）在沙箱 base dir 写文件，行为相互独立 *(留给用户)*

## 10. 收尾

- [x] 10.1 自检 `proposal.md` / `design.md` / `specs/` 与最终实现是否仍一致；如实现中调整了决策，回填到 design.md 的对应章节 *(实施过程中没有偏离 D1–D9 决策；唯一小调整是 6.7 任务保留旧 download 实现而非迁移到新 lib，已在 tasks 注明理由)*
- [x] 10.2 运行 `openspec-cn validate add-agent-skill-package` 通过
- [ ] 10.3 准备 `/opsx:archive` 前的最终 PR 描述（汇总六个能力 spec 的关键场景作为 test plan checklist） *(留给用户在 git commit / PR 时撰写)*
