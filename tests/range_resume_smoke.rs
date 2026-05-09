//! HTTP Range 续传 / ETag 校验 / Content-Range 校验路径的集成测试。
//!
//! 备注：测试用 `std::sync::Mutex` 序列化整个 test binary（HOME 等环境变量是进程级
//! 的，cargo test 默认并行 #[test] 会互相破坏）。clippy 的 `await_holding_lock` 是
//! 通用风险提示，但本场景下：(1) 锁仅在测试间互斥，不跨任务共享；(2) 持锁期间
//! await 是有意为之——把整个测试体保持在 lock guard 生命周期内才能让环境变量
//! 隔离生效。因此整个 file 抑制此 lint。
#![allow(clippy::await_holding_lock)]
//!
//! 在 lib 层直接调 `download_media` + wiremock，不走 MCP binary。
//! 覆盖：
//! - 206 续传成功 + atomic rename
//! - 200 fallback（server 不识别 Range）→ 删 partial 重头下
//! - ETag mismatch → 删 partial、删 cache 条目、重头下
//! - ETag cache 无对应条目 → 删 partial、重头下
//! - Content-Range mismatch → 删 partial、删 cache 条目、重头下

use std::path::PathBuf;
use std::sync::Arc;

use tempfile::tempdir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use x_likes_downloader::agent::download_media;
use x_likes_downloader::agent::etag_cache::{self, EtagCache};
use x_likes_downloader::agent::types::{
    DownloadOpts, DownloadStatus, MediaItem, MediaType, NullSink,
};

/// 全局 mutex 序列化整 binary 内的测试 —— 因为 isolate_cache_dir 设置进程级
/// 环境变量（HOME / XDG_CACHE_HOME），cargo test 默认并行执行 #[test] 会互相破坏。
static ENV_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 锁定 HOME / XDG_CACHE_HOME 等到一个 tempdir，确保 etag-cache.json 与本测试隔离
/// （不污染用户真实 cache）。返回的 guard 必须存活到测试结束。
fn isolate_cache_dir(dir: &std::path::Path) -> std::sync::MutexGuard<'static, ()> {
    let guard = ENV_GUARD
        .lock()
        .unwrap_or_else(|e| e.into_inner()); // poisoned ok（前一个测试 panic）
    std::env::set_var("HOME", dir);
    std::env::set_var("XDG_CACHE_HOME", dir.join("cache"));
    // macOS 用 ~/Library/Caches；Linux 用 XDG_CACHE_HOME；Windows 用 LOCALAPPDATA
    std::env::set_var("LOCALAPPDATA", dir.join("local-appdata"));
    // 防 dotenv / private_tokens 加载
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

fn opts_with_base(base: PathBuf) -> DownloadOpts {
    DownloadOpts {
        subdir: None,
        concurrency: 1,
        base_dir: Some(base),
        filename_format: None,
        set_mtime: false,
        cancel: None,
    }
}

const FILE_BYTES: &[u8] =
    b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()_+-=[]{}|;':\",./<>?\n";

