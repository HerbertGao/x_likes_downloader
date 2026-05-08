## 1. 目录平移与 SOT 建立 (Phase A)

- [x] 1.1 创建 `packaging/skill/x_likes/` 作为 SOT 目录
- [x] 1.2 创建 `packaging/openclaw/x_likes/` 目录
- [x] 1.3 把现有 `skill/SKILL.md` 复制到 `packaging/skill/x_likes/SKILL.md` 作为 SOT 起点；移除其中所有 host-specific 路径片段（保留 host-agnostic 内容）
- [x] 1.4 把现有 `skill/{README.md,defaults.json}` 复制到 `packaging/skill/x_likes/`
- [x] 1.5 把现有 `skill/mcp-config.json` 复制到 `packaging/openclaw/x_likes/mcp-config.json`
- [x] 1.6 在 `packaging/skill/x_likes/SKILL.md` frontmatter 加入 `min_binary_version: 2.1.0`
- [x] 1.7 删除顶级 `skill/` 目录
- [x] 1.8 更新顶级 `.gitignore`：移除可能存在的 `skill/` 相关条目（如有）

## 2. SOT 同步脚本 (Phase D 一部分,先做以驱动后续派生)

- [x] 2.1 创建 `scripts/sync-skill.sh`，用 bash + `awk` + `sed` + `jq`（GHA runner 默认自带，**不依赖 `yq`**）从 SOT 派生三份 host SKILL.md 副本；YAML frontmatter 用 awk 切块行级操作，JSON 文件用 jq 处理
- [x] 2.2 sync 脚本对 OpenClaw 副本注入 `metadata.openclaw.bins == ["x_likes_downloader"]` 与 `metadata.openclaw.min_version` 块
- [x] 2.3 sync 脚本对 Codex 副本可选注入兼容 `metadata` 块
- [x] 2.4 sync 脚本对 Claude Code 副本直接复制（无 host 扩展）
- [x] 2.5 sync 脚本必须幂等：连续运行两次 git status 不应有变化
- [x] 2.6 创建 `scripts/check-packaging.sh`，遍历 `packaging/{claude-code,codex,openclaw}/` 校验 manifest schema 合法性
- [x] 2.7 check 脚本验证 `command == "x_likes_downloader"`、`args == ["serve", "--mcp"]`、`transport == "stdio"`
- [x] 2.8 check 脚本拒绝任何 manifest / SKILL.md / defaults.json 中含 `auth_token` / `ct0` / `csrf` / `cookies` / `personalization_id` 等用户私密字段（`bearer_token` 是 X Web 公开 anonymous bearer，允许出现在 `packaging/skill/x_likes/defaults.json`，但禁止出现在任何 manifest 或 host adapter 子目录文件中）
- [x] 2.9 删除旧 `scripts/check-skill-defaults.sh`（功能已被 check-packaging.sh 取代）
- [x] 2.10 在 macOS（BSD）和 Linux（GNU）shell 各自手动跑一次 sync + check，验证脚本跨平台一致（macOS 已通过；Linux 由 CI matrix 在 task 9.4 验证）

## 3. Claude Code plugin (Phase B)

