#!/usr/bin/env python3
"""归类操作复用库。每个 apply 批次的脚本 import 它，保证：始终备份 alias、始终出 move-log。
apply 逻辑本身按当轮决策现写（哪些 username 归哪、建哪些新夹、合并哪些），import 这里的原子操作。

路径（BASE / 待归类目录 / alias / 工作区）由 config.py 解析，不写死。

典型用法:
    from organize_lib import Organizer
    o = Organizer()                      # 自动备份 alias 到 <WORK>/alias_backups/
    o.move_user("gemilgss", "烬爷S", mkdir=True)
    o.add_alias("超超超厉害", ["Linchaossss"])      # 追加别名到 seg0==主名 的行
    o.new_alias(["烬爷S", "gemilgss", "bozi_Ss"])   # 新增一行(去重,第一个是主名/夹名)
    o.merge_folder("虎杖Z", "虎咒X", drop_alias_seg0="虎杖Z", add_alias_to="虎咒X",
                   add_entries=["HEISHI1100","虎杖Z"])   # 整夹并入+删旧行+加别名
    o.rename_folder("深圳四爷-纯S", "山东四爷（深圳四爷）")
    o.finish("p2")                       # 写回 alias + 落 move-log-p2-<ts>.json

失败策略：任何文件操作失败都**中止**流程（绝不吞异常、绝不静默部分完成），
但报错要说清「哪一步、哪个文件、备份在哪」，而不是甩一个裸 traceback。
"""
import json
import os
import re
import shutil
import time
from typing import NoReturn

import config

BASE = config.BASE
BACKLOG = config.BACKLOG
ALIAS = config.ALIAS
WORK = config.WORK
FN_RE = re.compile(r"^(.+?)_(\d{15,20})_")


def _abort(desc, path, e) -> NoReturn:
    """统一的失败出口：中止流程 + 可执行的报错。"""
    raise SystemExit(
        f"{desc} 失败：{path}\n  ({e})\n"
        f"已中止，未继续后续操作。别名备份在 {WORK}/alias_backups/，"
        f"已完成的搬移见 {WORK}/move-log-*.json。"
    )


