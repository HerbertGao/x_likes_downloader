#!/bin/bash

# x_likes_downloader 构建脚本 (macOS/Linux)
# 用途：自动化本地 Release 构建

set -e

# 颜色定义
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}!${NC} $1"
}

print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

print_step() {
    echo -e "\n${BLUE}==>${NC} $1"
}

# 切换到项目根目录
cd "$(dirname "$0")/.."

# 获取版本号
get_version() {
    grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'
}

# 检查命令是否存在
command_exists() {
    command -v "$1" >/dev/null 2>&1
}

# 检查依赖
check_dependencies() {
    print_step "步骤 1/4: 检查依赖"

    local missing_deps=()

    if command_exists rustc; then
        RUST_VERSION=$(rustc --version | awk '{print $2}')
        print_success "Rust: v$RUST_VERSION"
    else
        missing_deps+=("Rust")
        print_error "Rust 未安装"
    fi

    if command_exists cargo; then
        CARGO_VERSION=$(cargo --version | awk '{print $2}')
        print_success "cargo: v$CARGO_VERSION"
    else
        missing_deps+=("cargo")
        print_error "cargo 未安装"
    fi

    if [ ${#missing_deps[@]} -gt 0 ]; then
        echo ""
        print_error "缺少以下依赖：${missing_deps[*]}"
        echo ""
        echo "请安装 Rust: https://rustup.rs/"
        exit 1
    fi

    echo ""
}

# 代码检查
run_checks() {
    print_step "步骤 2/4: 代码检查"

    print_info "运行 cargo check..."
    cargo check --quiet
    print_success "代码检查通过"

    echo ""
}

# 编译
build_release() {
    print_step "步骤 3/4: 编译 Release"

    print_info "开始编译..."
    cargo build --release

    print_success "编译完成"
    echo ""
}

# 验证产物
verify_output() {
    print_step "步骤 4/4: 验证构建产物"

    local binary="target/release/x_likes_downloader"
    if [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" ]]; then
        binary="${binary}.exe"
    fi

    if [ ! -f "$binary" ]; then
        print_error "构建产物不存在: $binary"
        exit 1
    fi

    # 检查文件大小
    local file_size
    if [[ "$OSTYPE" == "darwin"* ]]; then
        file_size=$(stat -f%z "$binary")
    else
        file_size=$(stat -c%s "$binary")
    fi
    local file_size_mb=$((file_size / 1024 / 1024))

    print_success "产物路径: $binary"
    print_success "文件大小: ${file_size_mb}MB"

    echo ""
}

# 打印构建总结
print_summary() {
    local version=$(get_version)

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    print_success "构建完成！"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "版本号:   $version"
    echo "产物路径: target/release/x_likes_downloader"
    echo "构建用时: $SECONDS 秒"
    echo ""
}

# 主函数
main() {
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  x_likes_downloader 构建脚本"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    check_dependencies
    run_checks
    build_release
    verify_output
    print_summary
}

SECONDS=0
main
