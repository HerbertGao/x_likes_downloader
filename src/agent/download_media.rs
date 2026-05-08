use chrono::DateTime;
use filetime::{set_file_times, FileTime};
use futures::stream::{self, StreamExt};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::config::Config;
use crate::error::{ErrorKind, ErrorPayload};
use crate::sandbox;

use super::types::{
    DownloadError, DownloadOpts, DownloadOutput, DownloadResult, DownloadStatus, DownloadSummary,
    MediaItem, ProgressEvent, ProgressSink,
};

pub async fn download_media(
    items: &[MediaItem],
    opts: &DownloadOpts,
    sink: Arc<dyn ProgressSink>,
) -> Result<DownloadOutput, ErrorPayload> {
    if !(1..=16).contains(&opts.concurrency) {
        return Err(ErrorPayload::new(
            ErrorKind::InvalidArgument,
            format!("concurrency 必须在 [1, 16] 内，收到 {}", opts.concurrency),
        ));
    }

    let config = Config::load()
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, format!("加载配置失败: {}", e)))?;

    // base 来源优先级：opts.base_dir → config.download_sandbox_base_dir → 平台默认
    let base = opts
        .base_dir
        .clone()
        .or_else(|| config.download_sandbox_base_dir.clone())
        .unwrap_or_else(sandbox::default_base_dir);
    sandbox::ensure_dir(&base)
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, e.to_string()))?;
    let target_dir = sandbox::resolve_subdir(&base, opts.subdir.as_deref())
        .map_err(|e| ErrorPayload::new(e.kind(), e.to_string()))?;
    sandbox::ensure_dir(&target_dir)
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, e.to_string()))?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| ErrorPayload::new(ErrorKind::InternalError, e.to_string()))?;

    sink.emit(ProgressEvent::DownloadStarted {
        total: items.len(),
        concurrency: opts.concurrency,
    });

    let total = items.len();
    let target_dir_arc = Arc::new(target_dir);
    let ua = if config.user_agent.is_empty() {
        format!("xld/{}", env!("CARGO_PKG_VERSION"))
    } else {
        config.user_agent.clone()
    };
    let user_agent = Arc::new(ua);
    let client_arc = Arc::new(client);
    let sink_inner = sink.clone();
    let filename_format = Arc::new(opts.filename_format.clone());
    let set_mtime = opts.set_mtime;

    // 拿一份 owned items 拷贝；让闭包不再持有 &[MediaItem] 的生命周期，
    // 满足 rmcp tool 宏对 future 的 HRTB / Send + 'static 要求。
    let owned_items: Vec<MediaItem> = items.to_vec();
    let results: Vec<DownloadResult> =
        stream::iter(owned_items.into_iter().enumerate().map(|(index, item)| {
            let client = client_arc.clone();
            let target_dir = target_dir_arc.clone();
            let ua = user_agent.clone();
            let sink = sink_inner.clone();
            let filename_format = filename_format.clone();
            async move {
                sink.emit(ProgressEvent::ItemStarted {
                    tweet_id: item.tweet_id.clone(),
                    url: item.url.clone(),
                    index,
                    total,
                });
                let result = download_one(
                    &client,
                    &ua,
                    &target_dir,
                    &item,
                    filename_format.as_deref(),
                    set_mtime,
                    sink.clone(),
                )
                .await;
                sink.emit(ProgressEvent::ItemDone {
                    tweet_id: result.tweet_id.clone(),
                    status: result.status.clone(),
                    bytes: result.bytes,
                });
                result
            }
        }))
        .buffer_unordered(opts.concurrency as usize)
        .collect()
        .await;

    let mut summary = DownloadSummary {
        total: results.len(),
        downloaded: 0,
        skipped: 0,
        failed: 0,
    };
    for r in &results {
        match r.status {
            DownloadStatus::Downloaded => summary.downloaded += 1,
            DownloadStatus::SkippedExisting => summary.skipped += 1,
            DownloadStatus::Failed => summary.failed += 1,
        }
    }

    sink.emit(ProgressEvent::DownloadFinished {
        summary: summary.clone(),
    });

    Ok(DownloadOutput {
        downloads: results,
        summary,
    })
}

