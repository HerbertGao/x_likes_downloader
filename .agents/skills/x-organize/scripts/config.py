"""x-organize 的路径与凭据解析。

仓库里不写死任何个人路径：BASE 必须由环境变量或用户配置文件提供。

优先级（高 → 低）：
  1. 环境变量（见下）
  2. 用户配置文件 $X_ORGANIZE_CONFIG（默认 ~/.config/x_organize/config.json），键名去掉 X_ORGANIZE_ 前缀并小写
  3. 无默认值 → 报错退出（BASE 必填）；其余取内置约定值

环境变量：
  X_ORGANIZE_BASE              父级 Twitter 目录（每人一夹），必填
  X_ORGANIZE_CONFIG            配置文件路径，默认 ~/.config/x_organize/config.json
  X_ORGANIZE_CREDS             凭据文件路径，默认 ~/.config/x_organize/creds.json
  X_ORGANIZE_WORK              备份 + move-log 工作区，默认 /tmp/x_organize
  X_ORGANIZE_PROXY             x.com 出网代理，如 http://127.0.0.1:7890（留空＝走系统/TUN）
  X_ORGANIZE_BACKLOG_DIRNAME   待归类目录名，默认 "Z - 自动下载待归类"
  X_ORGANIZE_ALIAS_FILENAME    别名文件名，默认 "username_aliases.txt"
  X_ORGANIZE_UNKNOWN_DIRNAME   无线索账号的去处，默认 "Z - Unknown"
"""
import json
import os

CONFIG_FILE = os.path.expanduser(
    os.environ.get("X_ORGANIZE_CONFIG", "~/.config/x_organize/config.json")
)


def _file_cfg():
    try:
        with open(CONFIG_FILE, encoding="utf-8") as fh:
            return json.load(fh)
    except Exception:
        return {}


_FILE = _file_cfg()


def get(key, env, default=""):
    """取配置：环境变量 > 配置文件 > 默认值。"""
    return os.environ.get(env) or _FILE.get(key) or default


def path(key, env, default=""):
    val = get(key, env, default)
    return os.path.expanduser(val) if val else ""


BASE = path("base", "X_ORGANIZE_BASE")
if not BASE:
    raise SystemExit(
        "未配置 BASE：请设环境变量 X_ORGANIZE_BASE，或在 "
        f"{CONFIG_FILE} 写入 {{\"base\": \"/path/to/Twitter\"}}"
    )

BACKLOG = os.path.join(BASE, get("backlog_dirname", "X_ORGANIZE_BACKLOG_DIRNAME",
                                 "Z - 自动下载待归类"))
ALIAS = os.path.join(BACKLOG, get("alias_filename", "X_ORGANIZE_ALIAS_FILENAME",
                                  "username_aliases.txt"))
UNKNOWN = os.path.join(BASE, get("unknown_dirname", "X_ORGANIZE_UNKNOWN_DIRNAME",
                                 "Z - Unknown"))
WORK = path("work", "X_ORGANIZE_WORK", "/tmp/x_organize")
CREDS = path("creds", "X_ORGANIZE_CREDS", "~/.config/x_organize/creds.json")
PROXY = get("proxy", "X_ORGANIZE_PROXY", "")
