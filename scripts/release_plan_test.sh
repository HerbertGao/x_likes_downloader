#!/usr/bin/env bash
# 离线测试 release PR 的推导、幂等更新和「合并后只发布」。
# 不推送 GitHub、不打远端 tag、不 dispatch workflow。
set -euo pipefail

# CI 会设置这些变量。测试必须自己控制，避免误 fetch 或误放行生产 remote。
unset GITHUB_ACTIONS GITHUB_TOKEN GH_TOKEN RELEASE_FETCH_TAGS || true

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# shellcheck source=scripts/release_plan.sh
source "$ROOT/scripts/release_plan.sh"

TMPS=()
cleanup() {
    rm -rf "${TMPS[@]}"
}
trap cleanup EXIT

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

assert_eq() {
    local got=$1 want=$2 label=$3
    if [[ $got != "$want" ]]; then
        fail "$label: got [$got] want [$want]"
    fi
}

CASES=0
ok() {
    CASES=$((CASES + 1))
    printf 'ok %s\n' "$1"
}

MOCK_BIN=$(mktemp -d)
TMPS+=("$MOCK_BIN")
cat >"$MOCK_BIN/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
[[ -n ${GH_MOCK_DIR:-} ]] || {
    echo "GH_MOCK_DIR missing" >&2
    exit 1
}
mkdir -p "$GH_MOCK_DIR"
printf '%s\n' "$*" >>"$GH_MOCK_DIR/log"
if [[ $1 == pr && $2 == list ]]; then
    if [[ -f $GH_MOCK_DIR/number ]]; then
        cat "$GH_MOCK_DIR/number"
    fi
    exit 0
fi
if [[ $1 == pr && $2 == create ]]; then
    if [[ -f $GH_MOCK_DIR/number ]]; then
        echo "duplicate pr create" >&2
        exit 1
    fi
    echo 41 >"$GH_MOCK_DIR/number"
    echo "https://example.invalid/pull/41"
    exit 0
fi
if [[ $1 == pr && $2 == edit ]]; then
    echo edit >>"$GH_MOCK_DIR/edits"
    exit 0
fi
if [[ $1 == pr && $2 == view ]]; then
    echo "https://example.invalid/pull/41"
    exit 0
fi
echo "unexpected gh: $*" >&2
exit 1
EOF
chmod +x "$MOCK_BIN/gh"

count_log() {
    local n=0
    if [[ -f $MOCK/log ]]; then
        n=$(grep -c "$1" "$MOCK/log" || true)
    fi
    printf '%s\n' "$n"
}

setup_repo() {
    local parent
    parent=$(mktemp -d)
    TMPS+=("$parent")
    REPO=$parent/repo
    MOCK=$parent/mock
    mkdir -p "$MOCK" "$REPO/scripts" "$REPO/.agents/skills/x_likes"
    git init -q -b master "$REPO"
    git -C "$REPO" config user.name release-test
    git -C "$REPO" config user.email release-test@example.invalid
    cp "$ROOT/scripts/release_plan.sh" "$ROOT/scripts/release_open_pr.sh" \
        "$ROOT/scripts/version.sh" "$REPO/scripts/"
    chmod +x "$REPO/scripts/"*.sh
    cat >"$REPO/Cargo.toml" <<'EOF'
[package]
name = "x_likes_downloader"
version = "2026.11.0"
edition = "2021"
EOF
    printf '版本 2026.11.0\n' >"$REPO/README.md"
    cat >"$REPO/.agents/skills/x_likes/SKILL.md" <<'EOF'
---
name: x_likes
description: fixture
min_binary_version: 2026.11.0
---

fixture
EOF
    git -C "$REPO" add Cargo.toml README.md .agents/skills/x_likes/SKILL.md scripts
    git -C "$REPO" commit -qm "chore: initial"
    git -C "$REPO" tag v2026.11.0
    git clone -q --bare "$REPO" "$parent/remote.git"
    git -C "$REPO" remote add origin "$parent/remote.git"
    git -C "$REPO" push -q origin master
    git -C "$REPO" push -q origin v2026.11.0
    case $(git -C "$REPO" remote get-url origin) in
    *github.com*) fail "fixture origin points at GitHub" ;;
    esac
}

add_commit() {
    local msg=$1
    git -C "$REPO" checkout -q master
    printf '%s\n' "$msg" >>"$REPO/notes.txt"
    git -C "$REPO" add notes.txt
    git -C "$REPO" commit -qm "$msg"
    git -C "$REPO" push -q origin master
}

