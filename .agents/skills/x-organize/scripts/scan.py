#!/usr/bin/env python3
"""严格匹配扫描：待归类文件按 username 聚合，映射到 alias 主名并找现有文件夹。
用法: python3 scan.py   （BASE 由 config 解析，见 SKILL.md）
输出 MATCHED（可直接搬）+ UNMATCHED（需 API 识别）。"""
import collections
import os
import re
import sys

import config

FN_RE = re.compile(r"^(.+?)_(\d{15,20})_")


def read_alias_map():
    """读别名文件 → {entry.lower(): 主名}。文件读不到时给出可执行的提示，而不是裸 traceback。"""
    try:
        text = open(config.ALIAS, encoding="utf-8").read()
    except OSError as e:
        raise SystemExit(
            f"读不到别名文件：{config.ALIAS}\n  ({e})\n"
            f"检查 X_ORGANIZE_BASE 是否指向正确的 Twitter 目录、磁盘是否已挂载。"
            f"当前 BASE = {config.BASE}"
        )
    u2p = {}
    for ln in text.splitlines():
        parts = [p.strip() for p in ln.split(",") if p.strip()]
        if not parts:
            continue
        for e in parts:
            u2p[e.lower()] = parts[0]
    return u2p


def list_dirs(path, what):
    try:
        return os.listdir(path)
    except OSError as e:
        raise SystemExit(
            f"读不到{what}：{path}\n  ({e})\n"
            f"检查 X_ORGANIZE_BASE 是否正确、磁盘是否已挂载。"
        )


def build_folder_index():
    """{folder名.lower(): folder名}，附 alias 用逗号分段的匹配表。"""
    plain = {}
    by_seg = {}
    for d in list_dirs(config.BASE, "父目录"):
        if not os.path.isdir(os.path.join(config.BASE, d)):
            continue
        plain[d.lower()] = d
        for seg in d.split(","):
            seg = seg.strip().lower()
            if seg:
                by_seg.setdefault(seg, d)
    return plain, by_seg


def find_folder(primary, plain, by_seg):
    pl = primary.lower()
    return plain.get(pl) or by_seg.get(pl)


def count_by_user(names):
    counts = collections.Counter()
    for f in names:
        if f in (os.path.basename(config.ALIAS), ".DS_Store"):
            continue
        m = FN_RE.match(f)
        if not m:
            print("NO-MATCH-FILENAME:", f)
            continue
        counts[m.group(1)] += 1
    return counts


def main():
    u2p = read_alias_map()
    plain, by_seg = build_folder_index()
    counts = count_by_user(list_dirs(config.BACKLOG, "待归类目录"))

    matched, unmatched = [], []
    for u, c in counts.most_common():
        p = u2p.get(u.lower())
        fld = find_folder(p, plain, by_seg) if p else find_folder(u, plain, by_seg)
        if p and fld:
            matched.append((u, c, p, fld))
        elif not p and fld:
            matched.append((u, c, "(no-alias)", fld))
        elif p and not fld:
            unmatched.append((u, c, f"alias->{p} NO FOLDER"))
        else:
            unmatched.append((u, c, "no alias, no folder"))

    print(f"=== {len(counts)} usernames, {sum(counts.values())} files ===")
    print(f"\n--- MATCHED ({len(matched)}) ---")
    for u, c, p, f in matched:
        print(f"  {u} ({c}) -> [{p}] folder={f}")
    print(f"\n--- UNMATCHED ({len(unmatched)}) ---")
    for u, c, why in unmatched:
        print(f"  {u} ({c}) : {why}")


if __name__ == "__main__":
    sys.exit(main())
