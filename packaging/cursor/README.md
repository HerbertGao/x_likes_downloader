# X Likes Downloader — Cursor adapter (placeholder, v2.2)

本目录是 Cursor host adapter 的占位。实际 host adapter 实现排期 **v2.2**——欢迎社区 PR 提前实现。

---

## 现状

`packaging/skill/x_likes/SKILL.md` 是 SOT，使用 Anthropic 风格 YAML frontmatter（`name` + `description` + 正文），**已验证 SKILL.md 跨工具兼容**——Cursor 的 skill 加载机制理论上能直接吃 SOT。

---

## 实施提示

Cursor 的 skill 路径有两种：

1. `.cursor/skills/x_likes/SKILL.md`（项目级）
2. 兼容 `.claude/skills/x_likes/SKILL.md`（用户级，Claude Code 风格软链）

参考 `packaging/codex/` 布局：

1. 在 `scripts/sync-skill.sh` 中加 `packaging/cursor/skills/x_likes/SKILL.md` 派生路径
2. Cursor 的 SKILL.md frontmatter 可选加 `paths: ["x.com", "twitter.com"]` glob（让 skill 仅在这些 URL 上下文激活）
3. 在 `scripts/check-packaging.sh` 中加 cursor 校验分支
4. 顶层 README 状态表把 Cursor 一行从 `🔜 v2.2` 改为 `✅ v2.2+`

---

## 外链

> 实施前请活体打开链接确认仍可访问；如 404/403 请删除并加注释说明。

- [Cursor Skills 官方文档](https://docs.cursor.com/) — 路径与 frontmatter 字段定义（搜索"skills"）
- [Cursor 社区 forum](https://forum.cursor.com/) — 加载机制 / 路径约定的活跃讨论

---

## 欢迎 PR

如果你正在用 Cursor 并想提前用上本工具，请提交 PR 到 [HerbertGao/x_likes_downloader](https://github.com/HerbertGao/x_likes_downloader)。参考 `packaging/codex/` 的 PR 形态。

---

## 进一步阅读

- [SOT SKILL.md](../skill/x_likes/SKILL.md)
- [`packaging/README.md`](../README.md) — 架构说明 + "如何添加新 host adapter"
