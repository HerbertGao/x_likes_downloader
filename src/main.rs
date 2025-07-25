mod config;
mod downloader;
mod organize_files;
mod setup;
mod x_api;

use anyhow::Result;
use clap::{Parser, Subcommand};

use config::Config;
use downloader::Downloader;
use organize_files::FileOrganizer;
use setup::SetupArgs;
use x_api::XApi;

#[derive(Parser)]
#[command(name = "x_likes_downloader")]
#[command(about = "X点赞推文媒体下载器")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// 初始化配置
    Setup(SetupArgs),
    /// 下载点赞的推文媒体
    Download,
    /// 整理下载的文件
    Organize {
        /// 源目录
        source_dir: Option<String>,
        /// 目标目录
        target_dir: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Setup(args) => {
            setup::run_setup(args.clone())?;
        }
        Commands::Download => {
            run_download().await?;
        }
        Commands::Organize { source_dir, target_dir } => {
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
        }
    }

    Ok(())
}

async fn run_download() -> Result<()> {
    // 加载配置
    let config = Config::load()?;
    
    // 创建API客户端
    let api = XApi::new(config.clone())?;
    
    // 创建下载器
    let mut downloader = Downloader::new(config.clone())?;

    // 获取点赞的推文
    let tweets = match api.get_liked_tweets_internal().await {
        Ok(tweets) => tweets,
        Err(e) => {
            println!("获取点赞 tweets 失败：{}", e);
            return Ok(());
        }
    };

    println!("从 API 获取到 {} 条点赞的 tweet 数据", tweets.len());
    println!("已记录的下载ID数量: {}", downloader.downloaded_count());

    let mut processed_count = 0;
    let mut download_success_count = 0;
    let mut download_failed_count = 0;

    let tweets_count = tweets.len();
    for entry in tweets {
        // 假设 tweet 数据结构中，rest_id 存储 tweet id
        let tweet_data = entry
            .get("content")
            .and_then(|c| c.get("itemContent"))
            .and_then(|ic| ic.get("tweet_results"))
            .and_then(|tr| tr.get("result"))
            .unwrap_or(&entry);

        let tweet_id = tweet_data
            .get("rest_id")
            .and_then(|id| id.as_str())
            .or_else(|| {
                tweet_data
                    .get("tweet")
                    .and_then(|t| t.get("rest_id"))
                    .and_then(|id| id.as_str())
            });

        if let Some(tweet_id) = tweet_id {
            if downloader.is_downloaded(tweet_id) {
                continue;
            }

            match downloader.call_media_downloader(tweet_data, tweet_id).await {
                Ok(Some(true)) => {
                    // 成功下载了媒体文件
                    processed_count += 1;
                    println!("处理 tweet ({}): {}", processed_count, tweet_id);
                    downloader.save_downloaded_id(tweet_id)?;
                    download_success_count += 1;
                    println!("✓ 成功下载并记录tweet ID: {}", tweet_id);
                }
                Ok(Some(false)) => {
                    // 实际尝试下载但失败了
                    processed_count += 1;
                    println!("处理 tweet ({}): {}", processed_count, tweet_id);
                    download_failed_count += 1;
                    println!("✗ 下载失败，不记录tweet ID: {}", tweet_id);
                }
                Ok(None) => {
                    // 没有媒体文件，不显示任何日志，也不计数
                }
                Err(e) => {
                    println!("处理 tweet {} 时发生错误: {}", tweet_id, e);
                }
            }
        }
    }

    println!("\n=== 处理总结 ===");
    println!("总tweet数量: {}", tweets_count);
    println!("已处理数量: {}", processed_count);
    println!("下载成功数量: {}", download_success_count);
    println!("下载失败数量: {}", download_failed_count);
    println!("全部处理完成。");

    if config.auto_organize {
        println!("开始整理下载的文件目录...");
        FileOrganizer::organize_files(&config.download_dir, &config.target_dir)?;
    }

    Ok(())
}
