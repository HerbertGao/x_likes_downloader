---
name: x-organize
description: 整理 X(Twitter) 下载的媒体文件——把「待归类」积压文件按人归入文件夹，用 UserByScreenName API 拿真名+bio 识别匿名 username，交叉比对 alias/folder，追踪 Unknown 旧账号的昵称与新号线索，并合并同一人的重复文件夹。当用户说「又攒了一批帮我归类」「查一下 Unknown 旧号」「扫一下重复文件夹」「XXX 和 YYY 是同一人合并」时用。
metadata:
  internal: true
---

# X 下载文件归类

处理 X 下载媒体的按人归类。所有操作**本地跑**（媒体库挂载在本机），绝不 ssh 到远端。

> `metadata.internal: true`：本 skill 从 `npx skills` 的常规发现中隐藏（需 `INSTALL_INTERNAL_SKILLS=1` 才可见）。它服务于播放库整理这类本地工作流，不属于对外分发的公开 skill。

## 配置（必读）

仓库里**不写死任何路径**。BASE 必填，其余有默认值：

| 环境变量 | 用途 | 默认 |
| --- | --- | --- |
| `X_ORGANIZE_BASE` | 父级库目录（每人一夹）**必填** | 无，缺失即报错 |
| `X_ORGANIZE_CREDS` | 凭据 JSON | `~/.config/x_organize/creds.json` |
| `X_ORGANIZE_WORK` | 备份 + move-log 工作区 | `/tmp/x_organize` |
| `X_ORGANIZE_PROXY` | x.com 出网代理，如 `http://127.0.0.1:7890` | 空＝走系统/TUN |
| `X_ORGANIZE_BACKLOG_DIRNAME` | 待归类目录名 | `Z - 自动下载待归类` |
| `X_ORGANIZE_ALIAS_FILENAME` | 别名文件名 | `username_aliases.txt` |
| `X_ORGANIZE_UNKNOWN_DIRNAME` | 无线索账号的去处 | `Z - Unknown` |

也可写用户级配置文件（默认 `~/.config/x_organize/config.json`，键名去掉前缀并小写）：

```json
{ "base": "/path/to/Twitter", "proxy": "http://127.0.0.1:7890" }
```

派生路径：`BACKLOG = BASE/<backlog_dirname>`，`ALIAS = BACKLOG/<alias_filename>`，`UNKNOWN = BASE/<unknown_dirname>`。以上全部由 `scripts/config.py` 解析，脚本直接 `import config` 取值。

## 格式

- alias 行：`主名,别名1,别名2,…`（主名多为中文昵称，别名多为 X username；也可能主名就是 username）
- 文件名：`<username>_<19位tweet_id>_<media_id>.<ext>`
- 文件夹命名不严格：`主名` / `主名,别名,…`(csv) / `主名+中文后缀`(如 `Emperor圣主`) 三种风格混用。**注意主名与夹名可能不相等**——`Emperor` 是别名主名而夹名是 `Emperor圣主`，这类差异会让「精确 splseg 匹配」失败。

## 前置：凭据 + 代理

- **x.com 在国内被墙**：直连常 SSL reset。先确认代理可用——`curl -x $X_ORGANIZE_PROXY https://x.com/robots.txt` 拿 200；TUN 模式下直连也会通。都不通就让用户开代理。
- 凭据键：`bearer/cookie/csrf/query_id`，可选 `user_agent/features/fieldToggles`。**绝不写 /tmp、绝不入仓**，建议 `chmod 600`。ct0 必须等于 `x-csrf-token`。过期(403/401)就让用户随便抓一个 x.com graphql 登录态 curl，取 bearer/cookie/ct0 更新。
- **留神 429**：连续拉取几十个账号很容易触发限流。分批、失败就停，别硬重试。

## 匹配偏好（重要）

**只做严格匹配**，不用置信度分级、不做前缀/模糊归一化。bio 取证必须「关键词 + @mention 近邻」——如「大号/原号/备用/小号/防走丢/防失联」后约 40 字符内含 `@xxx` 或某 alias 成员——**不能**用「bio 含主名就算」，也不要用「真名包含现有夹名」直接下结论：一堆人自称「直男S」会全误匹配（实测 `GIN(直男S)` 与已有夹 `直男S` 是**不同人**）。