class Organizer:
    def __init__(self):
        self.ts = time.strftime("%Y%m%d-%H%M%S", time.localtime())
        backups = os.path.join(WORK, "alias_backups")
        bak = os.path.join(backups, f"username_aliases.txt.bak.{self.ts}")
        try:
            os.makedirs(backups, exist_ok=True)
            shutil.copy2(ALIAS, bak)
        except OSError as e:
            _abort("备份别名文件", ALIAS, e)
        try:
            with open(ALIAS, encoding="utf-8") as fh:
                self.lines = fh.read().splitlines()
        except OSError as e:
            _abort("读取别名文件", ALIAS, e)
        self.log = {"ts": self.ts, "moves": [], "alias": [], "new_folders": [],
                    "deleted_folders": [], "renamed": []}

    # --- files ---
    def move_user(self, uname, dst_folder, mkdir=False):
        dst = os.path.join(BASE, dst_folder)
        moved = []
        try:
            if mkdir and not os.path.isdir(dst):
                os.makedirs(dst)
                self.log["new_folders"].append(dst_folder)
            for f in os.listdir(BACKLOG):
                m = FN_RE.match(f)
                if not m or m.group(1) != uname:
                    continue
                src, tgt = os.path.join(BACKLOG, f), os.path.join(dst, f)
                if os.path.exists(tgt):
                    os.remove(tgt)
                shutil.move(src, tgt)
                moved.append(f)
        except OSError as e:
            _abort(f"搬移 {uname} → {dst_folder}（已搬 {len(moved)} 个）", BACKLOG, e)
        if moved:
            self.log["moves"].append({"user": uname, "to": dst_folder, "n": len(moved)})
        return len(moved)

    def merge_folder(self, src_folder, dst_folder, drop_alias_seg0=None,
                     add_alias_to=None, add_entries=None):
        src, dst = os.path.join(BASE, src_folder), os.path.join(BASE, dst_folder)
        merged = []
        try:
            for f in os.listdir(src):
                if f == ".DS_Store":
                    continue
                s, t = os.path.join(src, f), os.path.join(dst, f)
                if os.path.exists(t):
                    os.remove(t)
                shutil.move(s, t)
                merged.append(f)
            ds = os.path.join(src, ".DS_Store")
            if os.path.exists(ds):
                os.remove(ds)
            if not [f for f in os.listdir(src)]:
                os.rmdir(src)
                self.log["deleted_folders"].append(src_folder)
        except OSError as e:
            _abort(f"合并文件夹 {src_folder} → {dst_folder}（已移 {len(merged)} 个）", src, e)
        self.log["moves"].append({"user": f"{src_folder}-folder-merge",
                                  "to": dst_folder, "n": len(merged)})
        if add_alias_to and add_entries:
            self.add_alias(add_alias_to, add_entries)
        if drop_alias_seg0:
            self.drop_alias(drop_alias_seg0)
        return len(merged)

    def rename_folder(self, old, new):
        try:
            os.rename(os.path.join(BASE, old), os.path.join(BASE, new))
        except OSError as e:
            _abort(f"重命名文件夹 {old} → {new}", old, e)
        self.log["renamed"].append([old, new])

    def delete_empty(self, folder):
        d = os.path.join(BASE, folder)
        try:
            ds = os.path.join(d, ".DS_Store")
            if os.path.exists(ds):
                os.remove(ds)
            os.rmdir(d)
        except OSError as e:
            _abort(f"删除空文件夹 {folder}", d, e)
        self.log["deleted_folders"].append(folder)

    # --- alias ---
    def add_alias(self, seg0, new_entries):
        out = []
        for ln in self.lines:
            parts = [p.strip() for p in ln.split(",") if p.strip()]
            if parts and parts[0] == seg0:
                have = {p.lower() for p in parts}
                add = [e for e in new_entries if e.lower() not in have]
                if add:
                    ln = ln.rstrip() + "," + ",".join(add)
                    self.log["alias"].append(f"加别名[{seg0}]: {add}")
            out.append(ln)
        self.lines = out

    def new_alias(self, entries):
        seen = []
        for e in entries:
            if e.lower() not in [s.lower() for s in seen]:
                seen.append(e)
        self.lines.append(",".join(seen))
        self.log["alias"].append(f"新行: {','.join(seen)}")

    def drop_alias(self, seg0):
        self.lines = [ln for ln in self.lines
                      if ([p.strip() for p in ln.split(",")] or [""])[0] != seg0]
        self.log["alias"].append(f"删行: {seg0}")

    def merge_alias(self, seg0_list, new_primary):
        """把多行(seg0 in seg0_list)合并成一行, 主名=new_primary, 其余原 entry 作别名去重。"""
        out, entries = [], [new_primary]
        for ln in self.lines:
            parts = [p.strip() for p in ln.split(",") if p.strip()]
            if parts and parts[0] in seg0_list:
                for e in parts:
                    if e.lower() not in [x.lower() for x in entries]:
                        entries.append(e)
                continue
            out.append(ln)
        out.append(",".join(entries))
        self.lines = out
        self.log["alias"].append(f"合并行: {','.join(entries)}")

    # --- commit ---
    def _atomic_write(self, path, text, desc):
        """先写同目录临时文件再 replace：Dropbox 卷上中途失败不会留下截断的别名文件。"""
        tmp = path + ".tmp"
        try:
            with open(tmp, "w", encoding="utf-8") as fh:
                fh.write(text)
            os.replace(tmp, path)
        except OSError as e:
            _abort(f"{desc}（临时文件 {tmp} 已保留）", path, e)

    def finish(self, tag="run"):
        self._atomic_write(ALIAS, "\n".join(self.lines) + "\n", "写回别名文件")
        self._atomic_write(
            os.path.join(WORK, f"move-log-{tag}-{self.ts}.json"),
            json.dumps(self.log, ensure_ascii=False, indent=2),
            "写 move-log",
        )
        try:
            names = os.listdir(BACKLOG)
        except OSError as e:
            _abort("统计待归类剩余", BACKLOG, e)
        rem = [f for f in names if f not in ("username_aliases.txt", ".DS_Store")]
        n = sum(m["n"] for m in self.log["moves"])
        print(f"[{tag}] moved {n} files | new {len(self.log['new_folders'])} | "
              f"alias {len(self.log['alias'])} | backlog remaining {len(rem)}")
        return rem
