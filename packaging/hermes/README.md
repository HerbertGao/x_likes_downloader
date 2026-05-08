# X Likes Downloader — Hermes adapter (placeholder, v2.2)

本目录是 Hermes host adapter 的占位。实际 host adapter 实现排期 **v2.2**——欢迎社区 PR 提前实现。

---

## 现状

`packaging/skill/x_likes/SKILL.md` 是 SOT，使用 Anthropic 风格 YAML frontmatter（`name` + `description` + 正文），**已验证 SKILL.md 跨工具兼容**——Hermes 的 skill 加载机制理论上能直接吃 SOT。

---

## 实施提示

参考 `packaging/codex/` 的目录布局：

1. 在 `scripts/sync-skill.sh` 中加一份派生路径 `packaging/hermes/skills/x_likes/SKILL.md`（直接 cp 或加 host-specific frontmatter 块）
2. 落地路径：通常是 `~/.hermes/skills/x_likes/SKILL.md`（具体路径视 Hermes 版本而异）
3. 如果 Hermes 需要独立 plugin manifest，在 `packaging/hermes/` 下加 `<host>-plugin.json`
4. 在 `scripts/check-packaging.sh` 中加 hermes 校验分支
5. 顶层 README 状态表把 Hermes 一行从 `🔜 v2.2` 改为 `✅ v2.2+`

---

## 外链

> 实施前请活体打开链接确认仍可访问；如 404/403 请删除并加注释说明。

- [Nous Research / Hermes GitHub](https://github.com/NousResearch) — 社区主入口
- [Anthropic Skills 设计参考](https://www.anthropic.com/research/agent-skills) — SKILL.md 格式标准

---

## 欢迎 PR

如果你正在用 Hermes 并想提前用上本工具，请提交 PR 到 [HerbertGao/x_likes_downloader](https://github.com/HerbertGao/x_likes_downloader)。参考 `packaging/codex/` 的 PR 形态（plugin manifest + sync 脚本派生 + check 脚本校验 + README）。

---

## 进一步阅读

- [SOT SKILL.md](../skill/x_likes/SKILL.md)
- [`packaging/README.md`](../README.md) — 架构说明 + "如何添加新 host adapter"