run_plan() {
    local date=$1 event=${2:-push} ref=${3:-refs/heads/master} explicit=${4:-}
    PLAN_OUT=$(mktemp)
    TMPS+=("$PLAN_OUT")
    (
        cd "$REPO"
        # shellcheck disable=SC2097,SC2098
        env -u RELEASE_EXPLICIT_VERSION \
            RELEASE_REV=master \
            RELEASE_DATE="$date" \
            RELEASE_EVENT="$event" \
            RELEASE_REF="$ref" \
            ${explicit:+RELEASE_EXPLICIT_VERSION="$explicit"} \
            bash scripts/release_plan.sh --github-output "$PLAN_OUT"
    )
}

plan_field() {
    sed -n "s/^$1=//p" "$PLAN_OUT" | head -n 1
}

run_open() {
    local date=$1 event=${2:-push} ref=${3:-refs/heads/master} explicit=${4:-}
    (
        cd "$REPO"
        # shellcheck disable=SC2097,SC2098
        env -u RELEASE_EXPLICIT_VERSION \
            PATH="$MOCK_BIN:$PATH" \
            GH_MOCK_DIR="$MOCK" \
            RELEASE_ALLOW_PUSH=1 \
            RELEASE_SKIP_CARGO=1 \
            RELEASE_SKIP_SKILL_CHECK=1 \
            RELEASE_REV=master \
            RELEASE_DATE="$date" \
            RELEASE_EVENT="$event" \
            RELEASE_REF="$ref" \
            ${explicit:+RELEASE_EXPLICIT_VERSION="$explicit"} \
            bash scripts/release_open_pr.sh
    )
}

file_version() {
    local rev=$1
    git -C "$REPO" show "$rev:Cargo.toml" | sed -n 's/^version = "\(.*\)"/\1/p' | head -n 1
}

expect_fail() {
    local desc=$1
    shift
    set +e
    local out
    out=$("$@" 2>&1)
    local code=$?
    set -e
    if [[ $code -eq 0 ]]; then
        fail "$desc: expected failure, got success: $out"
    fi
    printf '%s\n' "$out"
}

# --- 日历号，对齐 pi-extensions nextAggregateCalver 的用例 -------------------
assert_eq "$(next_calver 2026.8.0 2026-08-10)" "2026.8.1" "same month patch"
assert_eq "$(next_calver 2026.8.9 2026-08-31)" "2026.8.10" "end of month"
assert_eq "$(next_calver 2026.8.9 2026-09-01)" "2026.9.0" "next month"
assert_eq "$(next_calver 2026.12.3 2027-01-01)" "2027.1.0" "next year unpadded"
assert_eq "$(next_calver 2026.11.0 2026-11-19)" "2026.11.1" "november patch"
assert_eq "$(next_calver 2026.11.0 2026-12-01)" "2026.12.0" "december reset"
assert_eq "$(next_calver 2026.10.0 2026-11-01)" "2026.11.0" "october to november"
expect_fail "padded month" bash -c "source '$ROOT/scripts/release_plan.sh'; next_calver 2026.08.0 2026-08-10" >/dev/null
expect_fail "bad date" bash -c "source '$ROOT/scripts/release_plan.sh'; next_calver 2026.11.0 not-a-date" >/dev/null
ok "calendar versions"

ver=$(release_version_from_text $'Merge pull request #3 from example/release/next\n\nchore(release): 版本统一到 2026.12.0\n')
assert_eq "$ver" "2026.12.0" "merge commit body"
ver=$(release_version_from_text "chore(release): 版本统一到 2026.11.1 (#12)")
assert_eq "$ver" "2026.11.1" "squash title"
ver=$(release_version_from_text "请保留标题 chore(release): 版本统一到 2026.11.1")
assert_eq "$ver" "" "prose does not match"
is_releasable_subject "feat: alpha" || fail "feat"
is_releasable_subject "fix(release): tighten output" || fail "scoped fix"
is_releasable_subject "feat!: break" || fail "breaking feat"
is_releasable_subject "docs: readme" && fail "docs should not release"
is_releasable_subject "chore: tooling" && fail "chore should not release"
is_releasable_subject "refactor: move" && fail "refactor should not release"
is_releasable_subject "Feat: Alpha" && fail "uppercase type should not release"
ok "subject classification"

