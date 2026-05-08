---
description: First-time setup — import X cURL into x_likes_downloader (interactive).
disable-model-invocation: true
allowed-tools: Bash(x_likes_downloader:*), Bash(command:*)
---

!`command -v x_likes_downloader >/dev/null 2>&1 || { echo '请先安装 x_likes_downloader binary：https://github.com/HerbertGao/x_likes_downloader/releases'; exit 1; }; x_likes_downloader setup`

The interactive setup flow above prompts the user to paste a cURL block captured from the X Web UI (DevTools → Network → `/Likes` request → Copy as cURL). When done, it writes credentials to a stable host-managed path.

After the shell exits, summarize the outcome in one line:
- Success: `✅ Setup complete — credentials saved.`
- Failure: relay the error printed by the binary and suggest re-running `/x_likes:setup` after re-grabbing cURL.

Do not call any other tools. Do not echo any cURL/cookie content.
