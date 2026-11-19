#!/usr/bin/env bash
# 通过 GitHub Actions 发起一次 release PR（不在本地打 tag）。
#
# 流程：
#   1. 本脚本 dispatch .github/workflows/release.yml。version 可以省略。
#   2. CI 若看到未发布的 feat/fix，就在固定分支 release/next 上统一版本号并开/更新 PR。
#   3. 合并该 PR 到 master 后，CI 自动打 tag、构建 6 平台并发布 GitHub Release。
#
# 用法：
#   scripts/release.sh                    # 不填版本，由 CI 按日历号推导
#   scripts/release.sh 2026.12.0          # 显式版本，覆盖自动推导
#   scripts/release.sh --dry-run
#
# 说明：不再在本地打 tag / push（tag 推送仅保留为 CI 的恢复路径）。

set -euo pipefail

die() {
    printf 'release: %s\n' "$1" >&2
    exit "${2:-1}"
}

usage() {
    cat <<'USAGE'
用法: scripts/release.sh [version|major|minor|patch] [--dry-run|--watch]

  （省略）  dispatch 时不传 version，由 CI 按日历版本自动推导
  version   显式版本（N.N.N，可带前导 v），覆盖自动推导
  major     当前主版本 +1（传统位数，不是日历规则）
  minor     当前次版本 +1（传统位数，不是日历规则）
  patch     当前补丁 +1（传统位数，不是日历规则）
  --dry-run 只打印解析结果，不 dispatch
  --watch   dispatch 后等待该次 workflow 结束
USAGE
}

arg=""
watch=false
dry=false
for opt in "$@"; do
    case "$opt" in
    --watch) watch=true ;;
    --dry-run) dry=true ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        [[ -z $arg ]] || die "未知参数 '$opt'" 2
        arg="${opt#v}"
        ;;
    esac
done

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || die "无法定位仓库根目录"
cd "$root" || die "无法进入仓库根目录"

repo_slug="${RELEASE_REPO_SLUG:-HerbertGao/x_likes_downloader}"
branch="${RELEASE_BRANCH:-master}"

# ---- 解析目标版本 ----------------------------------------------------------
current="$(sed -n '/^\[package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[[ -n "$current" ]] || die "无法从 Cargo.toml 读取当前版本"

ver=""
if [[ -n $arg ]]; then
    case "$arg" in
    major | minor | patch)
        IFS='.' read -r MA MI PA <<<"$current"
        case "$arg" in
        major) ver="$((MA + 1)).0.0" ;;
        minor) ver="${MA}.$((MI + 1)).0" ;;
        patch) ver="${MA}.${MI}.$((PA + 1))" ;;
        esac
        ;;
    *) ver="$arg" ;;
    esac
    [[ "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || {
        die "无效版本 '$arg'（需要 N.N.N；当前为 ${current}）" 2
    }
fi

if [[ "$dry" == "true" ]]; then
    if [[ -n $ver ]]; then
        echo "release: DRY-RUN $current -> $ver (repo=$repo_slug, ref=$branch)"
        echo "release: 将 dispatch release.yml -f version=$ver"
    else
        echo "release: DRY-RUN auto from $current (repo=$repo_slug, ref=$branch)"
        echo "release: 将 dispatch release.yml（不传 version）"
    fi
    exit 0
fi

# ---- 前置检查 --------------------------------------------------------------
command -v gh >/dev/null 2>&1 || die "未找到 gh（GitHub CLI）"
gh auth status >/dev/null 2>&1 || die "gh 未登录（gh auth login）"

remote_url="$(git remote get-url --push origin 2>/dev/null || git remote get-url origin 2>/dev/null)" ||
    die "无法读取 origin remote"
case "$remote_url" in
"https://github.com/$repo_slug" | "https://github.com/$repo_slug.git" | \
    "git@github.com:$repo_slug.git" | "ssh://git@github.com/$repo_slug.git") ;;
*) die "origin '$remote_url' 与发布仓库 '$repo_slug' 不一致" ;;
esac

if [[ -n $ver ]]; then
    if git rev-parse -q --verify "refs/tags/v$ver" >/dev/null 2>&1 ||
        git ls-remote --exit-code --tags origin "refs/tags/v$ver" >/dev/null 2>&1; then
        die "tag v$ver 已存在，请换一个版本" 2
    fi
fi

# ---- 派发 workflow ---------------------------------------------------------
if [[ -n $ver ]]; then
    echo "release: $current -> $ver (repo=$repo_slug, ref=$branch)"
    gh workflow run release.yml --ref "$branch" -f "version=$ver" ||
        die "dispatch 失败（确认 $branch 上已有 release.yml）" 2
else
    echo "release: auto from $current (repo=$repo_slug, ref=$branch)"
    gh workflow run release.yml --ref "$branch" ||
        die "dispatch 失败（确认 $branch 上已有 release.yml）" 2
fi

# workflow_dispatch 的 run 不是同步创建的，轮询几次拿到本次 run。
run_id=""
for _ in 1 2 3 4 5; do
    sleep 2
    run_id="$(gh run list --workflow=release.yml --branch "$branch" \
        --limit 1 --json databaseId -q '.[0].databaseId // empty')"
    [[ -n "$run_id" ]] && break
done
[[ -n "$run_id" ]] || die "已 dispatch，但未能定位运行记录；请用 gh run list 查看"

echo "release: 已 dispatch run $run_id"
gh run list --workflow=release.yml --branch "$branch" --limit 1 \
    --json url,status -q '.[0] | "\(.status) \(.url)"'

echo ""
if [[ -n $ver ]]; then
    echo "下一步：等 CI 开/更新 release PR（release/next，版本 ${ver}），检查全绿后合并到 ${branch}。"
    echo "合并后 CI 会自动打 tag v$ver 并构建发布。"
else
    echo "下一步：等 CI 按日历版本开/更新 release PR（release/next），检查全绿后合并到 ${branch}。"
    echo "合并后 CI 会自动打 tag 并构建发布。没有可发布提交时这次运行会直接跳过。"
fi

if [[ "$watch" == "true" ]]; then
    exec gh run watch "$run_id" --exit-status
fi
