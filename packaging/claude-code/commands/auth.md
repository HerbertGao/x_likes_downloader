---
description: Check x_likes_downloader credential health (single network probe ~200ms).
disable-model-invocation: true
allowed-tools: Bash(x_likes_downloader:*), Bash(command:*)
---

!`command -v x_likes_downloader >/dev/null 2>&1 || { echo '请先安装 x_likes_downloader binary：https://github.com/HerbertGao/x_likes_downloader/releases'; exit 1; }; x_likes_downloader auth status --json`

The shell output above is a JSON envelope of shape `{ok, data?: {status, checked_at}, error?: {kind, message, hint}, meta}`.

Render exactly one line for the user:

- If `ok == true`: `✅ healthy (checked at <data.checked_at>)`
- If `ok == false`: `❌ <error.kind>: <error.message>` and append `→ <error.hint>` when `error.hint` is non-empty.

Do not call any tools, do not list likes, do not re-probe. The shell already produced the only datapoint we need.
