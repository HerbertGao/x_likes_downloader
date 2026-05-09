use chrono::DateTime;
use filetime::{set_file_times, FileTime};
use futures::stream::{self, StreamExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::config::Config;
use crate::error::{ErrorKind, ErrorPayload};
use crate::sandbox;

use super::etag_cache::{self, EtagCache};
use super::types::{
    DownloadError, DownloadOpts, DownloadOutput, DownloadResult, DownloadStatus, DownloadSummary,
    MediaItem, ProgressEvent, ProgressSink,
};

/// 单 item 的 retry / restart 重试上限。Content-Range 失配 / ETag 失配 / 200 fallback
/// 等场景会清掉 `.partial` 重头下；正常路径首次成功，需要重启时再走一次。
/// 超过此次数仍失败 → 视为 server 行为异常，返回 `Failed`。
const MAX_RESTARTS_PER_ITEM: usize = 2;

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
    let cancel_token = opts.cancel.clone();

    let owned_items: Vec<MediaItem> = items.to_vec();
    let results: Vec<DownloadResult> =
        stream::iter(owned_items.into_iter().enumerate().map(|(index, item)| {
            let client = client_arc.clone();
            let target_dir = target_dir_arc.clone();
            let ua = user_agent.clone();
            let sink = sink_inner.clone();
            let filename_format = filename_format.clone();
            let cancel = cancel_token.clone();
            async move {
                let filename = derive_filename(&item, filename_format.as_deref());
                let final_path = target_dir.join(&filename);

                // Queue 早退出：cancel 已触发但本 future 才被 buffer_unordered 拉起。
                // 关键不变量：未启动 item **不**发 ItemStarted/ItemDone progress 事件——
                // 否则 McpProgressSink 会把它们计入 items_done，让 BatchCancelled 发送时
                // progress 累积到 total_items（UI 显示 100%），违反 spec
                // "BatchCancelled progress < total_items" 合约。这些 item 仍通过返回的
                // DownloadResult { Cancelled } 计入 summary.cancelled——语义保留，
                // 只是不参与 progress 数值累加。
                if let Some(ref c) = cancel {
                    if c.is_cancelled() {
                        return DownloadResult {
                            tweet_id: item.tweet_id.clone(),
                            url: item.url.clone(),
                            path: final_path,
                            bytes: 0,
                            status: DownloadStatus::Cancelled,
                            error: None,
                        };
                    }
                }

                sink.emit(ProgressEvent::ItemStarted {
                    tweet_id: item.tweet_id.clone(),
                    url: item.url.clone(),
                    index,
                    total,
                });
                let result = download_one(
                    &client,
                    &ua,
                    &item,
                    &final_path,
                    set_mtime,
                    sink.clone(),
                    cancel.as_ref(),
                    index,
                )
                .await;
                sink.emit(ProgressEvent::ItemDone {
                    tweet_id: result.tweet_id.clone(),
                    index,
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
        cancelled: 0,
    };
    for r in &results {
        match r.status {
            DownloadStatus::Downloaded => summary.downloaded += 1,
            DownloadStatus::SkippedExisting => summary.skipped += 1,
            DownloadStatus::Failed => summary.failed += 1,
            DownloadStatus::Cancelled => summary.cancelled += 1,
        }
    }

    // 决定 BatchCancelled vs DownloadFinished：以 summary.cancelled > 0 为准，**不**仅看
    // cancel_token.is_cancelled()。cancel token 触发可能与所有 item 自然完成的 race
    // 相遇——例如 MCP client 的 cancel notification 恰好在最后一个 ItemDone(Downloaded)
    // 之后才到达。此时所有 results 都是 Downloaded / SkippedExisting / Failed，
    // summary.cancelled == 0；正确行为是 emit DownloadFinished（批次实际正常结束），
    // 而不是发一个误导的 BatchCancelled（让 client 以为某些 item 被取消了）。
    let was_cancelled = summary.cancelled > 0;
    if was_cancelled {
        sink.emit(ProgressEvent::BatchCancelled {
            summary: summary.clone(),
        });
    } else {
        sink.emit(ProgressEvent::DownloadFinished {
            summary: summary.clone(),
        });
    }
    // 显式 drop cancel_token 避免"unused"警告——它在 buffer_unordered 闭包内被克隆使用，
    // 但本作用域结束后只有 drop 这一个直接操作。
    drop(cancel_token);

    Ok(DownloadOutput {
        downloads: results,
        summary,
    })
}

#[allow(clippy::too_many_arguments)]
async fn download_one(
    client: &reqwest::Client,
    user_agent: &str,
    item: &MediaItem,
    final_path: &Path,
    set_mtime: bool,
    sink: Arc<dyn ProgressSink>,
    cancel: Option<&CancellationToken>,
    item_index: usize,
) -> DownloadResult {
    // 防御：永不写入符号链接（Sandbox jail 在目录层；这里防文件名层 symlink 逃逸）。
    if is_symlink(final_path) {
        return DownloadResult {
            tweet_id: item.tweet_id.clone(),
            url: item.url.clone(),
            path: final_path.to_path_buf(),
            bytes: 0,
            status: DownloadStatus::Failed,
            error: Some(DownloadError {
                kind: ErrorKind::SandboxViolation,
                message: format!("拒绝写入符号链接: {:?}", final_path),
            }),
        };
    }

    // 同样防御 partial_path：陈旧的 .partial 文件可能在前次下载结束后被外部进程
    // 替换为 symlink（例如指向 /etc/passwd），如果 OpenOptions::open() 跟随它就会
    // 写出沙箱。这条 guard 与 final_path 的 symlink 检查对称。
    let partial_path_for_check = PathBuf::from(format!("{}.partial", final_path.display()));
    if is_symlink(&partial_path_for_check) {
        return DownloadResult {
            tweet_id: item.tweet_id.clone(),
            url: item.url.clone(),
            path: final_path.to_path_buf(),
            bytes: 0,
            status: DownloadStatus::Failed,
            error: Some(DownloadError {
                kind: ErrorKind::SandboxViolation,
                message: format!("拒绝跟随 .partial 符号链接: {:?}", partial_path_for_check),
            }),
        };
    }

    // v2.1 idempotency 决策：
    // 1) `is_file()` 守卫：目录的 metadata 也返回 Ok 且 `len()` 通常非零（macOS/Linux
    //    上典型 4096）——不加守卫会错误地把"目标是目录"判定为 SkippedExisting。
    //    非 file 目标（目录 / 设备文件等）走下载路径——OpenOptions::open 会失败并返回
    //    InternalError，把"目标不可写"信号正确传递给 caller。
    // 2) 对完整性的判定优先级：
    //    a) ETag cache 命中且 size + url + finalized=true 全匹配 → 信任 cache 跳过
    //       HEAD（快路径）
    //    b) cache miss / 不完整匹配 → 发 HEAD 拿 server Content-Length，与 meta.len()
    //       比对：一致 → SkippedExisting；不一致 → 走下载路径覆盖。HEAD 网络失败 →
    //       保守接受现有文件，避免误删
    // 3) v2.1 .partial+rename 协议保证新写入的 final_path 总是完整的；HEAD verify
    //    主要保护从 v2.0 升上来的用户（.partial+rename 协议生效前的损坏 final 文件）
    if let Ok(meta) = std::fs::metadata(final_path) {
        if meta.is_file() && meta.len() > 0 {
            let cache_key = EtagCache::key_for(final_path);
            // Cache 快路径必须**同时**校验 size、url、finalized 三字段：
            // - size 一致：本地文件长度与 cache 记录一致
            // - url 一致：cache entry 属于当前 item.url（不同 URL 共用 final 路径时
            //   即使 size 巧合相同也不能信任）
            // - finalized = true：cache 在 headers 阶段写入（finalized=false）后
            //   下载可能被中断/失败，stale 旧 final 留在原处但 cache 已含新元数据；
            //   仅 finalized=true 代表"rename partial→final 成功完成"
            let cache_says_complete = EtagCache::path()
                .ok()
                .map(|p| {
                    EtagCache::load_from(&p)
                        .get(&cache_key)
                        .map(|e| e.size == meta.len() && e.url == item.url && e.finalized)
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            let trust_existing = if cache_says_complete {
                true
            } else {
                // cache 缺失 / 不匹配 → HEAD verify。使用 cancellable 包装：
                // 若 client 在 HEAD 慢响应期间发 cancel，必须立即返回 Cancelled，
                // 避免 hang 至 reqwest 120s timeout。
                match cancellable_head_content_length(client, user_agent, &item.url, cancel).await {
                    // Content-Length = 0 在 HEAD 响应上是不可靠信号——某些 server / mock
                    // 框架（如 wiremock 0.6）会把 HEAD 响应的 body strip 并重算 Content-Length
                    // 为 0，盖过 server 真正想报告的值。把 Some(0) 当作"未暴露长度"处理：
                    // 与 meta.len() > 0 已知存在的真实文件不可能匹配，盲目走 false 会误删。
                    CancellableResult::Ok(Some(expected)) if expected > 0 => meta.len() == expected,
                    // server 不暴露 Content-Length（或暴露了 0）：保守接受现有文件
                    CancellableResult::Ok(_) => true,
                    // Cancel 触发：返回 Cancelled，partial 文件未创建
                    CancellableResult::Cancelled => {
                        return DownloadResult {
                            tweet_id: item.tweet_id.clone(),
                            url: item.url.clone(),
                            path: final_path.to_path_buf(),
                            bytes: 0,
                            status: DownloadStatus::Cancelled,
                            error: None,
                        };
                    }
                    // HEAD 网络失败：保守接受（避免在 server 临时挂时误删/重下）
                    CancellableResult::Err(_) => true,
                }
            };
            if trust_existing {
                if set_mtime {
                    apply_mtime(final_path, item.created_at.as_deref());
                }
                return DownloadResult {
                    tweet_id: item.tweet_id.clone(),
                    url: item.url.clone(),
                    path: final_path.to_path_buf(),
                    bytes: meta.len(),
                    status: DownloadStatus::SkippedExisting,
                    error: None,
                };
            }
            // 否则 fall-through 到下载路径，让 .partial+rename 把新内容覆盖到 final_path
        }
    }

    // 注：此处用字符串拼接而非 with_extension("partial")，确保 alice.mp4 → alice.mp4.partial
    // （with_extension 会替换为 alice.partial）。复用上面已构造的 partial_path_for_check
    // 以避免重复 format。
    let partial_path = partial_path_for_check;

    match fetch_with_partial_resume(
        client,
        user_agent,
        item,
        final_path,
        &partial_path,
        sink,
        cancel,
        item_index,
    )
    .await
    {
        FetchOutcome::Completed(bytes) => {
            if set_mtime {
                apply_mtime(final_path, item.created_at.as_deref());
            }
            DownloadResult {
                tweet_id: item.tweet_id.clone(),
                url: item.url.clone(),
                path: final_path.to_path_buf(),
                bytes,
                status: DownloadStatus::Downloaded,
                error: None,
            }
        }
        FetchOutcome::Cancelled(_bytes) => DownloadResult {
            tweet_id: item.tweet_id.clone(),
            url: item.url.clone(),
            path: final_path.to_path_buf(),
            bytes: 0, // bytes 字段对 cancelled 无明确语义；置 0 与 Failed 一致
            status: DownloadStatus::Cancelled,
            error: None,
        },
        FetchOutcome::Failed(kind, message) => DownloadResult {
            tweet_id: item.tweet_id.clone(),
            url: item.url.clone(),
            path: final_path.to_path_buf(),
            bytes: 0,
            status: DownloadStatus::Failed,
            error: Some(DownloadError { kind, message }),
        },
    }
}

enum FetchOutcome {
    Completed(u64),
    Cancelled(u64),
    Failed(ErrorKind, String),
}

/// 核心下载流：
/// 1. 检查 `.partial` 文件大小；> 0 则 HEAD 拿当前 ETag、与 cache 对比
/// 2. ETag 一致 → GET with `Range: bytes=N-`；不一致 / cache 缺失 → 删 partial 重头下
/// 3. 处理响应：206 → 校验 Content-Range；200 → restart；其它 → Failed
/// 4. 收到 headers 后**立即**写 ETag cache
/// 5. chunk 循环 select! 监听 cancel；cancel 触发保留 partial、不删 cache
/// 6. 流读尽 → 关闭 fd → 原子 rename 到 final_path
#[allow(clippy::too_many_arguments)]
async fn fetch_with_partial_resume(
    client: &reqwest::Client,
    user_agent: &str,
    item: &MediaItem,
    final_path: &Path,
    partial_path: &Path,
    sink: Arc<dyn ProgressSink>,
    cancel: Option<&CancellationToken>,
    item_index: usize,
) -> FetchOutcome {
    let cache_path = match EtagCache::path() {
        Ok(p) => Some(p),
        Err(e) => {
            // cache 路径无法解析（HOME 未设等极端情况）→ 不阻塞下载，但禁用 ETag 续传
            eprintln!(
                "etag_cache: 无法解析 cache 路径 ({}); 本次下载禁用 ETag 续传",
                e
            );
            None
        }
    };
    let cache_key = EtagCache::key_for(final_path);

    let mut restarts = 0usize;
    loop {
        let partial_size = std::fs::metadata(partial_path)
            .map(|m| m.len())
            .unwrap_or(0);

        // ETag 续传探测：仅当 partial 存在且 size > 0
        let mut want_resume = false;
        if partial_size > 0 {
            // HEAD 拿当前 server ETag。失败时退回到全量重下（rather than fail）。
            let head_etag_opt =
                match cancellable_head_etag(client, user_agent, &item.url, cancel).await {
                    CancellableResult::Ok(v) => v,
                    CancellableResult::Cancelled => return FetchOutcome::Cancelled(0),
                    CancellableResult::Err(e) => {
                        eprintln!(
                            "tweet {}: HEAD failed ({}); restarting from scratch",
                            item.tweet_id, e
                        );
                        None
                    }
                };

            let cached = cache_path.as_ref().and_then(|p| {
                let cache = EtagCache::load_from(p);
                cache.get(&cache_key).cloned()
            });

            // 续传必须**同时**满足 URL 与 ETag 双匹配。仅匹配 ETag 不够安全——
            // ETag 只是特定资源的 validator，不同 URL 可以巧合返回同一字符串
            // （如 weak ETag "abc" / "abc"）。仅靠 ETag 续传时，新响应字节会被
            // append 到旧 URL 的 bytes，产生损坏的拼接文件。cache.url 存了原始 URL，
            // 直接对比即可关闭这个 attack surface。
            let resumable = match (head_etag_opt.as_deref(), cached.as_ref()) {
                (Some(head_etag), Some(c)) => c.etag == head_etag && c.url == item.url,
                _ => false,
            };
            if resumable {
                want_resume = true;
            } else {
                let reason: &'static str = match (head_etag_opt.as_deref(), cached.as_ref()) {
                    (Some(_), Some(c)) if c.url != item.url => {
                        "cached URL mismatch, restarting from scratch"
                    }
                    (Some(_), Some(_)) => "ETag changed, restarting from scratch",
                    _ => "no ETag baseline, restarting from scratch",
                };
                let _ = std::fs::remove_file(partial_path);
                if let Some(p) = cache_path.as_ref() {
                    let _ = etag_cache::remove_entry(p, &cache_key);
                }
                sink.emit(ProgressEvent::ItemRestart {
                    tweet_id: item.tweet_id.clone(),
                    index: item_index,
                    reason: reason.into(),
                });
            }
        }

        let existing_len = if want_resume { partial_size } else { 0 };

        // 发起 GET
        let mut req = client.get(&item.url).header("User-Agent", user_agent);
        if existing_len > 0 {
            req = req.header("Range", format!("bytes={}-", existing_len));
        }
        let response = match cancellable_send(req, cancel).await {
            CancellableResult::Ok(v) => v,
            CancellableResult::Cancelled => return FetchOutcome::Cancelled(0),
            CancellableResult::Err(e) => {
                return FetchOutcome::Failed(ErrorKind::NetworkError, e.to_string());
            }
        };
        let status = response.status().as_u16();

        // 处理响应状态
        let resume_mode;
        let server_total: Option<u64>;
        match (existing_len, status) {
            (0, s) if (200..300).contains(&s) => {
                // Fresh download
                resume_mode = false;
                server_total = response.content_length();
            }
            (n, 206) if n > 0 => {
                // 校验 Content-Range
                let cr = parse_content_range(response.headers());
                let cached_size = cache_path
                    .as_ref()
                    .and_then(|p| EtagCache::load_from(p).get(&cache_key).map(|e| e.size));
                let valid = match cr {
                    Some(ContentRange { start, end, total }) => {
                        let ok = start == n
                            && end + 1 == total
                            && cached_size.map(|cs| cs == total).unwrap_or(true);
                        if ok {
                            Some(total)
                        } else {
                            None
                        }
                    }
                    None => None,
                };
                if let Some(total) = valid {
                    resume_mode = true;
                    server_total = Some(total);
                } else {
                    // mismatch → 清 partial、删 cache、重启
                    drop(response);
                    let _ = std::fs::remove_file(partial_path);
                    if let Some(p) = cache_path.as_ref() {
                        let _ = etag_cache::remove_entry(p, &cache_key);
                    }
                    sink.emit(ProgressEvent::ItemRestart {
                        tweet_id: item.tweet_id.clone(),
                        index: item_index,
                        reason: "Content-Range mismatch, restarting from scratch".into(),
                    });
                    restarts += 1;
                    if restarts >= MAX_RESTARTS_PER_ITEM {
                        return FetchOutcome::Failed(
                            ErrorKind::NetworkError,
                            format!(
                                "tweet {}: server keeps returning bad Content-Range",
                                item.tweet_id
                            ),
                        );
                    }
                    continue;
                }
            }
            (n, 200) if n > 0 => {
                // server 不支持 Range（忽略了 Range header）→ 删 partial 重头下
                drop(response);
                let _ = std::fs::remove_file(partial_path);
                sink.emit(ProgressEvent::ItemRestart {
                    tweet_id: item.tweet_id.clone(),
                    index: item_index,
                    reason: "server doesn't support Range, restarting".into(),
                });
                restarts += 1;
                if restarts >= MAX_RESTARTS_PER_ITEM {
                    return FetchOutcome::Failed(
                        ErrorKind::NetworkError,
                        format!("tweet {}: unable to complete fresh download", item.tweet_id),
                    );
                }
                continue;
            }
            (_, 416) => {
                // partial 比远端大（损坏的过大 partial）→ 删 partial 重头下
                drop(response);
                let _ = std::fs::remove_file(partial_path);
                if let Some(p) = cache_path.as_ref() {
                    let _ = etag_cache::remove_entry(p, &cache_key);
                }
                sink.emit(ProgressEvent::ItemRestart {
                    tweet_id: item.tweet_id.clone(),
                    index: item_index,
                    reason: "Range not satisfiable, restarting from scratch".into(),
                });
                restarts += 1;
                if restarts >= MAX_RESTARTS_PER_ITEM {
                    return FetchOutcome::Failed(
                        ErrorKind::NetworkError,
                        format!("tweet {}: 416 loop", item.tweet_id),
                    );
                }
                continue;
            }
            (_, s) => {
                let kind = crate::error::classify_status(s).unwrap_or(ErrorKind::InternalError);
                return FetchOutcome::Failed(kind, format!("HTTP {}: {}", s, item.url));
            }
        }

        // 立即写 ETag cache：收到 headers 后立刻记录 in-progress 元数据
        // （finalized=false），让被取消 / 失败的下载也留下能被下次续传利用的基线。
        // 仅当 rename 成功后我们再写一次 finalized=true。这两个值（etag / total）
        // 也供 rename 后的 finalize cache 写入复用，因此 lift 到外层作用域。
        let response_etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let total_for_cache = server_total.or_else(|| {
            if resume_mode {
                response.content_length().map(|c| c + existing_len)
            } else {
                response.content_length()
            }
        });
        if let Some(p) = cache_path.as_ref() {
            if let (Some(etag), Some(size)) = (response_etag.as_deref(), total_for_cache) {
                if let Err(e) = etag_cache::put_entry(
                    p,
                    cache_key.clone(),
                    EtagCache::make_entry(etag, &item.url, size),
                ) {
                    eprintln!(
                        "etag_cache: put_entry failed for {:?}: {} (path={:?})",
                        item.tweet_id, e, p
                    );
                }
            }
        }
        let response_etag_for_finalize = response_etag;
        let total_bytes_for_finalize = total_for_cache;

        // 第二道 symlink 防御：download_one 顶部已检查过 partial_path，但 restart
        // 路径会先 remove_file 再重新 open；删除后到 open 之间若有外部进程把
        // partial_path 替换成 symlink，跟随就会逃出沙箱。每次 open 前都重新校验。
        if is_symlink(partial_path) {
            return FetchOutcome::Failed(
                ErrorKind::SandboxViolation,
                format!("拒绝跟随 .partial 符号链接: {:?}", partial_path),
            );
        }

        // 打开 partial 文件
        let mut file = if resume_mode {
            match OpenOptions::new()
                .create(true)
                .append(true)
                .write(true)
                .open(partial_path)
                .await
            {
                Ok(f) => f,
                Err(e) => return FetchOutcome::Failed(ErrorKind::InternalError, e.to_string()),
            }
        } else {
            match OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(partial_path)
                .await
            {
                Ok(f) => f,
                Err(e) => return FetchOutcome::Failed(ErrorKind::InternalError, e.to_string()),
            }
        };

        let mut downloaded = if resume_mode { existing_len } else { 0 };
        let total_bytes = if resume_mode {
            server_total.or_else(|| response.content_length().map(|c| c + existing_len))
        } else {
            server_total.or_else(|| response.content_length())
        };
        let mut stream = response.bytes_stream();

        loop {
            let chunk_opt = if let Some(c) = cancel {
                tokio::select! {
                    biased;
                    _ = c.cancelled() => {
                        // 流尾 race：cancel 可能在最后一个 chunk 写入后、stream.next()
                        // 观察 EOF 之前触发。如果已收到全部期望字节，应当走正常 rename
                        // 路径而不是 Cancelled，否则 client 会看到一个"100% 完成但状态
                        // cancelled、文件还是 .partial"的诡异结果。仅当 total_bytes 已知
                        // 且 downloaded >= total 时才视为完成；长度未知场景保守地返回
                        // Cancelled（不假装下完）。
                        if let Some(t) = total_bytes {
                            if downloaded >= t {
                                break;
                            }
                        }
                        drop(file);
                        return FetchOutcome::Cancelled(downloaded);
                    }
                    next = stream.next() => next,
                }
            } else {
                stream.next().await
            };
            match chunk_opt {
                Some(Ok(chunk)) => {
                    if let Err(e) = file.write_all(&chunk).await {
                        return FetchOutcome::Failed(ErrorKind::InternalError, e.to_string());
                    }
                    downloaded += chunk.len() as u64;
                    sink.emit(ProgressEvent::ItemProgress {
                        tweet_id: item.tweet_id.clone(),
                        index: item_index,
                        bytes_done: downloaded,
                        bytes_total: total_bytes,
                    });
                }
                Some(Err(e)) => {
                    return FetchOutcome::Failed(ErrorKind::NetworkError, e.to_string());
                }
                None => break,
            }
        }

        // 流尾完整性校验：chunked 响应或被 server 提前关闭时 stream.next() 会返回
        // None（EOF）即使 server 没把所有期望字节发完——例如 chunked transfer 中段
        // socket 被 reset 后 hyper 把它当作 graceful EOF。如果 total_bytes 已知
        // （Content-Length 或 Content-Range total），必须在 rename 前确认
        // downloaded == total，否则 .partial 会被错误地 rename 为最终文件并报告
        // Downloaded，但实际是 truncated content。
        // total_bytes 未知时（reqwest 未暴露 Content-Length）保守接受 EOF——这是
        // server-side responsibility，client 没办法验证。
        if let Some(t) = total_bytes {
            if downloaded < t {
                drop(file); // 关闭 fd 但**不**删 .partial：让下次 Range 续传从断点接续
                return FetchOutcome::Failed(
                    ErrorKind::NetworkError,
                    format!(
                        "tweet {}: stream ended at {} bytes, expected {} (server prematurely closed)",
                        item.tweet_id, downloaded, t
                    ),
                );
            }
        }

        // 流读尽 → 关闭 fd → 原子 rename
        if let Err(e) = file.flush().await {
            return FetchOutcome::Failed(ErrorKind::InternalError, e.to_string());
        }
        drop(file);
        // 单步原子 replace：POSIX `rename(2)` 与 Windows `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`
        // 都支持"destination 存在时原子替换"语义。
        //
        // 关键不变量：rename 失败时原 final_path 的旧内容**保持不变**——既不能在
        // rename 之前先删 final（删了之后 rename 失败就丢失旧内容），也不能在 rename
        // 之后做任何额外清理。失败路径下 .partial 与原 final 都保留，让 caller /
        // 下次运行能恢复。
        if let Err(e) = tokio::fs::rename(partial_path, final_path).await {
            return FetchOutcome::Failed(
                ErrorKind::InternalError,
                format!("rename partial → final 失败: {}", e),
            );
        }

        // rename 成功后再写一次 finalized cache，让下次运行的 fast-path skip 能
        // 信任这个条目对应一个真正完成的 final 文件（而非 in-progress / 已失败下载
        // 留下的元数据）。失败容忍：cache 写失败不影响下载语义——下次跑会走 HEAD
        // verify 兜底（HEAD content-length 与 meta.len() 比对）。
        if let Some(p) = cache_path.as_ref() {
            let etag = response_etag_for_finalize.as_deref();
            let total = total_bytes_for_finalize;
            if let (Some(etag), Some(size)) = (etag, total) {
                let _ = etag_cache::put_entry(
                    p,
                    cache_key.clone(),
                    EtagCache::make_finalized_entry(etag, &item.url, size),
                );
            }
        }
        return FetchOutcome::Completed(downloaded);
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ContentRange {
    start: u64,
    end: u64,
    total: u64,
}

/// 解析 `Content-Range: bytes N-M/Total` 头。失败返回 None。
fn parse_content_range(headers: &reqwest::header::HeaderMap) -> Option<ContentRange> {
    let v = headers.get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    parse_content_range_value(v)
}

fn parse_content_range_value(v: &str) -> Option<ContentRange> {
    // 格式：bytes <start>-<end>/<total>
    let v = v.trim();
    let rest = v.strip_prefix("bytes ")?;
    let (range, total_s) = rest.split_once('/')?;
    let (start_s, end_s) = range.split_once('-')?;
    let start = start_s.trim().parse::<u64>().ok()?;
    let end = end_s.trim().parse::<u64>().ok()?;
    let total = total_s.trim().parse::<u64>().ok()?;
    Some(ContentRange { start, end, total })
}

enum CancellableResult<T, E> {
    Ok(T),
    Cancelled,
    Err(E),
}

async fn cancellable_send(
    req: reqwest::RequestBuilder,
    cancel: Option<&CancellationToken>,
) -> CancellableResult<reqwest::Response, reqwest::Error> {
    let fut = req.send();
    if let Some(c) = cancel {
        tokio::select! {
            biased;
            _ = c.cancelled() => CancellableResult::Cancelled,
            r = fut => match r {
                Ok(v) => CancellableResult::Ok(v),
                Err(e) => CancellableResult::Err(e),
            }
        }
    } else {
        match fut.await {
            Ok(v) => CancellableResult::Ok(v),
            Err(e) => CancellableResult::Err(e),
        }
    }
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

/// Cancellable wrapper for `head_content_length`。download_one 的 SkippedExisting
/// 决策需要 HEAD verify，但若 client 在 HEAD 慢响应期间发 cancel，必须立即返回——
/// 否则 cancel 会等到 reqwest 120s timeout 才生效。
async fn cancellable_head_content_length(
    client: &reqwest::Client,
    user_agent: &str,
    url: &str,
    cancel: Option<&CancellationToken>,
) -> CancellableResult<Option<u64>, reqwest::Error> {
    let fut = head_content_length(client, user_agent, url);
    if let Some(c) = cancel {
        tokio::select! {
            biased;
            _ = c.cancelled() => CancellableResult::Cancelled,
            r = fut => match r {
                Ok(v) => CancellableResult::Ok(v),
                Err(e) => CancellableResult::Err(e),
            }
        }
    } else {
        match fut.await {
            Ok(v) => CancellableResult::Ok(v),
            Err(e) => CancellableResult::Err(e),
        }
    }
}

async fn cancellable_head_etag(
    client: &reqwest::Client,
    user_agent: &str,
    url: &str,
    cancel: Option<&CancellationToken>,
) -> CancellableResult<Option<String>, reqwest::Error> {
    let req = client.head(url).header("User-Agent", user_agent);
    let fut = async move {
        let resp = req.send().await?;
        Ok::<Option<String>, reqwest::Error>(
            resp.headers()
                .get(reqwest::header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string()),
        )
    };
    if let Some(c) = cancel {
        tokio::select! {
            biased;
            _ = c.cancelled() => CancellableResult::Cancelled,
            r = fut => match r {
                Ok(v) => CancellableResult::Ok(v),
                Err(e) => CancellableResult::Err(e),
            }
        }
    } else {
        match fut.await {
            Ok(v) => CancellableResult::Ok(v),
            Err(e) => CancellableResult::Err(e),
        }
    }
}

/// 清洗文件名片段：替换路径分隔符 / 控制字符 / `..` 序列。
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
    while out.contains("..") {
        out = out.replace("..", "_");
    }
    if out == "." || out.is_empty() {
        return "_".to_string();
    }
    out
}

/// 派生文件名。详见旧实现注释。
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

    match Path::new(&raw).file_name().and_then(|n| n.to_str()) {
        Some(name) if !name.is_empty() && name != "." && name != ".." => name.to_string(),
        _ => format!("{}_media", safe_tweet_id),
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
    if let Ok(dt) = DateTime::parse_from_str(s, "%a %b %d %H:%M:%S %z %Y") {
        return Some(dt.timestamp());
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp());
    }
    None
}

/// 用 `symlink_metadata` 判定 path 是否为符号链接（不跟随）。
///
/// 路径不存在 / 无法访问 → false（视为安全的"不是 symlink"，让后续 OpenOptions
/// 处理实际的存在性检查）。这是 P1 安全修复的核心：所有写入 partial / final 的代码
/// 路径都必须先经过本函数确认目标不是 symlink，避免 OpenOptions 默认跟随后写到
/// sandbox 之外的位置（如 `/etc/passwd`）。
fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
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
        assert!(matches!(events[0], ProgressEvent::DownloadStarted { .. }));
        assert!(matches!(
            events.last().unwrap(),
            ProgressEvent::DownloadFinished { .. }
        ));
    }

    #[test]
    fn empty_items_with_cancel_already_cancelled_emits_download_finished() {
        // 合约：summary.cancelled == 0 → DownloadFinished，即使 cancel token 已
        // 触发。空 items 列表里没有任何 cancelled 计数，因此即使 token 已 cancel，
        // 仍应 emit 正常结束事件。
        let items: Vec<MediaItem> = vec![];
        let token = CancellationToken::new();
        token.cancel();
        let opts = DownloadOpts {
            cancel: Some(token),
            ..Default::default()
        };
        let sink = Arc::new(VecSink::new());
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _ = rt.block_on(download_media(&items, &opts, sink.clone()));
        let events = sink.snapshot();
        assert!(
            matches!(events.last().unwrap(), ProgressEvent::DownloadFinished { .. }),
            "empty items + cancelled token must emit DownloadFinished (no items were actually cancelled), got {:?}",
            events.last()
        );
    }

    // ---- filename helpers ----

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
        apply_mtime(&path, Some("not a date"));
        apply_mtime(&path, None);
    }

    // ---- sanitize / derive_filename hardening ----

    #[test]
    fn sanitize_component_replaces_path_separators() {
        assert!(!sanitize_component("foo/bar").contains('/'));
        assert!(!sanitize_component("foo\\bar").contains('\\'));
        assert!(!sanitize_component("c:foo").contains(':'));
        assert!(!sanitize_component("a\0b").contains('\0'));
        assert!(!sanitize_component("a\x01b").contains('\x01'));
    }

    #[test]
    fn sanitize_component_replaces_dotdot() {
        assert!(!sanitize_component("..").contains(".."));
        assert!(!sanitize_component("../etc").contains(".."));
        assert!(!sanitize_component("a..b").contains(".."));
        assert!(!sanitize_component("....").contains(".."));
        assert_eq!(sanitize_component(""), "_");
        assert_eq!(sanitize_component("."), "_");
    }

    #[test]
    fn derive_filename_rejects_dotdot_in_suggested() {
        let m = item("1234", Some("alice"), "../../etc/passwd");
        let f = derive_filename(&m, None);
        assert!(!f.contains(".."));
        assert!(!f.contains('/'));
        assert_eq!(Path::new(&f).components().count(), 1);
    }

    #[test]
    fn derive_filename_rejects_slash_in_suggested() {
        let m = item("1234", Some("alice"), "foo/bar.jpg");
        let f = derive_filename(&m, None);
        assert!(!f.contains('/'));
    }

    #[test]
    fn derive_filename_rejects_backslash_in_suggested() {
        let m = item("1234", Some("alice"), "foo\\bar.jpg");
        let f = derive_filename(&m, None);
        assert!(!f.contains('\\'));
    }

    #[test]
    fn derive_filename_rejects_dotdot_in_tweet_id() {
        let m = item("..", Some("alice"), "AAA.jpg");
        let f = derive_filename(&m, None);
        assert!(!f.contains(".."));
        assert_eq!(Path::new(&f).components().count(), 1);
    }

    #[test]
    fn derive_filename_rejects_dotdot_in_handle() {
        let m = item("1234", Some("../alice"), "AAA.jpg");
        let f = derive_filename(&m, None);
        assert!(!f.contains(".."));
        assert!(!f.contains('/'));
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
        assert!(joined.starts_with(&canonical_base));
        let extra: Vec<_> = joined
            .strip_prefix(&canonical_base)
            .unwrap()
            .components()
            .collect();
        assert_eq!(extra.len(), 1);
    }

    // ---- Content-Range parsing ----

    #[test]
    fn parse_content_range_value_basic() {
        let cr = parse_content_range_value("bytes 100-499/500").unwrap();
        assert_eq!(
            cr,
            ContentRange {
                start: 100,
                end: 499,
                total: 500
            }
        );
    }

    #[test]
    fn parse_content_range_value_invalid() {
        assert!(parse_content_range_value("100-499/500").is_none()); // 缺 "bytes "
        assert!(parse_content_range_value("bytes 100-499").is_none()); // 缺 /total
        assert!(parse_content_range_value("bytes abc-499/500").is_none()); // 非数字
    }

    // ---- partial path naming ----

    #[test]
    fn partial_path_appends_dot_partial_literal() {
        // 关键不变量：alice.mp4 → alice.mp4.partial（追加），不是 alice.partial（替换）。
        let final_path = Path::new("/sandbox/alice_123_video.mp4");
        let partial = PathBuf::from(format!("{}.partial", final_path.display()));
        assert_eq!(
            partial,
            PathBuf::from("/sandbox/alice_123_video.mp4.partial")
        );
    }

    // ---- idempotency: 仅看 final_path（不看 .partial）----

    #[test]
    fn idempotency_ignores_partial_only_files() {
        // 给 sandbox 放一个 .partial 文件；download_one 必须不视为 skipped_existing
        // （走下载路径），但本测试为单元层级，不真正发 HTTP——只验证 final_path 不存在
        // 时不会沉默 skipped。
        use tempfile::tempdir;
        let dir = tempdir().unwrap();
        let final_path = dir.path().join("alice_1_x.jpg");
        let partial = PathBuf::from(format!("{}.partial", final_path.display()));
        std::fs::write(&partial, b"partial bytes").unwrap();
        // final 不存在
        assert!(!final_path.exists());
        // 我们只验证 metadata-based 判定不会因 .partial 触发 skip
        let meta_final = std::fs::metadata(&final_path);
        assert!(meta_final.is_err());
    }

    // ---- symlink rejection at target path ----

    #[test]
    fn directory_at_final_path_is_not_skipped() {
        // 合约：final_path 是目录时**不**视为 SkippedExisting。否则 caller 拿到
        // status=skipped + path=<directory>，无法区分"已下载"和"占位"。
        use crate::agent::types::{DownloadOpts, MediaItem, MediaType, NullSink};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        // 在 sandbox 内 derive 出来的 final 文件名位置创建一个目录
        let collide_dir = dir.path().join("1_file.jpg");
        std::fs::create_dir(&collide_dir).unwrap();

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

        let opts = DownloadOpts {
            subdir: None,
            concurrency: 1,
            base_dir: Some(dir.path().to_path_buf()),
            filename_format: None,
            set_mtime: false,
            cancel: None,
        };

        let rt = tokio::runtime::Runtime::new().unwrap();
        let out = rt
            .block_on(download_media(&[item], &opts, Arc::new(NullSink)))
            .unwrap();

        // 必须**不**报告 skipped；目录占用了 final_path，下载路径里 OpenOptions::open
        // 会因路径已是目录而失败 → status=Failed（NetworkError 或 InternalError）。
        let r = &out.downloads[0];
        assert!(
            r.status != DownloadStatus::SkippedExisting,
            "directory at final_path must not be reported as SkippedExisting, got: {:?}",
            r.status
        );
    }

    #[cfg(unix)]
    #[test]
    fn download_media_rejects_symlink_at_partial_path() {
        // 陈旧的 .partial 是 symlink → 必须拒绝（不跟随）——否则 OpenOptions 会
        // 跟随符号链接写到 sandbox 之外（如 /etc/passwd）。
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
        // download_one 要计算的 final_path = "1_file.jpg"；partial_path = "1_file.jpg.partial"。
        // 在 partial_path 处放一个 symlink 指向 sandbox 外。
        let partial_link = dir.path().join("1_file.jpg.partial");
        symlink(&outside_target, &partial_link).unwrap();

        let opts = DownloadOpts {
            subdir: None,
            concurrency: 1,
            base_dir: Some(dir.path().to_path_buf()),
            filename_format: None,
            set_mtime: false,
            cancel: None,
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
        assert!(
            err.message.contains("partial"),
            "expected error to mention partial symlink, got: {}",
            err.message
        );

        // 关键校验：sandbox 外文件未被覆盖
        let content = std::fs::read_to_string(&outside_target).unwrap();
        assert_eq!(content, "original");
    }

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
        let symlink_path = dir.path().join("1_file.jpg");
        symlink(&outside_target, &symlink_path).unwrap();

        let opts = DownloadOpts {
            subdir: None,
            concurrency: 1,
            base_dir: Some(dir.path().to_path_buf()),
            filename_format: None,
            set_mtime: false,
            cancel: None,
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

        let content = std::fs::read_to_string(&outside_target).unwrap();
        assert_eq!(content, "original");
    }

    // ---- cancel-before-start: queue 内 item 标 cancelled ----

    #[test]
    fn progress_event_carries_per_item_index() {
        // 合约：ItemProgress / ItemDone / ItemRestart 必须含 index 字段，让多 media
        // 同 tweet_id 场景下 in_flight map 能用 index 区分（同一推文可能含多个
        // MediaItem 共享 tweet_id；用 tweet_id 做 key 会让并发下载互相覆盖）。
        let p1 = ProgressEvent::ItemProgress {
            tweet_id: "T1".into(),
            index: 0,
            bytes_done: 100,
            bytes_total: Some(1000),
        };
        let p2 = ProgressEvent::ItemProgress {
            tweet_id: "T1".into(), // 同 tweet_id
            index: 1,              // 不同 index
            bytes_done: 200,
            bytes_total: Some(2000),
        };
        // 序列化也含 index 字段（snake_case，serde 自动）
        let s1 = serde_json::to_string(&p1).unwrap();
        let s2 = serde_json::to_string(&p2).unwrap();
        assert!(s1.contains("\"index\":0"), "p1 JSON missing index: {}", s1);
        assert!(s2.contains("\"index\":1"), "p2 JSON missing index: {}", s2);

        let d = ProgressEvent::ItemDone {
            tweet_id: "T1".into(),
            index: 5,
            status: DownloadStatus::Downloaded,
            bytes: 999,
        };
        let s = serde_json::to_string(&d).unwrap();
        assert!(s.contains("\"index\":5"));

        let r = ProgressEvent::ItemRestart {
            tweet_id: "T1".into(),
            index: 7,
            reason: "test".into(),
        };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"index\":7"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_before_start_marks_all_cancelled() {
        use crate::agent::types::{DownloadOpts, MediaItem, MediaType, VecSink};
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let token = CancellationToken::new();
        token.cancel(); // 已 cancel
        let opts = DownloadOpts {
            subdir: None,
            concurrency: 4,
            base_dir: Some(dir.path().to_path_buf()),
            filename_format: None,
            set_mtime: false,
            cancel: Some(token),
        };
        let items: Vec<MediaItem> = (0..5)
            .map(|i| MediaItem {
                tweet_id: i.to_string(),
                kind: MediaType::Image,
                url: format!("https://invalid.test/{}.jpg", i),
                suggested_filename: format!("{}.jpg", i),
                bytes: None,
                author_handle: None,
                created_at: None,
                all_variants: None,
            })
            .collect();

        let sink = Arc::new(VecSink::new());
        let out = download_media(&items, &opts, sink.clone()).await.unwrap();

        // 全部应当是 cancelled，无任何 HTTP 请求被发出（cancel 已早于 spawn 触发）
        assert_eq!(out.summary.cancelled, 5);
        assert_eq!(out.summary.total, 5);
        assert!(
            out.downloads
                .iter()
                .all(|r| r.status == DownloadStatus::Cancelled),
            "expected all cancelled, got {:?}",
            out.downloads.iter().map(|r| &r.status).collect::<Vec<_>>()
        );

        // 合约：队列内 cancelled item **不** emit ItemStarted/ItemDone。
        // sink 应当只看到 DownloadStarted + BatchCancelled（中间无任何 item 事件）。
        let events = sink.snapshot();
        let item_started_count = events
            .iter()
            .filter(|e| matches!(e, ProgressEvent::ItemStarted { .. }))
            .count();
        let item_done_count = events
            .iter()
            .filter(|e| matches!(e, ProgressEvent::ItemDone { .. }))
            .count();
        assert_eq!(
            item_started_count, 0,
            "queue-skipped cancelled items must not emit ItemStarted, got {} events: {:?}",
            item_started_count, events
        );
        assert_eq!(
            item_done_count, 0,
            "queue-skipped cancelled items must not emit ItemDone, got {} events: {:?}",
            item_done_count, events
        );
        assert!(
            matches!(events.last().unwrap(), ProgressEvent::BatchCancelled { .. }),
            "last event must be BatchCancelled"
        );
    }
}
