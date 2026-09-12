---
name: x-organize
description: 整理 X(Twitter) 下载的媒体文件——把「待归类」积压文件按人归入文件夹，用 UserByScreenName API 拿真名+bio 识别匿名 username，交叉比对 alias/folder，并合并同一人的重复文件夹。当用户说「又攒了一批帮我归类」「扫一下重复文件夹」「XXX 和 YYY 是同一人合并」时用。
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
- 注销号无线索 → 问用户或丢 `Z - Unknown`
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

## 安全 & 复盘

- 每步 `Organizer` 自动备份 alias 到 `<WORK>/alias_backups/`，出 `move-log-<tag>-<ts>.json` 便于回滚
- alias 与 move-log 均为**原子写入**（临时文件 + replace），Dropbox 卷上中断不会留下截断的别名文件
- 删「光秃秃 username 行」前确认父目录没有以该 username 命名的夹（否则 organizer 找不到那个夹）
- 每轮跑完报账：处理多少 / 剩多少 / 新建哪些夹 / 别名改了哪几行，让用户复核

## 相关 memory

`workflow-organize-backlog`、`workflow-dedup-folders`、`feedback-match-strict`、`project-tweet-detail-typename-wrapper`。