#[tokio::test(flavor = "multi_thread")]
async fn fresh_download_writes_to_partial_then_renames() {
    let tmp = tempdir().unwrap();
    let _env_guard = isolate_cache_dir(tmp.path());

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string())
                .set_body_bytes(FILE_BYTES),
        )
        .mount(&server)
        .await;

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();
    std::fs::create_dir_all(&sandbox).unwrap();
    let item = make_item("1", &format!("{}/x.bin", server.uri()), "x.bin");

    let out = download_media(&[item], &opts_with_base(sandbox.clone()), Arc::new(NullSink))
        .await
        .unwrap();
    assert_eq!(out.summary.downloaded, 1);
    // download_media 内部用 sandbox 模块 canonicalize 了 base_dir，因此 final_path
    // 含 macOS 上的 /private/tmp/... 形式（与测试本地构造的 sandbox.join() 不一致）。
    // 直接从 DownloadResult.path 读取已规范化的真实路径。
    let final_path = out.downloads[0].path.clone();
    assert!(final_path.exists(), "final file must exist");
    let partial_path = PathBuf::from(format!("{}.partial", final_path.display()));
    assert!(!partial_path.exists(), ".partial must be cleaned by rename");
    let bytes = std::fs::read(&final_path).unwrap();
    assert_eq!(bytes, FILE_BYTES);

    let cache_path = EtagCache::path().unwrap();
    let entry =
        etag_cache::peek_entry(&cache_path, &EtagCache::key_for(&final_path)).expect("cache entry");
    assert_eq!(entry.etag, "\"v1\"");
    assert_eq!(entry.size, FILE_BYTES.len() as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn range_resume_206_completes() {
    let tmp = tempdir().unwrap();
    let _env_guard = isolate_cache_dir(tmp.path());

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();
    std::fs::create_dir_all(&sandbox).unwrap();
    let final_path = sandbox.join("1_x.bin");
    let partial_path = PathBuf::from(format!("{}.partial", final_path.display()));

    // 先启 mock server，再用其真实 URL 播种 cache。否则 cache 中的 url 与 item.url
    // 不匹配，download_media 会走 URL-mismatch restart 而非 Range 续传——绕过本测试
    // 要覆盖的代码路径。
    let server = MockServer::start().await;
    let item_url = format!("{}/x.bin", server.uri());

    // 预置一个 partial 文件（前 30 字节）+ ETag cache 条目（etag 与 server 一致，url 与 item.url 一致）
    let partial_size = 30usize;
    std::fs::write(&partial_path, &FILE_BYTES[..partial_size]).unwrap();
    let cache_path = EtagCache::path().unwrap();
    etag_cache::put_entry(
        &cache_path,
        EtagCache::key_for(&final_path),
        EtagCache::make_entry("\"v1\"", item_url.clone(), FILE_BYTES.len() as u64),
    )
    .unwrap();

    // HEAD 返回 v1
    Mock::given(method("HEAD"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string()),
        )
        .mount(&server)
        .await;
    // GET with Range → 206 with Content-Range
    Mock::given(method("GET"))
        .and(path("/x.bin"))
        .respond_with(move |req: &Request| {
            let range_header = req
                .headers
                .get("range")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if range_header.starts_with("bytes=") {
                // 解析 start
                let start: usize = range_header
                    .trim_start_matches("bytes=")
                    .trim_end_matches('-')
                    .parse()
                    .unwrap_or(0);
                let body = &FILE_BYTES[start..];
                ResponseTemplate::new(206)
                    .insert_header("ETag", "\"v1\"")
                    .insert_header(
                        "Content-Range",
                        format!("bytes {}-{}/{}", start, FILE_BYTES.len() - 1, FILE_BYTES.len())
                            .as_str(),
                    )
                    .insert_header("Content-Length", body.len().to_string())
                    .set_body_bytes(body.to_vec())
            } else {
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"v1\"")
                    .set_body_bytes(FILE_BYTES.to_vec())
            }
        })
        .mount(&server)
        .await;

    let item = make_item("1", &item_url, "x.bin");
    let out = download_media(&[item], &opts_with_base(sandbox.clone()), Arc::new(NullSink))
        .await
        .unwrap();
    assert_eq!(out.summary.downloaded, 1);
    let bytes = std::fs::read(&final_path).unwrap();
    assert_eq!(bytes, FILE_BYTES, "续传后文件应当与原始一致");
    assert!(!partial_path.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn server_returns_200_to_range_triggers_restart() {
    let tmp = tempdir().unwrap();
    let _env_guard = isolate_cache_dir(tmp.path());

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();
    std::fs::create_dir_all(&sandbox).unwrap();
    let final_path = sandbox.join("1_x.bin");
    let partial_path = PathBuf::from(format!("{}.partial", final_path.display()));

    // 先启 server 拿真实 URL（cache.url 必须与 item.url 一致才能进入 Range 路径）
    let server = MockServer::start().await;
    let item_url = format!("{}/x.bin", server.uri());

    std::fs::write(&partial_path, &FILE_BYTES[..30]).unwrap();
    let cache_path = EtagCache::path().unwrap();
    etag_cache::put_entry(
        &cache_path,
        EtagCache::key_for(&final_path),
        EtagCache::make_entry("\"v1\"", item_url.clone(), FILE_BYTES.len() as u64),
    )
    .unwrap();

    Mock::given(method("HEAD"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string()),
        )
        .mount(&server)
        .await;
    // GET 永远返回 200，不识别 Range
    Mock::given(method("GET"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string())
                .set_body_bytes(FILE_BYTES.to_vec()),
        )
        .mount(&server)
        .await;

    let item = make_item("1", &item_url, "x.bin");
    let out = download_media(&[item], &opts_with_base(sandbox.clone()), Arc::new(NullSink))
        .await
        .unwrap();
    assert_eq!(out.summary.downloaded, 1);
    let bytes = std::fs::read(&final_path).unwrap();
    assert_eq!(bytes, FILE_BYTES);
}

#[tokio::test(flavor = "multi_thread")]
async fn etag_mismatch_triggers_restart() {
    let tmp = tempdir().unwrap();
    let _env_guard = isolate_cache_dir(tmp.path());

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();
    std::fs::create_dir_all(&sandbox).unwrap();
    let final_path = sandbox.join("1_x.bin");
    let partial_path = PathBuf::from(format!("{}.partial", final_path.display()));

    // 先启 server，让 cache 的 url 与 item.url 一致——本测试要测的是**ETag**
    // mismatch 路径，不是 URL mismatch。
    let server = MockServer::start().await;
    let item_url = format!("{}/x.bin", server.uri());

    // partial 存在；cache 中 etag 是 v1，server HEAD 返回 v2 → ETag 失配触发 restart
    std::fs::write(&partial_path, b"old partial bytes").unwrap();
    let cache_path = EtagCache::path().unwrap();
    etag_cache::put_entry(
        &cache_path,
        EtagCache::key_for(&final_path),
        EtagCache::make_entry("\"v1\"", item_url.clone(), 9999),
    )
    .unwrap();

    Mock::given(method("HEAD"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v2\"") // 与 cache 不一致
                .insert_header("Content-Length", FILE_BYTES.len().to_string()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v2\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string())
                .set_body_bytes(FILE_BYTES.to_vec()),
        )
        .mount(&server)
        .await;

    let item = make_item("1", &item_url, "x.bin");
    let out = download_media(&[item], &opts_with_base(sandbox.clone()), Arc::new(NullSink))
        .await
        .unwrap();
    assert_eq!(out.summary.downloaded, 1);
    let bytes = std::fs::read(&final_path).unwrap();
    assert_eq!(bytes, FILE_BYTES, "ETag 失配后应重新下到完整内容");
    // cache 应当含新 entry（etag = v2）
    let new_entry =
        etag_cache::peek_entry(&cache_path, &EtagCache::key_for(&final_path)).expect("cache");
    assert_eq!(new_entry.etag, "\"v2\"");
}

#[tokio::test(flavor = "multi_thread")]
async fn no_etag_cache_entry_triggers_restart() {
    let tmp = tempdir().unwrap();
    let _env_guard = isolate_cache_dir(tmp.path());

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();
    std::fs::create_dir_all(&sandbox).unwrap();
    let final_path = sandbox.join("1_x.bin");
    let partial_path = PathBuf::from(format!("{}.partial", final_path.display()));

    // partial 存在但 cache 无对应条目（cache 文件为空）
    std::fs::write(&partial_path, b"old partial").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string())
                .set_body_bytes(FILE_BYTES.to_vec()),
        )
        .mount(&server)
        .await;

    let item = make_item("1", &format!("{}/x.bin", server.uri()), "x.bin");
    let out = download_media(&[item], &opts_with_base(sandbox.clone()), Arc::new(NullSink))
        .await
        .unwrap();
    assert_eq!(out.summary.downloaded, 1);
    let bytes = std::fs::read(&final_path).unwrap();
    assert_eq!(bytes, FILE_BYTES);
}

