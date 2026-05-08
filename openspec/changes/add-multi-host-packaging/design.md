## 上下文

v2.0 完成 MCP server 闭环后，packaging 层留下两个事实：

1. **现状**：`skill/` 目录是 v1 OpenClaw 风格单 host adapter，含 SKILL.md / defaults.json / mcp-config.json / README.md。v2 升级后 SKILL.md 已重写为 MCP-based。
2. **生态调研**：5 个主流 Agent 工具（Claude Code / Codex CLI / OpenClaw / Hermes / Cursor v2.4+）均支持 Anthropic 风格 SKILL.md（YAML frontmatter `name`+`description` + Markdown 正文 + 可选 `scripts/` `references/` `assets/`）。差异仅在：
   - 路径约定（`~/.claude/skills/` vs `~/.codex/skills/` vs `~/.hermes/skills/` 等）
   - 可选扩展 frontmatter 块（`metadata.openclaw` / `agents/openai.yaml` / `paths:` glob）
   - 是否有独立 plugin manifest（Claude Code `.claude-plugin/plugin.json`、Codex `.codex-plugin/plugin.json`）
   - 是否有 slash command 概念（**仅 Claude Code** 有 `/<plugin>:<cmd>`；Codex 通过 LLM 隐式触发 skill）

利益相关者：

- **现有 v2.0 OpenClaw 用户**：少量；他们 ClawHub 注册 URL 需从 `<repo>/skill` 改为 `<repo>/packaging/openclaw/x_likes`。文档明确说明，但**路径变化是 packaging 层 breaking**
- **Claude Code 用户**：v2.0 装本 MCP 是手写 `.mcp.json`；v2.1 后 `claude plugin install x_likes` 一行装好，并获得 `/x_likes:list` 等 4 个 slash command
- **Codex CLI 用户**：v2.0 完全不支持；v2.1 提供 `codex plugin install x_likes`，自动加载 SKILL.md，binary 假设在 PATH
- **Hermes / Cursor 用户**：v2.1 占位文档说明已验证 SKILL.md 兼容；v2.2 出实做
- **未来贡献者**：理解 SOT + sync 派生契约后，加新 host 是机械工作（写 `packaging/<host>/`，加同步规则到 `sync-skill.sh`）

约束：

- 不触碰 src/、Cargo.toml、tests/、cargo install、GHA release.yml
- 不打包 binary 进 plugin（plugin 引用 PATH 上的 `x_likes_downloader`，跟 codex 插件引用 `${CLAUDE_PLUGIN_ROOT}/scripts/...` 调用 user 装好的 codex CLI 同模式）
- 所有 SKILL.md 副本必须从 SOT 派生；CI guard 防漂移

## 目标 / 非目标

**目标：**

- 一份 SOT (`packaging/skill/x_likes/SKILL.md`)，三个 host 实做（Claude Code / Codex CLI / OpenClaw）共享之
- Claude Code 用户能用 `/x_likes:list` `/x_likes:auth` `/x_likes:download` `/x_likes:setup` 4 个 slash command
- Codex CLI 用户在 codex 交互窗口里用自然语言（"show my X likes"）触发 SKILL.md 路由到 binary CLI
- 自建 GitHub-based marketplace：两份 marketplace.json 各自 schema 正确，`{claude,codex} plugin install x_likes` 一行装好
- v2.0 OpenClaw 用户走 `packaging/openclaw/x_likes/` 路径，行为不变
- Hermes / Cursor 占位文档清晰，README 状态表注明"v2.2"
- CI 校验 SOT 与三份 host 副本一致（sync 后 git diff 必须空）
- 不依赖任何外部服务（marketplace 完全是 git repo）

**非目标：**

- 实施 Hermes / Cursor 的 host adapter（v2.2）
- 实施 Continue / Cody / Aider 适配（不同范式，需独立 PR）
- 修改 MCP server 行为或 4 个 MCP 工具的 schema（那是 Change A 的事）
- 改动 Rust binary、Cargo.toml、tests/
- 把 binary 打进 plugin（保持"plugin 引用 PATH binary"模式）
- 上架第三方 marketplace（ClawHub / 官方 Anthropic marketplace 等），那是 v2.2+ 的事

## 决策

### D1：SOT 在 `packaging/skill/x_likes/SKILL.md`，host 副本 CI 同步生成

**选择**：`packaging/skill/x_likes/` 是单一可信源，含 host-agnostic SKILL.md + README.md + defaults.json。三个 host 目录（claude-code/codex/openclaw）的 `skills/x_likes/SKILL.md` 由 `scripts/sync-skill.sh` 派生：基于 SOT，按 host 添加扩展 frontmatter 块（OpenClaw 加 `metadata.openclaw`、Codex 可选加 `agents/openai.yaml`、Claude Code 不加扩展）。CI 在 PR 流程跑 `sync-skill.sh` 后 `git diff`，diff != 0 则 fail。

