
### 需求:packaging/ 目录结构与 SOT 单一可信源

仓库根必须新增 `packaging/` 顶级目录，承载所有 host adapter 与共享 SOT。`packaging/skill/x_likes/` 是单一可信源（SOT），其它 host 子目录中的 `SKILL.md` 副本必须由 SOT 派生，不得手工编辑。`packaging/` 子目录布局必须包括：

- `packaging/skill/x_likes/`（SOT）：含 host-agnostic `SKILL.md` + `README.md` + `defaults.json`
- `packaging/claude-code/`：Claude Code plugin 实做
- `packaging/codex/`：Codex CLI plugin 实做
- `packaging/openclaw/x_likes/`：OpenClaw skill 实做（接收 v2.0 `skill/` 平移）
- `packaging/hermes/`：v2.1+ active host adapter（SKILL.md sync-derived；MCP 通过 `~/.hermes/config.yaml` 手动 `mcp_servers` 注册——hermes CLI argparse 已知 bug 把 `--mcp` 抢走）
- `packaging/cursor/`：v2.2 占位
- `packaging/README.md`：架构说明 + host 适配状态表

`packaging/` 必须不进入 Cargo workspace；不得包含 Rust 源码、binary、私密凭据。

#### 场景:目录就位
- **当** 在主分支上 `ls packaging/`
- **那么** 必须看到 `skill`、`claude-code`、`codex`、`openclaw`、`hermes`、`cursor`、`README.md` 七个条目

#### 场景:SOT 唯一性
- **当** 在 `packaging/` 下查找所有名为 `SKILL.md` 的文件
- **那么** `packaging/skill/x_likes/SKILL.md` 必须存在；其它 `SKILL.md` 副本必须能由 `scripts/sync-skill.sh` 从 SOT 派生且与 SOT 内容一致（差异仅在 host-specific 扩展 frontmatter 块）

#### 场景:无用户私密字段穿透
- **当** 在 `packaging/` 任何 JSON / YAML / Markdown 文件中查找 `auth_token` / `ct0` / `csrf` / `cookies` / `personalization_id` 等用户私密字段
- **那么** 这些字段必须不存在（注：`bearer_token` 是 X Web 公开 anonymous bearer，允许出现在 SOT `defaults.json` 中作为协议参数兜底；不属于"用户私密字段"）

### 需求:Claude Code plugin 形态

`packaging/claude-code/` 必须是合法的 Claude Code plugin，包含：

- `.claude-plugin/plugin.json`：含 `name`（必须是 `x_likes`）、`version`（与 `Cargo.toml` 同步）、`description`、`author`、可选 `keywords`
- `.mcp.json`：MCP server 启动配置，`command` 必须是 `x_likes_downloader`，`args` 必须是 `["serve", "--mcp"]`，`transport` 必须是 `stdio`
- `commands/list.md`、`commands/auth.md`、`commands/download.md`、`commands/setup.md`：4 个 slash command 实做
- `skills/x_likes/SKILL.md`：从 SOT 派生的副本

`commands/list.md` 与 `commands/auth.md` 必须含 `disable-model-invocation: true`，分别 wrap `x_likes_downloader likes list --json` 和 `x_likes_downloader auth status --json`。`commands/download.md` 必须走 LLM 路由（不带 `disable-model-invocation`），通过 MCP 工具 `mcp__x_likes_downloader__list_likes` + `mcp__x_likes_downloader__download_media` 实施。`commands/setup.md` 必须 wrap `x_likes_downloader setup`。所有 commands 必须假设 binary 在 PATH，不得使用 `${CLAUDE_PLUGIN_ROOT}/...` 引用打包的 binary。

#### 场景:plugin.json 结构合法
- **当** 解析 `packaging/claude-code/.claude-plugin/plugin.json`
- **那么** 必须含 `name == "x_likes"`、`version`、`description`、`author` 四个必填字段；`mcpServers` 字段（如有）必须指向 `./.mcp.json`

