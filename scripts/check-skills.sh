#!/usr/bin/env bash
# Validate the x_likes skill artifact under .agents/skills/x_likes/.
#
# The skill is distributed as a plain Agent Skill (installable via `npx skills add`);
# there are no per-host adapter copies to cross-check any more. What still matters:
#   1. The three skill files exist (SKILL.md / README.md / defaults.json).
#   2. defaults.json keeps its closed key set, a positive schema_version, and no user secrets.
#      (`src/config.rs` include_str!s this file, so it ships inside the binary.)
#   3. SKILL.md frontmatter carries name / description / min_binary_version (SemVer).
#   4. min_binary_version matches Cargo.toml.
#   5. No credential VALUES leak into any shipped text or JSON artifact.
#
# Tools used: bash + jq. Read-only; safe to run in CI on any runner.
set -euo pipefail

cd "$(dirname "$0")/.."

RED=$'\033[0;31m'
GREEN=$'\033[0;32m'
NC=$'\033[0m'
errs=0
err() {
  echo -e "${RED}✗${NC} $*" >&2
  errs=$((errs + 1))
}
ok() { echo -e "${GREEN}✓${NC} $*"; }

SKILL_DIR=".agents/skills/x_likes"
SKILL_FILE="$SKILL_DIR/SKILL.md"
DEFAULTS_FILE="$SKILL_DIR/defaults.json"

# X account credentials / sensitive request fingerprint. `bearer_token` is deliberately NOT in
# this set: it is the public anonymous X Web bearer, and defaults.json legitimately carries it.
FORBIDDEN_USER_KEYS='["auth_token","ct0","csrf","cookies","personalization_id","user_id","user_agent"]'

# ---- helpers ------------------------------------------------------------------------------