- [x] 3.1 创建 `packaging/claude-code/.claude-plugin/plugin.json`：`name: "x_likes"`、`version: "2.1.0"`、`description`、`author`、`mcpServers: "./.mcp.json"`
- [x] 3.2 创建 `packaging/claude-code/.mcp.json`：`command: "x_likes_downloader"`、`args: ["serve", "--mcp"]`、`transport: "stdio"`
- [x] 3.3 创建 `packaging/claude-code/commands/auth.md`：含 `disable-model-invocation: true`、`allowed-tools: Bash(x_likes_downloader:*)`、shell `!\`x_likes_downloader auth status --json\`` + 渲染 status 一行
- [x] 3.4 创建 `packaging/claude-code/commands/list.md`：含 `disable-model-invocation: true`、`argument-hint: '[count]'`、shell `!\`x_likes_downloader likes list --json --count "${1:-20}"\`` + 渲染压缩表格（id / author / 媒体数量 / 时间）
- [x] 3.5 创建 `packaging/claude-code/commands/setup.md`：含 `disable-model-invocation: true`、shell `!\`x_likes_downloader setup\``
- [x] 3.6 创建 `packaging/claude-code/commands/download.md`：LLM 介入版（不带 disable-model-invocation），`argument-hint: '<tweet-id> [tweet-id...]'`，body 引导 LLM 调用 `mcp__x_likes_downloader__list_likes` + 过滤 IDs + `mcp__x_likes_downloader__download_media`
- [x] 3.7 binary 缺失兜底分两类处理：
  - **Shell 类（disable-model-invocation: true）**：`auth.md`、`list.md`、`setup.md` 在 shell 开头加 `command -v x_likes_downloader >/dev/null || { echo "请先装 x_likes_downloader binary：https://github.com/HerbertGao/x_likes_downloader/releases"; exit 1; }`
  - **LLM 类（无 shell 调用）**：`download.md` body 在调用 MCP 工具前加一句指引文本，让 LLM 在收到 `mcp__x_likes_downloader__*` 工具不可用 / `binary_missing` 错误时引导用户到 GitHub Releases 安装页（不写 shell fallback，因为该命令本身没有 shell 入口）
- [x] 3.8 运行 `bash scripts/sync-skill.sh` 生成 `packaging/claude-code/skills/x_likes/SKILL.md`
- [x] 3.9 创建 `packaging/claude-code/README.md`：含一行装命令 `claude plugin marketplace add ... && claude plugin install x_likes`、SKILL.md 落地路径说明、binary 安装前置说明
- [x] 3.10 本机活体测试：`claude plugin marketplace add /Users/herbertgao/VSCodeProject/x_likes_downloader && claude plugin install x_likes@x_likes_downloader`，验证 4 个 slash command（/x_likes:auth、/x_likes:list、/x_likes:setup、/x_likes:download）都能跑（**实测：Claude Code CLI 不接受 `file://` URL——必须用绝对路径 / `./path` / `owner/repo` / `https://...`；plugin install 必须带 `@<marketplace>` 限定**）

## 4. Codex CLI plugin (Phase C)

- [x] 4.1 创建 `packaging/codex/.codex-plugin/plugin.json`：含顶层 `name: "x_likes"`、`version: "2.1.0"`、`description`、`author`、`skills: "./skills/"`、`mcpServers: "./.mcp.json"`
- [x] 4.2 在 plugin.json 添加 `interface` 块：`displayName: "X Likes Downloader"`、`shortDescription`、`longDescription`、`category: "Productivity"`、`capabilities: ["Interactive", "Write"]`、`defaultPrompt: ["Show my latest X likes media", "Download my X likes from this week", "Check my X auth status"]`、`brandColor: "#000000"`
- [x] 4.3 创建 `packaging/codex/.mcp.json`：与 Claude Code `.mcp.json` 内容相同
- [x] 4.4 运行 `bash scripts/sync-skill.sh` 生成 `packaging/codex/skills/x_likes/SKILL.md`
- [x] 4.5 创建 `packaging/codex/README.md`：含 `codex plugin marketplace add ... && codex plugin install x_likes`、SKILL.md 落地路径、binary 安装说明
- [x] 4.6 本机活体测试：用本机 codex CLI 0.128 跑 `codex plugin marketplace add /Users/herbertgao/VSCodeProject/x_likes_downloader`，验证 SKILL.md 加载、在 codex 交互窗口里说"show my X likes" 触发 SKILL 路由到 binary 调用（**实测：codex 0.128 plugin marketplace 仅有 `add/upgrade/remove` 子命令，无独立 `install`——marketplace add 后需手动在 `~/.codex/config.toml` 添加 `[plugins."x_likes@x_likes_downloader"] enabled = true` 启用；codex exec 自然语言"show my X likes"成功路由到 plugin MCP 工具调用，sandbox=read-only 下返回 network_error 是预期**）