**理由**：

- 5 host 共享格式 80%，差异仅在路径和可选 frontmatter 块——SOT + 派生比"每个 host 独立维护"工程量低 5 倍
- "PR-time sync + diff guard"避免运行时漂移：贡献者修了 SOT 没同步会 CI 红
- SOT 路径放 `packaging/skill/x_likes/` 而非 `skill/`，避免与"普通 host adapter"看起来同级，强调它是源
- 不用 symlink（Windows 不友好、git 处理不一致）
- 派生脚本而不是 build-time 模板引擎：`sync-skill.sh` 用 awk/sed 即可，不引入新工具链

**替代方案**：

- **每个 host 独立维护 SKILL.md**：被否决——5 处复制 = 5 倍维护成本，漂移风险高
- **symlink 方式**：被否决——跨平台兼容性差，git 行为微妙
- **build.rs 在 cargo build 时生成 host 副本**：被否决——把 packaging 工作绑死到 cargo 工作流，违反"不动 src/"约束
- **SOT 直接放 `packaging/openclaw/x_likes/SKILL.md`，其他 host 从这同步**：被否决——OpenClaw 是个 host，不该兼当 SOT；同名混淆

### D2：自建 GitHub-based marketplace 而非上架第三方

**选择**：在仓库根放两份 marketplace.json：

- `.claude-plugin/marketplace.json`（Claude Code schema：`{name, owner, metadata, plugins[{name, source: "./packaging/claude-code", description}]}`）
- `.agents/plugins/marketplace.json`（Codex schema：`{name, interface: {displayName}, plugins[{name, source: {source: "local", path: "./packaging/codex"}, policy: {installation: "AVAILABLE", authentication: "ON_USE"}, category}]}`）

用户 `<host> plugin marketplace add https://github.com/HerbertGao/x_likes_downloader` + `<host> plugin install x_likes` 两行装好。

**理由**：

- 单仓库自治：发版即同步，不依赖第三方审核
- 用户路径短：从 git URL 直接装，不用先去 marketplace 网站找
- 升级语义自然：`git pull` 即升级（marketplace 客户端各自的 `upgrade` 命令也走同样机制）
- 双 schema 是必然——Claude Code 和 Codex 各自定义 marketplace.json 字段，拉平等于二选一

**替代方案**：

- **上架第三方 marketplace**：被否决——v2.1 阶段用户量小，审核流程拖延 2-4 周；v2.2 评估
- **单 marketplace.json**：被否决——schema 不兼容，强行拉平会让两边都跑不了
- **不做 marketplace，文档里教用户手装**：被否决——破坏"一行装好"体验

### D3：Claude Code plugin 含 4 个 slash commands，Codex plugin 不含

**选择**：

```
packaging/claude-code/                     packaging/codex/
├── .claude-plugin/plugin.json             ├── .codex-plugin/plugin.json (富 interface)
├── .mcp.json                              ├── .mcp.json
├── commands/                              └── skills/x_likes/SKILL.md
│   ├── list.md       → /x_likes:list
│   ├── auth.md       → /x_likes:auth
│   ├── download.md   → /x_likes:download
│   └── setup.md      → /x_likes:setup
└── skills/x_likes/SKILL.md
```

`/x_likes:list` `/x_likes:auth` 用 `disable-model-invocation: true`（确定性 shell + 表格渲染）；`/x_likes:download` `/x_likes:setup` 走 LLM（前者需要语义路由 ID 解析、后者需要交互引导）。

**理由**：

- Claude Code 是用户拍板的优先 host，slash command 是它独有的优势体验，必须用上
- `disable-model-invocation` 用在确定性命令上，避免 LLM 多余 round-trip，跟 codex 自己 marketplace 上 status/result 命令同模式
- Codex CLI 没有 `/<plugin>:<cmd>` 概念——硬塞会跑不起来；它的"快速触发"靠 SKILL.md 让 LLM 按描述识别意图，effectively 完全等价
- 4 个 slash command 选择对应 4 个 MCP 工具语义——不发明新动词

**替代方案**：

- **Claude Code 也只用 SKILL.md，不做 commands**：被否决——浪费 Claude Code 平台优势
- **8 个 commands**（每个 + json 输出版本）：被否决——爆炸；JSON 输出走 binary `--json` flag 即可

### D4：Slash command 实现走 binary CLI `--json` 出口而非 MCP