#[tokio::test(flavor = "multi_thread")]
async fn content_range_mismatch_triggers_restart() {
    let tmp = tempdir().unwrap();
    let _env_guard = isolate_cache_dir(tmp.path());

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();
    std::fs::create_dir_all(&sandbox).unwrap();
    let final_path = sandbox.join("1_x.bin");
    let partial_path = PathBuf::from(format!("{}.partial", final_path.display()));

    // 先启 server 让 cache.url 与 item.url 一致（避免走 URL-mismatch restart 路径）
    let server = MockServer::start().await;
    let item_url = format!("{}/x.bin", server.uri());

    // partial 30 bytes；cache etag / size / url 都一致 → 进入 Range 续传，server 返回错的 Content-Range 触发 mismatch
    std::fs::write(&partial_path, &FILE_BYTES[..30]).unwrap();
    let cache_path = EtagCache::path().unwrap();
    etag_cache::put_entry(
        &cache_path,
        EtagCache::key_for(&final_path),
        EtagCache::make_entry("\"v1\"", item_url.clone(), FILE_BYTES.len() as u64),
    )
    .unwrap();

    Mock::given(method("HEAD"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string()),
        )
        .mount(&server)
        .await;

    // 第一次 GET：返回 206 但 Content-Range 起始字节错（声称从 0 开始而不是 30）→ 触发 mismatch
    // 第二次 GET（restart 后无 Range header）：返回 200 全量
    use std::sync::atomic::{AtomicUsize, Ordering};
    static GET_COUNTER: AtomicUsize = AtomicUsize::new(0);
    GET_COUNTER.store(0, Ordering::SeqCst);
    Mock::given(method("GET"))
        .and(path("/x.bin"))
        .respond_with(move |req: &Request| {
            let n = GET_COUNTER.fetch_add(1, Ordering::SeqCst);
            let has_range = req.headers.get("range").is_some();
            if n == 0 && has_range {
                // 故意给错的 Content-Range（起始 0 而非 30）
                ResponseTemplate::new(206)
                    .insert_header("ETag", "\"v1\"")
                    .insert_header(
                        "Content-Range",
                        format!("bytes 0-{}/{}", FILE_BYTES.len() - 1, FILE_BYTES.len()).as_str(),
                    )
                    .insert_header("Content-Length", FILE_BYTES.len().to_string())
                    .set_body_bytes(FILE_BYTES.to_vec())
            } else {
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"v1\"")
                    .insert_header("Content-Length", FILE_BYTES.len().to_string())
                    .set_body_bytes(FILE_BYTES.to_vec())
            }
        })
        .mount(&server)
        .await;

    let item = make_item("1", &item_url, "x.bin");
    let out = download_media(&[item], &opts_with_base(sandbox.clone()), Arc::new(NullSink))
        .await
        .unwrap();
    // Content-Range mismatch 后应触发 restart 并 OK
    assert_eq!(out.summary.downloaded, 1, "{:?}", out.downloads);
    let bytes = std::fs::read(&final_path).unwrap();
    assert_eq!(bytes, FILE_BYTES);
}

