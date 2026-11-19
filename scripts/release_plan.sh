#!/usr/bin/env bash
# 决定这次运行是开/更新 release PR、发布，还是跳过。
#
# 对齐 changesets/action 的分流（没有待发布改动就发布，有则开/更新同一条 PR）：
# https://github.com/changesets/action/blob/a45c4d594aa4e2c509dc14a9f2b3b67ba3780d0d/src/index.ts
# 日历号对齐 pi-extensions 的 nextAggregateCalver（不补零的 YYYY.M.P，跨月补丁归零）：
# https://github.com/HerbertGao/pi-extensions/blob/master/scripts/version-packages.mjs
# 不使用 changesets 的 semver，也不使用其 aggregate 以外的 -local.N 格式。

die() {
    printf 'release-plan: %s\n' "$1" >&2
    exit "${2:-1}"
}

info() {
    printf 'release-plan: %s\n' "$*" >&2
}

usage() {
    cat <<'EOF'
用法: scripts/release_plan.sh [--rev REF] [--github-output FILE] [--notes FILE]

读取 RELEASE_DATE（UTC YYYY-MM-DD，默认今天）、RELEASE_EVENT、RELEASE_REF、
RELEASE_EXPLICIT_VERSION。只读 git，不推送、不打 tag。
EOF
}

# 成功时打印 "year month patch"（不补零）。失败返回 1。
calver_parts() {
    local ver=$1
    if [[ ! $ver =~ ^([0-9]{4})\.(1[0-2]|[1-9])\.([0-9]+)$ ]]; then
        return 1
    fi
    printf '%s %s %s\n' \
        "$((10#${BASH_REMATCH[1]}))" \
        "$((10#${BASH_REMATCH[2]}))" \
        "$((10#${BASH_REMATCH[3]}))"
}

# 打印 -1 / 0 / 1。两个参数都必须是日历版本。
cmp_calver() {
    local ay am ap by bm bp pa pb
    pa=$(calver_parts "$1") || return 2
    pb=$(calver_parts "$2") || return 2
    read -r ay am ap <<<"$pa"
    read -r by bm bp <<<"$pb"
    if ((ay != by)); then
        if ((ay > by)); then
            echo 1
        else
            echo -1
        fi
        return 0
    fi
    if ((am != bm)); then
        if ((am > bm)); then
            echo 1
        else
            echo -1
        fi
        return 0
    fi
    if ((ap != bp)); then
        if ((ap > bp)); then
            echo 1
        else
            echo -1
        fi
        return 0
    fi
    echo 0
}