**选择**：`commands/list.md` 内是 `!`xld likes list --json --count "$1"` `（disable-model-invocation），不经过 MCP server。`commands/download.md` 是唯一例外——它需要 progress 流，走 MCP `mcp__x_likes_downloader__download_media` 工具调用。

**理由**：

- 启动开销：CLI 一次 spawn ≈ 20ms；MCP 需先启动 server，stdio 握手 + tools/call ≈ 100-200ms
- 简单性：commands/*.md 直接用 binary `--json` 即可，不需要 MCP server 已加载
- progress 唯一是 `download_media` 的需求；其他工具都是一次性返回 JSON
- Setup 涉及交互（粘贴 cURL），spawn `xld setup` 拉起交互流就够；不需要 MCP 路径

**替代方案**：

- **全部走 MCP**：被否决——慢且要求 server 已加载
- **全部走 binary CLI**：被否决——download 没 progress 体验糟糕

### D5：Plugin 不打包 binary，引用 PATH 上的 `x_likes_downloader`

**选择**：plugin 内 commands/*.md 使用 `x_likes_downloader` 命令名（不带 `${CLAUDE_PLUGIN_ROOT}/...` 前缀）。安装文档要求用户先装 binary（`brew install`、`cargo install`、`curl | sh`、或下载 GHA release）。

**理由**：

- 跨平台：6 个 binary 平台用户没法在 plugin 里"写死哪个用"
- 升级语义：用户更新 binary（`brew upgrade`）和更新 plugin（`<host> plugin upgrade`）解耦
- 跟 codex 自家 plugin 同模式（codex plugin 引用 `node ${CLAUDE_PLUGIN_ROOT}/scripts/...` 调用用户装好的 codex 二进制）
- README 的"先装 binary"步骤可以加链接到 GitHub Releases，体验仍可控

**替代方案**：

- **打包 binary 到 plugin assets/**：被否决——plugin 体积爆炸（每个 host 拷贝 6 平台 binary），跨平台困难
- **plugin 第一次启动时下载 binary**：被否决——plugin 不该有 side-effect 安装，且离线场景失败

### D6：OpenClaw 路径迁移：`skill/*` → `packaging/openclaw/x_likes/*`

**选择**：v2.1 把 `skill/` 整体平移到 `packaging/openclaw/x_likes/`，这是 packaging 层 breaking。v2.0 OpenClaw 装机用户必须把 ClawHub 注册的 URL 改为新路径。

**理由**：

- v2.0 上线时 OpenClaw 用户量极少（OpenSpec archive 显示活体冒烟仅本机一台）
- 不迁移就得让 SOT 直接放 OpenClaw 路径下，污染抽象（OpenClaw 是个 host，不该兼当 SOT）
- v2.1 的 multi-host 抽象本身就是"path 是 host-specific"的强论点
- README + GitHub Release notes 明确说明此 breaking + 迁移命令一行

**替代方案**：

- **保留 `skill/` 作为 OpenClaw 路径**：被否决——抽象不一致
- **`skill/` 设为软链接到 `packaging/openclaw/x_likes/`**：被否决——Windows 不友好；让人误以为路径仍可用而埋雷

### D7：Hermes / Cursor v2.2 占位

**选择**：`packaging/{hermes,cursor}/` 仅含 README.md，说明：

- 已验证 SKILL.md 跨工具兼容（共享 frontmatter）
- 实现 = 把 SOT 副本放进对应路径 + 加 host-specific 扩展块（Cursor 加 `paths:`）
- 欢迎 PR；优先级 v2.2

**理由**：

- 占位比沉默好：清楚告诉两边用户"不是不支持，是排期"
- 避免 v2.1 范围爆炸——5 个 host 一起做风险高
- 让贡献者有明确入口（"看 packaging/codex 模仿"）

### D8：sync-skill.sh 用纯 bash + awk + jq，不引入 build 工具链

**选择**：`scripts/sync-skill.sh` 是 bash 脚本，从 SOT 读 SKILL.md。YAML frontmatter 用 `awk` 切块（`/^---$/` 起止），insert/replace 用 `awk`/`sed` 行级操作（不解析完整 YAML）；JSON 文件（defaults.json、plugin.json、marketplace.json）用 `jq` 处理。对 OpenClaw 副本插入 `metadata.openclaw` 块（awk 在 frontmatter 末尾追加固定行），对 Codex 副本可选插入 `metadata` 兼容块，对 Claude Code 副本直接 cp。CI 跑后 `git diff --exit-code` 校验。

**理由**：

- 工具链最小：`bash` + `awk` + `sed` + `jq` 全是 GHA `ubuntu-latest` / `macos-latest` 默认 runner 自带工具，零安装
- **明确不依赖 `yq`**：`yq` 在 GHA runner 默认不预装；多份实现（mikefarah/go-yq、kislyuk/yq）行为不一致；用 awk 做行级 frontmatter 操作即可，无需完整 YAML parser
- 不引入 Node/Python build 步骤——packaging 应保持轻量
- CI guard 简单：`bash scripts/sync-skill.sh && git diff --exit-code`

**替代方案**：

- **写 Rust 工具**：被否决——把 packaging 绑死到 cargo workspace
- **Python 模板引擎（Jinja2 等）**：被否决——为简单 substitution 引入运行时

### D9：commands/*.md 实现细节——4 个命令规约

**选择**：

| 命令 | frontmatter | 实现 |
|---|---|---|
| `/x_likes:auth` | `disable-model-invocation: true`<br>`allowed-tools: Bash(x_likes_downloader:*)` | `!`x_likes_downloader auth status --json` ` 然后渲染 status 一行（healthy / auth_expired / etc） |
| `/x_likes:list [count]` | `disable-model-invocation: true`<br>`allowed-tools: Bash(x_likes_downloader:*)`<br>`argument-hint: '[count]'` | `!`x_likes_downloader likes list --json --count "${1:-20}"` ` 然后渲染压缩表格（id / author / 媒体数量 / 时间） |
| `/x_likes:download <ids...>` | LLM 介入<br>`argument-hint: '<tweet-id> [tweet-id...]'` | LLM 路由：先调 `mcp__x_likes_downloader__list_likes`（带 count 拉到含目标 IDs 的页）→ 过滤匹配的 items → 调 `mcp__x_likes_downloader__download_media` |
| `/x_likes:setup` | `disable-model-invocation: true` | `!`x_likes_downloader setup` ` 拉起交互；用户手动 paste cURL |

**理由**：

- auth/list/setup 都是确定性，跳 LLM 省 token + 加速
- download 是唯一需要"理解 IDs 在哪一页"的语义路由，必须走 LLM 调 MCP
- 命令名跟 4 个 MCP 工具语义一致——用户记一套词

### D10：marketplace.json 双 schema，全字段写满 vs. 只写最小集

**选择**：双 marketplace.json 都写完整字段——Claude Code 含 `metadata.{description,version}`、Codex 含 `interface.displayName` + `policy.{installation,authentication}` + `category`。

**理由**：

- Codex plugin-creator skill 文档明确说"defaults 也要写出来"——避免运行时报缺字段
- 显式 > 隐式：未来人 review 时一目了然
- `policy.authentication: ON_USE` 比默认 `ON_INSTALL` 更适合本工具——cookies 时效短，在装的时候就让用户配置反而不流畅

### D11：版本绑定——所有 SKILL.md / plugin.json 自动带 binary 版本下界

**选择**：`packaging/skill/x_likes/SKILL.md` 在 frontmatter 含 `min_binary_version: 2.1.0`，`sync-skill.sh` 派生时把版本写入：

- OpenClaw 的 `metadata.openclaw.bins[].min_version`
- Codex `.codex-plugin/plugin.json.dependencies` 字段（如 schema 支持）
- Claude Code 的 `.claude-plugin/plugin.json.minimum_x_likes_downloader` （自定义字段，文档化用法）

`scripts/version.sh` 升级 Cargo.toml 时同步更新所有这些版本字段。

**理由**：

- 防止 v2.0 binary 装了 v2.1 plugin 跑出 schema 不匹配错误
- `version.sh` 已是版本入口，扩展自然
- 即使 host 不强制校验也写——用户 cat 看到能 self-diagnose

## 风险 / 权衡

| 风险 | 缓解 |
|---|---|
| Codex CLI plugin 实测发现 schema 我们没踩到的字段 → 装不上 | 实施期用本机 codex CLI 0.128 装一遍 + `codex plugin marketplace remove/add` 走完一个 round；遇到字段问题查 codex `plugin-creator` skill 自带 reference |
| sync-skill.sh 在 macOS（BSD sed）vs Linux CI（GNU sed）行为不一致 | 用 `awk` / `sed` / `jq` 工具链（与 D8 决策一致）；BSD/GNU 差异处用 awk 兜底；CI matrix 在 ubuntu + macos 各跑一次校验 |
| OpenClaw 老用户没看 release notes，按 v2.0 路径跑 | README + GitHub Release notes 大字标注；保留 `skill/` 一段时间作软链接？→ 否决（D6 理由）；改用 README 顶部一段警告 |
| 自建 marketplace 用户体验：`<host> plugin marketplace add` 失败 | 文档详细列两个 host 的具体命令；用 `--source local` 把仓库 clone 后本地装作为兜底 |
| Claude Code 的 commands/*.md 在不同 Claude Code 版本下 frontmatter 字段名变更 | 实施期实测最新 Claude Code（截至本次会话本机版本）；版本依赖在 plugin.json 里声明 |
| `disable-model-invocation` 在 Claude Code 新旧版本支持度 | 实测；如不支持降级为 LLM 介入版本（小性能损失，无功能损失） |
| Cargo.toml 与各 plugin.json 版本字段漂移 | `scripts/version.sh` 扩展为同步所有版本字段；CI 校验所有版本字段一致 |
| 没装 binary 的用户运行 slash command 看到 "command not found" | commands/*.md 内 shell 用 `command -v x_likes_downloader || { echo "请先装 binary：..."; exit 1; }` 兜底；提示链接到 GitHub Releases |
| Hermes / Cursor 用户期待 v2.1 就有，看到占位失望 | README 状态表清晰标注 "v2.2"；packaging/{hermes,cursor}/README.md 给出预估时间 |
| sync-skill.sh CI guard 假阴性（同步后有 diff 但 CI 没检测） | `git diff --exit-code` 是标准做法；CI 步骤显式 fail-on-diff |

## Migration Plan

1. **Phase A（结构准备）**：建 `packaging/skill/x_likes/`、`packaging/openclaw/x_likes/`，把 `skill/*` 内容平移过去。CI 不报错。
2. **Phase B（Claude Code adapter）**：建 `packaging/claude-code/`、`.claude-plugin/marketplace.json`、4 个 commands/*.md。本机 `claude plugin marketplace add file://...` 验证 4 个 slash command 都能跑。
3. **Phase C（Codex adapter）**：建 `packaging/codex/`、`.agents/plugins/marketplace.json`。本机 `codex plugin marketplace add file://...` + `codex plugin install x_likes` 验证 SKILL.md 加载、binary 调用成功。
4. **Phase D（同步契约）**：`scripts/sync-skill.sh` + CI guard 上线。运行后 git diff = 0。
5. **Phase E（占位 + 文档）**：`packaging/{hermes,cursor}/README.md`、顶层 README 状态表、packaging/README.md 架构说明。
6. **Phase F（破坏性迁移说明）**：GitHub Release notes 大字标注 OpenClaw 路径迁移；README 顶部加迁移指引。
7. **Phase G（活体冒烟）**：本机两个 host（claude / codex）各装一次 + 运行所有命令；OpenClaw 路径手测一次。
8. **Phase H（PR + Codex review 循环）**：跟 v2 同样的 review-fix 循环，到 codex review clear。

回滚策略：因为不动 src/，回滚 = `git revert` 整个变更 + 删掉新增 `packaging/` 子目录。OpenClaw 用户在回滚后路径仍指向 `packaging/openclaw/x_likes/`（已平移），需要恢复 `skill/` 软链接，或单独发 patch 把 `skill/` 加回。

## 已关闭决策（原 Open Questions）

- **D-OQ1（关闭）GHA 自动版本同步**：v2.1 **不**实施 GitHub Release Action 自动更新 marketplace.json 版本号。v2.1 通过本地 `scripts/version.sh` 手动同步 + `scripts/check-packaging.sh` 在 CI 校验所有版本字段一致；如出现漂移，CI 红灯，贡献者本地跑 `version.sh` 即可。GHA 自动化留 v2.2 评估（届时若 release 频率高再加）。
- **D-OQ2（关闭）OpenClaw 迁移直接破，不保留软链接**：v2.1 直接破坏 `skill/` 路径，不保留软链接过渡。理由：(1) v2.0 OpenClaw 装机量极少（OpenSpec archive 显示活体冒烟仅本机一台）；(2) Windows 不友好（git 软链接行为微妙）；(3) 软链接遗留 v2.2 仍要清理，不如一次到位。缓解：README 顶部加大字"v2.0 → v2.1 迁移指引"；GitHub Release notes 突出标注；`tasks.md` Phase F 已覆盖。
- **D-OQ3（关闭）Hermes 占位 README 指向**：`packaging/hermes/README.md` **同时**列出两个链接——nousresearch GitHub（社区主入口）+ Skills Hub 官方文档（如有最新版）；实施时活体打开两个 URL 各看一眼，取当前可访问的链接写入。如其中一个 404 / 403，则只保留另一个并加注释。Cursor 占位 README 同此处理（指向 Cursor Skills 官方文档 + 任何活跃社区入口）。
