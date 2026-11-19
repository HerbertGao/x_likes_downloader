#!/usr/bin/env bash
# 按 release_plan.sh 的结果，在固定分支 release/next 上统一版本并开/更新同一个 PR。
# publish / skip 时什么都不推。需要 RELEASE_ALLOW_PUSH=1 才会推送。
# 生产仓库只允许在 GitHub Actions 里推送。

set -euo pipefail

die() {
    printf 'release-open: %s\n' "$1" >&2
    exit "${2:-1}"
}

info() {
    printf 'release-open: %s\n' "$*" >&2
}

root=$(git rev-parse --show-toplevel)
cd "$root"

lock=$(git rev-parse --absolute-git-dir)/release-next.lock
exec 9>"$lock"
if ! command -v flock >/dev/null 2>&1; then
    die "需要 flock，以避免并发推送 release/next"
fi
flock 9

plan_out=$(mktemp)
notes_out=$(mktemp)
trap 'rm -f "$plan_out" "$notes_out"' EXIT

bash "$root/scripts/release_plan.sh" \
    --github-output "$plan_out" \
    --notes "$notes_out"

mode=""
version=""
branch=""
reason=""
while IFS= read -r line; do
    key=${line%%=*}
    val=${line#*=}
    case "$key" in
    mode) mode=$val ;;
    version) version=$val ;;
    branch) branch=$val ;;
    reason) reason=$val ;;
    esac
done <"$plan_out"

info "mode=${mode} version=${version} reason=${reason}"
if [[ $mode != prepare ]]; then
    info "不打开 release PR"
    exit 0
fi

[[ -n $version && -n $branch ]] || die "prepare 缺少 version 或 branch"
[[ $branch == release/next ]] || die "release 分支必须是 release/next，实际是 $branch"

origin_url=$(git remote get-url origin 2>/dev/null || true)
case "$origin_url" in
*HerbertGao/x_likes_downloader*)
    if [[ ${GITHUB_ACTIONS:-} != true ]]; then
        die "拒绝在 GitHub Actions 之外推送到生产仓库"
    fi
    ;;
esac
[[ ${RELEASE_ALLOW_PUSH:-} == 1 ]] || die "拒绝推送（仅 release workflow 设置 RELEASE_ALLOW_PUSH=1）"

rev=${RELEASE_REV:-HEAD}
base=${RELEASE_BASE:-master}
git rev-parse --verify --quiet "$rev" >/dev/null || die "找不到基线 $rev"

git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"

    if [[ ${RELEASE_FETCH_TAGS:-} == 1 ]]; then
        git fetch origin "refs/heads/${branch}:refs/remotes/origin/${branch}" || true
    fi

git checkout -B "$branch" "$rev"
bash "$root/scripts/version.sh" "$version"
if [[ ${RELEASE_SKIP_CARGO:-} != 1 && -f Cargo.lock ]]; then
    cargo metadata --format-version 1 >/dev/null
fi
bash "$root/scripts/version.sh" check
grep -q -x -F "min_binary_version: ${version}" \
    .agents/skills/x_likes/SKILL.md ||
    die "min_binary_version 不是 $version"
if [[ ${RELEASE_SKIP_SKILL_CHECK:-} != 1 ]]; then
    bash "$root/scripts/check-skills.sh"
fi

git add -- Cargo.toml README.md .agents/skills/x_likes/SKILL.md
if [[ -f Cargo.lock ]]; then
    git add -- Cargo.lock
fi
if git diff --cached --quiet; then
    die "nothing to bump (already ${version} everywhere)"
fi

title="chore(release): 版本统一到 ${version}"
commit_file=$(mktemp)
{
    printf '%s\n\n' "$title"
    cat "$notes_out"
} >"$commit_file"
git commit -F "$commit_file"
rm -f "$commit_file"

if git show-ref --verify --quiet "refs/remotes/origin/${branch}"; then
    git push \
        --force-with-lease="refs/heads/${branch}:refs/remotes/origin/${branch}" \
        origin "HEAD:refs/heads/${branch}"
else
    git push -u origin "HEAD:refs/heads/${branch}"
fi

command -v gh >/dev/null 2>&1 || die "未找到 gh"
export GH_TOKEN="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
[[ -n ${GH_TOKEN:-} || -n ${GH_MOCK_DIR:-} ]] || die "缺少 GH_TOKEN"

list_head=$branch
if [[ -n ${GITHUB_REPOSITORY_OWNER:-} ]]; then
    list_head="${GITHUB_REPOSITORY_OWNER}:${branch}"
fi
pr=$(
    gh pr list --head "$list_head" --base "$base" --state open \
        --json number -q '.[0].number // empty'
)
if [[ -n $pr ]]; then
    info "更新已有 PR #${pr}"
    gh pr edit "$pr" --title "$title" --body-file "$notes_out"
    gh pr view "$pr" --json url -q .url
else
    info "创建 release PR"
    gh pr create --base "$base" --head "$branch" \
        --title "$title" --body-file "$notes_out"
fi