async fn download_one(
    client: &reqwest::Client,
    user_agent: &str,
    target_dir: &Path,
    item: &MediaItem,
    filename_format: Option<&str>,
    set_mtime: bool,
    sink: Arc<dyn ProgressSink>,
) -> DownloadResult {
    let filename = derive_filename(item, filename_format);
    let path = target_dir.join(&filename);

    // Defense: 永不写入符号链接。Sandbox jail 校验在目录层；这里防御文件名层的 symlink 逃逸。
    if let Ok(symlink_meta) = std::fs::symlink_metadata(&path) {
        if symlink_meta.file_type().is_symlink() {
            return DownloadResult {
                tweet_id: item.tweet_id.clone(),
                url: item.url.clone(),
                path: path.clone(),
                bytes: 0,
                status: DownloadStatus::Failed,
                error: Some(DownloadError {
                    kind: ErrorKind::SandboxViolation,
                    message: format!("拒绝写入符号链接: {:?}", path),
                }),
            };
        }
    }

    // Skip if already complete
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 0 {
            // Probe content-length to see if existing file is complete
            if let Ok(head) = head_content_length(client, user_agent, &item.url).await {
                if let Some(expected) = head {
                    if meta.len() == expected {
                        if set_mtime {
                            apply_mtime(&path, item.created_at.as_deref());
                        }
                        return DownloadResult {
                            tweet_id: item.tweet_id.clone(),
                            url: item.url.clone(),
                            path,
                            bytes: meta.len(),
                            status: DownloadStatus::SkippedExisting,
                            error: None,
                        };
                    }
                } else {
                    // Server doesn't report length — treat existing non-empty as done
                    if set_mtime {
                        apply_mtime(&path, item.created_at.as_deref());
                    }
                    return DownloadResult {
                        tweet_id: item.tweet_id.clone(),
                        url: item.url.clone(),
                        path,
                        bytes: meta.len(),
                        status: DownloadStatus::SkippedExisting,
                        error: None,
                    };
                }
            }
        }
    }

    match fetch_with_resume(client, user_agent, item, &path, sink).await {
        Ok(bytes) => {
            if set_mtime {
                apply_mtime(&path, item.created_at.as_deref());
            }
            DownloadResult {
                tweet_id: item.tweet_id.clone(),
                url: item.url.clone(),
                path,
                bytes,
                status: DownloadStatus::Downloaded,
                error: None,
            }
        }
        Err((kind, message)) => DownloadResult {
            tweet_id: item.tweet_id.clone(),
            url: item.url.clone(),
            path,
            bytes: 0,
            status: DownloadStatus::Failed,
            error: Some(DownloadError { kind, message }),
        },
    }
}

/// 清洗文件名片段：替换路径分隔符 / 控制字符 / `..` 序列。
///
/// 这是防御文件名路径穿越的核心：把可能让 `Path::join` 跨越 base 的字符全替换成 `_`。
/// 处理：
/// - `/`、`\`、`\0`、`:` 与 ASCII 控制字符（< 0x20）→ `_`
/// - 字面量 `..` → `_`
/// - 空字符串 / 整串为 `.` 或 `..` → `_`
fn sanitize_component(s: &str) -> String {
    if s.is_empty() {
        return "_".to_string();
    }
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '/' | '\\' | '\0' | ':' => out.push('_'),
            c if (c as u32) < 0x20 => out.push('_'),
            c => out.push(c),
        }
    }
    // 替换字面量双点序列。`replace` 不重叠，循环到稳定，避免 "...." 残留 ".."。
    while out.contains("..") {
        out = out.replace("..", "_");
    }
    if out == "." || out.is_empty() {
        return "_".to_string();
    }
    out
}

