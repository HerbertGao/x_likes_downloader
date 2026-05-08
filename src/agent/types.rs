use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,
    Video,
    Gif,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MediaItem {
    pub tweet_id: String,
    #[serde(rename = "type")]
    pub kind: MediaType,
    pub url: String,
    pub suggested_filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// 父推文作者 @ 名。`list_likes` 总是填；Agent 手工构造可省略。
    /// 提供时被 `download_media` 默认命名格式 `{author_handle}_{tweet_id}_{suggested_filename}` 使用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_handle: Option<String>,
    /// 父推文发布时间（X 原始 RFC2822 字符串，如 `Thu Apr 06 15:24:15 +0000 2017`）。
    /// 提供时 `download_media` 默认会把文件 mtime 设到这个时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// 仅当 list_likes 以 include_raw=true 调用时携带；下载侧忽略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all_variants: Option<Vec<Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TweetSummary {
    pub id: String,
    pub author_handle: String,
    pub author_display_name: String,
    pub text: String,
    pub created_at: String,
    pub tweet_url: String,
    pub is_retweet: bool,
    pub is_reply: bool,
    pub media: Vec<MediaItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub liked_at: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ListOpts {
    pub all: bool,
    pub since_cursor: Option<String>,
    pub count: Option<u32>,
    pub include_raw: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListOutput {
    pub tweets: Vec<TweetSummary>,
    pub cursor: Option<String>,
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_entries: Option<Vec<Value>>,
}

#[derive(Debug, Clone)]
pub struct DownloadOpts {
    pub subdir: Option<String>,
    pub concurrency: u32,
    /// 注入沙箱 base 目录。`None` 时按既有顺序解析（config.download_sandbox_base_dir → 平台默认）。
    /// 旧 `xld download` 经此字段把 base 切到 `./downloads`，但 sandbox jail 校验仍生效。
    pub base_dir: Option<std::path::PathBuf>,
    /// 文件名模板，仅在为 `Some(_)` 时生效。占位符：`{USERNAME}` → author_handle、`{ID}` → tweet_id。
    /// 替换后空格转下划线，最终文件名为 `<replaced>_<suggested_filename>`。
    /// `None` 时用默认：`{author_handle?}_{tweet_id}_{suggested_filename}`（author_handle 缺则回退）。
    pub filename_format: Option<String>,
    /// 是否把文件 mtime 设到 `MediaItem.created_at` 解析出的时间。默认 true。
    pub set_mtime: bool,
}

impl Default for DownloadOpts {
    fn default() -> Self {
        Self {
            subdir: None,
            concurrency: 4,
            base_dir: None,
            filename_format: None,
            set_mtime: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Downloaded,
    SkippedExisting,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadResult {
    pub tweet_id: String,
    pub url: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub status: DownloadStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<DownloadError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadError {
    pub kind: crate::error::ErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSummary {
    pub total: usize,
    pub downloaded: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadOutput {
    pub downloads: Vec<DownloadResult>,
    pub summary: DownloadSummary,
}

/// Auth status enum mirroring the spec's six-state classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AuthStatus {
    Healthy { checked_at: String },
    AuthExpired,
    EndpointStale,
    RateLimited { retry_after: Option<u64> },
    NetworkError { message: String },
    NotConfigured,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOutput {
    /// True if private_tokens.env was written.
    pub written: bool,
    /// Path of the file that received the imported config.
    pub path: PathBuf,
    /// Whether the imported cURL also contained protocol params (url/features/fieldToggles).
    pub protocol_params_extracted: bool,
}

/// Progress events emitted by `download_media` to a `ProgressSink`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ProgressEvent {
    DownloadStarted {
        total: usize,
        concurrency: u32,
    },
    ItemStarted {
        tweet_id: String,
        url: String,
        index: usize,
        total: usize,
    },
    ItemProgress {
        tweet_id: String,
        bytes_done: u64,
        bytes_total: Option<u64>,
    },
    ItemDone {
        tweet_id: String,
        status: DownloadStatus,
        bytes: u64,
    },
    DownloadFinished {
        summary: DownloadSummary,
    },
}

/// Sink that receives progress events.
///
/// 当前实现：
/// - `NullSink`：丢弃所有事件（lib 函数内部调用、`xld download --json` 模式等）
/// - `IndicatifSink`：人类 CLI 进度条（`main.rs::IndicatifSink`，仅 binary 内部）
/// - `McpProgressSink`：把事件转 MCP `notifications/progress` 推送给客户端
///
/// 历史上有过 `NdjsonStderrSink` 把事件以 newline-delimited JSON 写到 stderr，
/// 它在 v2.0 已被移除（被 MCP progress notification 替代；人类 CLI 用 `IndicatifSink`）。
pub trait ProgressSink: Send + Sync {
    fn emit(&self, event: ProgressEvent);
}

/// No-op sink for callers that don't care about progress.
pub struct NullSink;

impl ProgressSink for NullSink {
    fn emit(&self, _event: ProgressEvent) {}
}

/// Test-only collector sink.
#[cfg(test)]
pub struct VecSink {
    pub events: std::sync::Mutex<Vec<ProgressEvent>>,
}

#[cfg(test)]
impl Default for VecSink {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl VecSink {
    pub fn new() -> Self {
        Self {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }
    pub fn snapshot(&self) -> Vec<ProgressEvent> {
        self.events.lock().unwrap().clone()
    }
}

#[cfg(test)]
impl ProgressSink for VecSink {
    fn emit(&self, event: ProgressEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// 计算 `want` 中“未在 `tweets` 里出现且带媒体”的 tweet ID 列表。
///
/// 一个 ID 被视为命中当且仅当 `tweets` 里存在同 ID 且其 `media` 非空；
/// 其余情况（不在列表里、或在列表里但无媒体）都算 missing。返回值按字典序排序，
/// 便于日志/测试稳定。
pub fn compute_missing_ids(
    want: &std::collections::HashSet<String>,
    tweets: &[TweetSummary],
) -> Vec<String> {
    let found: std::collections::HashSet<String> = tweets
        .iter()
        .filter(|t| want.contains(&t.id) && !t.media.is_empty())
        .map(|t| t.id.clone())
        .collect();
    let mut missing: Vec<String> = want
        .iter()
        .filter(|id| !found.contains(*id))
        .cloned()
        .collect();
    missing.sort();
    missing
}

#[cfg(test)]
mod missing_ids_tests {
    use super::*;
    use std::collections::HashSet;

    fn ts_with_media(id: &str, has_media: bool) -> TweetSummary {
        TweetSummary {
            id: id.to_string(),
            author_handle: "alice".into(),
            author_display_name: "Alice".into(),
            text: "".into(),
            created_at: "".into(),
            tweet_url: format!("https://x.com/alice/status/{}", id),
            is_retweet: false,
            is_reply: false,
            liked_at: None,
            media: if has_media {
                vec![MediaItem {
                    tweet_id: id.into(),
                    kind: MediaType::Image,
                    url: "https://x.tv/x.jpg".into(),
                    suggested_filename: "x.jpg".into(),
                    bytes: None,
                    author_handle: Some("alice".into()),
                    created_at: None,
                    all_variants: None,
                }]
            } else {
                vec![]
            },
        }
    }

    #[test]
    fn compute_missing_ids_all_present_with_media_returns_empty() {
        let want: HashSet<String> = ["1", "2"].into_iter().map(String::from).collect();
        let tweets = vec![ts_with_media("1", true), ts_with_media("2", true)];
        assert!(compute_missing_ids(&want, &tweets).is_empty());
    }

    #[test]
    fn compute_missing_ids_id_not_in_timeline_is_missing() {
        let want: HashSet<String> = ["1", "999"].into_iter().map(String::from).collect();
        let tweets = vec![ts_with_media("1", true)];
        assert_eq!(compute_missing_ids(&want, &tweets), vec!["999"]);
    }

    #[test]
    fn compute_missing_ids_id_with_no_media_is_missing() {
        let want: HashSet<String> = ["1", "2"].into_iter().map(String::from).collect();
        let tweets = vec![ts_with_media("1", true), ts_with_media("2", false)];
        assert_eq!(compute_missing_ids(&want, &tweets), vec!["2"]);
    }

    #[test]
    fn compute_missing_ids_all_missing_returns_all_sorted() {
        let want: HashSet<String> = ["3", "1", "2"].into_iter().map(String::from).collect();
        let tweets: Vec<TweetSummary> = vec![];
        assert_eq!(compute_missing_ids(&want, &tweets), vec!["1", "2", "3"]);
    }
}
