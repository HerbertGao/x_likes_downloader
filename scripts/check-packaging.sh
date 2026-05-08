#!/usr/bin/env bash
# Validate packaging/ artifacts (replaces old scripts/check-skill-defaults.sh).
#
# Checks:
#   1. SOT defaults.json: closed key set, contains schema_version (≥1), no user-private secrets.
#   2. SOT SKILL.md frontmatter: contains name, description, min_binary_version (semver).
#   3. Each host adapter manifest (plugin.json / mcp-config.json / .mcp.json / marketplace.json):
#      - Valid JSON
#      - command="x_likes_downloader", args contains "serve" and "--mcp", transport="stdio"
#      - No user-private secrets, no bearer_token (only allowed in SOT defaults.json)
#   4. SKILL.md derived copies match SOT (run sync-skill.sh; expect no git diff).
#   5. Version field consistency: Cargo.toml ↔ marketplace.json ↔ plugin.json ↔
#      mcp-config.json.minimum_xld_version ↔ SOT SKILL.md.min_binary_version.

set -euo pipefail

cd "$(dirname "$0")/.."

GREEN='\033[0;32m'
RED='\033[0;31m'
YELLOW='\033[1;33m'
NC='\033[0m'

errs=0

err() {
  echo -e "${RED}::error::${NC} $*" >&2
  errs=$((errs + 1))
}

ok() {
  echo -e "${GREEN}✓${NC} $*"
}

warn() {
  echo -e "${YELLOW}!${NC} $*"
}

# Forbidden user-private keys; checked recursively in JSON files.
# Does NOT include bearer_token — that key is the X Web public anonymous bearer and is allowed
# in SOT `packaging/skill/x_likes/defaults.json` only. Outside SOT, use FORBIDDEN_NONSOT_KEYS.
FORBIDDEN_USER_KEYS='["auth_token","ct0","csrf","cookies","personalization_id","user_id","user_agent"]'

# Forbidden keys for non-SOT JSON artifacts (manifests, marketplace, MCP configs, OpenClaw
# defaults). Adds bearer_token to FORBIDDEN_USER_KEYS — bearer_token must NEVER appear here.
FORBIDDEN_NONSOT_KEYS='["auth_token","ct0","csrf","cookies","personalization_id","user_id","user_agent","bearer_token"]'

