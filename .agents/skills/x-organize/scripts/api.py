#!/usr/bin/env python3
"""X UserByScreenName 拉真名+bio。需 creds.json 有效；x.com 若被墙则配 X_ORGANIZE_PROXY。
用法: python3 api.py <screen_name> [screen_name ...]
输出每行: screen_name<TAB>display_name<TAB>bio
注销/封禁账号显示 __TypeName__ 或 __NORESULT__。"""
import json
import sys
import time
import urllib.parse
import urllib.request

import config

try:
    C = json.load(open(config.CREDS, encoding="utf-8"))
except FileNotFoundError:
    raise SystemExit(
        f"凭据文件不存在：{config.CREDS}\n"
        "需要键 bearer / cookie / csrf / query_id（建议 chmod 600）。"
    )
except (OSError, ValueError) as e:
    raise SystemExit(f"凭据文件无法读取或解析：{config.CREDS}（{e}）")
QID = C.get("query_id", "IGgvgiOx4QZndDHuD3x9TQ")
FEAT, TOG = C.get("features"), C.get("fieldToggles")

_opener = urllib.request.build_opener(
    urllib.request.ProxyHandler({"http": config.PROXY, "https": config.PROXY})
    if config.PROXY
    else urllib.request.ProxyHandler({})
)


def fetch(screen):
    params = {"variables": json.dumps({"screen_name": screen}, separators=(",", ":"))}
    if FEAT:
        params["features"] = FEAT if isinstance(FEAT, str) else json.dumps(FEAT, separators=(",", ":"))
    if TOG:
        params["fieldToggles"] = TOG if isinstance(TOG, str) else json.dumps(TOG, separators=(",", ":"))
    url = f"https://x.com/i/api/graphql/{QID}/UserByScreenName?" + urllib.parse.urlencode(params)
    req = urllib.request.Request(url)
    req.add_header("authorization", C["bearer"])
    req.add_header("cookie", C["cookie"])
    req.add_header("x-csrf-token", C["csrf"])
    req.add_header("x-twitter-active-user", "yes")
    req.add_header("x-twitter-auth-type", "OAuth2Session")
    req.add_header("x-twitter-client-language", "en")
    req.add_header("user-agent", C.get("user_agent", "Mozilla/5.0"))
    try:
        with _opener.open(req, timeout=20) as r:
            return json.load(r)
    except ValueError as e:
        # 代理/登录态失效时常返回 HTML 而非 JSON，原样抛会只看到 JSONDecodeError
        raise RuntimeError(f"响应不是合法 JSON（可能代理或登录态异常）: {e}") from e


def name_bio(screen):
    try:
        d = fetch(screen)
    except Exception as e:
        return ("ERR:" + str(e)[:60], "")
    res = (((d or {}).get("data") or {}).get("user") or {}).get("result")
    if not res or res.get("__typename") != "User":
        return ("__" + (res.get("__typename") if res else "NORESULT") + "__", "")
    core, leg = res.get("core") or {}, res.get("legacy") or {}
    return (core.get("name") or leg.get("name") or "", leg.get("description") or "")


if __name__ == "__main__":
    for s in sys.argv[1:]:
        n, b = name_bio(s)
        print(f"{s}\t{n}\t{b}".replace("\n", " "), flush=True)
        time.sleep(2)
