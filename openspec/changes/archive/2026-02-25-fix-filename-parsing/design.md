## 上下文

`parse_filename` 负责从下载文件名中提取 username 和 tweet_id。当前使用"最后一个纯数字 token"策略，但 CDN 原始文件名中的短数字和尾部下划线导致的空串会干扰识别。

文件名格式固定为 `{USERNAME}_{TWEET_ID}_{ORIGINAL_NAME}.ext`（由 `downloader.rs` 生成），且 `FILE_FORMAT` 不会变更。

## 目标 / 非目标

**目标：**
- 修复 `parse_filename` 的 Tweet ID 识别逻辑，正确处理 CDN 文件名中的短数字和尾部下划线
- 利用 Snowflake ID 长度特征区分 Tweet ID 与纯数字 username

**非目标：**
- 不支持可变的 `FILE_FORMAT` 配置
- 不重构 `parse_filename` 之外的归档逻辑

## 决策

### 使用"第一个 >= 16 位纯数字 token"策略

**选择**：扫描 tokens，取第一个长度 >= 16 且全为 ASCII 数字的 token 作为 Tweet ID，该 token 之前的部分拼接为 username。

**理由**：
- Twitter Snowflake ID 为 18-19 位数字，username 上限 15 字符 → 16 位阈值提供安全间隔
- "第一个"而非"最后一个"：因为文件格式为 `USERNAME_TWEETID_ORIGINALNAME`，Tweet ID 总在 username 之后、CDN 文件名之前
- 单次遍历即可完成，无需额外数据结构

**替代方案**：
1. 过滤空串 + 保留"最后一个数字"策略 → 只修复 Bug 1，不修复 Bug 2
2. 基于 `file_format` 生成正则匹配 → 过度设计，且 format 不会变

### 阈值定义为常量

将 `16` 定义为 `TWEET_ID_MIN_DIGITS` 常量，便于理解意图。

## 风险 / 权衡

- **[风险] 未来 Tweet ID 位数变化** → Snowflake ID 目前为 19 位，64-bit 整数上限为 20 位。16 位阈值有充足余量。
- **[风险] 极端情况：CDN 文件名中恰好包含 >= 16 位纯数字** → Twitter CDN 文件名为短随机串（如 `8_kFpUGwReil7Vzo`），不会出现如此长的纯数字段。风险可忽略。
- **[权衡] 无法匹配时的错误信息更精确** → 旧逻辑对短数字也能"成功解析"（虽然结果错误），新逻辑会直接报错"未找到 Tweet ID"，这实际上是更好的行为。
