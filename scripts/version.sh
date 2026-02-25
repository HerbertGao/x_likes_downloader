#!/bin/bash

# x_likes_downloader 版本管理脚本
# 用途：升级版本号并同步到所有配置文件
# 使用：./scripts/version.sh [major|minor|patch|build|x.y.z|check|sync]

set -e

# 颜色定义
GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

print_success() {
    echo -e "${GREEN}✓${NC} $1"
}

print_error() {
    echo -e "${RED}✗${NC} $1"
}

print_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}!${NC} $1"
}

# 切换到项目根目录
cd "$(dirname "$0")/.."

# 验证版本号格式（支持 X.Y.Z 和 X.Y.Z.B）
validate_semver() {
    local version=$1
    if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)?$ ]]; then
        return 1
    fi
    return 0
}

# 从 Cargo.toml 读取版本号
get_cargo_version() {
    grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/'
}

# 从 README.md 读取版本号
get_readme_version() {
    if [ ! -f "README.md" ]; then
        echo ""
        return
    fi
    grep -oE '版本 [0-9]+\.[0-9]+\.[0-9]+(\.[0-9]+)?' README.md | head -1 | sed 's/版本 //'
}

# 更新 Cargo.toml 中的版本号（只更新 [package] 部分的版本）
update_cargo_version() {
    local new_version=$1
    awk -v ver="$new_version" '
        /^\[package\]/ { in_package=1 }
        /^\[/ && !/^\[package\]/ { in_package=0 }
        in_package && /^version = / { $0="version = \"" ver "\"" }
        { print }
    ' Cargo.toml > Cargo.toml.tmp && mv Cargo.toml.tmp Cargo.toml
}

# 更新 README.md 版本引用
update_readme_version() {
    local new_version=$1
    if [ -f "README.md" ]; then
        if [[ "$OSTYPE" == "darwin"* ]]; then
            sed -i '' -E "s/版本.*[0-9]+\.[0-9]+\.[0-9]+/版本 ${new_version}/g" README.md
        else
            sed -i -E "s/版本.*[0-9]+\.[0-9]+\.[0-9]+/版本 ${new_version}/g" README.md
        fi
        print_success "已更新 README.md"
    fi
}

# 计算新版本号
calculate_new_version() {
    local current=$1
    local bump_type=$2

    IFS='.' read -ra parts <<< "$current"
    local major=${parts[0]}
    local minor=${parts[1]}
    local patch=${parts[2]}
    local build=${parts[3]:-0}

    case $bump_type in
        major)
            echo "$((major + 1)).0.0"
            ;;
        minor)
            echo "${major}.$((minor + 1)).0"
            ;;
        patch)
            echo "${major}.${minor}.$((patch + 1))"
            ;;
        build)
            echo "${major}.${minor}.${patch}.$((build + 1))"
            ;;
        *)
            echo ""
            ;;
    esac
}

# 检查版本一致性
check_versions() {
    local cargo_version=$(get_cargo_version)
    local readme_version=$(get_readme_version)

    echo ""
    echo "版本号检查："
    echo "  Cargo.toml: $cargo_version"
    
    if [ -z "$readme_version" ]; then
        echo "  README.md:  (未找到版本号)"
        print_warning "README.md 中未找到版本号"
        exit 1
    else
        echo "  README.md:  $readme_version"
        
        if [ "$cargo_version" = "$readme_version" ]; then
            print_success "版本一致性检查通过"
        else
            print_error "版本不一致！Cargo.toml ($cargo_version) != README.md ($readme_version)"
            exit 1
        fi
    fi
}

# 显示使用帮助
show_usage() {
    echo ""
    echo "用法: ./scripts/version.sh [命令]"
    echo ""
    echo "命令:"
    echo "  (无参数)     显示当前版本号"
    echo "  major        升级主版本号 (1.0.0 → 2.0.0)"
    echo "  minor        升级次版本号 (1.0.0 → 1.1.0)"
    echo "  patch        升级补丁版本号 (1.0.0 → 1.0.1)"
    echo "  build        升级构建版本号 (1.0.0 → 1.0.0.1)"
    echo "  x.y.z        设置自定义版本号"
    echo "  check        检查版本一致性"
    echo "  help         显示此帮助信息"
    echo ""
    echo "示例:"
    echo "  ./scripts/version.sh patch    # 1.0.4 → 1.0.5"
    echo "  ./scripts/version.sh 2.0.0    # 设置版本为 2.0.0"
    echo ""
}

# 显示当前版本
show_current_version() {
    local cargo_version=$(get_cargo_version)

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  x_likes_downloader 版本信息"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    echo "  当前版本: v$cargo_version"

    # 显示最近的 Git 标签
    local latest_tag=$(git describe --tags --abbrev=0 2>/dev/null || echo "无")
    echo "  最新标签: $latest_tag"

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
}

# 执行版本升级
do_version_bump() {
    local current_version=$1
    local new_version=$2

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  x_likes_downloader 版本升级"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
    print_info "当前版本: $current_version"
    print_info "新版本:   $new_version"
    echo ""

    # 更新所有文件
    update_cargo_version "$new_version"
    print_success "已更新 Cargo.toml"

    update_readme_version "$new_version"

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    print_success "版本升级完成: $current_version → $new_version"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
}

# 主函数
main() {
    # 检查 Cargo.toml 存在
    if [ ! -f "Cargo.toml" ]; then
        print_error "Cargo.toml 不存在，请在项目根目录运行此脚本"
        exit 1
    fi

    local command=${1:-}

    case $command in
        "")
            show_current_version
            ;;
        help|--help|-h)
            show_usage
            ;;
        check)
            check_versions
            ;;
        major|minor|patch)
            local current_version=$(get_cargo_version)
            local new_version=$(calculate_new_version "$current_version" "$command")
            do_version_bump "$current_version" "$new_version"
            ;;
        build)
            local current_version=$(get_cargo_version)
            local new_version=$(calculate_new_version "$current_version" "$command")
            do_version_bump "$current_version" "$new_version"
            ;;
        *)
            # 检查是否是自定义版本号
            if validate_semver "$command"; then
                local current_version=$(get_cargo_version)
                do_version_bump "$current_version" "$command"
            else
                print_error "无效的命令或版本号格式: $command"
                echo ""
                echo "版本号必须符合 semver 格式: X.Y.Z (例如: 1.0.0, 0.2.1)"
                show_usage
                exit 1
            fi
            ;;
    esac
}

main "$@"
