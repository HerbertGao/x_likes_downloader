## 新增需求

### 需求:沙箱 base dir 配置项

系统必须新增配置项 `download_sandbox_base_dir`，作为 `xld media download` 写入文件的根目录。该配置项必须支持通过环境变量 / `.env` / `xld setup --download-dir <path>` 三种方式设置，加载优先级遵循项目既有配置层级。

#### 场景:用户显式配置 base dir
- **当** 用户运行 `xld setup --download-dir /Volumes/Archive/xld`
- **那么** 该路径必须被写入本地配置作为 `download_sandbox_base_dir` 的值

#### 场景:运行时读取配置
- **当** `xld media download` 启动时
- **那么** 系统必须从配置中读取 `download_sandbox_base_dir`，未配置时使用平台默认值（见下一需求）

### 需求:跨平台默认 base dir

未显式配置 `download_sandbox_base_dir` 时，系统必须依据操作系统选择以下默认值：

- macOS: `~/Library/Application Support/xld/downloads`
- Linux: `${XDG_DATA_HOME:-~/.local/share}/xld/downloads`
- Windows: `%LOCALAPPDATA%\xld\downloads`

系统在首次写入前必须递归创建该目录（若不存在）。

#### 场景:macOS 默认目录
- **当** 在 macOS 上未配置 base dir，运行 `xld media download --ids 123`
- **那么** 文件落盘路径必须以 `<HOME>/Library/Application Support/xld/downloads/` 开头

#### 场景:Linux XDG 兼容
- **当** 在 Linux 上未配置 base dir 且环境变量 `XDG_DATA_HOME` 未设置
- **那么** 文件落盘路径必须以 `<HOME>/.local/share/xld/downloads/` 开头

#### 场景:Windows 默认目录
- **当** 在 Windows 上未配置 base dir
- **那么** 文件落盘路径必须以 `%LOCALAPPDATA%\xld\downloads\` 开头

#### 场景:目录不存在时自动创建
- **当** 配置或默认 base dir 在文件系统上不存在
- **那么** 系统在首次写入前必须递归创建该目录，且不报错

### 需求:子目录参数 jail 校验

`xld media download --subdir <name>` 与 lib 层 `download_media(_, subdir)` 接收的子目录参数必须经过严格校验，符合以下规则才允许使用：

- 禁止包含路径组件 `..`
- 禁止以路径分隔符（`/` 或 `\`）开头（拒绝绝对路径）
- 禁止包含驱动器盘符（如 `C:`，防止 Windows 绝对路径）
- 解析为最终路径后必须仍位于 `download_sandbox_base_dir` 之内（按规范化路径前缀比较）
- 禁止跟随符号链接逃出 sandbox（最终落盘的物理父目录必须是 base dir 的子孙）

任何违反必须返回结构化错误 `{ kind: "sandbox_violation", detail: "<原因>" }`，禁止抛出未分类异常。

#### 场景:拒绝路径穿越
- **当** Agent 传入 `--subdir "../../../etc"`
- **那么** 系统返回 `sandbox_violation` 错误，文件不被写入

#### 场景:拒绝绝对路径
- **当** Agent 传入 `--subdir "/tmp/leak"` 或 `--subdir "C:\\Users"`
- **那么** 系统返回 `sandbox_violation` 错误

#### 场景:接受合法子目录
- **当** Agent 传入 `--subdir "2026-05/topic-rust"`
- **那么** 系统在 `<base>/2026-05/topic-rust/` 下创建目录并写入文件

#### 场景:符号链接逃逸防御
- **当** 沙箱内存在指向外部目录的符号链接，Agent 通过 `--subdir` 命中该链接
- **那么** 系统检测到最终物理路径不在 base dir 之内并返回 `sandbox_violation`

### 需求:旧 download 子命令的 base 通过 opts 注入

旧 `xld download` 子命令底层必须共用 lib 层 `download_media`，但其 base 必须由调用方显式注入为 `config.download_dir`（默认 `./downloads`，可被 `DOWNLOAD_DIR` 覆盖），禁止走平台默认沙箱目录。Sandbox jail 校验（拒 `..` / 绝对路径 / 符号链接逃逸）必须对该 base 同样生效——sandbox 边界跟随 base 移动。

`xld media download` 与 Agent 直接调用 `download_media` 时，base 仍由 sandbox 模块解析（`config.download_sandbox_base_dir` → 平台默认）。

#### 场景:旧 download 写入 ./downloads
- **当** 现有人类用户运行 `xld download` 且 `DOWNLOAD_DIR` 未修改
- **那么** 文件继续写入相对当前工作目录的 `./downloads` 子目录，行为与本变更前一致；同时 sandbox jail 仍生效（无法通过 subdir 写出该目录）

#### 场景:base 移动 sandbox 边界跟随
- **当** 调用方传入 `opts.base_dir = Some("/data/foo")`、`subdir = Some("..")`
- **那么** 仍必须返回 `sandbox_violation`（subdir 逃出 base，无论 base 来自哪一层）

### 需求:lib 层 sandbox 模块

系统必须在 lib crate 中提供 `sandbox` 模块，至少暴露 `resolve_subdir(base: &Path, subdir: Option<&str>) -> Result<PathBuf, SandboxError>` 函数，作为路径校验的唯一入口。`download_media` 与未来任何其它需要写入沙箱的能力必须经此函数解析路径，禁止绕过。

#### 场景:resolve_subdir 是唯一入口
- **当** 检视 `download_media` 的实现
- **那么** 其计算最终写入目录的代码路径必须显式调用 `sandbox::resolve_subdir(...)`，禁止用 `base.join(subdir)` 或 `Path::new` 直接拼接