# --- feat/fix 开 PR，重复推送更新同一 PR，跨月仍是同一分支 -------------------
setup_repo
add_commit "docs: ignore me"
add_commit "chore: tooling"
add_commit "feat: alpha"
add_commit "fix(release): tighten output"
run_open 2026-11-19
assert_eq "$(file_version release/next)" "2026.11.1" "bumped cargo"
assert_eq "$(file_version master)" "2026.11.0" "master cargo unchanged"
assert_eq "$(git -C "$REPO" rev-list --count master..release/next)" "1" "one release commit"
assert_eq "$(git -C "$REPO" log -1 --format=%s release/next)" \
    "chore(release): 版本统一到 2026.11.1" "release subject"
body=$(git -C "$REPO" log -1 --format=%B release/next)
printf '%s\n' "$body" | grep -F "feat: alpha" >/dev/null || fail "notes missing feat"
printf '%s\n' "$body" | grep -F "fix(release): tighten output" >/dev/null || fail "notes missing fix"
printf '%s\n' "$body" | grep -F "docs: ignore me" >/dev/null && fail "docs leaked into notes"
printf '%s\n' "$body" | grep -F "chore: tooling" >/dev/null && fail "chore leaked into notes"
skill=$(git -C "$REPO" show release/next:.agents/skills/x_likes/SKILL.md)
printf '%s\n' "$skill" | grep -x "min_binary_version: 2026.11.1" >/dev/null || fail "skill version"
readme=$(git -C "$REPO" show release/next:README.md)
printf '%s\n' "$readme" | grep -F "版本 2026.11.1" >/dev/null || fail "readme version"
assert_eq "$(count_log '^pr create')" "1" "created once"
assert_eq "$(count_log '^pr edit')" "0" "not edited yet"
assert_eq "$(git -C "$REPO" for-each-ref --format='%(refname:short)' 'refs/heads/release/*')" \
    "release/next" "stable branch"
ok "feat/fix opens one release PR"

add_commit "fix: beta"
run_open 2026-11-19
assert_eq "$(file_version release/next)" "2026.11.1" "version unchanged on repeat"
assert_eq "$(git -C "$REPO" rev-list --count master..release/next)" "1" "still one commit"
assert_eq "$(git -C "$REPO" merge-base release/next master)" \
    "$(git -C "$REPO" rev-parse master)" "release branch rebuilt on latest master"
body=$(git -C "$REPO" log -1 --format=%B release/next)
printf '%s\n' "$body" | grep -F "feat: alpha" >/dev/null || fail "alpha dropped"
printf '%s\n' "$body" | grep -F "fix: beta" >/dev/null || fail "beta missing"
assert_eq "$(count_log '^pr create')" "1" "no second PR"
assert_eq "$(count_log '^pr edit')" "1" "updated the open PR"
ok "repeat push updates the same PR"

run_open 2026-12-01
assert_eq "$(file_version release/next)" "2026.12.0" "month rollover"
assert_eq "$(git -C "$REPO" log -1 --format=%s release/next)" \
    "chore(release): 版本统一到 2026.12.0" "rolled title"
assert_eq "$(git -C "$REPO" for-each-ref --format='%(refname:short)' 'refs/heads/release/*')" \
    "release/next" "rollover keeps the branch"
assert_eq "$(count_log '^pr create')" "1" "rollover does not open another PR"
assert_eq "$(count_log '^pr edit')" "2" "rollover edits the same PR"
assert_eq "$(file_version master)" "2026.11.0" "master still unreleased"
ok "calendar month rollover updates the same PR"

git -C "$REPO" checkout -q master
git -C "$REPO" merge -q --no-ff release/next \
    -m "Merge pull request #3 from example/release/next" \
    -m "chore(release): 版本统一到 2026.12.0"
git -C "$REPO" push -q origin master
run_plan 2026-12-01
assert_eq "$(plan_field mode)" "publish" "release merge publishes"
assert_eq "$(plan_field version)" "2026.12.0" "publish version"
assert_eq "$(plan_field reason)" "publish:release-commit" "publish reason"
before=$(count_log '^pr create')
run_open 2026-12-01
assert_eq "$(count_log '^pr create')" "$before" "publish does not open a PR"
ok "release merge publishes and does not open a PR"

run_plan 2026-12-01 workflow_dispatch
assert_eq "$(plan_field mode)" "skip" "dispatch does not publish"
assert_eq "$(plan_field reason)" "skip:dispatch-no-publish" "dispatch reason"
ok "manual dispatch does not publish a release commit"

