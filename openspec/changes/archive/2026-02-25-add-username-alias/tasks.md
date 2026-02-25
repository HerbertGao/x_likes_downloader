## 1. 核心实现（organize_files.rs）

- [x] 1.1 新增 `load_username_aliases(alias_file_path: &str) -> HashMap<String, String>` 方法，从文本文件加载别名映射（alias → primary）。支持注释行、空行、首尾空格处理。文件不存在时返回空 HashMap。（`src/organize_files.rs`）
- [x] 1.2 修改 `organize_files(a_dir, b_dir)` 方法，在文件遍历前调用 `load_username_aliases`，别名文件路径为 `{a_dir}/username_aliases.txt`。（`src/organize_files.rs`）
- [x] 1.3 在文件匹配逻辑中（第 52-63 行区域），于现有匹配之前插入别名查询：若 username 存在于别名表中，替换为主名称后再执行文件夹匹配。（`src/organize_files.rs`）

## 2. 文档更新

- [x] 2.1 在 `env.example` 中添加 `username_aliases.txt` 的说明注释，描述文件格式和用法。（`env.example`）
- [x] 2.2 在 `README.md` 的文件整理章节中添加别名功能的使用说明。（`README.md`）

## 3. 手动验证

- [x] 3.1 验证：无别名文件时，归档行为与变更前完全一致
- [x] 3.2 验证：创建别名文件，确认别名用户的文件能正确归档到主名称文件夹
- [x] 3.3 验证：归档后文件名保留原始 username 不被改写
- [x] 3.4 验证：别名文件中的注释行、空行、单 username 行被正确跳过