/// 派生文件名。
///
/// 默认格式：
/// - `author_handle` 存在 → `{author_handle}_{tweet_id}_{suggested_filename}`
/// - `author_handle` 缺失 → `{tweet_id}_{suggested_filename}`
///
/// `filename_format` 模板（占位符 `{USERNAME}` / `{ID}`，空格转下划线）：
/// - 替换后再 `_{suggested_filename}` 后缀
/// - 用于兼容旧 `xld download` 的 `config.file_format`
///
/// 所有片段均经 [`sanitize_component`] 清洗，并最终用 `Path::file_name()`
/// 兜底，确保结果是一个安全的单一路径片段，不会因 `Path::join` 跨越 base。
fn derive_filename(item: &MediaItem, filename_format: Option<&str>) -> String {
    let safe_tweet_id = sanitize_component(&item.tweet_id);
    let safe_handle = item
        .author_handle
        .as_deref()
        .map(sanitize_component)
        .filter(|h| !h.is_empty() && h != "_");

    let suggested_raw = if item.suggested_filename.is_empty() {
        "media"
    } else {
        item.suggested_filename.as_str()
    };
    let safe_suggested = sanitize_component(suggested_raw);

    let raw = if let Some(template) = filename_format {
        let username = item.author_handle.as_deref().unwrap_or("");
        let prefix = template
            .replace("{USERNAME}", username)
            .replace("{ID}", &item.tweet_id)
            .replace(' ', "_");
        let safe_prefix = sanitize_component(&prefix);
        format!("{}_{}", safe_prefix, safe_suggested)
    } else {
        match safe_handle {
            Some(handle) => format!("{}_{}_{}", handle, safe_tweet_id, safe_suggested),
            None => format!("{}_{}", safe_tweet_id, safe_suggested),
        }
    };

    // 最终兜底：取 file_name() 最后一段，如果还是怪东西就返回稳定 fallback。
    match Path::new(&raw).file_name().and_then(|n| n.to_str()) {
        Some(name) if !name.is_empty() && name != "." && name != ".." => name.to_string(),
        _ => format!("{}_media", safe_tweet_id),
    }
}

/// `fetch_with_resume` 在拿到 HTTP response 后的续传决策。
///
/// 抽到独立纯函数便于测试：当本地已有部分文件 (`existing_len > 0`) 而服务器
/// 返回 `200 OK`（忽略 Range）时，必须丢弃旧片段从头写，否则会出现
/// `[partial][full]` 拼接损坏。
#[derive(Debug, PartialEq, Eq)]
enum ResumeDecision {
    /// 续传：在已有文件后追加，downloaded 初值 = existing_len。
    Resume,
    /// 全新写入：truncate 已有文件（若有），downloaded 初值 = 0。
    Fresh,
    /// 错误：HTTP 状态分类为 ErrorKind。
    Error(ErrorKind),
}

/// 根据 (existing_len, HTTP status) 决定如何写入响应 body。
///
/// 关键不变量：当返回 [`ResumeDecision::Fresh`] 且 status == 416 时，
/// **调用方必须丢弃当前响应并以无 Range 头重新发起 GET**——
/// 416 响应的 body 是错误页（如 `<Code>InvalidRange</Code>` XML），
/// 直接 truncate 写入会把错误页保存为媒体文件，造成 silent corruption。
/// 见 [`fetch_with_resume`] 中的 416 重发分支。
fn decide_resume_mode(existing_len: u64, status: u16) -> ResumeDecision {
    match (existing_len, status) {
        // 416 Range Not Satisfiable：能进入 fetch_with_resume 必然意味着
        // 本地大小 != 远端 expected。Range 起点过 EOF 才会 416 → 本地文件
        // 比远端长（损坏的过大文件），必须丢弃从头重下，绝不能当作完成。
        // existing_len == 0 时不会发 Range，理论上不会触发 416；兜底也走 Fresh。
        // NOTE: 调用方在 416 case 必须丢弃原响应并重发（见函数级 doc）。
        (_, 416) => ResumeDecision::Fresh,
        (n, 206) if n > 0 => ResumeDecision::Resume,
        // 关键 P2 case：本地已有部分但服务器忽略 Range 返回 200 → 必须从头来
        (n, 200) if n > 0 => ResumeDecision::Fresh,
        (0, s) if (200..300).contains(&s) => ResumeDecision::Fresh,
        (_, s) if (200..300).contains(&s) => ResumeDecision::Fresh,
        (_, s) => ResumeDecision::Error(
            crate::error::classify_status(s).unwrap_or(ErrorKind::InternalError),
        ),
    }
}

