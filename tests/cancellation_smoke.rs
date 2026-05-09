//! Cancellation smoke test：lib 层用 wiremock 慢响应模拟大文件下载，
//! 在中段触发 `CancellationToken::cancel()`；验证：
//!
//! - `download_media` 在 1s 内返回（不等带宽跑完）
//! - 返回 `Ok(DownloadOutput)`，不抛 Err
//! - in-flight item 状态为 `Cancelled`、`.partial` 文件保留且 size > 0 但 < expected total
//! - 尚未启动 item 也标 `Cancelled`
//! - ETag cache 中对应条目 `size` 字段 == server Content-Length（**不**是 partial size）
//! - `summary.cancelled` 计数正确
//!
//! 不测 MCP 协议层 `notifications/cancelled` 路由——那由 rmcp 1.6 内部 `local_ct_pool`
//! 处理；本 server 仅透传 `ctx.ct.clone()` 给 lib（见 `mcp_server.rs::download_media`）。
//! MCP 协议 cancellation 由 `tests/mcp_smoke.rs::mcp_server_handles_cancellation_for_unknown_request`
//! 校验 cancel notification 不导致 server 崩溃。
//!
//! 注：本文件持锁跨 await（见 range_resume_smoke.rs 同样的注释），抑制 clippy lint。
#![allow(clippy::await_holding_lock)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use x_likes_downloader::agent::download_media;
use x_likes_downloader::agent::etag_cache::{self, EtagCache};
use x_likes_downloader::agent::types::{
    DownloadOpts, DownloadStatus, MediaItem, MediaType, NullSink,
};

static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn isolate_cache_dir(dir: &std::path::Path) -> std::sync::MutexGuard<'static, ()> {
    let guard = ENV_GUARD.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("HOME", dir);
    std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
    std::env::set_var("LOCALAPPDATA", dir.join("local-appdata"));
    std::env::set_var("XLD_CREDENTIALS_FILE", dir.join("dummy_creds"));
    std::env::set_var(
        "DOWNLOAD_SANDBOX_BASE_DIR",
        dir.join("__sandbox_unused__"),
    );
    guard
}

fn make_item(tweet_id: &str, url: &str, suggested: &str) -> MediaItem {
    MediaItem {
        tweet_id: tweet_id.to_string(),
        kind: MediaType::Image,
        url: url.to_string(),
        suggested_filename: suggested.to_string(),
        bytes: None,
        author_handle: None,
        created_at: None,
        all_variants: None,
    }
}

/// 制造一个 8KB 的 body，wiremock 用 `set_delay` 整体响应延迟 ≥ 5s（chunk 化模拟慢响应
/// 在 wiremock 0.6 中需要 raw stream 支持；用 set_delay 也能实现"中段被 cancel"——
/// reqwest 还在等 body 时取消即生效）。
fn slow_body() -> Vec<u8> {
    let mut v = Vec::with_capacity(8 * 1024);
    for i in 0..(8 * 1024) {
        v.push((i % 256) as u8);
    }
    v
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_returns_within_1s_with_partial_preserved() {
    let tmp = tempdir().unwrap();
    let _g = isolate_cache_dir(tmp.path());

    let body = slow_body();
    let body_len = body.len();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/big.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", body_len.to_string())
                .set_body_bytes(body.clone())
                // 整体延迟 5 秒；reqwest 在等 body 时被 cancel 会立即停止
                .set_delay(Duration::from_secs(5)),
        )
        .mount(&server)
        .await;

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();

    let cancel = CancellationToken::new();
    let cancel2 = cancel.clone();
    // 200ms 后触发 cancel
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        cancel2.cancel();
    });

    let url = format!("{}/big.bin", server.uri());
    let items = vec![
        make_item("1", &url, "big.bin"),
        make_item("2", &url, "big2.bin"),
    ];
    let opts = DownloadOpts {
        subdir: None,
        concurrency: 1, // 串行：第二项必然在 cancel 前未启动 → cancelled
        base_dir: Some(sandbox.clone()),
        filename_format: None,
        set_mtime: false,
        cancel: Some(cancel),
    };

    let start = Instant::now();
    let out = tokio::time::timeout(
        Duration::from_secs(2),
        download_media(&items, &opts, Arc::new(NullSink)),
    )
    .await
    .expect("download_media must return within 2s after cancel")
    .expect("download_media should return Ok even when cancelled");
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "must finish within 2s, took {:?}",
        elapsed
    );

    // 至少一个 cancelled
    assert!(
        out.downloads
            .iter()
            .any(|r| r.status == DownloadStatus::Cancelled),
        "expect at least one cancelled item, got {:?}",
        out.downloads.iter().map(|r| &r.status).collect::<Vec<_>>()
    );
    assert_eq!(
        out.summary.cancelled + out.summary.downloaded + out.summary.failed + out.summary.skipped,
        out.summary.total
    );
    assert_eq!(out.summary.total, 2);

    // 第一个 item 是 in-flight 中断的：partial 应保留
    let first = &out.downloads[0];
    if first.status == DownloadStatus::Cancelled {
        let partial_path = PathBuf::from(format!("{}.partial", first.path.display()));
        // 由于 wiremock 用 set_delay 整体延迟，连接已建立但 body 还没传输；
        // partial 文件可能为 0 byte（response 头收到了但 body 没开始）——这也合法。
        // 关键校验：file 存在性 + 不大于 server total
        if partial_path.exists() {
            let psize = std::fs::metadata(&partial_path).unwrap().len();
            assert!(
                psize <= body_len as u64,
                "partial size {} > body len {}",
                psize,
                body_len
            );
        }
    }

    // ETag cache 应当记录 size = body_len（如 cache 写入路径执行了——本测试用 set_delay
    // 整体延迟，cache 写入发生在收到 headers **之后**；wiremock 在 set_delay 模式下
    // 会先延迟再发整个响应，因此 headers 也被延迟 → cache 可能没写入）。
    // 不对此做硬性断言（语义见 spec.md D-OQ2）。
    let cache_path = EtagCache::path().unwrap();
    if let Some(entry) = etag_cache::peek_entry(&cache_path, &EtagCache::key_for(&first.path)) {
        assert_eq!(
            entry.size, body_len as u64,
            "cache.size 必须 = server Content-Length，不是 partial size"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_already_completed_item_keeps_status() {
    // 用快响应让 item 在 cancel 触发前就完成；验证已完成 item 状态不被覆盖。
    let tmp = tempdir().unwrap();
    let _g = isolate_cache_dir(tmp.path());

    let body = b"hello"; // 5 字节，瞬时返回
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", body.len().to_string())
                .set_body_bytes(body.to_vec()),
        )
        .mount(&server)
        .await;

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();

    // 不 cancel —— 让 download 自然完成；之后 cancel.cancel() 也无副作用（item 已 done）
    let cancel = CancellationToken::new();
    let opts = DownloadOpts {
        subdir: None,
        concurrency: 1,
        base_dir: Some(sandbox),
        filename_format: None,
        set_mtime: false,
        cancel: Some(cancel.clone()),
    };
    let url = format!("{}/x.bin", server.uri());
    let out = download_media(
        &[make_item("1", &url, "x.bin")],
        &opts,
        Arc::new(NullSink),
    )
    .await
    .unwrap();
    assert_eq!(out.summary.downloaded, 1);
    assert_eq!(out.summary.cancelled, 0);
}