#### 场景:slash command 命名空间
- **当** 列出 `packaging/claude-code/commands/*.md`
- **那么** 必须看到 `list.md` / `auth.md` / `download.md` / `setup.md` 四个文件，且各自首行 frontmatter 含合法 `description` 字段

#### 场景:确定性命令跳过 LLM
- **当** 解析 `commands/list.md` 与 `commands/auth.md`
- **那么** 二者 frontmatter 必须含 `disable-model-invocation: true` 且 body 含以 `!` 开头的 shell 调用

#### 场景:MCP server 配置正确
- **当** 解析 `.mcp.json`
- **那么** `command` 必须是 `x_likes_downloader`，`args` 必须是 `["serve", "--mcp"]`

#### 场景:不打包 binary
- **当** 在 `packaging/claude-code/` 中查找可执行文件
- **那么** 必须不存在（plugin 假设用户已装 binary 在 PATH）

### 需求:Codex CLI plugin 形态

`packaging/codex/` 必须是合法的 Codex CLI plugin，包含：

- `.codex-plugin/plugin.json`：富 schema，必须含顶层 `name`（`x_likes`）、`version`、`description`、`author`、`skills: "./skills/"`、`mcpServers: "./.mcp.json"`、`interface` 块（`displayName`、`shortDescription`、`longDescription`、`category: "Productivity"`、`capabilities: ["Interactive", "Write"]`、`defaultPrompt[]` ≤ 3 项每项 ≤ 128 字符、`brandColor`）
- `.mcp.json`：与 Claude Code 相同的 MCP server 启动配置
- `skills/x_likes/SKILL.md`：从 SOT 派生（可选含 `agents/openai.yaml` 兼容块或 SKILL.md frontmatter 中加 `metadata` 块）

Codex CLI plugin 必须不含 slash commands（Codex 无此概念）；触发依赖 LLM 解析 SKILL.md 描述。

#### 场景:plugin.json schema 合法
- **当** 解析 `packaging/codex/.codex-plugin/plugin.json`
- **那么** 必须含顶层 `name == "x_likes"`、`version`、`description`、`author`，且 `interface` 块必须含 `displayName`、`shortDescription`、`longDescription`、`category`、`capabilities`、`defaultPrompt`

#### 场景:defaultPrompt 限制
- **当** 解析 `interface.defaultPrompt`
- **那么** 数组长度必须 ≤ 3；每个字符串长度必须 ≤ 128 字符

#### 场景:skills 目录指向有效
- **当** 检查 `plugin.json.skills` 字段指向的路径
- **那么** 必须为 `./skills/`，且该目录下必须含 `x_likes/SKILL.md`

#### 场景:MCP 配置一致
- **当** 比较 `packaging/codex/.mcp.json` 与 `packaging/claude-code/.mcp.json`
- **那么** `command` / `args` / `transport` 三个字段必须相等（同一 binary，同一启动方式）

### 需求:OpenClaw skill 形态

`packaging/openclaw/x_likes/` 必须接收 v2.0 `skill/` 的整体内容平移，包含：

- `SKILL.md`：含 `metadata.openclaw` 块（声明 `bins: ["x_likes_downloader"]`、`min_version`、`emoji` 可选）
- `README.md`：用户旅程文档
- `defaults.json`：协议参数兜底
- `mcp-config.json`：MCP server 启动配置（保留 v2.0 字段：`command`、`args`、`transport`、`minimum_xld_version`）

`packaging/openclaw/x_likes/SKILL.md` 必须由 `scripts/sync-skill.sh` 从 SOT 派生，注入 `metadata.openclaw` 块；不得手工编辑。

#### 场景:OpenClaw 块就位
- **当** 解析 `packaging/openclaw/x_likes/SKILL.md` frontmatter
- **那么** 必须含 `metadata.openclaw` 块，含 `bins` 数组且 `bins[0] == "x_likes_downloader"`

