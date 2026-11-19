//! `DOWNLOAD_RECORD` 与 MCP 两阶段记录确认。
//!
//! Legacy `xld download` 仍在「任一媒体成功」时自动追加 tweet ID。MCP `download_media`
//! 不写这份记录：它只把本次下载事实落成可重启恢复的批次。用户经客户端原生确认卡明确同意后，
//! Agent 再调用 `commit_download_record`，按批次里的历史事实追加选中的 ID。
//!
//! 服务端无法证明原生确认卡已被批准。本模块不接受、也不检查 `confirmed` 之类的参数。

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::Utc;
use fs2::FileExt;
use sha2::{Digest, Sha256};

use crate::agent::types::{
    DownloadOutput, DownloadResult, DownloadStatus, DownloadSummary, MediaItem, MediaType,
};
use crate::config::Config;
use crate::error::{ErrorKind, ErrorPayload};

/// 新流程的完成口径：只看本次调用提交的媒体项，不代表原推文的全部媒体。
pub const COVERAGE_SUPPLIED_ITEMS: &str = "supplied_items";

const COVERAGE_LIMIT: &str = "只表示该推文在本次调用中提交的媒体项全部为 downloaded 或 skipped_existing。不表示原推文的全部媒体都已下载。写入 DOWNLOAD_RECORD 后，今后 legacy 点赞下载会跳过这些 tweet ID。本记录不证明文件仍在原路径，也不证明它们在 legacy DOWNLOAD_DIR 中。";

const SCHEMA_VERSION: u32 = 1;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(1);
static BATCH_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 记录文件与批次目录。批次目录是记录文件所在目录下的 `download_record_batches`。
#[derive(Debug, Clone)]
pub struct RecordPaths {
    pub record_path: PathBuf,
    pub batches_dir: PathBuf,
}