判定归入现有夹的可靠依据，按强度排序：

1. 真名**等于**某现有夹的逗号分段（≥2 中文字符 / ≥4 英文字符）
2. 严格 @ 互指（关键词近邻）
3. 现有夹里已存在同一人的其它 username（扒文件夹内容核实）

名字只是**包含**关系（如 `浪子` vs 夹 `逆天浪子`、`李班长48码（成都）` vs 夹 `李班长`）必须问用户。

## 工作流

### 1. 严格匹配扫描

`python3 scripts/scan.py` → 输出 MATCHED（可直接搬）+ UNMATCHED（需识别）。

### 2. 搬 MATCHED

用 `organize_lib.Organizer` 现写 apply 脚本搬走。顺带处理大小写/拼写变体（如 `sszspmaster` vs 别名 `SszSMaster` 仅大小写差、`playdogboys` vs `playdogboy` 复数差 → 加别名归入）。

### 3. UNMATCHED 拉真名+bio

`python3 scripts/api.py <name1> <name2> …` → `screen_name<TAB>真名<TAB>bio`。注销号显示 `__NORESULT__` / `__UserUnavailable__`，网络失败显示 `ERR:…`（429 限流常见，稍后再试）。

### 4. 交叉比对

- bio 里 `大号@xxx`/`备用号@xxx` 互指 → 同一人，合并
- 真名 == 某现有夹的分段 → 归入该夹，不建新
- 真名对不上任何现有 → 新人，默认按真名建夹 + 新 alias 行；先查真名有没有跟现有夹撞名
- 不可用账号无线索 → 问用户或丢 `Z - Unknown`；需要追踪旧号时，走下方「Unknown / 不可用账号的追加搜索」，不要把 API 无结果等同于永久注销
- 仅名字包含关系 → **必须问用户**

### 5. 决策 + apply

拿不准的用 AskUserQuestion 批量问（合并用哪个名/注销号怎么处理/怪名用啥），一次最多 4 题。然后 import `organize_lib` 现写 apply：`move_user` / `add_alias` / `new_alias` / `merge_folder` / `rename_folder` / `merge_alias`。`Organizer()` 自动备份 alias，`finish(tag)` 落 move-log。

**先跑 `DRY_RUN=1`**，核对账号数/文件数/新夹名/冲突，再真跑。

新人夹名建议清掉 emoji 与特殊符号（`宇爹s🖤军dom` → `宇爹s军dom`、`Currifew♠` → `Currifew`、`Calcifer巳爷–DOM` → `Calcifer巳爷-DOM`），否则跨平台同步与命令行都会别扭。

## 重复文件夹合并

三层检测（弱→强）：① 文件夹名共享 token（噪声大，`master`/`圣主`/`直男S` 这类通用词会撞名，**必须核实**）② alias 行共享 entry ③ 同一 username 文件散落多夹（最强）。

区分**真合并**(同一人两夹) vs **错放文件**(不同人，某文件丢错夹——只移那个文件别合夹)。核实手段：扒候选夹里实际 username（文件名前缀）。username 完全不同 = 大概率不同人只是名字撞车。

用户直接断言同一人时（如「SD四爷=深圳四爷」）→ `merge_folder` + `merge_alias`，旧名都留作别名，重命名成用户指定的名。

**悬空 alias 行**：alias 主名指向的夹根本不存在时（如 `圣主,Daheiking11` 而父目录无 `圣主` 夹），这是别名重复而非夹重复 → 用 `merge_alias` 并到真实夹（如 `肖晨浩_圣主`），别去建一个空夹。

## Unknown / 不可用账号的追加搜索

先只读调查，不因搜索结果自动搬文件或修改 alias。用户明确确认身份且授权归类后，才按原有 dry-run / apply 流程操作。

### 1. 盘点与本地对照

- 按文件名前缀统计原 ID 和文件数；没有原 ID 的文件单列，不声称本方案覆盖它们。
- 先查已有 alias、夹名分段及夹内文件的 username；已有精确归属时无需猜新号。
- 已确认的原 ID 应保留为别名；疑似仿冒账号不能因引用昵称就加入别名。