# 同月补丁 +1，跨月（或跨年）则为 YYYY.M.0。月份不补零。
next_calver() {
    local current=$1 date=$2
    local y m p cy cm cd parts
    parts=$(calver_parts "$current") || die "无效日历版本: $current"
    read -r y m p <<<"$parts"
    if [[ ! $date =~ ^([0-9]{4})-([0-9]{2})-([0-9]{2})$ ]]; then
        die "无效发布日期: $date"
    fi
    cy=$((10#${BASH_REMATCH[1]}))
    cm=$((10#${BASH_REMATCH[2]}))
    cd=$((10#${BASH_REMATCH[3]}))
    if ((cm < 1 || cm > 12 || cd < 1 || cd > 31)); then
        die "无效发布日期: $date"
    fi
    if ((y == cy && m == cm)); then
        printf '%d.%d.%d\n' "$y" "$m" "$((p + 1))"
    else
        printf '%d.%d.%d\n' "$cy" "$cm" 0
    fi
}

bump_patch() {
    local y m p parts
    parts=$(calver_parts "$1") || die "无效日历版本: $1"
    read -r y m p <<<"$parts"
    printf '%d.%d.%d\n' "$y" "$m" "$((p + 1))"
}

is_releasable_subject() {
    [[ $1 =~ ^(feat|fix)(\([A-Za-z0-9._/-]+\))?(!)?:[[:space:]].+ ]]
}

# 打印提交说明里的发布版本；没有则空。只认整行，避免正文引用误判。
release_version_from_text() {
    printf '%s\n' "$1" | sed -nE \
        's/^chore\(release\): 版本统一到 ([0-9]+\.[0-9]+\.[0-9]+)( \(#[0-9]+\))?[[:space:]]*$/\1/p' |
        awk 'NR == 1 { print }'
}

tag_exists() {
    git show-ref --verify --quiet "refs/tags/v$1"
}

max_calver_tag() {
    local max="" name ver cmp
    while IFS= read -r name; do
        [[ -n $name ]] || continue
        ver=${name#v}
        calver_parts "$ver" >/dev/null || continue
        if [[ -z $max ]]; then
            max=$ver
            continue
        fi
        cmp=$(cmp_calver "$ver" "$max") || die "无法比较 tag $ver 与 $max"
        if [[ $cmp == 1 ]]; then
            max=$ver
        fi
    done < <(git tag -l 'v[0-9]*')
    printf '%s\n' "$max"
}

read_cargo_version() {
    local rev=$1
    git show "${rev}:Cargo.toml" | sed -n \
        '/^\[package\]/,/^\[/s/^version = "\(.*\)"/\1/p' | head -n 1
}

is_semver() {
    [[ $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
}

ancestor_tag() {
    git describe --tags --abbrev=0 "$1" 2>/dev/null || true
}

collect_releasable_subjects() {
    local range=$1 subject log
    [[ -n $range ]] || return 0
    log=$(git log --reverse --format='%s' "$range")
    [[ -n $log ]] || return 0
    while IFS= read -r subject; do
        if is_releasable_subject "$subject"; then
            printf '%s\n' "$subject"
        fi
    done <<<"$log"
}

derive_version() {
    local cargo=$1 date=$2
    local base max_tag cmp candidate guard
    calver_parts "$cargo" >/dev/null || die "Cargo.toml 不是日历版本 YYYY.M.P: $cargo"
    base=$cargo
    max_tag=$(max_calver_tag)
    if [[ -n $max_tag ]]; then
        cmp=$(cmp_calver "$max_tag" "$base") || die "无法比较 $max_tag 与 $base"
        if [[ $cmp == 1 ]]; then
            base=$max_tag
        fi
    fi
    candidate=$(next_calver "$base" "$date")
    cmp=$(cmp_calver "$candidate" "$base") || die "无法比较 $candidate 与 $base"
    if [[ $cmp != 1 ]]; then
        die "推导出版本 $candidate 没有超过已有版本 $base（日期 $date）。请用 workflow_dispatch 显式指定版本"
    fi
    guard=0
    while tag_exists "$candidate"; do
        candidate=$(bump_patch "$candidate")
        guard=$((guard + 1))
        if ((guard > 1000)); then
            die "连续 tag 过多，停止从 $base 继续递增"
        fi
    done
    printf '%s\n' "$candidate"
}

build_notes() {
    local version=$1 explicit=$2 untagged=$3
    shift 3
    local -a subjects=()
    if (($# > 0)); then
        subjects=("$@")
    fi
    local bullets body item header footer
    bullets="（这次没有新的 \`feat\` / \`fix\` 提交。）"
    if ((${#subjects[@]} > 0)); then
        bullets=""
        for item in "${subjects[@]}"; do
            bullets+="- ${item}"$'\n'
        done
        bullets=${bullets%$'\n'}
    fi
    header=$(
        cat <<EOF
## Release v${version}

本 PR 由 \`master\` 上尚未发布的 \`feat\` / \`fix\` 提交自动打开或更新。版本按日历 \`YYYY.M.P\` 推导（同一个 UTC 月份递增最后一位，跨月则从 0 开始），不需要手填版本号。后续推送会覆盖同一分支 \`release/next\`，因此只会有一个待合并的 release PR。

合并到 \`master\` 时请保留标题 \`chore(release): 版本统一到 ${version}\`。合并后 CI 只打 annotated tag、构建 6 个平台并发布，不会再开下一档 release PR。
EOF
    )
    if [[ $untagged == 1 ]]; then
        header+=$'\n\n'"上次 release 提交还没有对应 tag。这次不会补发那个版本；要补发请推送该 tag。新的 \`feat\` / \`fix\` 记入下面这个版本。"
    fi
    if [[ $explicit == 1 ]]; then
        header+=$'\n\n'"这次版本号来自 workflow_dispatch 的输入，没有走自动推导。"
    fi
    footer=$(
        cat <<EOF
### 版本文件

\`scripts/version.sh\` 会把 \`Cargo.toml\`、\`Cargo.lock\`、\`README.md\` 和 \`.agents/skills/x_likes/SKILL.md\` 的 \`min_binary_version\` 统一到 \`${version}\`。

### 合并之后

1. 在合并提交上创建 annotated tag \`v${version}\`
2. 创建 draft Release，构建 linux / macos / windows 的 x86_64 与 arm64
3. 上传产物、发布 Release，并写入下载表格

请在本 PR 检查通过后合并。不要在本地手动打 tag；推送 \`v${version}\` 只作为发布失败后的恢复路径。
EOF
    )
    body=$(
        cat <<EOF
${header}

### 变更

${bullets}

${footer}
EOF
    )
    if ((${#body} > 60000)); then
        body=$(
            cat <<EOF
${header}

### 变更

变更列表超过 GitHub PR 正文上限，已省略。

${footer}
EOF
        )
    fi
    printf '%s\n' "$body"
}

set_plan() {
    PLAN_MODE=$1
    PLAN_VERSION=$2
    PLAN_TAG=$3
    PLAN_BRANCH=$4
    PLAN_REASON=$5
    PLAN_NOTES=$6
}

decide_release_plan() {
    local rev=$1 date=$2 event=$3 ref=$4 explicit=$5
    local message release_ver ancestor subjects_text subject
    local -a subjects=()
    local cargo version untagged=0 explicit_flag=0 older old_ver

    if [[ $ref == refs/tags/v* ]]; then
        version=${ref#refs/tags/v}
        is_semver "$version" || die "无效 tag 版本: $version"
        set_plan publish "$version" "v${version}" release/next publish:tag ""
        return 0
    fi

    message=$(git log -1 --format='%B' "$rev")
    release_ver=$(release_version_from_text "$message" || true)
    if [[ -n $release_ver ]]; then
        if tag_exists "$release_ver"; then
            set_plan skip "$release_ver" "v${release_ver}" release/next \
                skip:release-tagged ""
            return 0
        fi
        if [[ $event == workflow_dispatch ]]; then
            set_plan skip "$release_ver" "v${release_ver}" release/next \
                skip:dispatch-no-publish ""
            return 0
        fi
        set_plan publish "$release_ver" "v${release_ver}" release/next \
            publish:release-commit ""
        return 0
    fi

    ancestor=$(ancestor_tag "$rev")
    if [[ $event == workflow_dispatch && -n $explicit ]]; then
        version=${explicit#v}
        is_semver "$version" || die "无效版本: $explicit"
        if tag_exists "$version"; then
            die "tag v${version} 已存在，请换一个版本"
        fi
        explicit_flag=1
    fi

    if [[ -z $ancestor && $explicit_flag != 1 ]]; then
        set_plan skip "" "" release/next skip:no-tag ""
        return 0
    fi

    subjects_text=""
    if [[ -n $ancestor ]]; then
        subjects_text=$(collect_releasable_subjects "${ancestor}..${rev}")
    fi
    if [[ -n $subjects_text ]]; then
        while IFS= read -r subject; do
            [[ -n $subject ]] || continue
            subjects+=("$subject")
        done <<<"$subjects_text"
    fi

    if [[ $explicit_flag != 1 && ${#subjects[@]} -eq 0 ]]; then
        set_plan skip "" "" release/next skip:no-releasable ""
        return 0
    fi

    if [[ $explicit_flag == 1 && -n $ancestor ]]; then
        if git rev-parse --verify --quiet "${rev}^" >/dev/null; then
            older=$(git log --format='%s%n%b' "${ancestor}..${rev}^" || true)
            old_ver=$(release_version_from_text "$older" || true)
            if [[ -n $old_ver ]] && ! tag_exists "$old_ver"; then
                untagged=1
            fi
        fi
    fi
    if [[ $explicit_flag != 1 ]]; then
        cargo=$(read_cargo_version "$rev")
        [[ -n $cargo ]] || die "无法从 ${rev} 的 Cargo.toml 读取版本"
        version=$(derive_version "$cargo" "$date")
        if git rev-parse --verify --quiet "${rev}^" >/dev/null; then
            older=$(git log --format='%s%n%b' "${ancestor}..${rev}^" || true)
            old_ver=$(release_version_from_text "$older" || true)
            if [[ -n $old_ver ]] && ! tag_exists "$old_ver"; then
                untagged=1
            fi
        fi
    fi

    if ((${#subjects[@]} > 0)); then
        PLAN_NOTES=$(build_notes "$version" "$explicit_flag" "$untagged" "${subjects[@]}")
    else
        PLAN_NOTES=$(build_notes "$version" "$explicit_flag" "$untagged")
    fi
    if [[ $explicit_flag == 1 ]]; then
        set_plan prepare "$version" "v${version}" release/next prepare:explicit \
            "$PLAN_NOTES"
    else
        set_plan prepare "$version" "v${version}" release/next prepare:releasable \
            "$PLAN_NOTES"
    fi
}

write_plan() {
    local dest=$1
    local content
    content=$(
        printf 'mode=%s\nversion=%s\ntag=%s\nbranch=%s\nreason=%s\n' \
            "$PLAN_MODE" "$PLAN_VERSION" "$PLAN_TAG" "$PLAN_BRANCH" "$PLAN_REASON"
    )
    if [[ -n $dest ]]; then
        printf '%s\n' "$content" >>"$dest"
    else
        printf '%s\n' "$content"
    fi
}

main() {
    set -euo pipefail
    local rev="${RELEASE_REV:-HEAD}"
    local output="" notes_path=""
    while [[ $# -gt 0 ]]; do
        case "$1" in
        --rev)
            rev=$2
            shift 2
            ;;
        --github-output)
            output=$2
            shift 2
            ;;
        --notes)
            notes_path=$2
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            die "未知参数: $1" 2
            ;;
        esac
    done

    local date=${RELEASE_DATE:-$(date -u +%Y-%m-%d)}
    local event=${RELEASE_EVENT:-push}
    local ref=${RELEASE_REF:-refs/heads/master}
    local explicit=${RELEASE_EXPLICIT_VERSION:-}

    if [[ ${RELEASE_FETCH_TAGS:-} == 1 ]]; then
        git fetch --tags --force origin
    fi

    decide_release_plan "$rev" "$date" "$event" "$ref" "$explicit"
    write_plan "$output"
    if [[ -n $notes_path ]]; then
        printf '%s\n' "$PLAN_NOTES" >"$notes_path"
    fi
    info "${PLAN_MODE} ${PLAN_VERSION} ${PLAN_REASON}"
}

if [[ ${BASH_SOURCE[0]} == "$0" ]]; then
    main "$@"
fi