#### 场景:mcp-config.json 兼容 v2.0
- **当** 解析 `packaging/openclaw/x_likes/mcp-config.json`
- **那么** 必须含 `command == "x_likes_downloader"`、`args == ["serve", "--mcp"]`、`transport == "stdio"`、`minimum_xld_version` 字段

### 需求:自建 GitHub-based marketplace 双 schema

仓库根必须含两份 marketplace 入口 manifest：

- `.claude-plugin/marketplace.json`（Claude Code schema）：含顶层 `name`（`x_likes_downloader`）、`owner.name`、`metadata.{description,version}`、`plugins[]` 数组；每个 plugin 条目含 `name`（`x_likes`）、`source`（`./packaging/claude-code`）、`description`
- `.agents/plugins/marketplace.json`（Codex CLI schema）：含顶层 `name`（`x_likes_downloader`）、`interface.displayName`、`plugins[]` 数组；每个 plugin 条目含 `name`（`x_likes`）、`source.{source: "local", path: "./packaging/codex"}`、`policy.{installation, authentication}`、`category`

两份 marketplace.json 必须使用户能用一行命令装好：

- `claude plugin marketplace add https://github.com/HerbertGao/x_likes_downloader && claude plugin install x_likes@x_likes_downloader`
- `codex plugin marketplace add https://github.com/HerbertGao/x_likes_downloader`（codex 0.128 无独立 `plugin install` 子命令；marketplace add 后用户在 `~/.codex/config.toml` 加 `[plugins."x_likes@x_likes_downloader"] enabled = true` 启用）

#### 场景:Claude Code marketplace 字段
- **当** 解析 `.claude-plugin/marketplace.json`
- **那么** 必须含 `name`、`owner.name`、`metadata.description`、`metadata.version`、`plugins[].source` 五个必填位置；`plugins[0].source` 必须等于 `./packaging/claude-code`

#### 场景:Codex marketplace 字段
- **当** 解析 `.agents/plugins/marketplace.json`
- **那么** 必须含 `name`、`interface.displayName`、`plugins[]`；每个 plugin 条目必须含 `name`、`source.source`、`source.path`、`policy.installation`、`policy.authentication`、`category` 六个必填字段

#### 场景:policy 默认值
- **当** 解析 Codex marketplace plugin 条目
- **那么** `policy.installation` 必须是 `AVAILABLE` 之一（不得为 `NOT_AVAILABLE`）；`policy.authentication` 必须是 `ON_USE`（cookies 时效短，每次使用现配更合适）

#### 场景:版本号同步
- **当** Cargo.toml 的 `version` 与 `.claude-plugin/marketplace.json` 的 `metadata.version` 比较
- **那么** 必须相等（由 `scripts/version.sh` 保证）

### 需求:scripts/sync-skill.sh SOT 派生契约

仓库必须新增 `scripts/sync-skill.sh`，从 `packaging/skill/x_likes/SKILL.md` SOT 派生四份 host 副本：

- `packaging/claude-code/skills/x_likes/SKILL.md`：直接复制（无 host 扩展）
- `packaging/codex/skills/x_likes/SKILL.md`：复制 + 可选注入 `metadata` 兼容块
- `packaging/hermes/skills/x_likes/SKILL.md`：直接复制（Hermes 0.12+ 原生消费 Anthropic 风格 frontmatter，无需 host 扩展）
- `packaging/openclaw/x_likes/SKILL.md`：复制 + 注入 `metadata.openclaw` 块（`bins` / `min_version`）

脚本必须使用 GHA `ubuntu-latest` 与 `macos-latest` runner 默认自带的工具：`bash` + `awk` + `sed` + `jq`。**不允许依赖 `yq`**（GHA runner 默认不预装；多份实现行为不一致）。YAML frontmatter 处理用 `awk` 切块行级操作（不需要完整 YAML parser）；JSON 处理用 `jq`。在 macOS（BSD sed）和 Linux（GNU sed）下行为必须一致——如有 GNU/BSD 差异需用 awk 兜底。脚本必须幂等：连续运行两次后 git diff 必须为空。