git -C "$REPO" tag v2026.12.0
run_plan 2026-12-01
assert_eq "$(plan_field mode)" "skip" "existing tag skips publish"
assert_eq "$(plan_field reason)" "skip:release-tagged" "tagged reason"
ok "existing release tag skips"

add_commit "docs: after release"
run_plan 2026-12-15
assert_eq "$(plan_field mode)" "skip" "docs are not releasable"
assert_eq "$(plan_field reason)" "skip:no-releasable" "no releasable reason"
run_open 2026-12-15
assert_eq "$(count_log '^pr create')" "1" "docs push does not create a PR"
ok "no releasable change skips"

add_commit "feat: gamma"
# 上一档 PR 已合并，列表里不再是 open。
rm -f "$MOCK/number"
run_open 2026-12-15
assert_eq "$(file_version release/next)" "2026.12.1" "patch after tagged release"
assert_eq "$(count_log '^pr create')" "2" "new cycle creates a PR again"
assert_eq "$(git -C "$REPO" for-each-ref --format='%(refname:short)' 'refs/heads/release/*')" \
    "release/next" "still one release branch"
ok "next cycle after a tagged release"

# --- tag 冲突、无 tag、显式版本、并发 ---------------------------------------
setup_repo
git -C "$REPO" tag v2026.11.4
add_commit "feat: needs a free tag"
run_plan 2026-11-19
assert_eq "$(plan_field mode)" "prepare" "collision still prepares"
assert_eq "$(plan_field version)" "2026.11.5" "skips occupied tags"
ok "existing higher tag"

setup_repo
git -C "$REPO" tag v2026.12.0
add_commit "feat: clock behind"
expect_fail "version would move backwards" run_plan 2026-11-19 >/dev/null
ok "refuses a version behind an existing tag"

setup_repo
git -C "$REPO" tag -d v2026.11.0 >/dev/null
add_commit "feat: no baseline"
run_plan 2026-11-19
assert_eq "$(plan_field mode)" "skip" "no tag"
assert_eq "$(plan_field reason)" "skip:no-tag" "no tag reason"
ok "no ancestor tag skips"

setup_repo
run_open 2026-11-19 workflow_dispatch refs/heads/master 2026.11.3
assert_eq "$(file_version release/next)" "2026.11.3" "explicit version"
assert_eq "$(file_version master)" "2026.11.0" "explicit leaves master"
assert_eq "$(count_log '^pr create')" "1" "explicit opens a PR"
run_plan 2026-11-19 workflow_dispatch refs/heads/master 2026.11.3
assert_eq "$(plan_field reason)" "prepare:explicit" "explicit reason"
ok "dispatch with an explicit version"

setup_repo
expect_fail "explicit tag exists" run_plan 2026-11-19 workflow_dispatch refs/heads/master 2026.11.0 >/dev/null
ok "explicit version rejects an existing tag"

setup_repo
git -C "$REPO" tag -d v2026.11.0 >/dev/null
git -C "$REPO" push -q origin :refs/tags/v2026.11.0
expect_fail "nothing to bump" run_open 2026-11-19 workflow_dispatch refs/heads/master 2026.11.0 >/dev/null
ok "explicit current version does not commit an empty bump"

setup_repo
add_commit "feat: race"
set +e
run_open 2026-11-19 &
p1=$!
run_open 2026-11-19 &
p2=$!
wait "$p1"
c1=$?
wait "$p2"
c2=$?
set -e
assert_eq "$c1" "0" "first concurrent open"
assert_eq "$c2" "0" "second concurrent open"
assert_eq "$(count_log '^pr create')" "1" "concurrent create once"
assert_eq "$(count_log '^pr edit')" "1" "concurrent update once"
assert_eq "$(git -C "$REPO" for-each-ref --format='%(refname:short)' 'refs/heads/release/*')" \
    "release/next" "concurrent single branch"
ok "concurrent prepares share one PR"

setup_repo
git -C "$REPO" checkout -q master
bash -c "cd '$REPO' && bash scripts/version.sh 2026.11.1" >/dev/null
git -C "$REPO" add Cargo.toml README.md .agents/skills/x_likes/SKILL.md
git -C "$REPO" commit -qm "chore(release): 版本统一到 2026.11.1"
add_commit "feat: after untagged release"
run_plan 2026-11-19
assert_eq "$(plan_field version)" "2026.11.2" "continues past untagged release"
notes_file=$(mktemp)
TMPS+=("$notes_file")
(
    cd "$REPO"
    env -u RELEASE_EXPLICIT_VERSION \
        RELEASE_REV=master \
        RELEASE_DATE=2026-11-19 \
        RELEASE_EVENT=push \
        RELEASE_REF=refs/heads/master \
        bash scripts/release_plan.sh --github-output "$PLAN_OUT" --notes "$notes_file" >/dev/null
)
grep -F "还没有对应 tag" "$notes_file" >/dev/null || fail "missing untagged notice"
ok "untagged previous release is not republished"