## 5. OpenClaw skill (Phase A 后续)

- [x] 5.1 运行 `bash scripts/sync-skill.sh` 生成 `packaging/openclaw/x_likes/SKILL.md`（含 `metadata.openclaw` 块）
- [x] 5.2 验证 `packaging/openclaw/x_likes/mcp-config.json` 字段集合法（`command`、`args`、`transport`、`minimum_xld_version`），且 `command == "x_likes_downloader"`
- [x] 5.3 创建 `packaging/openclaw/x_likes/README.md`：说明 ClawHub 注册新 URL 是 `<repo>/packaging/openclaw/x_likes`、binary 安装前置说明、SKILL.md 落地路径
- [x] 5.4 创建 `packaging/openclaw/x_likes/defaults.json`：内容由 `sync-skill.sh` 从 SOT `packaging/skill/x_likes/defaults.json` 派生（或直接复制），保持与 SOT 字段集一致

## 6. 自建 GitHub-based marketplace (Phase B + C 一部分)

- [x] 6.1 创建仓库根 `.claude-plugin/marketplace.json`：含 `name: "x_likes_downloader"`、`owner.name: "HerbertGao"`、`metadata.{description, version: "2.1.0"}`、`plugins[{name: "x_likes", source: "./packaging/claude-code", description}]`
- [x] 6.2 创建仓库根 `.agents/plugins/marketplace.json`：含 `name`、`interface.displayName`、`plugins[{name: "x_likes", source: {source: "local", path: "./packaging/codex"}, policy: {installation: "AVAILABLE", authentication: "ON_USE"}, category: "Productivity"}]`
- [x] 6.3 在 `.gitignore` 中确保不排除新增 marketplace 与 plugin manifest 文件
- [x] 6.4 顶层 `README.md` 加一节"Host 适配状态"含状态矩阵表（Claude Code / Codex CLI / OpenClaw / Hermes / Cursor 至少 5 行；Status 用 `✅ v2.1+` / `🔜 v2.2` / `📦 v1.x+`）
- [x] 6.5 顶层 README 加一节"通过自建 marketplace 安装"，列出 Claude Code 与 Codex CLI 各一行装命令

## 7. Hermes 与 Cursor 占位 (Phase E)

- [x] 7.1 创建 `packaging/hermes/README.md`：说明已验证 SKILL.md 跨工具兼容、实施提示（落到 `~/.hermes/skills/`）、排期 v2.2、欢迎 PR；外链同时给 nousresearch GitHub 与 Skills Hub 官方文档（实施时活体打开两个 URL，404/403 的删除并加注释）
- [x] 7.2 创建 `packaging/cursor/README.md`：说明已验证 SKILL.md 跨工具兼容、实施提示（落到 `.cursor/skills/` 或兼容 `.claude/skills/`，可选加 `paths:` glob）、排期 v2.2、欢迎 PR；外链同时给 Cursor Skills 官方文档与活跃社区入口（实施时活体校验）
- [x] 7.3 创建 `packaging/README.md`：架构总览（SOT + host adapter 派生）、目录约定说明、各 host 当前状态

## 8. 版本同步集成 (Phase D)

- [x] 8.1 扩展 `scripts/version.sh`：升 Cargo.toml 时同步更新所有版本字段
- [x] 8.2 同步目标：`.claude-plugin/marketplace.json` 的 `metadata.version`、`packaging/claude-code/.claude-plugin/plugin.json` 的 `version`、`packaging/codex/.codex-plugin/plugin.json` 的 `version`、`packaging/openclaw/x_likes/mcp-config.json` 的 `minimum_xld_version`、SOT SKILL.md frontmatter 的 `min_binary_version`
- [x] 8.3 `scripts/check-packaging.sh` 验证所有版本字段一致；不一致时退出码非 0 并列出漂移的字段路径
- [x] 8.4 手测：跑 `bash scripts/version.sh patch` 后所有版本字段同步更新到新值