# Recursively flag any JSON object key whose name is in the forbidden set, at any depth.
scan_forbidden_keys_json() {
  local file="$1" forbidden="$2" hits
  hits=$(jq --argjson f "$forbidden" '
    [paths | . as $p | $p[] | select(type == "string") | select(IN($f[]))
     | ([$p[] | tostring] | join("."))] | unique
  ' "$file")
  if [[ "$hits" != "[]" ]]; then
    err "$file contains forbidden key path(s): $hits"
  fi
}

# Flag credential VALUES (not bare field-name mentions, which are documentation).
scan_forbidden_text() {
  local file="$1" key
  for key in auth_token ct0 csrf cookies personalization_id user_id user_agent bearer_token; do
    grep -qE "\"${key}\"[[:space:]]*:[[:space:]]*\"[^\"]+\"" "$file" &&
      err "$file contains JSON literal '$key': \"...\" (value leak)"
    grep -qE "^[[:space:]]*${key}[[:space:]]*:[[:space:]]*['\"]?[^'\"[:space:]]+['\"]?[[:space:]]*$" "$file" &&
      err "$file contains YAML '$key:' with concrete value (value leak)"
    grep -qE "${key}=[A-Za-z0-9]" "$file" &&
      err "$file contains '$key=<concrete-value>' assignment (value leak)"
  done
  return 0
}

extract_frontmatter_field() {
  awk -v field="$2" '
    BEGIN { fm=0; depth=0 }
    /^---$/ { depth++; if (depth==1) { fm=1; next } if (depth==2) { fm=0; exit } next }
    fm && $0 ~ "^"field":" { sub("^"field":[[:space:]]*", "", $0); sub(/[[:space:]]*$/, "", $0); print $0; exit }
  ' "$1"
}

# ---- 1. required artifacts ----------------------------------------------------------------

echo "=== Required artifacts ==="
missing=0
for f in "$SKILL_FILE" "$SKILL_DIR/README.md" "$DEFAULTS_FILE"; do
  [[ -f "$f" ]] || {
    err "required artifact missing: $f"
    missing=$((missing + 1))
  }
done
[[ $missing -eq 0 ]] && ok "all 3 skill artifacts present"

# ---- 2. defaults.json ---------------------------------------------------------------------

echo ""
echo "=== defaults.json ==="
if [[ -f "$DEFAULTS_FILE" ]]; then
  if ! jq empty "$DEFAULTS_FILE" >/dev/null 2>&1; then
    err "$DEFAULTS_FILE is not valid JSON"
  else
    allowed='["schema_version","likes_api_url","likes_features","likes_fieldtoggles","tweet_detail_api_url","tweet_features","tweet_fieldtoggles","bearer_token"]'
    unknown=$(jq --argjson a "$allowed" '[keys[] as $k | select(($a | index($k)) | not)]' "$DEFAULTS_FILE")
    [[ "$unknown" != "[]" ]] && err "$DEFAULTS_FILE contains undeclared keys: $unknown"

    sv=$(jq -r '.schema_version // empty' "$DEFAULTS_FILE")
    if [[ -z "$sv" ]] || ! [[ "$sv" =~ ^[1-9][0-9]*$ ]]; then
      err "$DEFAULTS_FILE schema_version must be a positive integer; got '${sv:-<missing>}'"
    else
      ok "$DEFAULTS_FILE (schema_version=$sv)"
    fi

    # src/config.rs embeds this file; these keys are the >hardcoded-fallback protocol truth.
    for key in tweet_detail_api_url tweet_features tweet_fieldtoggles; do
      val=$(jq -r --arg k "$key" '.[$k] // empty' "$DEFAULTS_FILE")
      [[ -n "$val" ]] || err "$DEFAULTS_FILE field '$key' must be a non-empty string"
    done
    scan_forbidden_keys_json "$DEFAULTS_FILE" "$FORBIDDEN_USER_KEYS"
  fi
fi

# ---- 3. SKILL.md frontmatter --------------------------------------------------------------

echo ""
echo "=== SKILL.md frontmatter ==="
minver=""
if [[ -f "$SKILL_FILE" ]]; then
  name=$(extract_frontmatter_field "$SKILL_FILE" name)
  desc=$(extract_frontmatter_field "$SKILL_FILE" description)
  minver=$(extract_frontmatter_field "$SKILL_FILE" min_binary_version)

  [[ "$name" == "x-likes" ]] || err "$SKILL_FILE frontmatter 'name' must be 'x-likes'; got '${name:-<missing>}'"
  [[ -n "$desc" ]] || err "$SKILL_FILE frontmatter missing 'description'"
  if [[ -z "$minver" ]]; then
    err "$SKILL_FILE frontmatter missing 'min_binary_version'"
  elif ! [[ "$minver" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
    err "$SKILL_FILE min_binary_version must be SemVer; got '$minver'"
  fi
  [[ -n "$desc" && -n "$minver" ]] && ok "$SKILL_FILE frontmatter (name=$name, min_binary_version=$minver)"
fi

# ---- 4. version consistency ---------------------------------------------------------------

echo ""
echo "=== Version consistency ==="
cargo_ver=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
if [[ -z "$cargo_ver" ]]; then
  err "Cargo.toml version not found"
else
  echo "expected: $cargo_ver"
  if [[ -n "$minver" && "$minver" != "$cargo_ver" ]]; then
    err "$SKILL_FILE min_binary_version='$minver' != Cargo.toml='$cargo_ver'"
  elif [[ -n "$minver" ]]; then
    ok "all version fields consistent ($cargo_ver)"
  fi
fi

# ---- 5. credential-value leak scan --------------------------------------------------------

echo ""
echo "=== Credential leak scan (.agents/skills/) ==="
n=0
while IFS= read -r f; do
  scan_forbidden_text "$f"
  n=$((n + 1))
done < <(find .agents/skills -type f \( -name '*.md' -o -name '*.json' -o -name '*.yaml' -o -name '*.yml' \) | sort)
ok "scanned $n shipped skill artifact(s)"

echo ""
if [[ $errs -gt 0 ]]; then
  echo -e "${RED}check-skills failed: $errs error(s)${NC}" >&2
  exit 1
fi
echo -e "${GREEN}check-skills: all checks passed${NC}"
