# X Likes Downloader (Rust版本)

一个用Rust编写的X（Twitter）点赞推文媒体下载器，可以自动下载你点赞的推文中的图片和视频。

## 功能特性

- 🔐 支持X内部API，无需第三方服务
- 📥 自动下载点赞推文中的图片和视频
- 🔄 支持断点续传，避免重复下载
- 📁 自动文件整理和分类
- 🚀 异步下载，支持进度显示
- 🌐 支持HTTP代理
- 📊 详细的下载统计信息

## 安装

### 前置要求

- Rust 1.70+
- 有效的X账号和登录状态

### 方法一：下载预编译版本（推荐）

从 [GitHub Releases](https://github.com/HerbertGao/x_likes_downloader/releases) 下载对应平台的预编译版本：

- **macOS ARM64** (Apple Silicon): `x_likes_downloader_macos_arm64`
- **macOS x86_64** (Intel): `x_likes_downloader_macos_x86_64`
- **Linux x86_64**: `x_likes_downloader_linux_x86_64`
- **Linux ARM64**: `x_likes_downloader_linux_arm64`
- **Windows x86_64**: `x_likes_downloader_windows_x86_64.exe`
- **Windows ARM64**: `x_likes_downloader_windows_arm64.exe`

下载后解压并运行：

```bash
# macOS/Linux
chmod +x x_likes_downloader
./x_likes_downloader --help

# Windows
x_likes_downloader.exe --help
```

### 方法二：从源码编译

```bash
# 克隆项目
git clone <repository-url>
cd x_likes_downloader

# 编译当前平台版本
cargo build --release

# 安装到系统
cargo install --path .
```

## 使用方法

### 1. 初始化配置

首先需要从浏览器中获取X的API请求信息：

1. 打开X网站并登录
2. 打开开发者工具（F12）
3. 进入Network标签页
4. 刷新页面，找到任意一个API请求
5. 右键点击请求 -> Copy -> Copy as cURL
6. 将cURL命令保存到 `curl_command.txt` 文件中

然后运行初始化命令：

```bash
# 使用默认的curl_command.txt文件
x_likes_downloader setup

# 或指定自定义文件
x_likes_downloader setup --curl-file my_curl.txt
```

### 2. 下载媒体文件

```bash
# 开始下载
x_likes_downloader download
```

### 3. 整理文件（可选）

```bash
# 使用默认目录
x_likes_downloader organize

# 或指定自定义目录
x_likes_downloader organize --source-dir downloads --target-dir organized
```

## 配置选项

### 方法一：使用 .env 文件（推荐）

1. 复制示例配置文件：

    ```bash
    cp env.example .env
    ```

2. 编辑 `.env` 文件，根据需要修改配置：

    ```ini
    # 下载配置
    COUNT=50                    # 每次获取的推文数量
    ALL=true                    # 是否下载所有点赞推文
    DOWNLOAD_DIR=data/downloads # 下载目录
    FILE_FORMAT={USERNAME}_{ID} # 文件命名格式

    # 自动整理
    AUTO_ORGANIZE=true          # 下载完成后自动整理
    TARGET_DIR=data/organized   # 整理目标目录
    ```

### 方法二：环境变量

也可以通过环境变量设置配置：

```bash
# 下载配置
export COUNT=50                    # 每次获取的推文数量
export ALL=true                    # 是否下载所有点赞推文
export DOWNLOAD_DIR="downloads"    # 下载目录
export FILE_FORMAT="{USERNAME} {ID}"  # 文件命名格式

# 自动整理
export AUTO_ORGANIZE=true          # 下载完成后自动整理
export TARGET_DIR="organized"      # 整理目标目录
```

## 项目结构

```text
x_likes_downloader/
├── src/
│   ├── main.rs           # 主程序入口和命令行界面
│   ├── config.rs         # 配置管理
│   ├── setup.rs          # 初始化工具
│   ├── x_api.rs          # X API 调用
│   ├── downloader.rs     # 媒体下载器
│   ├── updater.rs        # 版本检查与自动更新
│   └── organize_files.rs # 文件整理工具
├── data/                  # 运行时自动生成
│   └── private_tokens.env    # 私有令牌配置
├── .env                      # 环境配置文件（用户创建）
├── env.example               # 示例配置文件
└── Cargo.toml
```

## 主要模块说明

### config.rs

- 加载和管理配置信息
- 从.env文件、环境变量和私有令牌文件读取配置
- 支持代理、下载目录、文件格式等配置
- 优先级：.env文件 > 环境变量 > 默认值

### setup.rs

- 解析cURL命令提取认证信息
- 生成私有令牌配置文件
- 支持cookie解析和URL解码

### x_api.rs

- 调用X内部GraphQL API
- 支持分页获取点赞推文
- 处理API响应和错误

### downloader.rs

- 异步下载媒体文件
- 支持图片和视频下载
- 断点续传和进度显示
- 文件完整性验证

### organize_files.rs

- 根据文件名解析用户信息
- 自动分类整理文件
- 处理重复文件

### updater.rs

- 检查 GitHub 上的最新版本
- 判断当前版本是否需要更新
- 下载并替换可执行文件（自动更新）

## 注意事项

1. **认证信息安全**: `data/private_tokens.env` 包含敏感信息，请妥善保管
2. **API限制**: 请合理控制请求频率，避免触发X的限流
3. **代理设置**: 如果无法直接访问X，请配置有效的代理
4. **存储空间**: 下载大量媒体文件会占用较多存储空间

## 故障排除

### 常见问题

1. **认证失败**: 检查 `data/private_tokens.env` 文件是否存在且内容正确
2. **网络错误**: 确认代理设置正确，或尝试更换代理
3. **下载失败**: 检查网络连接和存储空间
4. **文件整理错误**: 确认目标目录存在且有写入权限

### 调试模式

设置环境变量启用详细日志：

```bash
export RUST_LOG=debug
x_likes_downloader download
```

## 许可证

MIT License

## 贡献

欢迎提交Issue和Pull Request！

## 免责声明

本工具仅供学习和个人使用，请遵守X的服务条款和相关法律法规。使用者需自行承担使用风险。