### 2. X 站内搜索：用户 + 最新

按用户授权使用 `ego-browser`，先读其 Skill，复用已有登录态。每个原 ID 依次搜「用户」（原 ID）和「最新」（引号包裹原 ID），分批先看首屏；有相关线索再定向展开，报告是否分页。

通过页面正常搜索捕获实际 `SearchTimeline` GraphQL 请求，`variables.product` 分别为 `People` / `Latest`。query ID、features 等取当前页面请求，不硬编码或猜测。可用 Ego `page.cdp("Network.enable")`、`page.events()` 及 `Network.getResponseBody` 读取响应；`page.events()` 读取后清空缓冲。不要依赖该接口未提供的 `waitForResponse`，也不要调用 `page.cdp.Network.enable()`。

- `People`：找原 handle、明确引用原 ID 的显示名/bio、主备账号声明。
- `Latest`：看历史回复及 mention 的 handle/name/user ID，记录原始帖子 URL。mention 的 name 只称「历史 mention 显示名线索」，不当作当前本人资料或独立核实的旧昵称；回复者不是目标本人。
- 每次确认 HTTP 状态及 API errors；空响应不当作空结果。非成功响应、鉴权失败、网络错误或 429 立即停止当前批次，不硬重试；明确哪些查询未完成。
- 在页面内使用登录态，不导出凭据。CDP 事件可能含 cookie、authorization、csrf、postData；只输出筛选后的公开账号/帖子字段，不打印、保存完整请求或事件。

### 3. 仿冒引用与公开索引补查

「昵称｜原 ID」、bio 中的 `#原ID`、复制简介或外部平台迁移文案，可用于提出昵称候选，**不能证明候选是本人新号，也不能仅凭这些特征断定仿冒**。昵称再对照现有夹；仅包含关系仍需问用户。

站内线索不足或限流时，可搜索公开网页索引，不继续重试 X API：精确原 ID、候选昵称 + 原 ID，以及已公开的共同标识/简介短句。保留完整来源 URL、可引用原文及资料属于当前页面还是历史缓存；不要把搜索工具的生成摘要当作已读到的原始页面。原站 403、仅摘要命中、其它渠道未复现时明确未完成独立核验，不能宣称已验证当前资料。

主备身份仍按「关键词 + @mention 近邻」核对；普通 @mention、同名、复制号引用或单一共同标识不足以自行合并。要求几个候选时，只列有具体关联依据的账号；不足几个就如实说明，同一人物的多个账号不要冒充多个不同人选。

### 4. 报告与确认后归类

每个原 ID 记录：文件数、候选 handle/昵称、具体关联文字、帖子或资料 URL、本地夹/alias 对照及未确认事项。公开快照可放 `<WORK>/research/`，不存凭据；报清完成/失败查询数、首屏或分页范围及无 ID 文件数。

把「已有严格归属」「待用户确认」「本轮未找到线索」分开说明，不将没搜到等同于不存在。身份确认与搬移授权分别核实；只调查的请求不能顺带修改媒体或 alias。

从 Unknown 搬动时先核实 helper 的实际源目录：`move_user` 默认只扫 `BACKLOG`，不要直接套用待归类脚本。明确这一批的源/目标，保留原 ID，按下面的冲突检查、备份与 move-log 规则执行。

## 安全 & 复盘

- 搬前逐文件检查目标同名文件；遇到冲突先停并核对，绝不覆盖已有文件。dry-run 后正式操作前再确认源文件和 alias 未变化，搬后核对文件数与内容一致
- 每步 `Organizer` 自动备份 alias 到 `<WORK>/alias_backups/`，出 `move-log-<tag>-<ts>.json` 便于回滚
- alias 与 move-log 均为**原子写入**（临时文件 + replace），Dropbox 卷上中断不会留下截断的别名文件
- 删「光秃秃 username 行」前确认父目录没有以该 username 命名的夹（否则 organizer 找不到那个夹）
- 每轮跑完报账：处理多少 / 剩多少 / 新建哪些夹 / 别名改了哪几行，让用户复核

## 相关 memory

`workflow-organize-backlog`、`workflow-dedup-folders`、`feedback-match-strict`、`project-tweet-detail-typename-wrapper`。