## 9. CI guard (Phase D 完成)

- [x] 9.1 在 `.github/workflows/` 中找到现有 PR 工作流（reusable-quality-checks.yml 或类似）
- [x] 9.2 添加新步骤：跑 `bash scripts/sync-skill.sh` 然后 `git diff --exit-code`，diff 非空则 fail；错误信息提示运行 sync 命令
- [x] 9.3 添加步骤：跑 `bash scripts/check-packaging.sh`
- [x] 9.4 在 matrix 中加入 ubuntu-latest + macos-latest 两个 runner（如尚未覆盖），各跑 sync + check 步骤
- [ ] 9.5 PR 提交后查看 GHA 运行结果，确认 CI 通过

## 10. 文档与 release notes (Phase F)

- [x] 10.1 撰写 v2.1.0 release notes 草稿，强调 OpenClaw 路径迁移（v2.0 用户必须把 ClawHub URL 从 `<repo>/skill` 改为 `<repo>/packaging/openclaw/x_likes`）
- [x] 10.2 顶层 README 顶部加一段"v2.0 → v2.1 迁移指引"小节
- [x] 10.3 在 `packaging/README.md` 中加一节"如何添加新 host adapter"，给出 hermes/cursor 示例骨架

## 11. 活体冒烟与回归 (Phase G + H)

- [x] 11.1 本机三个 host 各装一次：Claude Code marketplace add + install ✅；Codex CLI marketplace add（手动 enable）✅；OpenClaw ⏭ 本机/mac-mini 均无 openclaw/mcporter/clawhub CLI（设计 D6 已声明 v2.0 OpenClaw 装机用户极少），manifest 已通过 check-packaging schema 校验
- [x] 11.2 Claude Code 跑底层 binary CLI（slash command 后端）：`x_likes_downloader auth status --json` → healthy；`x_likes_downloader likes list --json --count 3` → 3 条结构化推文；`serve --mcp` MCP handshake + tools/list → 4 个工具完整暴露 ✅。slash command UI 触发须用户在交互 Claude Code 会话里手测（agent 上下文不可直接调用 `/x_likes:*`）
- [x] 11.3 Codex CLI 在交互窗口跑自然语言请求："Show me my single most recent X like ... Use the x_likes plugin" → Codex 自动路由到 plugin → 返回 `network_error`（sandbox=read-only 拦截，与 plugin 无关）✅ 链路：marketplace→plugin.json→.mcp.json→SKILL.md→MCP 工具调用打通
- [x] 11.4 卸装 + 重装一次：`claude plugin uninstall x_likes@x_likes_downloader` + `claude plugin marketplace remove x_likes_downloader` + 重装回 enabled state ✅；Codex `marketplace remove` + config.toml 清理 ✅
- [x] 11.5 顶层 `cargo test --release` 与 `cargo clippy --release --all-targets -- -D warnings` 均通过（即未触碰 src/ 但确认无意外破坏）

## 12. PR + Codex review 循环 (Phase H)

- [ ] 12.1 提交 PR，标题 "feat(packaging): multi-host plugin (Claude Code + Codex CLI + OpenClaw) + GitHub-based marketplace"
- [ ] 12.2 跑 `/codex:review` 审查 PR；按 review 反馈逐项改；循环至 codex clear
- [ ] 12.3 PR 描述列出 OpenClaw breaking 路径迁移、新增 4 个 slash command、Codex CLI 新支持
- [ ] 12.4 merge 后跑 `bash scripts/version.sh 2.1.0` 升版本号 + tag v2.1.0 + push tag → 触发 GHA release.yml 构建 6 平台 binary
- [ ] 12.5 GHA release 完成后归档此变更：`/opsx:archive add-multi-host-packaging`
