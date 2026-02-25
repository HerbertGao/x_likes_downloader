## 为什么

`organize_files.rs` 中的 `parse_filename` 使用"最后一个纯数字 token"来识别 Tweet ID，但这个启发式在两种常见情况下失败：
1. 文件名尾部下划线产生的空串被 `"".chars().all(...)` 判为数字（vacuously true）
2. CDN 原始文件名中的短数字（如 "8"）抢占了真正的 Tweet ID 位置

导致大量文件归档时 username 被错误拼接、无法匹配目标文件夹。

## 变更内容

- 将 Tweet ID 识别策略从"最后一个纯数字 token"改为"第一个 >= 16 位的纯数字 token"
- 利用 Twitter Snowflake ID（18-19 位）与 username 上限（15 字符）的天然间隔，安全区分两者
- 同时消除空串和短数字的干扰，无需额外过滤逻辑

## 功能 (Capabilities)

### 新增功能

### 修改功能
- `username-alias`: `parse_filename` 的 Tweet ID 识别规则变更，影响 username 提取结果

## 影响

- 受影响代码：`src/organize_files.rs`（仅 `parse_filename` 函数）
- 向后兼容：完全兼容，不影响文件命名格式、别名映射或其他模块
- 不涉及 X API 交互变更
- 不涉及配置项变更
