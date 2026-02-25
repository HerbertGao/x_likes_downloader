#!/bin/bash

# x_likes_downloader 发布脚本
# 用途：整合版本升级、构建验证和 Git 操作的一键发布流程
# 使用：./scripts/release.sh [major|minor|patch|build|x.y.z]

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
PROJECT_ROOT=$(pwd)

# 检查是否在 Git 仓库中
check_git_repo() {
    if ! git rev-parse --is-inside-work-tree > /dev/null 2>&1; then
        print_error "当前目录不是 Git 仓库"
        exit 1
    fi
    print_success "Git 仓库检查通过"
}

# 检查当前分支
check_branch() {
    local current_branch=$(git branch --show-current)
    local main_branches=("master" "main")

    local is_main=false
    for branch in "${main_branches[@]}"; do
        if [ "$current_branch" = "$branch" ]; then
            is_main=true
            break
        fi
    done

    if ! $is_main; then
        print_warning "当前分支: $current_branch (非主分支)"
        echo ""
        read -p "是否继续在非主分支上发布? [y/N] " -n 1 -r
        echo ""
        if [[ ! $REPLY =~ ^[Yy]$ ]]; then
            print_info "已取消发布"
            exit 0
        fi
    else
        print_success "当前分支: $current_branch"
    fi
}

# 检查工作区状态
check_working_directory() {
    if ! git diff --quiet || ! git diff --cached --quiet; then
        print_error "工作区有未提交的更改"
        echo ""
        echo "未提交的文件："
        git status --short
        echo ""
        echo "请先提交或暂存更改后再发布"
        exit 1
    fi

    # 检查未跟踪的文件（仅警告）
    local untracked=$(git ls-files --others --exclude-standard)
    if [ -n "$untracked" ]; then
        print_warning "存在未跟踪的文件（不影响发布）"
    fi

    print_success "工作区干净"
}

# 拉取最新代码
pull_latest() {
    print_info "拉取最新代码..."

    local remote=$(git remote | head -1)
    if [ -z "$remote" ]; then
        print_warning "未配置远程仓库，跳过拉取"
        return
    fi

    local current_branch=$(git branch --show-current)

    # 检查是否有上游分支
    if git rev-parse --abbrev-ref --symbolic-full-name @{u} > /dev/null 2>&1; then
        git fetch "$remote" > /dev/null 2>&1
        git pull "$remote" "$current_branch" --ff-only > /dev/null 2>&1 || {
            print_warning "无法快进合并，请手动解决冲突"
            exit 1
        }
        print_success "已拉取最新代码"
    else
        print_warning "当前分支未设置上游，跳过拉取"
    fi
}

# 运行构建验证
run_build_check() {
    print_info "运行构建验证..."

    if command -v cargo &> /dev/null; then
        echo "  检查 Rust 代码..."
        if ! cargo check --quiet 2>&1; then
            print_error "Rust 代码检查失败"
            exit 1
        fi
        print_success "Rust 代码检查通过"
    else
        print_error "未安装 cargo"
        exit 1
    fi
}

# 检查标签是否存在
check_tag_exists() {
    local tag=$1
    if git tag -l | grep -q "^${tag}$"; then
        return 0
    fi
    return 1
}

# 创建版本提交
create_version_commit() {
    local version=$1

    print_info "创建版本提交..."

    # 添加版本相关文件
    git add Cargo.toml
    [ -f "Cargo.lock" ] && git add Cargo.lock
    [ -f "README.md" ] && git add README.md

    # 检查是否有更改
    if git diff --cached --quiet; then
        print_warning "没有版本文件更改，跳过提交"
        return
    fi

    git commit -m "chore: bump version to $version"
    print_success "已创建提交: chore: bump version to $version"
}

# 创建 Git 标签
create_tag() {
    local version=$1
    local tag="v$version"

    print_info "创建 Git 标签: $tag"

    if check_tag_exists "$tag"; then
        print_error "标签 $tag 已存在"
        echo ""
        echo "解决方案："
        echo "  1. 删除已存在的标签: git tag -d $tag && git push origin :refs/tags/$tag"
        echo "  2. 或使用其他版本号"
        exit 1
    fi

    git tag -a "$tag" -m "Release $tag"
    print_success "已创建标签: $tag"
}

# 推送到远程
push_to_remote() {
    local version=$1
    local tag="v$version"

    echo ""
    read -p "是否推送到远程仓库? [Y/n] " -n 1 -r
    echo ""

    if [[ $REPLY =~ ^[Nn]$ ]]; then
        print_info "已跳过推送"
        echo ""
        echo "稍后手动推送："
        echo "  git push"
        echo "  git push origin $tag"
        return
    fi

    local remote=$(git remote | head -1)
    if [ -z "$remote" ]; then
        print_warning "未配置远程仓库，无法推送"
        return
    fi

    print_info "推送到远程..."
    local current_branch=$(git branch --show-current)
    git push "$remote" "$current_branch" > /dev/null 2>&1
    git push "$remote" "$tag" > /dev/null 2>&1
    print_success "已推送到远程"

    echo ""
    print_info "GitHub Actions 将自动构建并创建 Release"
    echo "  查看状态: https://github.com/HerbertGao/x_likes_downloader/actions"
}

# 显示使用帮助
show_usage() {
    echo ""
    echo "用法: ./scripts/release.sh [版本类型]"
    echo ""
    echo "版本类型:"
    echo "  major        主版本升级 (1.0.0 → 2.0.0)"
    echo "  minor        次版本升级 (1.0.0 → 1.1.0)"
    echo "  patch        补丁版本升级 (1.0.0 → 1.0.1)"
    echo "  build        构建版本升级 (1.0.0 → 1.0.0.1)"
    echo "  x.y.z        自定义版本号"
    echo ""
    echo "示例:"
    echo "  ./scripts/release.sh patch    # 发布补丁版本"
    echo "  ./scripts/release.sh 1.0.0    # 发布 1.0.0 版本"
    echo ""
    echo "流程:"
    echo "  1. 检查 Git 仓库状态"
    echo "  2. 检查当前分支"
    echo "  3. 检查工作区干净"
    echo "  4. 拉取最新代码"
    echo "  5. 升级版本号"
    echo "  6. 运行构建验证"
    echo "  7. 创建版本提交"
    echo "  8. 创建 Git 标签"
    echo "  9. 推送到远程（可选）"
    echo ""
}

# 主函数
main() {
    local version_arg=${1:-}

    if [ -z "$version_arg" ] || [ "$version_arg" = "help" ] || [ "$version_arg" = "--help" ] || [ "$version_arg" = "-h" ]; then
        show_usage
        exit 0
    fi

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  x_likes_downloader 发布工具"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""

    # 步骤 1: 检查 Git 仓库
    check_git_repo

    # 步骤 2: 检查当前分支
    check_branch

    # 步骤 3: 检查工作区状态
    check_working_directory

    # 步骤 4: 拉取最新代码
    pull_latest

    echo ""

    # 步骤 5: 升级版本号
    print_info "升级版本号..."
    ./scripts/version.sh "$version_arg"

    # 获取新版本号
    local new_version=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')

    echo ""

    # 步骤 6: 运行构建验证
    run_build_check

    echo ""

    # 步骤 7: 创建版本提交
    create_version_commit "$new_version"

    # 步骤 8: 创建 Git 标签
    create_tag "$new_version"

    # 步骤 9: 推送到远程
    push_to_remote "$new_version"

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    print_success "发布准备完成: v$new_version"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""
}

main "$@"