impl RecordPaths {
    pub fn from_record_path(record_path: impl Into<PathBuf>) -> Self {
        let record_path = record_path.into();
        let parent = record_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            record_path,
            batches_dir: parent.join("download_record_batches"),
        }
    }

    pub fn from_config(config: &Config) -> Self {
        Self::from_record_path(&config.download_record)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendStats {
    pub written: Vec<String>,
    pub already_present: Vec<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RecordBatchState {
    Pending,
    Committed,
    NothingToRecord,
    /// 只出现在工具响应里：批次没能落盘，不能提交。
    Unavailable,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ExclusionReason {
    /// 本次提交的媒体项没有全部成功（含部分失败、取消、缺失）。
    IncompleteSuppliedItems,
    AllFailed,
    /// 本次提交的项全部取消或中断，没有成功项。
    Cancelled,
    /// 下载结果无法与提交的媒体项配对，不能当作成功。
    UnpairedResult,
    /// ID 不能安全地作为一行写入记录文件。
    UnsafeTweetId,
    NoItems,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct CandidateTweet {
    pub tweet_id: String,
    pub media_ids: Vec<String>,
    pub supplied_count: u32,
    pub succeeded_count: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ExcludedTweet {
    pub tweet_id: String,
    pub reason: ExclusionReason,
    pub supplied_count: u32,
    pub succeeded_count: u32,
    pub failed_count: u32,
    pub cancelled_count: u32,
    pub missing_count: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ItemFact {
    pub media_id: String,
    pub media_type: String,
    /// 不是媒体 URL。看起来像 URL 的文件名会改成占位，避免把会过期的地址写入批次。
    pub suggested_filename: String,
    pub status: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct TweetFact {
    pub tweet_id: String,
    pub eligible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<ExclusionReason>,
    pub supplied_count: u32,
    pub succeeded_count: u32,
    pub failed_count: u32,
    pub cancelled_count: u32,
    pub missing_count: u32,
    pub items: Vec<ItemFact>,
}

/// MCP `download_media` 的成功结果。`downloads` / `summary` 与原 `DownloadOutput` 相同，
/// `record_confirmation` 是追加字段。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct McpDownloadMediaResult {
    pub downloads: Vec<DownloadResult>,
    pub summary: DownloadSummary,
    pub record_confirmation: RecordConfirmation,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct RecordConfirmation {
    pub state: RecordBatchState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<String>,
    pub coverage: String,
    /// 有成功候选时为 true，表示 Agent 应展示原生确认卡。不是用户已经同意。
    pub agent_must_ask_user: bool,
    /// 恒为 false。工具返回值不能证明用户许可。
    pub user_permission_verified: bool,
    pub candidates: Vec<CandidateTweet>,
    pub exclusions: Vec<ExcludedTweet>,
    pub guidance: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchSummary {
    pub batch_id: String,
    pub created_at: String,
    pub state: RecordBatchState,
    pub coverage: String,
    /// 尚未写入记录的成功候选。
    pub candidate_tweet_ids: Vec<String>,
    pub committed_tweet_ids: Vec<String>,
    pub exclusion_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BatchDetail {
    pub batch_id: String,
    pub created_at: String,
    pub state: RecordBatchState,
    pub coverage: String,
    pub coverage_limit: String,
    pub agent_must_ask_user: bool,
    pub user_permission_verified: bool,
    pub candidates: Vec<CandidateTweet>,
    pub exclusions: Vec<ExcludedTweet>,
    pub committed_tweet_ids: Vec<String>,
    pub tweets: Vec<TweetFact>,
    pub guidance: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct GetDownloadRecordBatchOutput {
    pub pending: Vec<BatchSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch: Option<BatchDetail>,
    pub user_permission_verified: bool,
    pub guidance: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct CommitDownloadRecordOutput {
    pub batch_id: String,
    pub written_tweet_ids: Vec<String>,
    pub already_recorded_tweet_ids: Vec<String>,
    pub remaining_candidate_tweet_ids: Vec<String>,
    pub state: RecordBatchState,
    pub record_format: String,
    pub user_permission_verified: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct PersistedBatch {
    schema_version: u32,
    batch_id: String,
    created_at: String,
    state: RecordBatchState,
    coverage: String,
    tweets: Vec<TweetFact>,
    committed_tweet_ids: Vec<String>,
}

struct RecordLock {
    file: Option<File>,
}

impl Drop for RecordLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            let _ = FileExt::unlock(&file);
        }
    }
}

/// Legacy `xld download` 的记录口径：任一媒体 `downloaded` 或 `skipped_existing` 即记录该推文。
/// `failed` 与 `cancelled` 本身不构成记录。MCP 批次不使用这条规则。
pub fn legacy_recordable_tweet_ids(downloads: &[DownloadResult]) -> HashSet<String> {
    let mut succeeded = HashSet::new();
    for result in downloads {
        match result.status {
            DownloadStatus::Downloaded | DownloadStatus::SkippedExisting => {
                succeeded.insert(result.tweet_id.clone());
            }
            DownloadStatus::Failed | DownloadStatus::Cancelled => {}
        }
    }
    succeeded
}

/// 在独占锁内重读记录文件，只追加尚不存在的安全 ID，并用同目录临时文件加 rename 替换。
/// 崩溃发生在 rename 之前时原文件保持不变；重试会再次重读，因此不会重复追加，也不会丢掉已有 ID。
pub fn append_new_ids(record_path: &Path, ids: &[String]) -> Result<AppendStats, ErrorPayload> {
    let lock = lock_record(record_path)?;
    append_ids_locked(&lock, record_path, ids)
}

pub fn read_record_ids(record_path: &Path) -> Result<Vec<String>, ErrorPayload> {
    read_record_lines(record_path)
}

/// 根据本次下载结果生成批次并落盘。不写 `DOWNLOAD_RECORD`，不访问网络。
pub fn capture_download_batch(
    paths: &RecordPaths,
    items: &[MediaItem],
    output: &DownloadOutput,
) -> Result<McpDownloadMediaResult, ErrorPayload> {
    let confirmation = if items.is_empty() && output.downloads.is_empty() {
        RecordConfirmation {
            state: RecordBatchState::NothingToRecord,
            batch_id: None,
            coverage: COVERAGE_SUPPLIED_ITEMS.to_string(),
            agent_must_ask_user: false,
            user_permission_verified: false,
            candidates: Vec::new(),
            exclusions: Vec::new(),
            guidance: nothing_guidance(),
            error: None,
        }
    } else {
        let batch = build_batch(items, &output.downloads);
        persist_batch(paths, &batch)?;
        confirmation_from_batch(&batch, None)
    };
    Ok(McpDownloadMediaResult {
        downloads: output.downloads.clone(),
        summary: output.summary.clone(),
        record_confirmation: confirmation,
    })
}

pub fn capture_unavailable(
    output: &DownloadOutput,
    message: impl Into<String>,
) -> McpDownloadMediaResult {
    let message = message.into();
    McpDownloadMediaResult {
        downloads: output.downloads.clone(),
        summary: output.summary.clone(),
        record_confirmation: RecordConfirmation {
            state: RecordBatchState::Unavailable,
            batch_id: None,
            coverage: COVERAGE_SUPPLIED_ITEMS.to_string(),
            agent_must_ask_user: false,
            user_permission_verified: false,
            candidates: Vec::new(),
            exclusions: Vec::new(),
            guidance: unavailable_guidance(),
            error: Some(message),
        },
    }
}

/// 把批次中用户选中的成功候选写入记录。拒绝候选之外的 ID，整次不写。
/// 不读下载路径，不访问网络。
pub fn commit_download_record(
    paths: &RecordPaths,
    batch_id: &str,
    tweet_ids: &[String],
) -> Result<CommitDownloadRecordOutput, ErrorPayload> {
    validate_batch_id(batch_id)?;
    if tweet_ids.is_empty() {
        return Err(invalid(
            "tweet_ids 为空。没有选中的 ID 时不要写入；用户拒绝或未回答时不要调用提交",
        ));
    }
    let mut selected = Vec::new();
    let mut seen = HashSet::new();
    for id in tweet_ids {
        if !is_safe_tweet_id(id) {
            return Err(invalid(
                "tweet_ids 含非法 ID（只允许 1–32 位数字）。已拒绝整次写入",
            ));
        }
        if seen.insert(id.clone()) {
            selected.push(id.clone());
        }
    }

    let _lock = lock_record(&paths.record_path)?;
    let path = batch_file(paths, batch_id)?;
    ensure_regular_file(&path)?;
    let mut batch = load_persisted(&path)?;
    if batch.batch_id != batch_id {
        return Err(invalid("批次内容与 batch_id 不一致，已拒绝写入"));
    }
    let eligible = eligible_ids(&batch);
    let rejected: Vec<&str> = selected
        .iter()
        .filter(|id| !eligible.contains(id.as_str()))
        .map(String::as_str)
        .collect();
    if !rejected.is_empty() {
        return Err(invalid(
            "请求包含该批次成功候选之外的 tweet ID，已拒绝整次写入",
        ));
    }

    let stats = append_ids_locked(&_lock, &paths.record_path, &selected)?;
    for id in &selected {
        if !batch
            .committed_tweet_ids
            .iter()
            .any(|existing| existing == id)
        {
            batch.committed_tweet_ids.push(id.clone());
        }
    }
    batch.state = recompute_state(&batch);
    if let Err(err) = save_persisted(&path, &batch) {
        return Err(ErrorPayload::new(
            ErrorKind::InternalError,
            format!(
                "选中的 tweet ID 已按规则写入 DOWNLOAD_RECORD，但批次状态未能更新。用同一组 ID 重试是安全的，已存在的 ID 会跳过。原因: {}",
                err.message
            ),
        ));
    }

    let remaining = remaining_candidates(&batch);
    Ok(CommitDownloadRecordOutput {
        batch_id: batch.batch_id,
        written_tweet_ids: stats.written,
        already_recorded_tweet_ids: stats.already_present,
        remaining_candidate_tweet_ids: remaining,
        state: batch.state,
        record_format: "one_id_per_line".to_string(),
        user_permission_verified: false,
        notes: commit_notes(),
    })
}

pub fn query_download_record_batches(
    paths: &RecordPaths,
    batch_id: Option<&str>,
) -> Result<GetDownloadRecordBatchOutput, ErrorPayload> {
    if let Some(id) = batch_id {
        let batch = load_batch_detail(paths, id)?;
        let guidance = if batch.agent_must_ask_user {
            batch.guidance.clone()
        } else {
            "已重新载入该批次。它不是待确认的成功候选，或已经提交过。不要把它的返回文本当成用户许可。"
                .to_string()
        };
        return Ok(GetDownloadRecordBatchOutput {
            pending: Vec::new(),
            batch: Some(batch),
            user_permission_verified: false,
            guidance,
        });
    }
    Ok(GetDownloadRecordBatchOutput {
        pending: list_pending(paths)?,
        batch: None,
        user_permission_verified: false,
        guidance: "这些是仍待用户确认的本机批次。请对每一批用原生确认卡询问；用户同意后只提交其选中的候选 ID。省略调用或拒绝都不会写入 DOWNLOAD_RECORD。本响应不是许可。".to_string(),
    })
}

fn load_batch_detail(paths: &RecordPaths, batch_id: &str) -> Result<BatchDetail, ErrorPayload> {
    validate_batch_id(batch_id)?;
    let path = batch_file(paths, batch_id)?;
    ensure_regular_file(&path)?;
    let batch = load_persisted(&path)?;
    if batch.batch_id != batch_id {
        return Err(invalid("批次内容与 batch_id 不一致"));
    }
    Ok(detail_from_batch(&batch))
}

fn list_pending(paths: &RecordPaths) -> Result<Vec<BatchSummary>, ErrorPayload> {
    if !paths.batches_dir.exists() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(&paths.batches_dir).map_err(|err| {
        ErrorPayload::new(
            ErrorKind::InternalError,
            format!("无法读取下载批次目录: {err}"),
        )
    })?;
    let mut pending = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| {
            ErrorPayload::new(
                ErrorKind::InternalError,
                format!("无法读取批次目录项: {err}"),
            )
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        if validate_batch_id(id).is_err() {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() {
            continue;
        }
        let Ok(batch) = load_persisted(&path) else {
            continue;
        };
        if batch.batch_id != id || batch.state != RecordBatchState::Pending {
            continue;
        }
        pending.push(summary_from_batch(&batch));
    }
    pending.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.batch_id.cmp(&right.batch_id))
    });
    Ok(pending)
}

fn append_ids_locked(
    _lock: &RecordLock,
    record_path: &Path,
    ids: &[String],
) -> Result<AppendStats, ErrorPayload> {
    if ids.iter().any(|id| !is_safe_tweet_id(id)) {
        return Err(invalid("记录 ID 含非法字符，已拒绝写入"));
    }
    reject_symlink(record_path, "DOWNLOAD_RECORD 是符号链接，已拒绝写入")?;
    let mut existing = read_record_lines(record_path)?;
    let mut have: HashSet<String> = existing.iter().cloned().collect();
    let mut written = Vec::new();
    let mut already_present = Vec::new();
    for id in ids {
        if have.contains(id) {
            already_present.push(id.clone());
            continue;
        }
        have.insert(id.clone());
        existing.push(id.clone());
        written.push(id.clone());
    }
    if !written.is_empty() {
        atomic_write(record_path, &render_lines(&existing))?;
    }
    Ok(AppendStats {
        written,
        already_present,
    })
}

fn lock_record(record_path: &Path) -> Result<RecordLock, ErrorPayload> {
    let lock_path = lock_path_for(record_path);
    if let Some(parent) = lock_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|err| {
                ErrorPayload::new(ErrorKind::InternalError, format!("无法创建记录目录: {err}"))
            })?;
        }
    }
    reject_symlink(&lock_path, "记录锁路径是符号链接，已拒绝")?;
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|err| {
            ErrorPayload::new(ErrorKind::InternalError, format!("无法打开记录锁: {err}"))
        })?;
    FileExt::lock_exclusive(&file).map_err(|err| {
        ErrorPayload::new(
            ErrorKind::InternalError,
            format!("无法锁定 DOWNLOAD_RECORD: {err}"),
        )
    })?;
    Ok(RecordLock { file: Some(file) })
}

fn lock_path_for(record_path: &Path) -> PathBuf {
    let mut name = record_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("download_record"))
        .to_os_string();
    name.push(".lock");
    match record_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

fn read_record_lines(path: &Path) -> Result<Vec<String>, ErrorPayload> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    reject_symlink(path, "DOWNLOAD_RECORD 是符号链接，已拒绝读取")?;
    let content = fs::read_to_string(path).map_err(|err| {
        ErrorPayload::new(
            ErrorKind::InternalError,
            format!("无法读取 DOWNLOAD_RECORD: {err}"),
        )
    })?;
    let mut lines = Vec::new();
    let mut seen = HashSet::new();
    for line in content.lines() {
        let id = line.trim();
        if id.is_empty() {
            continue;
        }
        if seen.insert(id.to_string()) {
            lines.push(id.to_string());
        }
    }
    Ok(lines)
}

fn render_lines(lines: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for line in lines {
        bytes.extend_from_slice(line.as_bytes());
        bytes.push(b'\n');
    }
    bytes
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ErrorPayload> {
    reject_symlink(path, "目标路径是符号链接，已拒绝替换")?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|err| {
        ErrorPayload::new(ErrorKind::InternalError, format!("无法创建目录: {err}"))
    })?;
    let file_name = path
        .file_name()
        .ok_or_else(|| ErrorPayload::new(ErrorKind::InternalError, "目标路径缺少文件名"))?;
    let mut tmp_name = file_name.to_os_string();
    tmp_name.push(format!(
        ".{}.{}.tmp",
        std::process::id(),
        TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = parent.join(tmp_name);
    let write_result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(err) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(ErrorPayload::new(
            ErrorKind::InternalError,
            format!("写入临时文件失败: {err}"),
        ));
    }
    if let Err(err) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(ErrorPayload::new(
            ErrorKind::InternalError,
            format!("替换记录文件失败: {err}"),
        ));
    }
    Ok(())
}

fn build_batch(items: &[MediaItem], results: &[DownloadResult]) -> PersistedBatch {
    let mut order = Vec::new();
    let mut groups: HashMap<String, Vec<Slot>> = HashMap::new();
    for item in items {
        if !groups.contains_key(&item.tweet_id) {
            order.push(item.tweet_id.clone());
        }
        let ordinal = groups.get(&item.tweet_id).map(Vec::len).unwrap_or(0) as u32;
        groups.entry(item.tweet_id.clone()).or_default().push(Slot {
            ordinal,
            item: item.clone(),
            result: None,
        });
    }

    let mut unpaired: HashMap<String, Vec<DownloadResult>> = HashMap::new();
    for result in results {
        let matched = groups.get_mut(&result.tweet_id).and_then(|slots| {
            slots
                .iter_mut()
                .find(|slot| slot.result.is_none() && slot.item.url == result.url)
        });
        if let Some(slot) = matched {
            slot.result = Some(result.clone());
        } else {
            if !groups.contains_key(&result.tweet_id) && !unpaired.contains_key(&result.tweet_id) {
                order.push(result.tweet_id.clone());
            }
            unpaired
                .entry(result.tweet_id.clone())
                .or_default()
                .push(result.clone());
        }
    }

    let mut tweets = Vec::new();
    for tweet_id in order {
        let slots = groups.remove(&tweet_id).unwrap_or_default();
        let extra = unpaired.remove(&tweet_id).unwrap_or_default();
        tweets.push(tweet_fact(&tweet_id, &slots, &extra));
    }

    let state = if tweets.iter().any(|tweet| tweet.eligible) {
        RecordBatchState::Pending
    } else {
        RecordBatchState::NothingToRecord
    };
    PersistedBatch {
        schema_version: SCHEMA_VERSION,
        batch_id: new_batch_id(),
        created_at: Utc::now().to_rfc3339(),
        state,
        coverage: COVERAGE_SUPPLIED_ITEMS.to_string(),
        tweets,
        committed_tweet_ids: Vec::new(),
    }
}

struct Slot {
    ordinal: u32,
    item: MediaItem,
    result: Option<DownloadResult>,
}

fn tweet_fact(tweet_id: &str, slots: &[Slot], unpaired: &[DownloadResult]) -> TweetFact {
    let mut items = Vec::new();
    let mut succeeded = 0u32;
    let mut failed = 0u32;
    let mut cancelled = 0u32;
    let mut missing = 0u32;
    for slot in slots {
        let (status, bytes) = match &slot.result {
            Some(result) => (status_name(&result.status), result.bytes),
            None => ("missing", 0),
        };
        match status {
            "downloaded" | "skipped_existing" => succeeded += 1,
            "failed" => failed += 1,
            "cancelled" => cancelled += 1,
            _ => missing += 1,
        }
        items.push(ItemFact {
            media_id: media_id(
                tweet_id,
                slot.ordinal,
                media_type_name(&slot.item.kind),
                &slot.item.suggested_filename,
            ),
            media_type: media_type_name(&slot.item.kind).to_string(),
            suggested_filename: stored_filename(&slot.item.suggested_filename),
            status: status.to_string(),
            bytes,
        });
    }
    for (index, result) in unpaired.iter().enumerate() {
        items.push(ItemFact {
            media_id: unpaired_media_id(tweet_id, index as u32, &result.url, &result.status),
            media_type: "unknown".to_string(),
            suggested_filename: "(unpaired)".to_string(),
            status: status_name(&result.status).to_string(),
            bytes: result.bytes,
        });
    }

    let supplied = slots.len() as u32;
    let (eligible, reason) = classify_tweet(
        tweet_id,
        supplied,
        succeeded,
        failed,
        cancelled,
        missing,
        unpaired.len() as u32,
    );
    TweetFact {
        tweet_id: tweet_id.to_string(),
        eligible,
        reason,
        supplied_count: supplied,
        succeeded_count: succeeded,
        failed_count: failed,
        cancelled_count: cancelled,
        missing_count: missing,
        items,
    }
}

fn classify_tweet(
    tweet_id: &str,
    supplied: u32,
    succeeded: u32,
    failed: u32,
    cancelled: u32,
    missing: u32,
    unpaired: u32,
) -> (bool, Option<ExclusionReason>) {
    if !is_safe_tweet_id(tweet_id) {
        return (false, Some(ExclusionReason::UnsafeTweetId));
    }
    if supplied == 0 {
        let reason = if unpaired > 0 {
            ExclusionReason::UnpairedResult
        } else {
            ExclusionReason::NoItems
        };
        return (false, Some(reason));
    }
    let complete =
        unpaired == 0 && missing == 0 && failed == 0 && cancelled == 0 && succeeded == supplied;
    if complete {
        return (true, None);
    }
    if succeeded == 0 && failed > 0 && cancelled == 0 && missing == 0 && unpaired == 0 {
        return (false, Some(ExclusionReason::AllFailed));
    }
    if succeeded == 0 && cancelled > 0 && failed == 0 && missing == 0 && unpaired == 0 {
        return (false, Some(ExclusionReason::Cancelled));
    }
    if unpaired > 0 && succeeded == 0 && failed == 0 && cancelled == 0 && missing == 0 {
        return (false, Some(ExclusionReason::UnpairedResult));
    }
    (false, Some(ExclusionReason::IncompleteSuppliedItems))
}

fn confirmation_from_batch(batch: &PersistedBatch, error: Option<String>) -> RecordConfirmation {
    let candidates = candidates_of(batch, false);
    let exclusions = exclusions_of(batch);
    let ask = batch.state == RecordBatchState::Pending && !candidates.is_empty();
    let guidance = if ask {
        pending_guidance(&batch.batch_id, &candidates)
    } else if error.is_some() {
        unavailable_guidance()
    } else {
        nothing_guidance()
    };
    RecordConfirmation {
        state: batch.state,
        batch_id: Some(batch.batch_id.clone()),
        coverage: batch.coverage.clone(),
        agent_must_ask_user: ask,
        user_permission_verified: false,
        candidates,
        exclusions,
        guidance,
        error,
    }
}

fn detail_from_batch(batch: &PersistedBatch) -> BatchDetail {
    let candidates = candidates_of(batch, true);
    let ask = batch.state == RecordBatchState::Pending && !candidates.is_empty();
    let guidance = if ask {
        pending_guidance(&batch.batch_id, &candidates)
    } else {
        nothing_guidance()
    };
    BatchDetail {
        batch_id: batch.batch_id.clone(),
        created_at: batch.created_at.clone(),
        state: batch.state,
        coverage: batch.coverage.clone(),
        coverage_limit: COVERAGE_LIMIT.to_string(),
        agent_must_ask_user: ask,
        user_permission_verified: false,
        candidates,
        exclusions: exclusions_of(batch),
        committed_tweet_ids: batch.committed_tweet_ids.clone(),
        tweets: batch.tweets.clone(),
        guidance,
    }
}

fn summary_from_batch(batch: &PersistedBatch) -> BatchSummary {
    BatchSummary {
        batch_id: batch.batch_id.clone(),
        created_at: batch.created_at.clone(),
        state: batch.state,
        coverage: batch.coverage.clone(),
        candidate_tweet_ids: candidates_of(batch, true)
            .into_iter()
            .map(|candidate| candidate.tweet_id)
            .collect(),
        committed_tweet_ids: batch.committed_tweet_ids.clone(),
        exclusion_count: batch.tweets.iter().filter(|tweet| !tweet.eligible).count(),
    }
}

fn candidates_of(batch: &PersistedBatch, skip_committed: bool) -> Vec<CandidateTweet> {
    batch
        .tweets
        .iter()
        .filter(|tweet| tweet.eligible)
        .filter(|tweet| {
            !skip_committed
                || !batch
                    .committed_tweet_ids
                    .iter()
                    .any(|id| id == &tweet.tweet_id)
        })
        .map(|tweet| CandidateTweet {
            tweet_id: tweet.tweet_id.clone(),
            media_ids: tweet
                .items
                .iter()
                .map(|item| item.media_id.clone())
                .collect(),
            supplied_count: tweet.supplied_count,
            succeeded_count: tweet.succeeded_count,
        })
        .collect()
}

fn exclusions_of(batch: &PersistedBatch) -> Vec<ExcludedTweet> {
    batch
        .tweets
        .iter()
        .filter(|tweet| !tweet.eligible)
        .map(|tweet| ExcludedTweet {
            tweet_id: tweet.tweet_id.clone(),
            reason: tweet
                .reason
                .unwrap_or(ExclusionReason::IncompleteSuppliedItems),
            supplied_count: tweet.supplied_count,
            succeeded_count: tweet.succeeded_count,
            failed_count: tweet.failed_count,
            cancelled_count: tweet.cancelled_count,
            missing_count: tweet.missing_count,
        })
        .collect()
}

fn eligible_ids(batch: &PersistedBatch) -> HashSet<String> {
    batch
        .tweets
        .iter()
        .filter(|tweet| tweet.eligible)
        .map(|tweet| tweet.tweet_id.clone())
        .collect()
}

fn remaining_candidates(batch: &PersistedBatch) -> Vec<String> {
    candidates_of(batch, true)
        .into_iter()
        .map(|candidate| candidate.tweet_id)
        .collect()
}

fn recompute_state(batch: &PersistedBatch) -> RecordBatchState {
    let eligible = eligible_ids(batch);
    if eligible.is_empty() {
        RecordBatchState::NothingToRecord
    } else if eligible
        .iter()
        .all(|id| batch.committed_tweet_ids.iter().any(|done| done == id))
    {
        RecordBatchState::Committed
    } else {
        RecordBatchState::Pending
    }
}

fn persist_batch(paths: &RecordPaths, batch: &PersistedBatch) -> Result<(), ErrorPayload> {
    let path = batch_file(paths, &batch.batch_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            ErrorPayload::new(ErrorKind::InternalError, format!("无法创建批次目录: {err}"))
        })?;
    }
    save_persisted(&path, batch)
}