// 注：流尾完整性校验（chunked 206 响应 EOF 时 downloaded < total）的端到端测试需要
// chunked transfer 流式 mock，wiremock 0.6 不支持。退化方案"server body 短于
// Content-Length"会让 reqwest 等到 client timeout（120s）才报错，单测耗时不可接受。
// 校验逻辑参见 download_media.rs 中 chunk loop 的 EOF 分支（None => 检查
// total_bytes 是否已达），靠人工审阅与活体测试覆盖。

/// 配置一个 HEAD mock：让 wiremock 同时设 body（自动算 Content-Length）和 explicit
/// header。reqwest 处理 HEAD 响应时只读 header 不读 body，但 wiremock 必须用 body
/// 把 Content-Length 头串过来。
fn head_mock_with_length(etag: &str, length: usize) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("ETag", etag)
        .insert_header("Content-Length", length.to_string())
        .set_body_bytes(vec![0u8; length])
}

// 注：原本计划测试"v2.0 truncated final 文件 → HEAD verify 后重新下载"——但 wiremock
// 0.6 在 HEAD 响应上 strip body 并重算 Content-Length=0（即使 mock 显式 insert_header
// 一个非零值也被覆盖）。production 的 X CDN 不存在这个 quirk——HEAD 返回真实
// Content-Length。这条测试因 mock 框架限制无法可靠实现，依赖代码层面的 Some(0)→trust
// 兜底（详见 download_media.rs 中 SkippedExisting 决策注释）。其它场景（complete file
// 应当 skip / .partial 续传）的覆盖已足够说明 SkippedExisting 合约。

// 注：cache fast-path URL 双匹配的端到端测试因 wiremock HEAD body-strip 限制
// （Content-Length=0 兜底为 trust）无法可靠区分"快路径假命中"vs "fall-through 到
// HEAD verify trust"。校验逻辑是单行 `size && url && finalized` 三字段 guard
// （见 download_media.rs SkippedExisting 决策）；同型场景由
// cached_url_mismatch_triggers_restart（resume 路径的双匹配）覆盖。

#[tokio::test(flavor = "multi_thread")]
async fn complete_v20_final_file_skipped_after_head_verify() {
    // 合约：v2.0 升级场景——final 文件已存在且 size 与 server Content-Length 一致
    // （v2.0 完整下载的文件）。HEAD verify 通过 → 仍走 SkippedExisting（升级用户
    // 不需要重新下完整文件）。
    let tmp = tempdir().unwrap();
    let _env_guard = isolate_cache_dir(tmp.path());

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();
    let final_path = sandbox.join("1_x.bin");

    // v2.0 留下的完整 final：长度与 server Content-Length 一致
    std::fs::write(&final_path, FILE_BYTES).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/x.bin"))
        .respond_with(head_mock_with_length("\"v1\"", FILE_BYTES.len()))
        .mount(&server)
        .await;

    let item = make_item("1", &format!("{}/x.bin", server.uri()), "x.bin");
    let out = download_media(&[item], &opts_with_base(sandbox.clone()), Arc::new(NullSink))
        .await
        .unwrap();
    assert_eq!(out.summary.skipped, 1, "complete v2.0 file should be skipped, got: {:?}", out.downloads);
    assert_eq!(out.downloads[0].status, DownloadStatus::SkippedExisting);
}

