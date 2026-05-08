#!/usr/bin/env bash
# 校验 skill/defaults.json：
# 1. 是合法 JSON
# 2. 顶层键集合 ⊆ {schema_version, likes_api_url, likes_features, likes_fieldtoggles, bearer_token}
# 3. 不含任何敏感字段（auth_token / ct0 / user_id / user_agent / personalization_id）
# 4. 含 schema_version (整数 ≥ 1)

set -euo pipefail

DEFAULTS="${1:-skill/defaults.json}"

if [[ ! -f "$DEFAULTS" ]]; then
  echo "::error::$DEFAULTS 不存在" >&2
  exit 1
fi

if ! jq empty "$DEFAULTS" >/dev/null 2>&1; then
  echo "::error::$DEFAULTS 不是合法 JSON" >&2
  exit 1
fi

ALLOWED_KEYS='["schema_version","likes_api_url","likes_features","likes_fieldtoggles","bearer_token"]'
FORBIDDEN_KEYS='["auth_token","ct0","user_id","user_agent","personalization_id"]'

# Forbidden keys check
forbidden_found=$(jq --argjson f "$FORBIDDEN_KEYS" '
  [keys[] as $k | select($f | index($k))]
' "$DEFAULTS")

if [[ "$forbidden_found" != "[]" ]]; then
  echo "::error::$DEFAULTS 包含禁止字段: $forbidden_found" >&2
  exit 1
fi

# Allowed keys closure check
unknown_keys=$(jq --argjson a "$ALLOWED_KEYS" '
  [keys[] as $k | select(($a | index($k)) | not)]
' "$DEFAULTS")

if [[ "$unknown_keys" != "[]" ]]; then
  echo "::error::$DEFAULTS 包含未声明字段: $unknown_keys" >&2
  exit 1
fi

# schema_version
sv=$(jq -r '.schema_version // empty' "$DEFAULTS")
if [[ -z "$sv" ]]; then
  echo "::error::$DEFAULTS 缺少 schema_version" >&2
  exit 1
fi
if ! [[ "$sv" =~ ^[1-9][0-9]*$ ]]; then
  echo "::error::$DEFAULTS schema_version 必须为正整数，得到 $sv" >&2
  exit 1
fi

echo "skill/defaults.json 校验通过 (schema_version=$sv)"