fn save_persisted(path: &Path, batch: &PersistedBatch) -> Result<(), ErrorPayload> {
    let mut bytes = serde_json::to_vec_pretty(batch).map_err(|err| {
        ErrorPayload::new(
            ErrorKind::InternalError,
            format!("无法序列化下载批次: {err}"),
        )
    })?;
    bytes.push(b'\n');
    atomic_write(path, &bytes)
}

fn load_persisted(path: &Path) -> Result<PersistedBatch, ErrorPayload> {
    let bytes = fs::read(path).map_err(|err| {
        ErrorPayload::new(ErrorKind::InternalError, format!("无法读取下载批次: {err}"))
    })?;
    let batch: PersistedBatch = serde_json::from_slice(&bytes).map_err(|_| {
        ErrorPayload::new(
            ErrorKind::InvalidArgument,
            "下载批次已损坏，不能用于写入记录",
        )
    })?;
    if batch.schema_version != SCHEMA_VERSION {
        return Err(invalid("下载批次版本不受支持"));
    }
    if validate_batch_id(&batch.batch_id).is_err() {
        return Err(invalid("下载批次内的 batch_id 非法"));
    }
    Ok(batch)
}

fn batch_file(paths: &RecordPaths, batch_id: &str) -> Result<PathBuf, ErrorPayload> {
    validate_batch_id(batch_id)?;
    let name = format!("{batch_id}.json");
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(invalid("batch_id 非法"));
    }
    Ok(paths.batches_dir.join(name))
}