#[tokio::test(flavor = "multi_thread")]
async fn cached_url_mismatch_triggers_restart() {
    // 合约：cache 中记录的 url 与当前 item.url 不同时，**即使** ETag 字符串相同
    // 也不能续传。否则不同 URL 同 final 文件名 + 巧合相同 ETag 会拼接旧/新字节，
    // corrupt 最终文件。
    let tmp = tempdir().unwrap();
    let _env_guard = isolate_cache_dir(tmp.path());

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();
    let final_path = sandbox.join("1_x.bin");
    let partial_path = PathBuf::from(format!("{}.partial", final_path.display()));

    // 预置 partial + cache entry：cache 记的 URL 是 "https://old-host/x.bin"，
    // 当前 item URL 将指向 wiremock。两者 ETag 字符串故意相同。
    std::fs::write(&partial_path, b"old-bytes-from-different-host").unwrap();
    let cache_path = EtagCache::path().unwrap();
    etag_cache::put_entry(
        &cache_path,
        EtagCache::key_for(&final_path),
        EtagCache::make_entry("\"v1\"", "https://old-host.example/x.bin", FILE_BYTES.len() as u64),
    )
    .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"") // 与 cache 中相同
                .insert_header("Content-Length", FILE_BYTES.len().to_string()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string())
                .set_body_bytes(FILE_BYTES.to_vec()),
        )
        .mount(&server)
        .await;

    // item.url 与 cache 中记的 url 不同（即使两者 ETag 都是 "v1"）
    let item = make_item("1", &format!("{}/x.bin", server.uri()), "x.bin");
    let out = download_media(&[item], &opts_with_base(sandbox.clone()), Arc::new(NullSink))
        .await
        .unwrap();
    assert_eq!(out.summary.downloaded, 1);
    let bytes = std::fs::read(&final_path).unwrap();
    // 必须等于完整 FILE_BYTES（即从头下载），而不是 "old-bytes-from-different-host" + FILE_BYTES 拼接
    assert_eq!(bytes, FILE_BYTES, "URL mismatch 必须重头下，不能 append 旧 partial 字节");
    // cache 应当被 URL 匹配后的新条目覆盖
    let new_entry =
        etag_cache::peek_entry(&cache_path, &EtagCache::key_for(&final_path)).expect("cache");
    assert!(
        new_entry.url.contains(&server.uri()),
        "cache.url should be updated to new server URL, got: {}",
        new_entry.url
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn partial_alone_does_not_trigger_skipped_existing() {
    // 验证：仅 .partial 存在（final 不存在）→ 系统走下载路径，不沉默 skip。
    let tmp = tempdir().unwrap();
    let _env_guard = isolate_cache_dir(tmp.path());

    let sandbox = tmp.path().join("sandbox");
    std::fs::create_dir_all(&sandbox).unwrap();
    let sandbox = sandbox.canonicalize().unwrap();
    std::fs::create_dir_all(&sandbox).unwrap();
    let final_path = sandbox.join("1_x.bin");
    let partial_path = PathBuf::from(format!("{}.partial", final_path.display()));
    std::fs::write(&partial_path, b"partial").unwrap();
    assert!(!final_path.exists());

    let server = MockServer::start().await;
    Mock::given(method("HEAD"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/x.bin"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .insert_header("Content-Length", FILE_BYTES.len().to_string())
                .set_body_bytes(FILE_BYTES.to_vec()),
        )
        .mount(&server)
        .await;

    let item = make_item("1", &format!("{}/x.bin", server.uri()), "x.bin");
    let out = download_media(&[item], &opts_with_base(sandbox.clone()), Arc::new(NullSink))
        .await
        .unwrap();
    assert_eq!(out.summary.downloaded, 1);
    assert_eq!(out.summary.skipped, 0);
    assert_eq!(
        out.downloads[0].status,
        DownloadStatus::Downloaded,
        "partial-only must NOT be skipped_existing"
    );
}