`scripts/check-packaging.sh` 必须取代旧 `check-skill-defaults.sh`，遍历 `packaging/{claude-code,codex,openclaw}/` 各自校验 manifest schema：

- 各 plugin.json / mcp-config.json / marketplace.json 必填字段是否存在
- 各 SKILL.md frontmatter 是否含 `name` + `description`
- 各 manifest 中 `command` 字段必须是 `x_likes_downloader`
- 无敏感字段穿透

#### 场景:sync 幂等
- **当** 连续两次运行 `bash scripts/sync-skill.sh`
- **那么** 第二次运行后 `git status` 报告必须无 untracked / modified 文件

#### 场景:派生一致性
- **当** 修改 `packaging/skill/x_likes/SKILL.md` 后运行 `bash scripts/sync-skill.sh`
- **那么** 三份 host 副本必须更新；副本核心内容（除 host 扩展 frontmatter 块外）必须与 SOT 一致

#### 场景:check-packaging 校验通过
- **当** 在干净仓库运行 `bash scripts/check-packaging.sh`
- **那么** 退出码必须是 0；输出必须列出每个 host packaging 目录的校验结果

#### 场景:check-packaging 拒绝敏感字段
- **当** 在 `packaging/` 任何 JSON 文件注入 `auth_token` 字段后运行 `bash scripts/check-packaging.sh`
- **那么** 退出码必须非 0，输出必须指明违反字段及文件路径

### 需求:CI 一致性 guard

`.github/workflows/` 中的 CI 流水线必须新增步骤：

1. PR 触发时跑 `bash scripts/sync-skill.sh`
2. 跑 `git diff --exit-code`，diff 非空则 fail（强制贡献者修改 SOT 后同步副本）
3. 跑 `bash scripts/check-packaging.sh`，校验 manifest schema
4. matrix 在 ubuntu-latest 与 macos-latest 各跑一次，验证脚本跨平台一致

#### 场景:同步漂移触发 CI 失败
- **当** PR 仅修改 SOT `packaging/skill/x_likes/SKILL.md` 而未运行 sync
- **那么** CI 必须 fail，错误信息必须提示运行 `bash scripts/sync-skill.sh`

#### 场景:matrix 跨平台一致
- **当** PR 触发 CI
- **那么** ubuntu 与 macos 两个 runner 必须均运行 sync + check 步骤；二者结果必须一致（exit code 相等、git diff 输出相同）

### 需求:Hermes host adapter (v2.1+ active)

`packaging/hermes/` 必须是 v2.1+ active host adapter，包含：

- `skills/x_likes/SKILL.md`：从 SOT 派生（直接复制，无 host 扩展；Hermes 0.12+ 原生消费 Anthropic 风格 frontmatter）
- `README.md`：含一行 `hermes skills install <raw-URL> --yes` 装命令、`~/.hermes/config.yaml` 手动 `mcp_servers` 注册步骤（hermes 0.12 CLI argparse 把 `--mcp` 当顶层 flag，`hermes mcp add` 不可用，需直接 YAML 编辑）、SKILL.md 落地路径（`~/.hermes/skills/x_likes/SKILL.md`）

`packaging/hermes/skills/x_likes/SKILL.md` 必须由 `scripts/sync-skill.sh` 从 SOT 派生；不得手工编辑。

#### 场景:Hermes adapter 完整
- **当** `ls packaging/hermes/`
- **那么** 必须看到 `skills/x_likes/SKILL.md` 与 `README.md` 两份产出

#### 场景:Hermes README 含装命令
- **当** 解析 `packaging/hermes/README.md`
- **那么** 必须含 `hermes skills install` 命令、`~/.hermes/config.yaml` 配置示例、binary 安装前置说明

### 需求:Cursor v2.2 占位

`packaging/cursor/README.md` 必须存在并包含：

