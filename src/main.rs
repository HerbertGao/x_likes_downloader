use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};

use x_likes_downloader::agent::types::{
    AuthStatus, DownloadOpts, ListOpts, MediaItem, NdjsonStderrSink, ProgressEvent, ProgressSink,
};
use x_likes_downloader::agent::{auth_status, download_media, list_likes};
use x_likes_downloader::config::{self, Config};
use x_likes_downloader::envelope::{Meta, OutputEnvelope};
use x_likes_downloader::error::{ErrorKind, ErrorPayload};
use x_likes_downloader::organize_files::FileOrganizer;
use x_likes_downloader::setup::{self, SetupArgs};
use x_likes_downloader::updater::Updater;

#[derive(Parser)]
#[command(name = "x_likes_downloader")]
#[command(about = "X点赞推文媒体下载器（兼具 Agent Skill 模式）")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 初始化配置
    Setup(SetupArgs),
    /// 下载点赞的推文媒体（人类一键模式，行为同旧版本）
    Download,
    /// 整理下载的文件
    Organize {
        source_dir: Option<String>,
        target_dir: Option<String>,
    },
    /// 检查并更新到最新版本
    Update,
    /// 列出当前账号的点赞推文（Agent 友好的 JSON 输出）
    Likes {
        #[command(subcommand)]
        action: LikesAction,
    },
    /// 按 media item 下载媒体到沙箱目录（Agent 友好）
    Media {
        #[command(subcommand)]
        action: MediaAction,
    },
    /// 凭据健康自检
    Auth {
        #[command(subcommand)]
        action: AuthAction,
    },
}

