#!/usr/bin/env bash
# 校验 skill/ 下的元数据 JSON 文件：
#
# 1. skill/defaults.json：
#    - 合法 JSON
#    - 顶层键 ⊆ {schema_version, likes_api_url, likes_features, likes_fieldtoggles, bearer_token}
#    - 不含敏感字段（auth_token / ct0 / user_id / user_agent / personalization_id）
#    - 含 schema_version（整数 ≥ 1）
#
# 2. skill/mcp-config.json（v2 新增）：
#    - 合法 JSON
#    - 顶层键 ⊆ {command, args, transport, minimum_xld_version}
#    - 不含敏感字段
#    - command="xld"、args 含 "serve" 与 "--mcp"、transport="stdio"

set -euo pipefail

SKILL_DIR="${1:-skill}"
DEFAULTS="$SKILL_DIR/defaults.json"
MCP_CONFIG="$SKILL_DIR/mcp-config.json"

FORBIDDEN_KEYS='["auth_token","ct0","user_id","user_agent","personalization_id","bearer","auth"]'

check_json_exists() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "::error::$path 不存在" >&2
    exit 1
  fi
  if ! jq empty "$path" >/dev/null 2>&1; then
    echo "::error::$path 不是合法 JSON" >&2
    exit 1
  fi
}

check_no_secrets() {
  local path="$1"
  local forbidden_found
  forbidden_found=$(jq --argjson f "$FORBIDDEN_KEYS" '[keys[] as $k | select($f | index($k))]' "$path")
  if [[ "$forbidden_found" != "[]" ]]; then
    echo "::error::$path 包含禁止字段: $forbidden_found" >&2
    exit 1
  fi
}

check_keys_closed() {
  local path="$1"
  local allowed="$2"
  local unknown_keys
  unknown_keys=$(jq --argjson a "$allowed" '[keys[] as $k | select(($a | index($k)) | not)]' "$path")
  if [[ "$unknown_keys" != "[]" ]]; then
    echo "::error::$path 包含未声明字段: $unknown_keys" >&2
    exit 1
  fi
}

# ---- 1. defaults.json ----
check_json_exists "$DEFAULTS"
check_no_secrets "$DEFAULTS"
DEFAULTS_ALLOWED='["schema_version","likes_api_url","likes_features","likes_fieldtoggles","bearer_token"]'
check_keys_closed "$DEFAULTS" "$DEFAULTS_ALLOWED"

sv=$(jq -r '.schema_version // empty' "$DEFAULTS")
if [[ -z "$sv" ]]; then
  echo "::error::$DEFAULTS 缺少 schema_version" >&2
  exit 1
fi
if ! [[ "$sv" =~ ^[1-9][0-9]*$ ]]; then
  echo "::error::$DEFAULTS schema_version 必须为正整数，得到 $sv" >&2
  exit 1
fi
echo "$DEFAULTS 校验通过 (schema_version=$sv)"

# ---- 2. mcp-config.json ----
check_json_exists "$MCP_CONFIG"
check_no_secrets "$MCP_CONFIG"
MCP_ALLOWED='["command","args","transport","minimum_xld_version"]'
check_keys_closed "$MCP_CONFIG" "$MCP_ALLOWED"

# command 必须是 cargo 实际安装的 binary 名 "x_likes_downloader"
# （用户想用 "xld" 短名需自己 ln -s，不能假设别名存在）
mcp_cmd=$(jq -r '.command // empty' "$MCP_CONFIG")
if [[ "$mcp_cmd" != "x_likes_downloader" ]]; then
  echo "::error::$MCP_CONFIG command 必须为 'x_likes_downloader'（cargo install 的实际产物），得到 '$mcp_cmd'" >&2
  exit 1
fi

# transport="stdio"
mcp_transport=$(jq -r '.transport // empty' "$MCP_CONFIG")
if [[ "$mcp_transport" != "stdio" ]]; then
  echo "::error::$MCP_CONFIG transport 必须为 'stdio'，得到 '$mcp_transport'" >&2
  exit 1
fi

# args 含 "serve" 与 "--mcp"
if ! jq -e '.args | index("serve")' "$MCP_CONFIG" >/dev/null; then
  echo "::error::$MCP_CONFIG args 必须含 'serve'" >&2
  exit 1
fi
if ! jq -e '.args | index("--mcp")' "$MCP_CONFIG" >/dev/null; then
  echo "::error::$MCP_CONFIG args 必须含 '--mcp'" >&2
  exit 1
fi

# minimum_xld_version 必须存在且像 SemVer
min_ver=$(jq -r '.minimum_xld_version // empty' "$MCP_CONFIG")
if [[ -z "$min_ver" ]]; then
  echo "::error::$MCP_CONFIG 缺少 minimum_xld_version" >&2
  exit 1
fi
if ! [[ "$min_ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+ ]]; then
  echo "::error::$MCP_CONFIG minimum_xld_version 必须遵循 SemVer (x.y.z)，得到 '$min_ver'" >&2
  exit 1
fi
echo "$MCP_CONFIG 校验通过 (minimum_xld_version=$min_ver)"

echo "skill/ 全部元数据 JSON 校验通过"