- 说明已验证 SKILL.md 跨工具兼容（共享 frontmatter）
- 实施提示：把 SOT 副本放到 `.cursor/skills/` 或兼容 `.claude/skills/`，可选加 `paths:` glob
- 排期标注：v2.2
- 欢迎 PR 链接

#### 场景:占位 README 存在
- **当** `ls packaging/cursor/`
- **那么** 必须看到 `README.md`

#### 场景:README 含必要信息
- **当** 解析 `packaging/cursor/README.md`
- **那么** 必须含 "v2.2"、"SOT"、"PR" 三个关键词

### 需求:顶层 README host 适配状态矩阵

仓库根 `README.md` 必须含一节"Host 适配状态"，以表格形式列出：

| Host | Status | Path | Notes |

每行对应一个 host（Claude Code / Codex CLI / OpenClaw / Hermes / Cursor 至少 5 行），Status 字段使用 `✅ v2.1+` / `🔜 v2.2` / `📦 v1.x+`，Path 列指向 `packaging/<host>/`。

#### 场景:状态表存在
- **当** 解析顶层 `README.md`
- **那么** 必须找到至少 5 行的 host 适配状态表，且 Claude Code / Codex CLI / OpenClaw 三行的 Status 必须含 `v2.1` 或更新版本号

### 需求:Plugin 不打包 binary

`packaging/{claude-code,codex,openclaw}/` 内必须不含 binary 可执行文件；commands / SKILL.md / manifest 内引用 binary 必须使用裸名 `x_likes_downloader`，假设其在 PATH。

每个 host adapter 的 README.md（或对应 SKILL.md）必须列出 binary 安装路径选项：

- `cargo install x_likes_downloader`（如果发布到 crates.io）
- `brew install ...`（如有 tap）
- `curl -fsSL <url> | sh`（GHA release 安装脚本）
- 直接下载 GitHub Releases binary 并放到 PATH

#### 场景:无打包 binary
- **当** 在 `packaging/` 下用 `find . -type f -perm +111` 查找可执行文件
- **那么** 必须不存在 `x_likes_downloader` 或类似名称的 binary

#### 场景:command 引用裸名
- **当** 解析 `packaging/claude-code/commands/*.md` 与 `packaging/{codex,openclaw}/skills/x_likes/SKILL.md` 中所有 shell 调用
- **那么** binary 调用必须使用 `x_likes_downloader`（不得使用 `${CLAUDE_PLUGIN_ROOT}/...` 或绝对路径）

### 需求:版本绑定 — binary 与 plugin 版本字段同步

`scripts/version.sh` 在升级 `Cargo.toml` 版本时必须同步更新以下字段：

- `.claude-plugin/marketplace.json` 的 `metadata.version`
- `.agents/plugins/marketplace.json`（如该 schema 有顶层 version 字段）
- `packaging/claude-code/.claude-plugin/plugin.json` 的 `version`
- `packaging/codex/.codex-plugin/plugin.json` 的 `version`
- `packaging/skill/x_likes/SKILL.md` frontmatter 的 `min_binary_version`
- `packaging/openclaw/x_likes/mcp-config.json` 的 `minimum_xld_version`

所有版本字段必须始终与 `Cargo.toml` 同步（含 patch 升级）；不存在"patch 不动"的例外。统一规则简化 `version.sh` 与 `check-packaging.sh` 实现，且 `min_binary_version: 2.1.5` 等价于"binary 至少 2.1.5"语义合法。

`scripts/check-packaging.sh` 必须验证以上版本字段一致；不一致时 fail。

#### 场景:版本字段同步
- **当** 运行 `bash scripts/version.sh patch`（或 minor / major / 显式版本号）
- **那么** Cargo.toml、所有 plugin.json、所有 marketplace.json 的 version 字段必须更新到同一值

#### 场景:check 检测漂移
- **当** 手工修改 `Cargo.toml` 版本但未运行 `version.sh`
- **那么** `bash scripts/check-packaging.sh` 退出码必须非 0，输出必须列出所有不一致的版本字段路径