# Recursively scan a JSON file for any forbidden KEY (regardless of value shape:
# string, number, array, object, null — any of them counts as a hit if the KEY name
# matches the forbidden set). This must walk every object key at every nesting depth,
# not only the last segment of `paths(scalars)` (which would miss
# `{"cookies":[...]}` or `{"ct0":{"value":"..."}}` because their forbidden key is
# above a non-scalar value).
# $1 file path, $2 JSON array of forbidden keys
scan_forbidden_keys_json() {
  local file="$1"
  local forbidden="$2"
  # Collect ALL object-key paths (each path is a list of segments, only string segments
  # — array indices are integers and are excluded by `select(type == "string")`).
  # A hit = any path segment whose name is in the forbidden set.
  local hits
  hits=$(jq --argjson f "$forbidden" '
    [paths
      | . as $p
      | $p[]
      | select(type == "string")
      | select(IN($f[]))
      | ([$p[] | tostring] | join("."))
    ] | unique
  ' "$file")
  if [[ "$hits" != "[]" ]]; then
    err "$file contains forbidden key path(s): $hits"
  fi
}

# Scan a Markdown / YAML / text file for forbidden credential VALUES.
# Naked mentions of field names (e.g., "凭据 auth_token 不出 MCP 通道") are documentation
# describing the security model and must remain allowed. We only flag patterns that look like
# actual key/value assignments — JSON ("auth_token": "..."), YAML (auth_token: 'something'),
# or shell env (auth_token=value) — which would indicate a real leaked credential.
#
# Forbidden key set covers everything that could leak a real X account credential or sensitive
# request fingerprint: auth_token, ct0, csrf, cookies, personalization_id, user_id, user_agent,
# and bearer_token (bearer_token is allowed as a JSON KEY only in SOT defaults — never as a
# concrete value assignment in shipped Markdown / YAML).
scan_forbidden_text() {
  local file="$1"
  local key
  for key in auth_token ct0 csrf cookies personalization_id user_id user_agent bearer_token; do
    # JSON: "key": "value"  (any non-empty quoted value)
    if grep -qE "\"${key}\"[[:space:]]*:[[:space:]]*\"[^\"]+\"" "$file"; then
      err "$file contains JSON literal '$key': \"...\" (value leak)"
    fi
    # YAML: key: 'value' or key: "value" or key: rawvalue (non-empty)
    if grep -qE "^[[:space:]]*${key}[[:space:]]*:[[:space:]]*['\"]?[^'\"[:space:]]+['\"]?[[:space:]]*$" "$file"; then
      err "$file contains YAML '$key:' with concrete value (value leak)"
    fi
    # Shell/env or in-string assignment: key=<concrete value start>.
    # Matches `cookies=auth_token=ABC` (line with bare assignment), `cookies="auth_token=ABC"`
    # (assignment inside a quoted string), `... auth_token=ABC123 ...` etc. Requires the
    # character right after `=` to be alphanumeric so docs that describe placeholders like
    # `auth_token=<your-value>` or `auth_token=$VAR` won't false-positive.
    if grep -qE "${key}=[A-Za-z0-9]" "$file"; then
      err "$file contains '$key=<concrete-value>' assignment (value leak)"
    fi
  done
}

# Validate SOT defaults.json
validate_sot_defaults() {
  local file="packaging/skill/x_likes/defaults.json"
  if [[ ! -f "$file" ]]; then
    err "$file not found"
    return
  fi
  if ! jq empty "$file" >/dev/null 2>&1; then
    err "$file is not valid JSON"
    return
  fi
  local allowed='["schema_version","likes_api_url","likes_features","likes_fieldtoggles","bearer_token"]'
  local unknown
  unknown=$(jq --argjson a "$allowed" '[keys[] as $k | select(($a | index($k)) | not)]' "$file")
  if [[ "$unknown" != "[]" ]]; then
    err "$file contains undeclared keys: $unknown"
  fi
  scan_forbidden_keys_json "$file" "$FORBIDDEN_USER_KEYS"
  local sv
  sv=$(jq -r '.schema_version // empty' "$file")
  if [[ -z "$sv" ]] || ! [[ "$sv" =~ ^[1-9][0-9]*$ ]]; then
    err "$file schema_version must be a positive integer; got '${sv:-<missing>}'"
  else
    ok "$file (schema_version=$sv)"
  fi
}

# Extract a field from SOT SKILL.md frontmatter (line-based).
extract_frontmatter_field() {
  local file="$1"
  local field="$2"
  awk -v field="$field" '
    BEGIN { fm=0; depth=0 }
    /^---$/ {
      depth++
      if (depth == 1) { fm=1; next }
      if (depth == 2) { fm=0; exit }
      next
    }
    fm {
      if ($0 ~ "^"field":") {
        sub("^"field":[[:space:]]*", "", $0)
        sub(/[[:space:]]*$/, "", $0)
        print $0
        exit
      }
    }
  ' "$file"
}

# Validate SOT SKILL.md frontmatter
validate_sot_skill_frontmatter() {
  local file="packaging/skill/x_likes/SKILL.md"
  if [[ ! -f "$file" ]]; then
    err "$file not found"
    return
  fi
  local name desc minver
  name=$(extract_frontmatter_field "$file" "name")
  desc=$(extract_frontmatter_field "$file" "description")
  minver=$(extract_frontmatter_field "$file" "min_binary_version")
  if [[ -z "$name" ]]; then err "$file frontmatter missing 'name'"; fi
  if [[ -z "$desc" ]]; then err "$file frontmatter missing 'description'"; fi
  if [[ -z "$minver" ]]; then
    err "$file frontmatter missing 'min_binary_version'"
  elif ! [[ "$minver" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
    err "$file min_binary_version must be SemVer; got '$minver'"
  fi
  scan_forbidden_text "$file"
  ok "$file frontmatter (name=$name, min_binary_version=$minver)"
}

# Validate an MCP server config (mcp-config.json or .mcp.json).
# Schema is the proposal-defined CLOSED set:
#   { command: "x_likes_downloader", args: ["serve","--mcp"], transport: "stdio",
#     minimum_xld_version?: "<semver>" }
# Closed = no extra root keys beyond the four above. The wrapped form
# `{ mcpServers: { ... } }` is REJECTED (intentionally divergent from a project-root
# .mcp.json — a host plugin manifest's `mcpServers` field is expected to point at this
# file as the server config itself, not as a wrapper).
# $1 path, $2 require_min_xld_version (true|false)
validate_mcp_config() {
  local file="$1"
  local require_min="$2"
  if [[ ! -f "$file" ]]; then
    err "$file not found"
    return
  fi
  if ! jq empty "$file" >/dev/null 2>&1; then
    err "$file is not valid JSON"
    return
  fi
  # Use the non-SOT forbidden key set: bearer_token MUST NOT appear in any MCP server config.
  scan_forbidden_keys_json "$file" "$FORBIDDEN_NONSOT_KEYS"

  local allowed='["command","args","transport","minimum_xld_version"]'
  local unknown
  unknown=$(jq --argjson a "$allowed" '[keys[] as $k | select(($a | index($k)) | not)]' "$file")
  if [[ "$unknown" != "[]" ]]; then
    err "$file contains undeclared root keys: $unknown (closed set: $allowed)"
  fi
  if jq -e 'has("mcpServers")' "$file" >/dev/null 2>&1; then
    err "$file uses wrapped { mcpServers: ... } schema; proposal requires root command/args/transport"
  fi

  local cmd transport
  cmd=$(jq -r '.command // empty' "$file")
  transport=$(jq -r '.transport // empty' "$file")

  if [[ "$cmd" != "x_likes_downloader" ]]; then
    err "$file command must be 'x_likes_downloader'; got '$cmd'"
  fi
  if [[ "$transport" != "stdio" ]]; then
    err "$file transport must be 'stdio'; got '$transport'"
  fi
  # Strict equality: args must be exactly the array ["serve", "--mcp"].
  # jq's `index()` is polymorphic over strings/arrays, so containment checks would let a
  # string like "serve --mcp" or a reordered/extended array slip through. Use `==` instead.
  if ! jq -e '.args == ["serve", "--mcp"]' "$file" >/dev/null 2>&1; then
    local args_actual
    args_actual=$(jq -c '.args' "$file")
    err "$file args must be exactly [\"serve\", \"--mcp\"]; got $args_actual"
  fi

  if [[ "$require_min" == "true" ]]; then
    local min_ver
    min_ver=$(jq -r '.minimum_xld_version // empty' "$file")
    if [[ -z "$min_ver" ]] || ! [[ "$min_ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
      err "$file minimum_xld_version must be SemVer; got '${min_ver:-<missing>}'"
    fi
  fi

  ok "$file MCP schema valid (command=$cmd, transport=$transport)"
}

# Validate Claude Code plugin.json
validate_cc_plugin_json() {
  local file="packaging/claude-code/.claude-plugin/plugin.json"
  if [[ ! -f "$file" ]]; then
    err "$file not found"
    return
  fi
  if ! jq empty "$file" >/dev/null 2>&1; then
    err "$file is not valid JSON"
    return
  fi
  scan_forbidden_keys_json "$file" "$FORBIDDEN_NONSOT_KEYS"
  local field
  for field in name version description author; do
    if [[ "$(jq -r --arg f "$field" '.[$f] // empty' "$file")" == "" ]]; then
      err "$file missing required field: $field"
    fi
  done
  local name
  name=$(jq -r '.name // empty' "$file")
  if [[ "$name" != "x_likes" ]]; then
    err "$file name must be 'x_likes'; got '$name'"
  fi
  # mcpServers must be a string equal to './.mcp.json' and the target must exist.
  # Missing / non-string / wrong path are all rejected — otherwise a regression could ship
  # a Claude plugin without any MCP server wired up.
  local mcp_ref
  mcp_ref=$(jq -r 'if (.mcpServers | type) == "string" then .mcpServers else empty end' "$file")
  if [[ -z "$mcp_ref" ]]; then
    err "$file mcpServers must be a string path './.mcp.json' (got non-string or missing)"
  elif [[ "$mcp_ref" != "./.mcp.json" ]]; then
    err "$file mcpServers must be './.mcp.json'; got '$mcp_ref'"
  elif [[ ! -f "packaging/claude-code/.mcp.json" ]]; then
    err "$file mcpServers references missing file: packaging/claude-code/.mcp.json"
  fi
  ok "$file plugin manifest valid"
}

# Validate Codex CLI plugin.json (with rich interface block)
validate_codex_plugin_json() {
  local file="packaging/codex/.codex-plugin/plugin.json"
  if [[ ! -f "$file" ]]; then
    err "$file not found"
    return
  fi
  if ! jq empty "$file" >/dev/null 2>&1; then
    err "$file is not valid JSON"
    return
  fi
  scan_forbidden_keys_json "$file" "$FORBIDDEN_NONSOT_KEYS"
  local field
  for field in name version description author; do
    if [[ "$(jq -r --arg f "$field" '.[$f] // empty' "$file")" == "" ]]; then
      err "$file missing required field: $field"
    fi
  done
  local name
  name=$(jq -r '.name // empty' "$file")
  if [[ "$name" != "x_likes" ]]; then
    err "$file name must be 'x_likes'; got '$name'"
  fi
  # interface block required fields
  for field in displayName shortDescription longDescription category capabilities defaultPrompt; do
    if [[ "$(jq -r --arg f "$field" '.interface[$f] // empty' "$file")" == "" ]]; then
      err "$file missing interface.$field"
    fi
  done
  # defaultPrompt: array length ≤ 3, each ≤ 128 chars
  local dp_len
  dp_len=$(jq '.interface.defaultPrompt | length' "$file")
  if [[ "$dp_len" -gt 3 ]]; then
    err "$file interface.defaultPrompt must have ≤ 3 items; got $dp_len"
  fi
  local too_long
  too_long=$(jq '[.interface.defaultPrompt[] | select(length > 128)] | length' "$file")
  if [[ "$too_long" != "0" ]]; then
    err "$file interface.defaultPrompt items must be ≤ 128 chars each; $too_long over limit"
  fi
  # skills field must equal "./skills/" and the directory must contain x_likes/SKILL.md
  local skills_ref
  skills_ref=$(jq -r '.skills // empty' "$file")
  if [[ "$skills_ref" != "./skills/" ]]; then
    err "$file skills must be './skills/'; got '$skills_ref'"
  fi
  if [[ ! -f "packaging/codex/skills/x_likes/SKILL.md" ]]; then
    err "$file skills references missing file: packaging/codex/skills/x_likes/SKILL.md"
  fi
  # mcpServers field must equal "./.mcp.json" and the file must exist
  local codex_mcp_ref
  codex_mcp_ref=$(jq -r 'if (.mcpServers | type) == "string" then .mcpServers else empty end' "$file")
  if [[ -z "$codex_mcp_ref" ]]; then
    err "$file mcpServers must be a string path (got non-string or missing)"
  elif [[ "$codex_mcp_ref" != "./.mcp.json" ]]; then
    err "$file mcpServers must be './.mcp.json'; got '$codex_mcp_ref'"
  elif [[ ! -f "packaging/codex/.mcp.json" ]]; then
    err "$file mcpServers references missing file: packaging/codex/.mcp.json"
  fi
  ok "$file Codex plugin manifest valid"
}

# Validate Claude Code marketplace.json
validate_cc_marketplace() {
  local file=".claude-plugin/marketplace.json"
  if [[ ! -f "$file" ]]; then
    err "$file not found"
    return
  fi
  if ! jq empty "$file" >/dev/null 2>&1; then
    err "$file is not valid JSON"
    return
  fi
  scan_forbidden_keys_json "$file" "$FORBIDDEN_NONSOT_KEYS"
  for path in '.name' '.owner.name' '.metadata.description' '.metadata.version' '.plugins[0].name' '.plugins[0].source'; do
    if [[ "$(jq -r "$path // empty" "$file")" == "" ]]; then
      err "$file missing required field: $path"
    fi
  done
  # plugins[0].source must equal "./packaging/claude-code" and that directory must exist.
  local cc_source
  cc_source=$(jq -r '.plugins[0].source // empty' "$file")
  if [[ "$cc_source" != "./packaging/claude-code" ]]; then
    err "$file plugins[0].source must be './packaging/claude-code'; got '$cc_source'"
  elif [[ ! -d "packaging/claude-code" ]]; then
    err "$file plugins[0].source references missing directory: packaging/claude-code"
  elif [[ ! -f "packaging/claude-code/.claude-plugin/plugin.json" ]]; then
    err "$file plugins[0].source has no .claude-plugin/plugin.json under packaging/claude-code"
  fi
  ok "$file Claude Code marketplace valid"
}

# Validate Codex marketplace.json
validate_codex_marketplace() {
  local file=".agents/plugins/marketplace.json"
  if [[ ! -f "$file" ]]; then
    err "$file not found"
    return
  fi
  if ! jq empty "$file" >/dev/null 2>&1; then
    err "$file is not valid JSON"
    return
  fi
  scan_forbidden_keys_json "$file" "$FORBIDDEN_NONSOT_KEYS"
  for path in '.name' '.interface.displayName' '.plugins[0].name' '.plugins[0].source.source' '.plugins[0].source.path' '.plugins[0].policy.installation' '.plugins[0].policy.authentication' '.plugins[0].category'; do
    if [[ "$(jq -r "$path // empty" "$file")" == "" ]]; then
      err "$file missing required field: $path"
    fi
  done
  local install_policy
  install_policy=$(jq -r '.plugins[0].policy.installation' "$file")
  if [[ "$install_policy" != "AVAILABLE" ]]; then
    err "$file plugins[0].policy.installation must be AVAILABLE; got '$install_policy'"
  fi
  local auth_policy
  auth_policy=$(jq -r '.plugins[0].policy.authentication' "$file")
  if [[ "$auth_policy" != "ON_USE" ]]; then
    err "$file plugins[0].policy.authentication must be ON_USE; got '$auth_policy'"
  fi
  # plugins[0].source.{source,path} must be {"local","./packaging/codex"} and the dir must exist.
  local codex_src codex_path
  codex_src=$(jq -r '.plugins[0].source.source // empty' "$file")
  codex_path=$(jq -r '.plugins[0].source.path // empty' "$file")
  if [[ "$codex_src" != "local" ]]; then
    err "$file plugins[0].source.source must be 'local'; got '$codex_src'"
  fi
  if [[ "$codex_path" != "./packaging/codex" ]]; then
    err "$file plugins[0].source.path must be './packaging/codex'; got '$codex_path'"
  elif [[ ! -d "packaging/codex" ]]; then
    err "$file plugins[0].source.path references missing directory: packaging/codex"
  elif [[ ! -f "packaging/codex/.codex-plugin/plugin.json" ]]; then
    err "$file plugins[0].source.path has no .codex-plugin/plugin.json under packaging/codex"
  fi
  ok "$file Codex marketplace valid"
}

# Validate version-field consistency across all artifacts.
validate_version_consistency() {
  local cargo_ver
  cargo_ver=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
  if [[ -z "$cargo_ver" ]]; then
    err "Cargo.toml version not found"
    return
  fi
  echo ""
  echo "Version consistency check (expected: $cargo_ver)"

  local bad=0
  local f v

  f=".claude-plugin/marketplace.json"
  if [[ -f "$f" ]]; then
    v=$(jq -r '.metadata.version // empty' "$f")
    if [[ "$v" != "$cargo_ver" ]]; then
      err "$f metadata.version='$v' != Cargo.toml='$cargo_ver'"
      bad=1
    fi
  fi

  f=".agents/plugins/marketplace.json"
  if [[ -f "$f" ]]; then
    # Codex marketplace top-level version is sync'd by version.sh; require consistency.
    v=$(jq -r '.version // empty' "$f")
    if [[ -z "$v" ]]; then
      err "$f missing top-level version field (sync'd by scripts/version.sh)"
      bad=1
    elif [[ "$v" != "$cargo_ver" ]]; then
      err "$f version='$v' != Cargo.toml='$cargo_ver'"
      bad=1
    fi
  fi

  f="packaging/claude-code/.claude-plugin/plugin.json"
  if [[ -f "$f" ]]; then
    v=$(jq -r '.version // empty' "$f")
    if [[ "$v" != "$cargo_ver" ]]; then
      err "$f version='$v' != Cargo.toml='$cargo_ver'"
      bad=1
    fi
  fi

  f="packaging/codex/.codex-plugin/plugin.json"
  if [[ -f "$f" ]]; then
    v=$(jq -r '.version // empty' "$f")
    if [[ "$v" != "$cargo_ver" ]]; then
      err "$f version='$v' != Cargo.toml='$cargo_ver'"
      bad=1
    fi
  fi

  f="packaging/openclaw/x_likes/mcp-config.json"
  if [[ -f "$f" ]]; then
    v=$(jq -r '.minimum_xld_version // empty' "$f")
    if [[ "$v" != "$cargo_ver" ]]; then
      err "$f minimum_xld_version='$v' != Cargo.toml='$cargo_ver'"
      bad=1
    fi
  fi

  f="packaging/skill/x_likes/SKILL.md"
  if [[ -f "$f" ]]; then
    v=$(extract_frontmatter_field "$f" "min_binary_version")
    if [[ "$v" != "$cargo_ver" ]]; then
      err "$f min_binary_version='$v' != Cargo.toml='$cargo_ver'"
      bad=1
    fi
  fi

  if [[ $bad -eq 0 ]]; then
    ok "all version fields consistent ($cargo_ver)"
  fi
}

# --- Main ---

# Step 0: enforce that EVERY required artifact exists. Validators below are gated by `-f`,
# so a missing required file would otherwise silently skip validation. Each entry here is a
# spec-mandated artifact for `add-multi-host-packaging`. Optional / future-host files
# (hermes/cursor placeholders) are excluded from this hard list.
REQUIRED_ARTIFACTS=(
  # SOT
  "packaging/skill/x_likes/SKILL.md"
  "packaging/skill/x_likes/defaults.json"
  "packaging/skill/x_likes/README.md"
  # OpenClaw
  "packaging/openclaw/x_likes/SKILL.md"
  "packaging/openclaw/x_likes/mcp-config.json"
  "packaging/openclaw/x_likes/defaults.json"
  "packaging/openclaw/x_likes/README.md"
  # Claude Code plugin
  "packaging/claude-code/.claude-plugin/plugin.json"
  "packaging/claude-code/.mcp.json"
  "packaging/claude-code/commands/auth.md"
  "packaging/claude-code/commands/list.md"
  "packaging/claude-code/commands/setup.md"
  "packaging/claude-code/commands/download.md"
  "packaging/claude-code/skills/x_likes/SKILL.md"
  "packaging/claude-code/README.md"
  # Codex CLI plugin
  "packaging/codex/.codex-plugin/plugin.json"
  "packaging/codex/.mcp.json"
  "packaging/codex/skills/x_likes/SKILL.md"
  "packaging/codex/README.md"
  # v2.2 placeholders + packaging architecture doc
  "packaging/hermes/README.md"
  "packaging/cursor/README.md"
  "packaging/README.md"
  # Self-hosted marketplaces
  ".claude-plugin/marketplace.json"
  ".agents/plugins/marketplace.json"
)

echo "=== Required artifact presence ==="
missing_required=0
for f in "${REQUIRED_ARTIFACTS[@]}"; do
  if [[ ! -f "$f" ]]; then
    err "required artifact missing: $f"
    missing_required=$((missing_required + 1))
  fi
done
if [[ $missing_required -eq 0 ]]; then
  ok "all ${#REQUIRED_ARTIFACTS[@]} required artifacts present"
fi

echo ""
echo "=== SOT validation ==="
validate_sot_defaults
validate_sot_skill_frontmatter

echo ""
echo "=== Host adapter manifests ==="
# All three host MCP configs use the same closed root schema.
# OpenClaw mcp-config.json requires minimum_xld_version; Claude Code / Codex .mcp.json may omit it
# (host plugin.json carries the version).
[[ -f "packaging/openclaw/x_likes/mcp-config.json" ]] && \
  validate_mcp_config "packaging/openclaw/x_likes/mcp-config.json" "true"

[[ -f "packaging/claude-code/.mcp.json" ]] && \
  validate_mcp_config "packaging/claude-code/.mcp.json" "false"

[[ -f "packaging/codex/.mcp.json" ]] && \
  validate_mcp_config "packaging/codex/.mcp.json" "false"

[[ -f "packaging/claude-code/.claude-plugin/plugin.json" ]] && validate_cc_plugin_json
[[ -f "packaging/codex/.codex-plugin/plugin.json" ]] && validate_codex_plugin_json
[[ -f ".claude-plugin/marketplace.json" ]] && validate_cc_marketplace
[[ -f ".agents/plugins/marketplace.json" ]] && validate_codex_marketplace

# OpenClaw defaults.json is a sync-copy of SOT defaults but lives outside SOT, so the
# stricter NONSOT key set applies (bearer_token must NOT appear here even though it's
# allowed in SOT defaults — if SOT ever adds it, sync would propagate and this check
# would fire to flag the leak through to host adapter publishable space).
if [[ -f "packaging/openclaw/x_likes/defaults.json" ]]; then
  if ! jq empty "packaging/openclaw/x_likes/defaults.json" >/dev/null 2>&1; then
    err "packaging/openclaw/x_likes/defaults.json is not valid JSON"
  else
    scan_forbidden_keys_json "packaging/openclaw/x_likes/defaults.json" "$FORBIDDEN_NONSOT_KEYS"
    ok "packaging/openclaw/x_likes/defaults.json scanned (no forbidden keys)"
  fi
fi

echo ""
echo "=== SKILL.md derived copies ==="
# CI uses `bash sync-skill.sh && git diff --exit-code` to catch drift; here we only verify
# the derived copies exist and run text scans inline below.
for f in \
  packaging/claude-code/skills/x_likes/SKILL.md \
  packaging/codex/skills/x_likes/SKILL.md \
  packaging/openclaw/x_likes/SKILL.md; do
  if [[ ! -f "$f" ]]; then
    err "$f missing — run 'bash scripts/sync-skill.sh'"
  else
    ok "$f exists"
  fi
done

echo ""
echo "=== Slash command contract (Claude Code) ==="
# Each slash command file must satisfy a spec-mandated contract beyond mere existence.
# auth/list/setup are deterministic shell commands (disable-model-invocation: true) wrapping
# specific binary subcommands; download is the LLM-routed MCP path and must NOT set the
# disable-model-invocation flag.
validate_slash_command_contract() {
  local file="$1"
  local must_disable="$2"      # true | false
  local must_contain="$3"      # required substring in body (binary invocation OR MCP tool name)
  local must_contain_label="$4" # human label for the substring
  if [[ ! -f "$file" ]]; then
    err "$file not found"
    return
  fi
  # Frontmatter description required.
  local desc
  desc=$(awk '
    BEGIN { fm=0; depth=0 }
    /^---[[:space:]]*$/ {
      depth++
      if (depth == 1) { fm=1; next }
      if (depth == 2) { fm=0; exit }
    }
    fm && /^description:/ { sub(/^description:[[:space:]]*/, "", $0); print; exit }
  ' "$file")
  if [[ -z "$desc" ]]; then
    err "$file frontmatter missing 'description'"
  fi

  # disable-model-invocation must match expected per command.
  local has_disable
  has_disable=$(awk '
    BEGIN { fm=0; depth=0; found=0 }
    /^---[[:space:]]*$/ {
      depth++
      if (depth == 1) { fm=1; next }
      if (depth == 2) { fm=0; exit }
    }
    fm && /^disable-model-invocation:[[:space:]]*true[[:space:]]*$/ { found=1 }
    END { print (found ? "true" : "false") }
  ' "$file")
  if [[ "$must_disable" == "true" && "$has_disable" != "true" ]]; then
    err "$file must declare 'disable-model-invocation: true' in frontmatter"
  fi
  if [[ "$must_disable" == "false" && "$has_disable" == "true" ]]; then
    err "$file must NOT declare 'disable-model-invocation: true' (LLM-routed command)"
  fi

  # Required substring in body — checks the spec-mandated invocation (binary subcommand or MCP tool).
  if ! grep -qF "$must_contain" "$file"; then
    err "$file must reference $must_contain_label '$must_contain' in body"
  fi
}

# Per-command contract (matches design D9 + tasks 3.3/3.4/3.5/3.6).
validate_slash_command_contract "packaging/claude-code/commands/auth.md" \
  "true" "x_likes_downloader auth status --json" "binary invocation"
validate_slash_command_contract "packaging/claude-code/commands/list.md" \
  "true" "x_likes_downloader likes list --json" "binary invocation"
validate_slash_command_contract "packaging/claude-code/commands/setup.md" \
  "true" "x_likes_downloader setup" "binary invocation"
# download.md must use MCP routing — assert it references both MCP tools by exact name.
validate_slash_command_contract "packaging/claude-code/commands/download.md" \
  "false" "mcp__x_likes_downloader__list_likes" "MCP tool name"
if ! grep -qF "mcp__x_likes_downloader__download_media" "packaging/claude-code/commands/download.md"; then
  err "packaging/claude-code/commands/download.md must reference MCP tool 'mcp__x_likes_downloader__download_media'"
fi
ok "slash command contract verified for all 4 Claude Code commands"

echo ""
echo "=== No bundled binaries under host adapter dirs ==="
# Spec D5 / multi-host-packaging requirement "Plugin 不打包 binary": host adapter directories
# must not contain executable files. The plugin assumes the user installed `x_likes_downloader`
# in PATH; bundling per-platform binaries would violate the trust/size/versioning model.
exec_hits=0
while IFS= read -r exe; do
  err "host adapter contains executable file (forbidden by 'no bundled binary' rule): $exe"
  exec_hits=$((exec_hits + 1))
done < <(find packaging/claude-code packaging/codex packaging/openclaw packaging/hermes packaging/cursor \
            -type f \( -perm -100 -o -perm -010 -o -perm -001 \) 2>/dev/null | sort)
if [[ $exec_hits -eq 0 ]]; then
  ok "no executable files under host adapter directories"
fi

echo ""
echo "=== Universal JSON scan under packaging/ ==="
# Catch-all: any JSON file shipped under packaging/ (existing or future-added) is checked
# for valid JSON and forbidden keys. The known-manifest validators above provide tighter
# schema enforcement; this pass guards the trust boundary against drive-by additions like
# `packaging/claude-code/assets/private.json` that wouldn't otherwise be in any allowlist.
# Only `packaging/skill/x_likes/defaults.json` may carry the public bearer_token (SOT
# convention); all other JSON uses the NONSOT key set (bearer_token forbidden).
json_total=0
json_bad=0
while IFS= read -r jf; do
  json_total=$((json_total + 1))
  if ! jq empty "$jf" >/dev/null 2>&1; then
    err "$jf is not valid JSON"
    json_bad=$((json_bad + 1))
    continue
  fi
  if [[ "$jf" == "packaging/skill/x_likes/defaults.json" ]]; then
    scan_forbidden_keys_json "$jf" "$FORBIDDEN_USER_KEYS"
  else
    scan_forbidden_keys_json "$jf" "$FORBIDDEN_NONSOT_KEYS"
  fi
done < <(find packaging -type f -name '*.json' | sort)
# Don't print a green ✓ if any JSON failed parse — `err()` has already incremented the
# error counter so the script will exit non-zero, but the visible line in CI logs would
# otherwise read "✓ scanned N JSON file(s) (M invalid)" which is contradictory.
if [[ $json_bad -gt 0 ]]; then
  warn "scanned $json_total JSON file(s) under packaging/ — $json_bad invalid (see errors above)"
else
  ok "scanned $json_total JSON file(s) under packaging/ (all valid)"
fi

echo ""
echo "=== Public packaging text artifacts (Markdown / YAML) ==="
# Scan EVERY Markdown / YAML file under packaging/ — host adapter READMEs, SOT README,
# command files (packaging/claude-code/commands/*.md), packaging-level docs, host placeholders.
# These are part of the shipped plugin/marketplace surface and must not contain pasted
# cURL / cookie / bearer values. The scanner only flags assignment-style patterns; documentation
# prose mentioning the field names remains allowed.
public_text_count=0
while IFS= read -r f; do
  scan_forbidden_text "$f"
  public_text_count=$((public_text_count + 1))
done < <(find packaging -type f \( -name '*.md' -o -name '*.yaml' -o -name '*.yml' \) | sort)
ok "scanned $public_text_count public packaging text artifact(s)"

echo ""
echo "=== Version consistency ==="
validate_version_consistency

echo ""
if [[ $errs -gt 0 ]]; then
  echo -e "${RED}check-packaging failed: $errs error(s)${NC}" >&2
  exit 1
fi
echo -e "${GREEN}check-packaging: all checks passed${NC}"