fn validate_batch_id(id: &str) -> Result<(), ErrorPayload> {
    let ok = id.len() == 32
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if ok {
        Ok(())
    } else {
        Err(invalid(
            "batch_id 非法。只接受服务端生成的 32 位小写十六进制，拒绝路径",
        ))
    }
}

fn ensure_regular_file(path: &Path) -> Result<(), ErrorPayload> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(invalid("批次路径是符号链接，已拒绝")),
        Ok(meta) if meta.is_file() => Ok(()),
        Ok(_) => Err(invalid("未找到该下载批次")),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(invalid("未找到该下载批次")),
        Err(err) => Err(ErrorPayload::new(
            ErrorKind::InternalError,
            format!("无法检查下载批次: {err}"),
        )),
    }
}

fn reject_symlink(path: &Path, message: &str) -> Result<(), ErrorPayload> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.file_type().is_symlink() {
            return Err(ErrorPayload::new(ErrorKind::InternalError, message));
        }
    }
    Ok(())
}

fn is_safe_tweet_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|byte| byte.is_ascii_digit())
}

fn new_batch_id() -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"xld-download-batch-v1");
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(BATCH_COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    hasher.update(nanos.to_le_bytes());
    let digest = hasher.finalize();
    let mut id = String::with_capacity(32);
    for byte in digest.iter().take(16) {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

fn media_id(tweet_id: &str, ordinal: u32, media_type: &str, filename: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(tweet_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(ordinal.to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(media_type.as_bytes());
    hasher.update(b"\n");
    hasher.update(filename.as_bytes());
    hex_digest(&hasher.finalize())
}

fn unpaired_media_id(tweet_id: &str, index: u32, url: &str, status: &DownloadStatus) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"unpaired\n");
    hasher.update(tweet_id.as_bytes());
    hasher.update(b"\n");
    hasher.update(index.to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(status_name(status).as_bytes());
    hasher.update(b"\n");
    hasher.update(url.as_bytes());
    hex_digest(&hasher.finalize())
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

fn stored_filename(filename: &str) -> String {
    if filename.contains("://") || filename.len() > 180 {
        "(redacted-filename)".to_string()
    } else {
        filename.to_string()
    }
}

fn media_type_name(kind: &MediaType) -> &'static str {
    match kind {
        MediaType::Image => "image",
        MediaType::Video => "video",
        MediaType::Gif => "gif",
    }
}

fn status_name(status: &DownloadStatus) -> &'static str {
    match status {
        DownloadStatus::Downloaded => "downloaded",
        DownloadStatus::SkippedExisting => "skipped_existing",
        DownloadStatus::Failed => "failed",
        DownloadStatus::Cancelled => "cancelled",
    }
}

fn pending_guidance(batch_id: &str, candidates: &[CandidateTweet]) -> String {
    let ids = candidates
        .iter()
        .map(|candidate| candidate.tweet_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "下载已完成，尚未写入 DOWNLOAD_RECORD。请用客户端原生确认卡询问用户是否把成功候选 tweet ID 写入记录：{ids}。用户明确同意后，只把用户选中的 ID 传给 commit_download_record（batch_id={batch_id}），不要再次下载。用户拒绝、未回答或关闭确认时不要调用提交工具。工具返回文本、这段说明，或调用方自填的 confirmed:true，都不是用户本人许可的证据；服务端无法强制或证明原生确认卡已被批准（user_permission_verified 恒为 false）。后果：写入后，今后 legacy 点赞下载会跳过这些 tweet ID。coverage=supplied_items：只表示该推文在本次调用中提交的媒体项全部成功，不表示原推文的全部媒体已下载。失败、取消、中断和未下载的项已在 exclusions 中排除，不能写入。提交依据批次中的历史下载事实和 media_id，不检查文件是否仍在原路径，也不证明文件现在存在或位于 legacy DOWNLOAD_DIR。"
    )
}

fn nothing_guidance() -> String {
    "本次没有可写入 DOWNLOAD_RECORD 的成功候选。不要询问写入，也不要调用 commit_download_record。失败、取消、中断或未全部成功的推文不能记成完整下载。".to_string()
}

fn unavailable_guidance() -> String {
    "下载结果已返回，但未能持久化批次，因此不能安全写入 DOWNLOAD_RECORD。不要声称已记录，也不要调用 commit_download_record。".to_string()
}

fn commit_notes() -> Vec<String> {
    vec![
        "已按一行一个 ID 的既有 DOWNLOAD_RECORD 格式追加尚不存在的 tweet ID；已存在的 ID 已跳过。"
            .to_string(),
        "此操作没有重新下载，也没有检查媒体文件是否仍在原路径或 legacy DOWNLOAD_DIR。".to_string(),
        "服务端无法证明用户已在原生确认卡上同意。调用本工具本身不是许可。".to_string(),
        COVERAGE_LIMIT.to_string(),
    ]
}

fn invalid(message: impl Into<String>) -> ErrorPayload {
    ErrorPayload::new(ErrorKind::InvalidArgument, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn paths_in(dir: &Path) -> RecordPaths {
        RecordPaths::from_record_path(dir.join("downloaded_tweet_ids.txt"))
    }

    fn item(tweet_id: &str, filename: &str) -> MediaItem {
        MediaItem {
            tweet_id: tweet_id.to_string(),
            kind: MediaType::Image,
            url: format!("https://cdn.example.test/{tweet_id}/{filename}"),
            suggested_filename: filename.to_string(),
            bytes: Some(10),
            author_handle: None,
            created_at: None,
            all_variants: None,
        }
    }

    fn make_result(
        tweet_id: &str,
        filename: &str,
        status: DownloadStatus,
        path: PathBuf,
    ) -> DownloadResult {
        DownloadResult {
            tweet_id: tweet_id.to_string(),
            url: format!("https://cdn.example.test/{tweet_id}/{filename}"),
            path,
            bytes: if status == DownloadStatus::Downloaded {
                10
            } else {
                0
            },
            status,
            error: None,
        }
    }

    fn result_ok(dir: &Path, tweet_id: &str, filename: &str) -> DownloadResult {
        make_result(
            tweet_id,
            filename,
            DownloadStatus::Downloaded,
            dir.join(filename),
        )
    }

    fn output(results: Vec<DownloadResult>) -> DownloadOutput {
        let mut summary = DownloadSummary {
            total: results.len(),
            downloaded: 0,
            skipped: 0,
            failed: 0,
            cancelled: 0,
        };
        for result in &results {
            match result.status {
                DownloadStatus::Downloaded => summary.downloaded += 1,
                DownloadStatus::SkippedExisting => summary.skipped += 1,
                DownloadStatus::Failed => summary.failed += 1,
                DownloadStatus::Cancelled => summary.cancelled += 1,
            }
        }
        DownloadOutput {
            downloads: results,
            summary,
        }
    }

    fn capture(
        dir: &Path,
        items: &[MediaItem],
        results: Vec<DownloadResult>,
    ) -> McpDownloadMediaResult {
        capture_download_batch(&paths_in(dir), items, &output(results)).unwrap()
    }

    #[test]
    fn capture_does_not_write_download_record() {
        let dir = tempfile::tempdir().unwrap();
        let captured = capture(
            dir.path(),
            &[item("1001", "a.jpg"), item("1001", "b.jpg")],
            vec![
                result_ok(dir.path(), "1001", "a.jpg"),
                result_ok(dir.path(), "1001", "b.jpg"),
            ],
        );
        assert!(!paths_in(dir.path()).record_path.exists());
        assert_eq!(
            captured.record_confirmation.state,
            RecordBatchState::Pending
        );
        assert!(captured.record_confirmation.agent_must_ask_user);
        assert!(!captured.record_confirmation.user_permission_verified);
        assert!(captured.record_confirmation.batch_id.is_some());
        assert_eq!(captured.record_confirmation.candidates.len(), 1);
        assert!(captured.record_confirmation.guidance.contains("原生确认卡"));
        assert!(captured
            .record_confirmation
            .guidance
            .contains("confirmed:true"));
        assert!(captured
            .record_confirmation
            .guidance
            .contains("DOWNLOAD_RECORD"));
    }

    #[test]
    fn complete_success_partial_failure_cancel_and_unselected() {
        let dir = tempfile::tempdir().unwrap();
        let items = vec![
            item("1001", "a.jpg"),
            item("1001", "b.jpg"),
            item("1002", "c.jpg"),
            item("1002", "d.jpg"),
            item("1003", "e.jpg"),
            item("1004", "f.jpg"),
            item("1005", "g.jpg"),
        ];
        let results = vec![
            result_ok(dir.path(), "1001", "a.jpg"),
            result_ok(dir.path(), "1001", "b.jpg"),
            result_ok(dir.path(), "1002", "c.jpg"),
            make_result(
                "1002",
                "d.jpg",
                DownloadStatus::Failed,
                dir.path().join("d.jpg"),
            ),
            make_result(
                "1003",
                "e.jpg",
                DownloadStatus::Cancelled,
                dir.path().join("e.jpg"),
            ),
            make_result(
                "1004",
                "f.jpg",
                DownloadStatus::Failed,
                dir.path().join("f.jpg"),
            ),
            make_result(
                "1005",
                "g.jpg",
                DownloadStatus::SkippedExisting,
                dir.path().join("g.jpg"),
            ),
        ];
        let captured = capture(dir.path(), &items, results);
        let confirmation = &captured.record_confirmation;
        let candidate_ids: Vec<_> = confirmation
            .candidates
            .iter()
            .map(|candidate| candidate.tweet_id.as_str())
            .collect();
        assert_eq!(candidate_ids, vec!["1001", "1005"]);
        let reason = |id: &str| {
            confirmation
                .exclusions
                .iter()
                .find(|exclusion| exclusion.tweet_id == id)
                .map(|exclusion| exclusion.reason)
                .unwrap()
        };
        assert_eq!(reason("1002"), ExclusionReason::IncompleteSuppliedItems);
        assert_eq!(reason("1003"), ExclusionReason::Cancelled);
        assert_eq!(reason("1004"), ExclusionReason::AllFailed);

        let batch_id = confirmation.batch_id.clone().unwrap();
        let paths = paths_in(dir.path());
        let first = commit_download_record(&paths, &batch_id, &["1001".to_string()]).unwrap();
        assert_eq!(first.written_tweet_ids, vec!["1001".to_string()]);
        assert_eq!(
            first.remaining_candidate_tweet_ids,
            vec!["1005".to_string()]
        );
        assert_eq!(first.state, RecordBatchState::Pending);
        assert_eq!(
            read_record_ids(&paths.record_path).unwrap(),
            vec!["1001".to_string()]
        );

        let pending = query_download_record_batches(&paths, None).unwrap();
        assert_eq!(pending.pending.len(), 1);
        assert_eq!(
            pending.pending[0].candidate_tweet_ids,
            vec!["1005".to_string()]
        );
    }

    #[test]
    fn missing_result_and_unpaired_success_are_not_eligible() {
        let dir = tempfile::tempdir().unwrap();
        let items = vec![item("1001", "a.jpg"), item("1001", "b.jpg")];
        let results = vec![result_ok(dir.path(), "1001", "a.jpg")];
        let captured = capture(dir.path(), &items, results);
        assert!(captured.record_confirmation.candidates.is_empty());
        assert_eq!(
            captured.record_confirmation.exclusions[0].reason,
            ExclusionReason::IncompleteSuppliedItems
        );
        assert_eq!(captured.record_confirmation.exclusions[0].missing_count, 1);

        let items = vec![item("2001", "a.jpg")];
        let mut foreign = make_result(
            "2002",
            "z.jpg",
            DownloadStatus::Downloaded,
            dir.path().join("z.jpg"),
        );
        foreign.url = "https://cdn.example.test/not-the-item".to_string();
        let captured = capture(
            dir.path(),
            &items,
            vec![result_ok(dir.path(), "2001", "a.jpg"), foreign],
        );
        assert!(captured
            .record_confirmation
            .candidates
            .iter()
            .all(|candidate| candidate.tweet_id != "2002"));
        assert!(captured
            .record_confirmation
            .exclusions
            .iter()
            .any(|exclusion| exclusion.tweet_id == "2002"
                && exclusion.reason == ExclusionReason::UnpairedResult));
    }

    #[test]
    fn unsafe_tweet_id_cannot_become_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let captured = capture(
            dir.path(),
            &[item("1001\n9999", "a.jpg")],
            vec![result_ok(dir.path(), "1001\n9999", "a.jpg")],
        );
        assert!(captured.record_confirmation.candidates.is_empty());
        assert_eq!(
            captured.record_confirmation.exclusions[0].reason,
            ExclusionReason::UnsafeTweetId
        );
    }

    #[test]
    fn commit_is_idempotent_including_retry_after_partial_write() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        fs::write(&paths.record_path, "1001\n1001\n1002\n").unwrap();
        let captured = capture(
            dir.path(),
            &[item("1002", "a.jpg"), item("1003", "b.jpg")],
            vec![
                result_ok(dir.path(), "1002", "a.jpg"),
                result_ok(dir.path(), "1003", "b.jpg"),
            ],
        );
        let batch_id = captured.record_confirmation.batch_id.unwrap();
        let once =
            commit_download_record(&paths, &batch_id, &["1002".to_string(), "1003".to_string()])
                .unwrap();
        assert_eq!(once.written_tweet_ids, vec!["1003".to_string()]);
        assert_eq!(once.already_recorded_tweet_ids, vec!["1002".to_string()]);
        let twice =
            commit_download_record(&paths, &batch_id, &["1002".to_string(), "1003".to_string()])
                .unwrap();
        assert!(twice.written_tweet_ids.is_empty());
        assert_eq!(
            twice.already_recorded_tweet_ids,
            vec!["1002".to_string(), "1003".to_string()]
        );
        assert_eq!(
            read_record_ids(&paths.record_path).unwrap(),
            vec!["1001".to_string(), "1002".to_string(), "1003".to_string()]
        );
        let raw = fs::read_to_string(&paths.record_path).unwrap();
        assert_eq!(raw.lines().filter(|line| *line == "1001").count(), 1);
        assert_eq!(twice.state, RecordBatchState::Committed);
    }

    #[test]
    fn batch_reloads_after_process_restart_equivalent() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let captured = capture(
            dir.path(),
            &[item("1001", "a.jpg")],
            vec![result_ok(dir.path(), "1001", "a.jpg")],
        );
        let batch_id = captured.record_confirmation.batch_id.clone().unwrap();
        drop(captured);
        let reloaded = query_download_record_batches(&paths, Some(&batch_id)).unwrap();
        let batch = reloaded.batch.expect("batch");
        assert_eq!(batch.candidates.len(), 1);
        assert_eq!(batch.candidates[0].tweet_id, "1001");
        assert!(!batch.user_permission_verified);
        commit_download_record(&paths, &batch_id, &["1001".to_string()]).unwrap();
        assert_eq!(
            read_record_ids(&paths.record_path).unwrap(),
            vec!["1001".to_string()]
        );
    }

    #[test]
    fn commit_uses_historical_facts_not_current_paths() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let original = dir.path().join("sandbox").join("a.jpg");
        fs::create_dir_all(original.parent().unwrap()).unwrap();
        fs::write(&original, b"bytes").unwrap();
        let captured = capture(
            dir.path(),
            &[item("1001", "a.jpg")],
            vec![make_result(
                "1001",
                "a.jpg",
                DownloadStatus::Downloaded,
                original.clone(),
            )],
        );
        let moved = dir.path().join("organized").join("a.jpg");
        fs::create_dir_all(moved.parent().unwrap()).unwrap();
        fs::rename(&original, &moved).unwrap();
        assert!(!original.exists());
        let batch_id = captured.record_confirmation.batch_id.unwrap();
        let batch_path = paths.batches_dir.join(format!("{batch_id}.json"));
        let stored = fs::read_to_string(&batch_path).unwrap();
        assert!(!stored.contains("https://"));
        assert!(!stored.contains("sandbox"));
        assert!(!stored.contains("\"url\""));
        assert!(!stored.contains("\"path\""));
        assert!(!stored.contains("\"text\""));
        commit_download_record(&paths, &batch_id, &["1001".to_string()]).unwrap();
        assert_eq!(
            read_record_ids(&paths.record_path).unwrap(),
            vec!["1001".to_string()]
        );
        assert!(moved.exists());
    }

    #[test]
    fn invalid_batch_and_ids_are_rejected_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        fs::write(&paths.record_path, "1001\n").unwrap();
        let captured = capture(
            dir.path(),
            &[item("1001", "a.jpg"), item("1002", "b.jpg")],
            vec![
                result_ok(dir.path(), "1001", "a.jpg"),
                make_result(
                    "1002",
                    "b.jpg",
                    DownloadStatus::Failed,
                    dir.path().join("b.jpg"),
                ),
            ],
        );
        let batch_id = captured.record_confirmation.batch_id.unwrap();
        let before = fs::read_to_string(&paths.record_path).unwrap();
        for bad in ["../etc/passwd", "..", "not-a-batch", "ABCDEF", ""] {
            let err = commit_download_record(&paths, bad, &["1001".to_string()]).unwrap_err();
            assert_eq!(err.kind, ErrorKind::InvalidArgument, "{bad}");
        }
        assert!(commit_download_record(&paths, &batch_id, &[]).is_err());
        assert!(commit_download_record(&paths, &batch_id, &["1002".to_string()]).is_err());
        assert!(commit_download_record(&paths, &batch_id, &["4040".to_string()]).is_err());
        assert!(commit_download_record(&paths, &batch_id, &["1001\n1009".to_string()]).is_err());
        assert!(query_download_record_batches(&paths, Some("../etc/passwd")).is_err());
        assert_eq!(fs::read_to_string(&paths.record_path).unwrap(), before);
        assert!(!dir.path().join("etc").exists());
    }

    #[test]
    fn two_writers_keep_every_id_once() {
        for _ in 0..6 {
            let dir = tempfile::tempdir().unwrap();
            let path = paths_in(dir.path()).record_path;
            fs::write(&path, "7\n").unwrap();
            let barrier = Arc::new(Barrier::new(2));
            let left_path = path.clone();
            let right_path = path.clone();
            let left_barrier = Arc::clone(&barrier);
            let right_barrier = barrier;
            let left = thread::spawn(move || {
                left_barrier.wait();
                let ids: Vec<String> = (1..=80).map(|id| id.to_string()).collect();
                append_new_ids(&left_path, &ids).unwrap();
            });
            let right = thread::spawn(move || {
                right_barrier.wait();
                let ids: Vec<String> = (40..=120).map(|id| id.to_string()).collect();
                append_new_ids(&right_path, &ids).unwrap();
            });
            left.join().unwrap();
            right.join().unwrap();
            let raw = fs::read_to_string(&path).unwrap();
            let lines: Vec<String> = raw
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect();
            let mut expected: Vec<String> = (1..=120).map(|id| id.to_string()).collect();
            expected.push("7".to_string());
            expected.sort();
            expected.dedup();
            let mut actual = lines.clone();
            actual.sort();
            actual.dedup();
            assert_eq!(actual, expected);
            assert_eq!(
                lines.len(),
                expected.len(),
                "record lines duplicated or dropped"
            );
        }
    }

    #[test]
    fn legacy_writer_and_commit_share_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let captured = capture(
            dir.path(),
            &[item("3000", "a.jpg"), item("3001", "b.jpg")],
            vec![
                result_ok(dir.path(), "3000", "a.jpg"),
                result_ok(dir.path(), "3001", "b.jpg"),
            ],
        );
        let batch_id = captured.record_confirmation.batch_id.unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let record = paths.record_path.clone();
        let left_barrier = Arc::clone(&barrier);
        let legacy = thread::spawn(move || {
            left_barrier.wait();
            append_new_ids(&record, &["3000".to_string(), "3002".to_string()]).unwrap();
        });
        let commit_paths = paths.clone();
        let commit_barrier = barrier;
        let committer = thread::spawn(move || {
            commit_barrier.wait();
            commit_download_record(
                &commit_paths,
                &batch_id,
                &["3000".to_string(), "3001".to_string()],
            )
            .unwrap()
        });
        legacy.join().unwrap();
        committer.join().unwrap();
        let raw = fs::read_to_string(&paths.record_path).unwrap();
        let mut lines: Vec<String> = raw
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
        lines.sort();
        assert_eq!(
            lines,
            vec!["3000".to_string(), "3001".to_string(), "3002".to_string()]
        );
    }

    #[test]
    fn legacy_any_success_rule_stays_distinct() {
        let downloads = vec![
            make_result("1001", "a.jpg", DownloadStatus::Failed, PathBuf::from("a")),
            make_result(
                "1001",
                "b.jpg",
                DownloadStatus::Downloaded,
                PathBuf::from("b"),
            ),
            make_result(
                "1002",
                "c.jpg",
                DownloadStatus::Cancelled,
                PathBuf::from("c"),
            ),
            make_result("1003", "d.jpg", DownloadStatus::Failed, PathBuf::from("d")),
        ];
        let mut ids: Vec<_> = legacy_recordable_tweet_ids(&downloads)
            .into_iter()
            .collect();
        ids.sort();
        assert_eq!(ids, vec!["1001".to_string()]);
    }

    #[test]
    fn nothing_to_record_is_not_pending() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let captured = capture(
            dir.path(),
            &[item("1001", "a.jpg")],
            vec![make_result(
                "1001",
                "a.jpg",
                DownloadStatus::Cancelled,
                dir.path().join("a.jpg"),
            )],
        );
        assert_eq!(
            captured.record_confirmation.state,
            RecordBatchState::NothingToRecord
        );
        assert!(!captured.record_confirmation.agent_must_ask_user);
        assert!(!paths.record_path.exists());
        let listed = query_download_record_batches(&paths, None).unwrap();
        assert!(listed.pending.is_empty());
    }
}
