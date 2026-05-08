#!/usr/bin/env bash
# Sync SKILL.md (and other SOT artifacts) from packaging/skill/x_likes/ to host adapter dirs.
#
# Source of truth (SOT):
#   packaging/skill/x_likes/SKILL.md
#   packaging/skill/x_likes/defaults.json
#
# Generated copies:
#   packaging/claude-code/skills/x_likes/SKILL.md   (direct copy, no host extension)
#   packaging/codex/skills/x_likes/SKILL.md         (direct copy, no host extension)
#   packaging/openclaw/x_likes/SKILL.md             (copy + inject metadata.openclaw block)
#   packaging/openclaw/x_likes/defaults.json        (direct copy from SOT)
#
# Tools used: bash + awk + sed + jq. NO yq dependency (GHA runners do not preinstall it,
# and multiple yq implementations behave differently).
#
# Idempotent: running twice produces zero git diff.

set -euo pipefail

cd "$(dirname "$0")/.."

SOT_SKILL="packaging/skill/x_likes/SKILL.md"
SOT_DEFAULTS="packaging/skill/x_likes/defaults.json"

CC_SKILL="packaging/claude-code/skills/x_likes/SKILL.md"
CODEX_SKILL="packaging/codex/skills/x_likes/SKILL.md"
HERMES_SKILL="packaging/hermes/skills/x_likes/SKILL.md"
OC_SKILL="packaging/openclaw/x_likes/SKILL.md"
OC_DEFAULTS="packaging/openclaw/x_likes/defaults.json"

if [[ ! -f "$SOT_SKILL" ]]; then
  echo "::error::SOT not found: $SOT_SKILL" >&2
  exit 1
fi
if [[ ! -f "$SOT_DEFAULTS" ]]; then
  echo "::error::SOT not found: $SOT_DEFAULTS" >&2
  exit 1
fi

# Read min_binary_version from SOT frontmatter (line-based; not full YAML parser).
# Looks for `min_binary_version: <semver>` between first two `---` markers.
extract_min_binary_version() {
  awk '
    BEGIN { fm=0; depth=0 }
    /^---$/ {
      depth++
      if (depth == 1) { fm=1; next }
      if (depth == 2) { fm=0; exit }
      next
    }
    fm && /^min_binary_version:/ {
      sub(/^min_binary_version:[[:space:]]*/, "", $0)
      sub(/[[:space:]]*$/, "", $0)
      print $0
      exit
    }
  ' "$SOT_SKILL"
}

MIN_VER="$(extract_min_binary_version)"
if [[ -z "$MIN_VER" ]]; then
  echo "::error::min_binary_version not found in $SOT_SKILL frontmatter" >&2
  exit 1
fi

mkdir -p "$(dirname "$CC_SKILL")" "$(dirname "$CODEX_SKILL")" "$(dirname "$HERMES_SKILL")" "$(dirname "$OC_SKILL")"

# --- Claude Code: direct copy ---
cp "$SOT_SKILL" "$CC_SKILL"

# --- Codex: direct copy (no host extension; SKILL.md already host-agnostic) ---
cp "$SOT_SKILL" "$CODEX_SKILL"

# --- Hermes: direct copy (Hermes 0.12+ consumes Anthropic-style frontmatter natively;
# install via `hermes skills install <raw-URL>` lands here in ~/.hermes/skills/x_likes/) ---
cp "$SOT_SKILL" "$HERMES_SKILL"

# --- OpenClaw: inject `metadata.openclaw` block at end of frontmatter ---
# Inject these lines (idempotent: replace block if it already exists):
#   metadata:
#     openclaw:
#       bins:
#         - x_likes_downloader
#       min_version: <MIN_VER>
#
# Strategy: awk strips any existing metadata-rooted block inside frontmatter,
# then re-inserts canonical block before closing `---`.
awk -v min="$MIN_VER" '
  BEGIN { fm=0; depth=0; in_metadata=0; injected=0 }
  /^---$/ {
    depth++
    if (depth == 1) { print; fm=1; next }
    if (depth == 2) {
      # End of frontmatter — inject metadata block before closing ---.
      print "metadata:"
      print "  openclaw:"
      print "    bins:"
      print "      - x_likes_downloader"
      print "    min_version: " min
      print
      fm=0
      injected=1
      next
    }
    print
    next
  }
  fm && /^metadata:[[:space:]]*$/ { in_metadata=1; next }
  fm && in_metadata {
    # Drop indented continuation lines (start with space/tab) that belong to metadata block.
    if ($0 ~ /^[[:space:]]/) { next }
    in_metadata=0
    # Fall through to print non-indented line
  }
  { print }
' "$SOT_SKILL" > "$OC_SKILL.tmp"
mv "$OC_SKILL.tmp" "$OC_SKILL"

# --- OpenClaw defaults.json: derived from SOT, with bearer_token stripped ---
# The optional `bearer_token` field is allowed only in SOT (it is the X Web public anonymous
# bearer, not a user credential). Any host adapter publishable surface must NOT carry it —
# `scripts/check-packaging.sh` enforces this with FORBIDDEN_NONSOT_KEYS. We strip it here so
# adding `bearer_token` to SOT later does not propagate into shipped artifacts.
jq 'del(.bearer_token)' "$SOT_DEFAULTS" > "$OC_DEFAULTS"

echo "synced SOT → host adapter copies (min_binary_version=$MIN_VER):"
echo "  $CC_SKILL"
echo "  $CODEX_SKILL"
echo "  $HERMES_SKILL"
echo "  $OC_SKILL (with metadata.openclaw block)"
echo "  $OC_DEFAULTS"
