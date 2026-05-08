---
description: List my latest X likes as a compact table (id / author / media count / time).
disable-model-invocation: true
allowed-tools: Bash(x_likes_downloader:*), Bash(command:*)
argument-hint: '[count]'
---

!`command -v x_likes_downloader >/dev/null 2>&1 || { echo '请先安装 x_likes_downloader binary：https://github.com/HerbertGao/x_likes_downloader/releases'; exit 1; }; x_likes_downloader likes list --json --count "${1:-20}"`

The shell output is a JSON envelope `{ok, data: {tweets[], cursor, schema_version}, meta}`. Render the response as a compact markdown table with columns:

| ID | Author | Media | Liked at |

For each `tweet` in `data.tweets`:
- `ID`: `tweet.id` (link to `tweet.tweet_url` if available, otherwise raw)
- `Author`: `@<tweet.author_handle>` and the display name in parentheses
- `Media`: `len(tweet.media)` followed by `📷` for image, `🎬` for video, `🎞` for gif (if mixed, show counts)
- `Liked at`: `tweet.created_at` (or `tweet.liked_at` if present and non-null)

After the table, on a new line print `cursor: <data.cursor>` so the user can paginate. If `ok == false`, render `❌ <error.kind>: <error.message>` and append `→ <error.hint>` when present. Do not call any other tools.
