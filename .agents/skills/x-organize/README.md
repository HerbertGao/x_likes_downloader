# x-organize — X 下载媒体按人归类

把 X 下载器攒下的「待归类」积压文件按人分到各自文件夹，靠 X 的 UserByScreenName API 拿账号真名与 bio 来识别匿名 username 的归属。

> 本 skill 标了 `metadata.internal: true`，`npx skills` 的常规发现会隐藏它（需 `INSTALL_INTERNAL_SKILLS=1` 才可见）。它服务本地媒体库整理，不是对外分发的公开 skill；仓库里也不含任何个人路径。

## 一次性配置

### 1. 路径（必填 BASE）

`scripts/config.py` 从环境变量或用户配置文件解析路径，仓库里不写死任何个人路径。

写配置文件（推荐）：

```bash
mkdir -p ~/.config/x_organize
cat > ~/.config/x_organize/config.json <<'JSON'
{ "base": "/path/to/Twitter", "proxy": "http://127.0.0.1:7890" }
JSON
chmod 600 ~/.config/x_organize/config.json
```

或用环境变量：`X_ORGANIZE_BASE`（必填）、`X_ORGANIZE_CREDS`、`X_ORGANIZE_WORK`、`X_ORGANIZE_PROXY`、`X_ORGANIZE_BACKLOG_DIRNAME`、`X_ORGANIZE_ALIAS_FILENAME`、`X_ORGANIZE_UNKNOWN_DIRNAME`。缺 BASE 时脚本会直接报错并告诉你该配哪一项。

`base` 指向父级库目录；待归类目录默认取 `base/Z - 自动下载待归类`，别名文件取该目录下的 `username_aliases.txt`。

### 2. 凭据

X 的 UserByScreenName 需要一个登录态。存成 JSON（键：`bearer`、`cookie`、`csrf`、`query_id`，可选 `user_agent`、`features`、`fieldToggles`）：

```jsonc
// ~/.config/x_organize/creds.json   (chmod 600)
{
  "bearer":   <authorization bearer>,
  "cookie":   <完整 cookie 串，含 auth_token 与 ct0>,
  "csrf":     <与 cookie 里的 ct0 一致>,
  "query_id": "IGgvgiOx4QZndDHuD3x9TQ"   // UserByScreenName 的 query id，会随 X 滚动
}
```

取值方式：浏览器登录 x.com → DevTools Network → 随便点开一个 `/graphql/.../UserByScreenName` 请求 → Copy as cURL，从里面取 authorization / cookie / x-csrf-token。

**这个文件永远不要提交、不要写进 /tmp、不要贴进对话。**

### 3. 代理

x.com 在部分网络下不可直连。设 `proxy` 后 `api.py` 会显式走它；不设则依赖系统代理或 TUN 模式。

自检：

```bash
curl -x http://127.0.0.1:7890 -o /dev/null -w '%{http_code}\n' https://x.com/robots.txt   # 期望 200
python3 scripts/api.py x   # 期望输出一行 "x<TAB>真名<TAB>bio"
```

## 用法

三条命令，都在 skill 目录下跑：

```bash
# 1. 严格匹配扫描：输出 MATCHED（可直接搬）+ UNMATCHED（需识别）
python3 scripts/scan.py

# 2. 批量拉真名+bio
python3 scripts/api.py <username> [username ...]

# 3. 实际归类：现写一个 apply 脚本，import organize_lib 的原子操作
```

apply 脚本长这样（先 `DRY_RUN=1` 核对，再真跑）：

```python
import sys
sys.path.insert(0, "<skill-dir>/scripts")
from organize_lib import Organizer

o = Organizer()                                   # 自动备份 alias
o.move_user("someaccount", "某人", mkdir=True)     # 搬某账号的文件到该夹
o.add_alias("某人", ["someaccount"])               # 追加别名到已有行
o.new_alias(["新人名", "newaccount"])              # 新增一行（第一个是主名/夹名）
o.finish("p1")                                    # 原子写回 alias + 落 move-log
```

## 设计约束（踩过的坑）

- **严格匹配**，不用置信度分级。bio 取证必须「大号/原号/备用/防走丢」等关键词与 `@xxx` **近邻**；「bio 含主名就算」会批量误判（很多人自称「直男S」）。
- **真名包含现有夹名 ≠ 同一人**（`GIN(直男S)` 与夹 `直男S` 实测是不同人）。
- **夹名可能与 alias 主名不等**（`Emperor` vs 夹 `Emperor圣主`），精确匹配会漏。
- **别硬重试 429**：连拉几十个账号很容易被限流。
- 每步都自动备份 alias、落 move-log；alias 与 move-log 均原子写入，中断不会截断文件。