#[derive(Subcommand)]
enum LikesAction {
    /// 列出点赞，输出 JSON 到 stdout
    List {
        /// 拉取全部历史点赞
        #[arg(long)]
        all: bool,
        /// 从指定游标继续拉取
        #[arg(long)]
        since_cursor: Option<String>,
        /// 单页条数
        #[arg(long)]
        count: Option<u32>,
        /// 同时携带原始 GraphQL entry（大幅增加输出体积，仅用于调试）
        #[arg(long)]
        include_raw: bool,
        /// 显式声明 JSON 输出（保留供未来扩展，新子命令默认即 JSON）
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum MediaAction {
    /// 按 media item 下载到沙箱
    Download {
        /// MediaItem[] JSON（直接传值或 @<file>）
        #[arg(long)]
        items: Option<String>,
        /// 逗号分隔的 tweet ID 列表（快捷方式：内部触发 list_likes 拉详情）
        #[arg(long)]
        ids: Option<String>,
        /// 沙箱内子目录名
        #[arg(long)]
        subdir: Option<String>,
        /// 并发下载数（[1, 16]，默认 4）
        #[arg(long, default_value_t = 4)]
        concurrency: u32,
        /// JSON 输出（stdout 信封 + stderr NDJSON 进度）
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum AuthAction {
    /// 探测当前凭据是否仍可访问 Likes 端点
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let exit_code = match cli.command {
        Commands::Setup(args) => {
            if args.json {
                match setup::run_setup_json(args) {
                    Ok(out) => {
                        let data = serde_json::to_value(&out).unwrap_or(serde_json::Value::Null);
                        OutputEnvelope::success(data).emit()
                    }
                    Err(payload) => OutputEnvelope::failure(payload).emit(),
                }
            } else {
                setup::run_setup(args)?;
                0
            }
        }
        Commands::Download => {
            run_download_legacy().await?;
            0
        }
        Commands::Organize {
            source_dir,
            target_dir,
        } => {
            run_organize(source_dir, target_dir)?;
            0
        }
        Commands::Update => {
            let updater = Updater::new()?;
            updater.update().await?;
            0
        }
        Commands::Likes { action } => match action {
            LikesAction::List {
                all,
                since_cursor,
                count,
                include_raw,
                json: _,
            } => run_likes_list(all, since_cursor, count, include_raw).await,
        },
        Commands::Media { action } => match action {
            MediaAction::Download {
                items,
                ids,
                subdir,
                concurrency,
                json,
            } => run_media_download(items, ids, subdir, concurrency, json).await,
        },
        Commands::Auth { action } => match action {
            AuthAction::Status { json: _ } => run_auth_status().await,
        },
    };

    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

fn run_organize(source_dir: Option<String>, target_dir: Option<String>) -> Result<()> {
    let config = config::Config::load()?;
    let src_owned;
    let tgt_owned;
    let src = match source_dir {
        Some(ref s) => s.as_str(),
        None => {
            if config.download_dir.is_empty() {
                return Err(anyhow::anyhow!("请通过参数或.env指定源目录"));
            }
            src_owned = config.download_dir.clone();
            &src_owned
        }
    };
    let tgt = match target_dir {
        Some(ref t) => t.as_str(),
        None => {
            if config.target_dir.is_empty() {
                return Err(anyhow::anyhow!("请通过参数或.env指定目标目录"));
            }
            tgt_owned = config.target_dir.clone();
            &tgt_owned
        }
    };
    FileOrganizer::organize_files(src, tgt)?;
    Ok(())
}

async fn run_likes_list(
    all: bool,
    since_cursor: Option<String>,
    count: Option<u32>,
    include_raw: bool,
) -> i32 {
    let opts = ListOpts {
        all,
        since_cursor,
        count,
        include_raw,
    };

    match list_likes(opts).await {
        Ok(out) => {
            let cursor = out.cursor.clone();
            let data = serde_json::to_value(&out).unwrap_or(serde_json::Value::Null);
            let mut env = OutputEnvelope::success(data);
            env.meta.cursor = cursor;
            env.emit()
        }
        Err(payload) => OutputEnvelope::failure(payload).emit(),
    }
}

async fn run_media_download(
    items_arg: Option<String>,
    ids_arg: Option<String>,
    subdir: Option<String>,
    concurrency: u32,
    json: bool,
) -> i32 {
    if items_arg.is_some() && ids_arg.is_some() {
        return OutputEnvelope::failure_kind(ErrorKind::InvalidArgument, "--items 与 --ids 互斥")
            .emit();
    }
    if items_arg.is_none() && ids_arg.is_none() {
        return OutputEnvelope::failure_kind(
            ErrorKind::InvalidArgument,
            "必须提供 --items 或 --ids 之一",
        )
        .emit();
    }

    let items: Vec<MediaItem> = if let Some(items_input) = items_arg {
        match parse_items_input(&items_input) {
            Ok(v) => v,
            Err(e) => {
                return OutputEnvelope::failure(e).emit();
            }
        }
    } else {
        // --ids path: validate, then call list_likes to derive items
        let ids = ids_arg.unwrap();
        for id in ids.split(',') {
            let id = id.trim();
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
                return OutputEnvelope::failure_kind(
                    ErrorKind::InvalidArgument,
                    format!("非法 tweet ID: {:?}", id),
                )
                .emit();
            }
        }
        let want: std::collections::HashSet<String> = ids
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        match list_likes(ListOpts {
            all: true,
            since_cursor: None,
            count: None,
            include_raw: false,
        })
        .await
        {
            Ok(out) => {
                // Fail-fast：任何 requested ID 若未在最近点赞中找到（且带媒体），
                // 都返回 failure 信封并列出全部 missing，避免 Agent 误以为都已处理。
                let missing =
                    x_likes_downloader::agent::types::compute_missing_ids(&want, &out.tweets);
                if !missing.is_empty() {
                    return OutputEnvelope::failure_kind(
                        ErrorKind::InvalidArgument,
                        format!(
                            "以下 ID 未在最近点赞中找到媒体: {}（提示：可改用 --items 显式传入媒体描述，或先调 list_likes 确认 ID 在最近点赞内）",
                            missing.join(", ")
                        ),
                    )
                    .emit();
                }

                // 全部 IDs 都有 media，收集后进入下载
                let mut collected: Vec<MediaItem> = Vec::new();
                for tweet in out.tweets {
                    if want.contains(&tweet.id) {
                        collected.extend(tweet.media);
                    }
                }
                collected
            }
            Err(payload) => return OutputEnvelope::failure(payload).emit(),
        }
    };

    let opts = DownloadOpts {
        subdir,
        concurrency,
        ..Default::default()
    };

    let sink: Arc<dyn ProgressSink> = if json {
        Arc::new(NdjsonStderrSink)
    } else {
        Arc::new(IndicatifSink::new())
    };

    match download_media(&items, &opts, sink).await {
        Ok(out) => {
            let any_success = out.summary.downloaded > 0 || out.summary.skipped > 0;
            let total = out.summary.total;
            let data = serde_json::to_value(&out).unwrap_or(serde_json::Value::Null);

            if total > 0 && !any_success {
                // 全失败：emit failure 信封，但保留 data 让 Agent 仍能看到逐项失败详情。
                let payload = ErrorPayload::new(
                    ErrorKind::NetworkError,
                    format!("全部 {} 个 media item 下载失败", total),
                );
                let envelope = OutputEnvelope {
                    ok: false,
                    data: Some(data),
                    meta: Meta::default(),
                    error: Some(payload),
                };
                return envelope.emit();
            }

            OutputEnvelope::success(data).emit()
        }
        Err(payload) => OutputEnvelope::failure(payload).emit(),
    }
}

async fn run_auth_status() -> i32 {
    let status = auth_status().await;
    match status {
        AuthStatus::Healthy { ref checked_at } => {
            let env = OutputEnvelope::success(serde_json::json!({
                "status": "healthy",
                "checked_at": checked_at,
            }));
            env.emit()
        }
        AuthStatus::AuthExpired => {
            OutputEnvelope::failure(ErrorPayload::new(ErrorKind::AuthExpired, "凭据失效")).emit()
        }
        AuthStatus::EndpointStale => OutputEnvelope::failure(ErrorPayload::new(
            ErrorKind::EndpointStale,
            "X GraphQL 端点已变更",
        ))
        .emit(),
        AuthStatus::RateLimited { retry_after } => {
            let mut payload = ErrorPayload::new(ErrorKind::RateLimited, "被限流");
            if let Some(ra) = retry_after {
                payload = payload.with_retry_after(ra);
            }
            OutputEnvelope::failure(payload).emit()
        }
        AuthStatus::NetworkError { message } => {
            OutputEnvelope::failure(ErrorPayload::new(ErrorKind::NetworkError, message)).emit()
        }
        AuthStatus::NotConfigured => {
            OutputEnvelope::failure(ErrorPayload::new(ErrorKind::NotConfigured, "未导入凭据"))
                .emit()
        }
    }
}

fn parse_items_input(input: &str) -> Result<Vec<MediaItem>, ErrorPayload> {
    let json_str = if let Some(path) = input.strip_prefix('@') {
        std::fs::read_to_string(path).map_err(|e| {
            ErrorPayload::new(
                ErrorKind::InvalidArgument,
                format!("读取 items 文件失败: {}", e),
            )
        })?
    } else {
        input.to_string()
    };
    serde_json::from_str::<Vec<MediaItem>>(&json_str).map_err(|e| {
        ErrorPayload::new(
            ErrorKind::InvalidItem,
            format!("解析 items JSON 失败: {}", e),
        )
    })
}

/// Indicatif-backed progress sink for non-JSON mode.
struct IndicatifSink {
    bar: std::sync::Mutex<Option<ProgressBar>>,
}

impl IndicatifSink {
    fn new() -> Self {
        Self {
            bar: std::sync::Mutex::new(None),
        }
    }
}

impl ProgressSink for IndicatifSink {
    fn emit(&self, event: ProgressEvent) {
        match event {
            ProgressEvent::DownloadStarted { total, .. } => {
                let pb = ProgressBar::new(total as u64);
                pb.set_style(
                    ProgressStyle::default_bar()
                        .template(
                            "{spinner:.green} [{elapsed_precise}] [{bar:30.cyan/blue}] {pos}/{len} ({eta})",
                        )
                        .unwrap()
                        .progress_chars("#>-"),
                );
                *self.bar.lock().unwrap() = Some(pb);
            }
            ProgressEvent::ItemDone { .. } => {
                if let Some(pb) = self.bar.lock().unwrap().as_ref() {
                    pb.inc(1);
                }
            }
            ProgressEvent::DownloadFinished { .. } => {
                if let Some(pb) = self.bar.lock().unwrap().as_ref() {
                    pb.finish_with_message("下载完成");
                }
            }
            _ => {}
        }
    }
}

// ---------- legacy `xld download` (人类一键模式) ----------
//
// 行为同旧版本，向后兼容：
// - 写入 config.download_dir（默认 `./downloads`），不走平台沙箱目录
// - 沿用 config.file_format 占位符（默认 `{USERNAME} {ID}`）
// - 沿用 data/downloaded_tweet_ids.txt 跳过已处理 tweet（兼容历史记录）
// - 设置文件 mtime 到推文发布时间
// - config.auto_organize 触发归档
//
// 但底层切到了 agent::list_likes + agent::download_media，HTTP 重试 / 状态分类 /
// 断点续传 / 并发等逻辑由 lib 层统一提供。
async fn run_download_legacy() -> Result<()> {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use x_likes_downloader::agent::types::{DownloadOpts, MediaItem, NullSink};
    use x_likes_downloader::agent::{download_media as agent_download, list_likes as agent_list};

    let config = Config::load()?;

    let known_ids: HashSet<String> = load_downloaded_ids(&config.download_record);
    println!("已记录的下载ID数量: {}", known_ids.len());

    let list = match agent_list(x_likes_downloader::agent::types::ListOpts {
        all: config.all,
        since_cursor: None,
        count: config.count.parse::<u32>().ok(),
        include_raw: false,
    })
    .await
    {
        Ok(out) => out,
        Err(e) => {
            eprintln!("获取点赞 tweets 失败：{} ({:?})", e.message, e.kind);
            return Err(anyhow::anyhow!(
                "获取点赞 tweets 失败 ({:?}): {}",
                e.kind,
                e.message
            ));
        }
    };

    println!("从 API 获取到 {} 条点赞的 tweet 数据", list.tweets.len());
    let tweets_count = list.tweets.len();

    // 把未处理过的 tweet 摊平为 MediaItem[]
    let mut pending: Vec<MediaItem> = Vec::new();
    let mut tweets_with_media: HashSet<String> = HashSet::new();
    for tweet in &list.tweets {
        if known_ids.contains(&tweet.id) {
            continue;
        }
        if tweet.media.is_empty() {
            continue;
        }
        tweets_with_media.insert(tweet.id.clone());
        pending.extend(tweet.media.iter().cloned());
    }

    if pending.is_empty() {
        println!("\n=== 处理总结 ===");
        println!("总tweet数量: {}", tweets_count);
        println!("已处理数量: 0");
        println!("全部处理完成。");
        if config.auto_organize {
            println!("开始整理下载的文件目录...");
            FileOrganizer::organize_files(&config.download_dir, &config.target_dir)?;
        }
        return Ok(());
    }

    let opts = DownloadOpts {
        subdir: None,
        concurrency: 4,
        base_dir: Some(PathBuf::from(&config.download_dir)),
        filename_format: Some(config.file_format.clone()),
        set_mtime: true,
    };

    let result = agent_download(&pending, &opts, std::sync::Arc::new(NullSink))
        .await
        .map_err(|e| anyhow::anyhow!("下载失败: {} ({:?})", e.message, e.kind))?;

    // 按 tweet_id 聚合：任何一个媒体成功（或已跳过即视为完成）就记录 tweet_id
    let mut succeeded_tweets: HashSet<String> = HashSet::new();
    let mut tweets_all_failed: HashSet<String> = HashSet::new();
    for r in &result.downloads {
        match r.status {
            x_likes_downloader::agent::types::DownloadStatus::Downloaded
            | x_likes_downloader::agent::types::DownloadStatus::SkippedExisting => {
                succeeded_tweets.insert(r.tweet_id.clone());
            }
            x_likes_downloader::agent::types::DownloadStatus::Failed => {
                tweets_all_failed.insert(r.tweet_id.clone());
            }
        }
    }
    // 记录每条任何媒体都成功的 tweet
    for tid in &succeeded_tweets {
        tweets_all_failed.remove(tid);
        if !known_ids.contains(tid) {
            append_downloaded_id(&config.download_record, tid)?;
        }
    }

    println!("\n=== 处理总结 ===");
    println!("总tweet数量: {}", tweets_count);
    println!("已处理数量: {}", tweets_with_media.len());
    println!("下载成功数量: {}", succeeded_tweets.len());
    println!("下载失败数量: {}", tweets_all_failed.len());
    println!(
        "媒体文件: 已下载 {}，已跳过 {}，失败 {}",
        result.summary.downloaded, result.summary.skipped, result.summary.failed
    );
    println!("全部处理完成。");

    if config.auto_organize {
        println!("开始整理下载的文件目录...");
        FileOrganizer::organize_files(&config.download_dir, &config.target_dir)?;
    }

    Ok(())
}

fn load_downloaded_ids(filename: &str) -> std::collections::HashSet<String> {
    let path = std::path::Path::new(filename);
    if !path.exists() {
        return std::collections::HashSet::new();
    }
    std::fs::read_to_string(filename)
        .map(|content| content.lines().map(|s| s.trim().to_string()).collect())
        .unwrap_or_default()
}

fn append_downloaded_id(filename: &str, tweet_id: &str) -> Result<()> {
    use std::io::Write;
    if let Some(parent) = std::path::Path::new(filename).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(filename)?
        .write_all(format!("{}\n", tweet_id).as_bytes())?;
    Ok(())
}