setup_repo
add_commit "chore(release): 版本统一到 2026.11.1 (#9)"
run_plan 2026-11-19
assert_eq "$(plan_field mode)" "publish" "squash release commit"
assert_eq "$(plan_field version)" "2026.11.1" "squash version"
run_plan 2026-11-19 push refs/tags/v1.2.3
assert_eq "$(plan_field mode)" "publish" "tag recovery"
assert_eq "$(plan_field reason)" "publish:tag" "tag reason"
assert_eq "$(plan_field version)" "1.2.3" "tag version"
expect_fail "bad tag" run_plan 2026-11-19 push refs/tags/v1.0.0.1 >/dev/null
ok "squash release commit and tag recovery"

# 生产 URL 不能在 Actions 之外被这个脚本推送。
setup_repo
add_commit "feat: do not push production"
git -C "$REPO" remote set-url origin "https://example.invalid/HerbertGao/x_likes_downloader.git"
expect_fail "production guard" run_open 2026-11-19 >/dev/null
ok "refuses the production remote outside Actions"

# workflow 接线
wf="$ROOT/.github/workflows/release.yml"
grep -F "required: false" "$wf" >/dev/null || fail "version input still required"
grep -F "needs.plan.outputs.mode == 'prepare'" "$wf" >/dev/null || fail "prepare gate"
grep -F "needs.plan.outputs.mode == 'publish'" "$wf" >/dev/null || fail "publish gate"
grep -F "pull-requests: write" "$wf" >/dev/null || fail "missing pull-requests permission"
grep -F "contents: write" "$wf" >/dev/null || fail "missing contents write"
grep -F "release-prepare-" "$wf" >/dev/null || fail "missing prepare concurrency group"
grep -F "release-publish-tag-" "$wf" >/dev/null || fail "missing tag concurrency group"
grep -F "release-publish-" "$wf" >/dev/null || fail "missing publish concurrency group"
grep -F "chore(release): 版本统一到" "$wf" >/dev/null || fail "publish group missing release phrase"
grep -F "cancel-in-progress:" "$wf" >/dev/null || fail "missing cancel expression"
grep -F "bash scripts/release_open_pr.sh" "$wf" >/dev/null || fail "prepare does not call opener"
grep -F "bash scripts/release_plan.sh" "$wf" >/dev/null || fail "plan does not call planner"
if grep -F "should_release" "$wf" >/dev/null; then
    fail "old should_release output remains"
fi
prepare_if=$(
    awk '
        $0 ~ /^  prepare-release:/ { p = 1 }
        p && $0 ~ /^    if:/ { print; exit }
    ' "$wf"
)
printf '%s\n' "$prepare_if" | grep -F "mode == 'prepare'" >/dev/null ||
    fail "prepare job is not gated on mode: $prepare_if"
ok "workflow wiring"

# release.sh 只打印，不调用 gh。
FAIL_BIN=$(mktemp -d)
TMPS+=("$FAIL_BIN")
cat >"$FAIL_BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo "gh should not be called" >&2
exit 99
EOF
chmod +x "$FAIL_BIN/gh"
dry_auto=$(PATH="$FAIL_BIN:/usr/bin:/bin" bash "$ROOT/scripts/release.sh" --dry-run)
printf '%s\n' "$dry_auto" | grep -F "不传 version" >/dev/null || fail "auto dry-run: $dry_auto"
dry_ver=$(PATH="$FAIL_BIN:/usr/bin:/bin" bash "$ROOT/scripts/release.sh" 2026.12.0 --dry-run)
printf '%s\n' "$dry_ver" | grep -F "version=2026.12.0" >/dev/null || fail "explicit dry-run: $dry_ver"
dry_patch=$(PATH="$FAIL_BIN:/usr/bin:/bin" bash "$ROOT/scripts/release.sh" patch --dry-run)
printf '%s\n' "$dry_patch" | grep -F "2026.11.0 -> 2026.11.1" >/dev/null ||
    fail "patch dry-run: $dry_patch"
ok "release.sh dry-run does not dispatch"

printf 'release_plan_test: OK (%s cases)\n' "$CASES"
