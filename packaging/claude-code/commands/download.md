---
description: Download media for one or more X like tweet IDs via the MCP server.
argument-hint: '<tweet-id> [tweet-id...]'
---

The user wants to download media from specific liked tweets identified by IDs in `$ARGUMENTS`.

## Plan

1. Parse the space-separated tweet IDs from `$ARGUMENTS` into a list `target_ids`. If the list is empty, ask the user which tweet IDs to download and stop.
2. Call `mcp__x_likes_downloader__list_likes` with `count: 50` (or higher, up to 100) to fetch a recent page of likes containing the targets. If any of `target_ids` is not present in the result and the response includes a `cursor`, you may call `list_likes` again with `since_cursor: <cursor>` up to **2 more times** to find the remaining IDs. Stop after that — request the user paginate further or run `/x_likes:list` first if IDs still missing.
3. From the returned `data.tweets[]`, pick tweets whose `id` is in `target_ids`. Concatenate their `media[]` arrays into a single `items` array.
4. Call `mcp__x_likes_downloader__download_media` with that `items` array. Pass a sensible `concurrency` (default 4). Use `subdir` only if the user explicitly asked for a subfolder.
5. After completion, render a one-line summary per tweet: `<id> @<author>: <downloaded>/<total> media → <path>` and a final aggregate.

## Failure handling

- If `mcp__x_likes_downloader__*` tools are unavailable (MCP server not running) or the call returns `kind: binary_missing`, tell the user: `请先安装 x_likes_downloader binary 并确保 MCP server 已启动：https://github.com/HerbertGao/x_likes_downloader/releases`. Do not retry.
- If `auth_expired` / `endpoint_stale` is returned, suggest the user run `/x_likes:setup` to re-import cURL.
- If `sandbox_violation` (e.g., bad subdir), surface the message verbatim.
- If a target ID is not found after pagination, list the missing IDs at the end of the summary so the user knows what was skipped.

Do not call `auth_status` proactively — it is reserved for `/x_likes:auth`. Do not invoke shell or any unrelated tool.