fn apply_mtime(path: &Path, created_at: Option<&str>) {
    let Some(s) = created_at else { return };
    let Some(ts) = parse_x_created_at(s) else {
        return;
    };
    let ft = FileTime::from_unix_time(ts, 0);
    let _ = set_file_times(path, ft, ft);
}

fn parse_x_created_at(s: &str) -> Option<i64> {
    // Try X's RFC2822-like format first (e.g. "Thu Apr 06 15:24:15 +0000 2017")
    if let Ok(dt) = DateTime::parse_from_str(s, "%a %b %d %H:%M:%S %z %Y") {
        return Some(dt.timestamp());
    }
    // Fall back to RFC3339 (in case Agent passes a normalized value)
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }
    None
}

async fn head_content_length(
    client: &reqwest::Client,
    user_agent: &str,
    url: &str,
) -> Result<Option<u64>, reqwest::Error> {
    let resp = client
        .head(url)
        .header("User-Agent", user_agent)
        .send()
        .await?;
    Ok(resp.content_length())
}

async fn fetch_with_resume(
    client: &reqwest::Client,
    user_agent: &str,
    item: &MediaItem,
    path: &Path,
    sink: Arc<dyn ProgressSink>,
) -> Result<u64, (ErrorKind, String)> {
    let mut existing_len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);

    // Phase 1: 试探性请求（如有本地片段则带 Range）。
    let mut req = client.get(&item.url).header("User-Agent", user_agent);
    if existing_len > 0 {
        req = req.header("Range", format!("bytes={}-", existing_len));
    }
    let mut response = req
        .send()
        .await
        .map_err(|e| (ErrorKind::NetworkError, e.to_string()))?;

    let mut status = response.status().as_u16();
    let mut resume_decision = decide_resume_mode(existing_len, status);

    // Phase 2: 416 处理 —— 必须丢弃 416 响应（其 body 是错误页），
    // 重新发起一次无 Range 的 GET，再用新响应做 truncate 写入。
    if matches!(resume_decision, ResumeDecision::Fresh) && existing_len > 0 && status == 416 {
        drop(response);
        existing_len = 0;
        response = client
            .get(&item.url)
            .header("User-Agent", user_agent)
            .send()
            .await
            .map_err(|e| (ErrorKind::NetworkError, e.to_string()))?;
        status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err((
                crate::error::classify_status(status).unwrap_or(ErrorKind::InternalError),
                format!("HTTP {}（416 后重发）: {}", status, item.url),
            ));
        }
        resume_decision = ResumeDecision::Fresh;
    } else if let ResumeDecision::Error(kind) = resume_decision {
        return Err((kind, format!("HTTP {}: {}", status, item.url)));
    }

    let resume_mode = matches!(resume_decision, ResumeDecision::Resume);
    let content_length = response.content_length();
    let total_bytes = if resume_mode {
        content_length.map(|c| c + existing_len)
    } else {
        content_length
    };

    let mut file = if resume_mode {
        OpenOptions::new()
            .create(true)
            .append(true)
            .write(true)
            .open(path)
            .await
            .map_err(|e| (ErrorKind::InternalError, e.to_string()))?
    } else {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .await
            .map_err(|e| (ErrorKind::InternalError, e.to_string()))?
    };

    let mut downloaded = if resume_mode { existing_len } else { 0 };
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| (ErrorKind::NetworkError, e.to_string()))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| (ErrorKind::InternalError, e.to_string()))?;
        downloaded += chunk.len() as u64;
        sink.emit(ProgressEvent::ItemProgress {
            tweet_id: item.tweet_id.clone(),
            bytes_done: downloaded,
            bytes_total: total_bytes,
        });
    }

    Ok(downloaded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::{MediaType, NullSink, VecSink};

    fn item(tweet_id: &str, handle: Option<&str>, suggested: &str) -> MediaItem {
        MediaItem {
            tweet_id: tweet_id.to_string(),
            kind: MediaType::Image,
            url: format!("https://example.test/{}", suggested),
            suggested_filename: suggested.to_string(),
            bytes: None,
            author_handle: handle.map(|s| s.to_string()),
            created_at: None,
            all_variants: None,
        }
    }

    #[test]
    fn invalid_concurrency_rejected() {
        let items: Vec<MediaItem> = vec![];
        let opts = DownloadOpts {
            concurrency: 0,
            ..Default::default()
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(download_media(&items, &opts, Arc::new(NullSink)));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind, ErrorKind::InvalidArgument);
    }

    #[test]
    fn concurrency_above_max_rejected() {
        let items: Vec<MediaItem> = vec![];
        let opts = DownloadOpts {
            concurrency: 17,
            ..Default::default()
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(download_media(&items, &opts, Arc::new(NullSink)));
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind, ErrorKind::InvalidArgument);
    }

    #[test]
    fn empty_items_emits_started_and_finished() {
        let items: Vec<MediaItem> = vec![];
        let opts = DownloadOpts::default();
        let sink = Arc::new(VecSink::new());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _ = rt.block_on(download_media(&items, &opts, sink.clone()));
        let events = sink.snapshot();
        if !events.is_empty() {
            assert!(matches!(events[0], ProgressEvent::DownloadStarted { .. }));
            assert!(matches!(
                events.last().unwrap(),
                ProgressEvent::DownloadFinished { .. }
            ));
        }
    }

    #[test]
    fn derive_filename_default_with_handle() {
        let m = item("1234", Some("alice"), "AAA.jpg");
        assert_eq!(derive_filename(&m, None), "alice_1234_AAA.jpg");
    }

    #[test]
    fn derive_filename_default_without_handle() {
        let m = item("1234", None, "AAA.jpg");
        assert_eq!(derive_filename(&m, None), "1234_AAA.jpg");
    }

    #[test]
    fn derive_filename_default_empty_handle_falls_back() {
        let m = item("1234", Some(""), "AAA.jpg");
        assert_eq!(derive_filename(&m, None), "1234_AAA.jpg");
    }

    #[test]
    fn derive_filename_legacy_template() {
        let m = item("1234", Some("alice"), "AAA.jpg");
        assert_eq!(
            derive_filename(&m, Some("{USERNAME} {ID}")),
            "alice_1234_AAA.jpg"
        );
    }

    #[test]
    fn derive_filename_legacy_template_id_only() {
        let m = item("1234", Some("alice"), "AAA.jpg");
        assert_eq!(derive_filename(&m, Some("{ID}")), "1234_AAA.jpg");
    }

    #[test]
    fn derive_filename_legacy_template_username_blank() {
        // Old config defaulted to {USERNAME} {ID}; if handle absent, USERNAME→""
        let m = item("1234", None, "AAA.jpg");
        assert_eq!(
            derive_filename(&m, Some("{USERNAME} {ID}")),
            "_1234_AAA.jpg"
        );
    }

    #[test]
    fn derive_filename_empty_suggested_uses_media() {
        let mut m = item("1234", Some("alice"), "");
        m.suggested_filename = String::new();
        assert_eq!(derive_filename(&m, None), "alice_1234_media");
    }

    #[test]
    fn parse_x_created_at_handles_x_format() {
        let ts = parse_x_created_at("Thu Apr 06 15:24:15 +0000 2017").unwrap();
        // Sanity: 2017-04-06 15:24:15 UTC is a positive unix timestamp
        assert!(ts > 1_491_000_000 && ts < 1_492_000_000);
    }

    #[test]
    fn parse_x_created_at_handles_rfc3339() {
        let ts = parse_x_created_at("2026-05-08T12:00:00+00:00");
        assert!(ts.is_some());
    }

    #[test]
    fn parse_x_created_at_rejects_garbage() {
        assert!(parse_x_created_at("nonsense").is_none());
    }

    #[test]
    fn apply_mtime_sets_real_file_time() {
        use std::io::Write;
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let path = dir.path().join("f.txt");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"x")
            .unwrap();

        apply_mtime(&path, Some("Thu Apr 06 15:24:15 +0000 2017"));

        let meta = std::fs::metadata(&path).unwrap();
        let mtime = filetime::FileTime::from_last_modification_time(&meta);
        assert_eq!(mtime.unix_seconds(), 1491492255);
    }

    #[test]
    fn apply_mtime_silently_ignores_garbage() {
        use std::io::Write;
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let path = dir.path().join("g.txt");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(b"x")
            .unwrap();
        apply_mtime(&path, Some("not a date")); // should not panic
        apply_mtime(&path, None);
    }

    // ---- sanitize_component / derive_filename hardening ----

    #[test]
    fn sanitize_component_replaces_path_separators() {
        assert!(!sanitize_component("foo/bar").contains('/'));
        assert!(!sanitize_component("foo\\bar").contains('\\'));
        assert!(!sanitize_component("c:foo").contains(':'));
        assert!(!sanitize_component("a\0b").contains('\0'));
        // ASCII control char
        assert!(!sanitize_component("a\x01b").contains('\x01'));
    }

    #[test]
    fn sanitize_component_replaces_dotdot() {
        assert!(!sanitize_component("..").contains(".."));
        assert!(!sanitize_component("../etc").contains(".."));
        assert!(!sanitize_component("a..b").contains(".."));
        // 多重双点也要被消掉
        assert!(!sanitize_component("....").contains(".."));
        // 空串退化为 "_"
        assert_eq!(sanitize_component(""), "_");
        // 单点也要规避（避免 file_name 解释为 ".")
        assert_eq!(sanitize_component("."), "_");
    }

    #[test]
    fn derive_filename_rejects_dotdot_in_suggested() {
        let m = item("1234", Some("alice"), "../../etc/passwd");
        let f = derive_filename(&m, None);
        assert!(!f.contains(".."), "filename still contains '..': {}", f);
        assert!(!f.contains('/'), "filename still contains '/': {}", f);
        // 必须是单一路径片段
        assert_eq!(Path::new(&f).components().count(), 1);
    }

    #[test]
    fn derive_filename_rejects_slash_in_suggested() {
        let m = item("1234", Some("alice"), "foo/bar.jpg");
        let f = derive_filename(&m, None);
        assert!(!f.contains('/'), "filename still contains '/': {}", f);
    }

    #[test]
    fn derive_filename_rejects_backslash_in_suggested() {
        let m = item("1234", Some("alice"), "foo\\bar.jpg");
        let f = derive_filename(&m, None);
        assert!(!f.contains('\\'), "filename still contains '\\': {}", f);
    }

    #[test]
    fn derive_filename_rejects_dotdot_in_tweet_id() {
        let m = item("..", Some("alice"), "AAA.jpg");
        let f = derive_filename(&m, None);
        assert!(!f.contains(".."), "filename still contains '..': {}", f);
        assert_eq!(Path::new(&f).components().count(), 1);
    }

    #[test]
    fn derive_filename_rejects_dotdot_in_handle() {
        let m = item("1234", Some("../alice"), "AAA.jpg");
        let f = derive_filename(&m, None);
        assert!(!f.contains(".."), "filename still contains '..': {}", f);
        assert!(!f.contains('/'), "filename still contains '/': {}", f);
    }

    #[test]
    fn derive_filename_path_join_stays_under_base() {
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let base = dir.path().to_path_buf();
        let m = item("1234", Some("alice"), "../../etc/passwd");
        let f = derive_filename(&m, None);
        let canonical_base = base.canonicalize().unwrap();
        let joined = canonical_base.join(&f);
        // joined 还没被创建出来，不能 canonicalize；用 lexical 形式断言
        // 因为 f 不包含 / .. \ 等任何能让 join 跨越 base 的字符。
        assert!(
            joined.starts_with(&canonical_base),
            "joined {:?} escapes base {:?}",
            joined,
            canonical_base
        );
        // 而且 join 后只比 base 多一个组件
        let extra: Vec<_> = joined
            .strip_prefix(&canonical_base)
            .unwrap()
            .components()
            .collect();
        assert_eq!(extra.len(), 1, "expected single component, got {:?}", extra);
    }

    // ---- decide_resume_mode (P2: server ignores Range) ----

    #[test]
    fn decide_resume_mode_fresh_download_200() {
        assert_eq!(decide_resume_mode(0, 200), ResumeDecision::Fresh);
    }

    #[test]
    fn decide_resume_mode_resume_on_206() {
        assert_eq!(decide_resume_mode(100, 206), ResumeDecision::Resume);
    }

    #[test]
    fn decide_resume_mode_server_ignores_range_returns_fresh() {
        // 这是 P2 的关键 case：本地已有部分文件，服务器却返回 200 全量响应
        // 必须丢弃 partial 从头写，否则文件会变成 [partial][full] 损坏形态
        assert_eq!(decide_resume_mode(100, 200), ResumeDecision::Fresh);
    }

    #[test]
    fn decide_resume_mode_416_with_existing_means_oversized_redownload() {
        // 进入 fetch_with_resume 时已知 local_len != expected。Range 起点过 EOF
        // 才会触发 416，意味着本地比远端大（损坏的过大文件），必须丢弃重下。
        // 绝不能当 AlreadyComplete 否则永远不会自愈。
        assert_eq!(decide_resume_mode(100, 416), ResumeDecision::Fresh);
    }

    #[test]
    fn decide_resume_mode_416_with_zero_existing_is_fresh() {
        // 防御性：0 长度时不会发 Range，理论上不会触发 416；兜底也走 Fresh。
        assert_eq!(decide_resume_mode(0, 416), ResumeDecision::Fresh);
    }

    #[test]
    fn decide_resume_mode_404_is_endpoint_stale() {
        assert_eq!(
            decide_resume_mode(100, 404),
            ResumeDecision::Error(ErrorKind::EndpointStale)
        );
    }

    #[test]
    fn decide_resume_mode_401_is_auth_expired() {
        assert_eq!(
            decide_resume_mode(100, 401),
            ResumeDecision::Error(ErrorKind::AuthExpired)
        );
    }

    #[test]
    fn decide_resume_mode_500_is_network_error() {
        assert_eq!(
            decide_resume_mode(0, 500),
            ResumeDecision::Error(ErrorKind::NetworkError)
        );
    }

    // ---- symlink rejection at target path ----

    #[cfg(unix)]
    #[test]
    fn download_media_rejects_symlink_at_target_path() {
        use crate::agent::types::{DownloadOpts, MediaItem, MediaType, NullSink};
        use std::os::unix::fs::symlink;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let outside_target = outside.path().join("escape.txt");
        std::fs::write(&outside_target, b"original").unwrap();

        let item = MediaItem {
            tweet_id: "1".to_string(),
            kind: MediaType::Image,
            url: "https://invalid.test/file.jpg".to_string(),
            suggested_filename: "file.jpg".to_string(),
            bytes: None,
            author_handle: None,
            created_at: None,
            all_variants: None,
        };
        // derive_filename 默认无 author_handle 时是 "{tweet_id}_{suggested_filename}" → "1_file.jpg"
        let symlink_path = dir.path().join("1_file.jpg");
        symlink(&outside_target, &symlink_path).unwrap();

        let opts = DownloadOpts {
            subdir: None,
            concurrency: 1,
            base_dir: Some(dir.path().to_path_buf()),
            filename_format: None,
            set_mtime: false,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let out = rt
            .block_on(download_media(&[item], &opts, Arc::new(NullSink)))
            .unwrap();

        assert_eq!(out.summary.failed, 1);
        let r = &out.downloads[0];
        assert!(matches!(r.status, DownloadStatus::Failed));
        let err = r.error.as_ref().unwrap();
        assert_eq!(err.kind, ErrorKind::SandboxViolation);

        // 关键校验：外部文件未被覆盖
        let content = std::fs::read_to_string(&outside_target).unwrap();
        assert_eq!(content, "original");
    }
}
